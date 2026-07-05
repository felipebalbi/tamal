//! Consistent Overhead Byte Stuffing — a Rust mirror of the HDL `Tamal.Wire.Cobs`.
//! Output never contains 0x00 and excludes the frame delimiter (the frame layer
//! appends it). Byte-exact with the HDL, including the 254/255 group boundary.

/// Why [`cobs_decode`] rejected its input. The frame layer lifts these to
/// `WireError::BadCobs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CobsError {
    /// The input was empty (a frame must have at least one code byte).
    #[error("empty COBS input")]
    Empty,
    /// A code byte demanded more data bytes than remain.
    #[error("truncated COBS group")]
    TruncatedGroup,
    /// A literal 0x00 appeared inside the COBS data (illegal).
    #[error("interior zero in COBS data")]
    InteriorZero,
}

/// COBS-encode. The result never contains 0x00 and does NOT include the frame
/// delimiter. `cobs_encode(&[]) == [0x01]`.
pub fn cobs_encode(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + data.len() / 254 + 2);
    let mut group: Vec<u8> = Vec::with_capacity(254);
    let n = data.len();
    for (i, &b) in data.iter().enumerate() {
        let last = i + 1 == n;
        if b == 0 {
            out.push(group.len() as u8 + 1);
            out.extend_from_slice(&group);
            group.clear();
            if last {
                out.push(1); // trailing zero owes a final empty group
            }
        } else {
            group.push(b);
            if group.len() == 254 {
                // Full group: flush EAGERLY as a 0xFF continuation (no implied
                // zero), then start a fresh group. Flushing here — not lazily on
                // the next byte — is load-bearing: a 0x00 arriving on a full group
                // must terminate a *fresh* empty group (…FF,<254>,01,…), never fold
                // into the full group (0xFF carries no implied zero) and be dropped.
                out.push(0xFF);
                out.extend_from_slice(&group);
                group.clear();
            } else if last {
                out.push(group.len() as u8 + 1);
                out.extend_from_slice(&group);
                group.clear();
            }
        }
    }
    if n == 0 {
        out.push(1); // empty input -> a single empty final group
    }
    out
}

/// COBS-decode a delimiter-stripped buffer, or a classified error.
pub fn cobs_decode(data: &[u8]) -> Result<Vec<u8>, CobsError> {
    if data.is_empty() {
        return Err(CobsError::Empty);
    }
    let mut out = Vec::with_capacity(data.len());
    let mut i = 0;
    while i < data.len() {
        let code = data[i];
        if code == 0 {
            return Err(CobsError::InteriorZero);
        }
        i += 1;
        let ndata = code as usize - 1;
        if i + ndata > data.len() {
            return Err(CobsError::TruncatedGroup);
        }
        for _ in 0..ndata {
            if data[i] == 0 {
                return Err(CobsError::InteriorZero);
            }
            out.push(data[i]);
            i += 1;
        }
        // inject one zero at a group end when the group was not full and input remains
        if code != 0xFF && i < data.len() {
            out.push(0);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn golden() -> Vec<(Vec<u8>, Vec<u8>)> {
        vec![
            (vec![0x00], vec![0x01, 0x01]),
            (
                vec![0x11, 0x22, 0x00, 0x33],
                vec![0x03, 0x11, 0x22, 0x02, 0x33],
            ),
            (
                vec![0x11, 0x00, 0x00, 0x00],
                vec![0x02, 0x11, 0x01, 0x01, 0x01],
            ),
            (vec![], vec![0x01]),
        ]
    }

    #[test]
    fn golden_vectors_encode_and_round_trip() {
        for (raw, enc) in golden() {
            assert_eq!(cobs_encode(&raw), enc, "encode {raw:02x?}");
            assert_eq!(cobs_decode(&enc).unwrap(), raw, "decode {enc:02x?}");
        }
    }

    #[test]
    fn boundary_254_is_single_full_group() {
        let raw: Vec<u8> = (1..=254u16).map(|x| x as u8).collect();
        let mut exp = vec![0xFFu8];
        exp.extend_from_slice(&raw);
        assert_eq!(cobs_encode(&raw), exp);
        assert_eq!(cobs_decode(&exp).unwrap(), raw);
    }

    #[test]
    fn boundary_255_full_group_then_singleton() {
        let raw: Vec<u8> = (1..=255u16).map(|x| x as u8).collect();
        let mut exp = vec![0xFFu8];
        exp.extend_from_slice(&raw[..254]);
        exp.push(0x02);
        exp.push(raw[254]);
        assert_eq!(cobs_encode(&raw), exp);
        assert_eq!(cobs_decode(&exp).unwrap(), raw);
    }

    #[test]
    fn boundary_254_then_zero_keeps_the_zero() {
        // A full 254-byte group terminated by a zero must flush as a 0xFF
        // continuation and then emit a SEPARATE empty group for the zero
        // (FF,<254>,01,01) — folding the zero into the 0xFF group would drop it.
        // (Regression: this exact case was a shared bug in the plan + HDL oracle.)
        let mut raw: Vec<u8> = (1..=254u16).map(|x| x as u8).collect();
        raw.push(0x00);
        let mut exp: Vec<u8> = vec![0xFFu8];
        exp.extend_from_slice(&raw[..254]);
        exp.push(0x01);
        exp.push(0x01);
        assert_eq!(cobs_encode(&raw), exp);
        assert_eq!(cobs_decode(&exp).unwrap(), raw);
    }

    #[test]
    fn malformed_inputs_classified() {
        assert!(matches!(cobs_decode(&[]), Err(CobsError::Empty)));
        assert!(matches!(
            cobs_decode(&[0x05, 0x11]),
            Err(CobsError::TruncatedGroup)
        ));
        assert!(matches!(
            cobs_decode(&[0x02, 0x11, 0x00, 0x03]),
            Err(CobsError::InteriorZero)
        ));
    }

    proptest! {
        #[test]
        fn round_trips_and_never_emits_zero(raw in prop::collection::vec(any::<u8>(), 0..600)) {
            let enc = cobs_encode(&raw);
            prop_assert!(!enc.contains(&0x00));
            prop_assert_eq!(cobs_decode(&enc).unwrap(), raw);
        }
    }
}
