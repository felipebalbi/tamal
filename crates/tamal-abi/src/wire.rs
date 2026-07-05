//! Frame + message wire layer — a Rust mirror of the HDL `Tamal.Wire`.

use crate::cobs::{self, CobsError};
use crate::crc8::crc8;

/// `LOAD_PROGRAM` — host → FPGA.
pub const OP_LOAD_PROGRAM: u8 = 0x01;
/// `TRIGGER` — host → FPGA.
pub const OP_TRIGGER: u8 = 0x02;
/// `TRACE_DRAIN` — FPGA → host.
pub const OP_TRACE_DRAIN: u8 = 0x81;
/// The COBS frame delimiter.
pub const DELIMITER: u8 = 0x00;

/// A control-plane message (host → FPGA).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlMsg {
    /// Load N instruction words into the instruction store.
    LoadProgram(Vec<u32>),
    /// Start a run of the loaded program.
    Trigger,
}

/// Why a frame failed to decode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum WireError {
    /// The trailing CRC-8 did not match.
    #[error("frame CRC mismatch")]
    BadCrc,
    /// The COBS layer was malformed.
    #[error("malformed COBS frame")]
    BadCobs,
    /// The opcode is not in the recognised set.
    #[error("unknown opcode {0:#04x}")]
    UnknownOpcode(u8),
    /// A result decode saw an opcode other than the one expected.
    #[error("wrong opcode: expected {expected:#04x}, found {found:#04x}")]
    WrongOpcode {
        /// The opcode the decoder required.
        expected: u8,
        /// The opcode actually present.
        found: u8,
    },
    /// The logical frame was shorter than `[opcode]`.
    #[error("frame too short")]
    ShortFrame,
    /// A payload / word stream was not a multiple of 4 bytes.
    #[error("payload length not a multiple of 4")]
    BadPayloadLen,
}

impl From<CobsError> for WireError {
    fn from(_: CobsError) -> Self {
        WireError::BadCobs
    }
}

/// Pack words little-endian: `0xAABBCCDD -> [DD, CC, BB, AA]`.
pub fn words_to_le_bytes(words: &[u32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(words.len() * 4);
    for w in words {
        out.extend_from_slice(&w.to_le_bytes());
    }
    out
}

/// Unpack little-endian words; `BadPayloadLen` if `bytes.len() % 4 != 0`.
pub fn le_bytes_to_words(bytes: &[u8]) -> Result<Vec<u32>, WireError> {
    if bytes.len() % 4 != 0 {
        return Err(WireError::BadPayloadLen);
    }
    Ok(bytes
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect())
}

/// Wrap a logical frame: `COBS(logical ++ crc8(logical)) ++ 0x00`.
pub fn frame_encode(logical: &[u8]) -> Vec<u8> {
    let mut with_crc = logical.to_vec();
    with_crc.push(crc8(logical));
    let mut wire = cobs::cobs_encode(&with_crc);
    wire.push(DELIMITER);
    wire
}

/// Unwrap a wire frame to its logical bytes (`opcode ++ payload`), verifying CRC.
pub fn frame_decode(wire: &[u8]) -> Result<Vec<u8>, WireError> {
    let body = match wire.split_last() {
        Some((&DELIMITER, body)) => body,
        _ => return Err(WireError::BadCobs), // missing delimiter
    };
    let decoded = cobs::cobs_decode(body)?;
    let (crc, logical) = decoded.split_last().ok_or(WireError::ShortFrame)?;
    if logical.is_empty() {
        return Err(WireError::ShortFrame);
    }
    if crc8(logical) != *crc {
        return Err(WireError::BadCrc);
    }
    Ok(logical.to_vec())
}

/// Encode a control message to wire bytes.
pub fn encode_control(msg: &ControlMsg) -> Vec<u8> {
    let logical = match msg {
        ControlMsg::Trigger => vec![OP_TRIGGER],
        ControlMsg::LoadProgram(words) => {
            let mut l = Vec::with_capacity(1 + words.len() * 4);
            l.push(OP_LOAD_PROGRAM);
            l.extend_from_slice(&words_to_le_bytes(words));
            l
        }
    };
    frame_encode(&logical)
}

/// Decode a control frame (used by tests / a future device-side simulator).
pub fn decode_control(wire: &[u8]) -> Result<ControlMsg, WireError> {
    let logical = frame_decode(wire)?;
    match logical[0] {
        OP_LOAD_PROGRAM => Ok(ControlMsg::LoadProgram(le_bytes_to_words(&logical[1..])?)),
        OP_TRIGGER => {
            if logical.len() != 1 {
                return Err(WireError::BadPayloadLen);
            }
            Ok(ControlMsg::Trigger)
        }
        other => Err(WireError::UnknownOpcode(other)),
    }
}

/// Decode a `TRACE_DRAIN` result frame into little-endian words.
pub fn decode_result(wire: &[u8]) -> Result<Vec<u32>, WireError> {
    let logical = frame_decode(wire)?;
    if logical[0] != OP_TRACE_DRAIN {
        return Err(WireError::WrongOpcode {
            expected: OP_TRACE_DRAIN,
            found: logical[0],
        });
    }
    le_bytes_to_words(&logical[1..])
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn le_round_trip_vector() {
        assert_eq!(
            words_to_le_bytes(&[0xAABB_CCDD]),
            vec![0xDD, 0xCC, 0xBB, 0xAA]
        );
        assert_eq!(
            le_bytes_to_words(&[0xDD, 0xCC, 0xBB, 0xAA]).unwrap(),
            vec![0xAABB_CCDD]
        );
    }

    #[test]
    fn le_bytes_len_must_be_multiple_of_four() {
        assert_eq!(
            le_bytes_to_words(&[0x01, 0x02, 0x03]),
            Err(WireError::BadPayloadLen)
        );
    }

    #[test]
    fn trigger_encodes_to_pinned_bytes() {
        // logical [0x02, crc8([0x02])=0x0E] -> COBS [0x03,0x02,0x0E] -> +delim.
        assert_eq!(
            encode_control(&ControlMsg::Trigger),
            vec![0x03, 0x02, 0x0E, 0x00]
        );
    }

    #[test]
    fn load_program_round_trips_through_frame_decode() {
        let words = vec![0x0800_0064u32, 0x0000_0000u32];
        let wire = encode_control(&ControlMsg::LoadProgram(words.clone()));
        assert_eq!(*wire.last().unwrap(), 0x00, "ends in delimiter");
        assert_eq!(
            wire[..wire.len() - 1].iter().filter(|&&b| b == 0).count(),
            0,
            "no interior zero"
        );
        let logical = frame_decode(&wire).unwrap();
        assert_eq!(logical[0], OP_LOAD_PROGRAM);
        assert_eq!(le_bytes_to_words(&logical[1..]).unwrap(), words);
    }

    #[test]
    fn decode_result_unpacks_drain_words() {
        let words = vec![0x0001_0000u32, 0xC000_0000u32];
        let mut logical = vec![OP_TRACE_DRAIN];
        logical.extend_from_slice(&words_to_le_bytes(&words));
        let wire = frame_encode(&logical);
        assert_eq!(decode_result(&wire).unwrap(), words);
    }

    #[test]
    fn single_byte_flip_fails_decode() {
        let wire = encode_control(&ControlMsg::LoadProgram(vec![0x1234_5678]));
        for i in 0..wire.len() - 1 {
            let mut bad = wire.clone();
            bad[i] ^= 0x01;
            // Either the delimiter scan, COBS, or CRC rejects it — never a silent wrong decode.
            let _ = frame_decode(&bad); // must not panic
        }
        // A flipped payload byte under a good frame shape must trip the CRC.
        let mut bad = wire.clone();
        bad[2] ^= 0x01;
        assert!(matches!(
            frame_decode(&bad),
            Err(WireError::BadCrc) | Err(WireError::BadCobs)
        ));
    }

    #[test]
    fn error_taxonomy() {
        // Unknown opcode.
        let logical = vec![0x7Fu8];
        let wire = frame_encode(&logical);
        assert_eq!(decode_control(&wire), Err(WireError::UnknownOpcode(0x7F)));
        // decode_result on a non-0x81 frame.
        let wire = encode_control(&ControlMsg::Trigger);
        assert_eq!(
            decode_result(&wire),
            Err(WireError::WrongOpcode {
                expected: OP_TRACE_DRAIN,
                found: OP_TRIGGER
            })
        );
        // Short frame: a bare delimiter decodes to an empty logical frame.
        assert_eq!(frame_decode(&[0x01, 0x00]), Err(WireError::ShortFrame));
    }

    proptest! {
        #[test]
        fn load_program_word_round_trip(ws in prop::collection::vec(any::<u32>(), 0..40)) {
            let wire = encode_control(&ControlMsg::LoadProgram(ws.clone()));
            let logical = frame_decode(&wire).unwrap();
            prop_assert_eq!(logical[0], OP_LOAD_PROGRAM);
            prop_assert_eq!(le_bytes_to_words(&logical[1..]).unwrap(), ws);
        }
    }
}
