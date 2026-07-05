# Host Loader (`tamal-loader` / `tamal-loader-cli`) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a host-side loader that COBS/CRC-8-frames the raw bytecode `tamal-asm` emits, sends `LOAD_PROGRAM` + `TRIGGER` over UART, reads back the `TRACE_DRAIN` frame, and decodes it into a human-readable trace + HALT/TRAP verdict.

**Architecture:** Three crates. `tamal-abi` gains the transport-agnostic wire mirror (`crc8`, `cobs`, `wire`, `trace` leaves — byte-exact copies of the HDL `Tamal.Crc`/`Tamal.Wire.Cobs`/`Tamal.Wire` and the engine trace-record encodings). `tamal-loader` adds a `Transport` trait + `UartTransport` (serialport) + a `Device<T>` session with a timeout/auto-retry `run` loop. `tamal-loader-cli` is a `clap` front-end with one `run` subcommand.

**Tech Stack:** Rust 2024, `serialport` 4.9 (UART), `thiserror` (library errors), `clap` + `color-eyre` (CLI). No new dependencies.

## Global Constraints

- Edition `2024`, `rust-version = 1.85`; crates are MIT (host tooling).
- Keep `#![forbid(unsafe_code)]` at the top of `tamal-abi/src/lib.rs` and `tamal-loader/src/lib.rs`.
- **No new dependencies** in any crate. `tamal-abi` stays on `thiserror` only.
- `tamal-abi` is **transport-agnostic**: its new modules must not reference `serialport`, `std::io`, or any transport type.
- Byte-exactness with the HDL is mandatory: the COBS 254/255 boundary is `FF ++ <254 bytes>` for exactly 254 non-zero bytes and `FF ++ <254 bytes> ++ 02 ++ <1 byte>` for 255 — **not** the classic-reference trailing `01`.
- CRC-8: poly `0x07`, init `0x00`, MSB-first, no reflection, no final XOR (eSPI/SMBus PEC).
- Wire opcodes: `LOAD_PROGRAM=0x01`, `TRIGGER=0x02`, `TRACE_DRAIN=0x81`, delimiter `0x00`. Little-endian words on the wire (`0xAABBCCDD -> [DD,CC,BB,AA]`).
- Run tests per crate with `cargo test -p <crate>`; gate with `cargo clippy --workspace --all-targets` and `cargo fmt --check`.

---

### Task 1: `tamal-abi::crc8` — CRC-8 fold

**Files:**
- Create: `crates/tamal-abi/src/crc8.rs`
- Modify: `crates/tamal-abi/src/lib.rs` (add `pub mod crc8;`)

**Interfaces:**
- Produces: `pub fn crc8_update(crc: u8, byte: u8) -> u8`, `pub fn crc8(bytes: &[u8]) -> u8`

- [ ] **Step 1: Declare the module.** In `crates/tamal-abi/src/lib.rs`, add after the existing `pub mod config;` line:

```rust
/// CRC-8 (eSPI/SMBus PEC) — a Rust mirror of the HDL `Tamal.Crc`.
pub mod crc8;
```

- [ ] **Step 2: Write the failing test.** Create `crates/tamal-abi/src/crc8.rs` with only the tests:

```rust
//! CRC-8 (poly 0x07, init 0x00, MSB-first, no reflection, no final XOR) — a Rust
//! mirror of the HDL `Tamal.Crc.crc8Update`. The residue of a message followed by
//! its CRC byte is 0x00.

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn single_byte_vector() {
        // Pinned: crc8([0x02]) = 0x0E (the TRIGGER opcode's CRC).
        assert_eq!(crc8(&[0x02]), 0x0E);
    }

    #[test]
    fn empty_is_init() {
        assert_eq!(crc8(&[]), 0x00);
    }

    proptest! {
        #[test]
        fn residue_is_zero(msg in prop::collection::vec(any::<u8>(), 0..64)) {
            let c = crc8(&msg);
            let mut with = msg.clone();
            with.push(c);
            prop_assert_eq!(crc8(&with), 0x00);
        }
    }
}
```

- [ ] **Step 3: Run the test to verify it fails.**

Run: `cargo test -p tamal-abi crc8`
Expected: FAIL — `cannot find function crc8`.

- [ ] **Step 4: Write the implementation.** Prepend the functions above the `#[cfg(test)]` module in `crates/tamal-abi/src/crc8.rs`:

```rust
/// Fold one byte into the running CRC-8, processing bit 7 down to bit 0.
pub fn crc8_update(crc: u8, byte: u8) -> u8 {
    let mut c = crc;
    for i in (0..8).rev() {
        let feedback = ((c >> 7) & 1) ^ ((byte >> i) & 1);
        c <<= 1;
        if feedback == 1 {
            c ^= 0x07;
        }
    }
    c
}

/// Fold a byte slice from the initial value 0x00.
pub fn crc8(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0u8, |c, &b| crc8_update(c, b))
}
```

- [ ] **Step 5: Run the test to verify it passes.**

Run: `cargo test -p tamal-abi crc8`
Expected: PASS (3 tests).

- [ ] **Step 6: Commit.**

```bash
git add crates/tamal-abi/src/crc8.rs crates/tamal-abi/src/lib.rs
git commit -m "feat(abi): CRC-8 fold mirroring Tamal.Crc"
```

---

### Task 2: `tamal-abi::cobs` — COBS codec

**Files:**
- Create: `crates/tamal-abi/src/cobs.rs`
- Modify: `crates/tamal-abi/src/lib.rs` (add `pub mod cobs;`)

**Interfaces:**
- Produces: `pub fn cobs_encode(data: &[u8]) -> Vec<u8>`, `pub fn cobs_decode(data: &[u8]) -> Result<Vec<u8>, CobsError>`, `pub enum CobsError { Empty, TruncatedGroup, InteriorZero }`

**Note:** The 254/255 boundary MUST match the HDL oracle exactly (verified): 254 non-zero bytes encode to `FF ++ <254>` (no trailing `01`); 255 encode to `FF ++ <254> ++ 02 ++ <1>`.

- [ ] **Step 1: Declare the module.** In `crates/tamal-abi/src/lib.rs`, add after `pub mod crc8;`:

```rust
/// COBS framing (spec §5) — a Rust mirror of the HDL `Tamal.Wire.Cobs`.
pub mod cobs;
```

- [ ] **Step 2: Write the failing tests.** Create `crates/tamal-abi/src/cobs.rs` with only the tests:

```rust
//! Consistent Overhead Byte Stuffing — a Rust mirror of the HDL `Tamal.Wire.Cobs`.
//! Output never contains 0x00 and excludes the frame delimiter (the frame layer
//! appends it). Byte-exact with the HDL, including the 254/255 group boundary.

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn golden() -> Vec<(Vec<u8>, Vec<u8>)> {
        vec![
            (vec![0x00], vec![0x01, 0x01]),
            (vec![0x11, 0x22, 0x00, 0x33], vec![0x03, 0x11, 0x22, 0x02, 0x33]),
            (vec![0x11, 0x00, 0x00, 0x00], vec![0x02, 0x11, 0x01, 0x01, 0x01]),
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
        assert!(matches!(cobs_decode(&[0x05, 0x11]), Err(CobsError::TruncatedGroup)));
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
```

- [ ] **Step 3: Run the tests to verify they fail.**

Run: `cargo test -p tamal-abi cobs`
Expected: FAIL — `cannot find function cobs_encode`.

- [ ] **Step 4: Write the implementation.** Prepend above the `#[cfg(test)]` module:

```rust
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
```

- [ ] **Step 5: Run the tests to verify they pass.**

Run: `cargo test -p tamal-abi cobs`
Expected: PASS (5 tests).

- [ ] **Step 6: Commit.**

```bash
git add crates/tamal-abi/src/cobs.rs crates/tamal-abi/src/lib.rs
git commit -m "feat(abi): COBS codec mirroring Tamal.Wire.Cobs"
```

---

### Task 3: `tamal-abi::wire` — frame + message layer

**Files:**
- Create: `crates/tamal-abi/src/wire.rs`
- Modify: `crates/tamal-abi/src/lib.rs` (replace the placeholder `control` module with `pub mod wire;`)

**Interfaces:**
- Consumes: `crc8::crc8`, `cobs::{cobs_encode, cobs_decode, CobsError}` (Tasks 1–2)
- Produces:
  - consts `OP_LOAD_PROGRAM: u8 = 0x01`, `OP_TRIGGER: u8 = 0x02`, `OP_TRACE_DRAIN: u8 = 0x81`, `DELIMITER: u8 = 0x00`
  - `pub enum ControlMsg { LoadProgram(Vec<u32>), Trigger }`
  - `pub enum WireError { BadCrc, BadCobs, UnknownOpcode(u8), WrongOpcode { expected: u8, found: u8 }, ShortFrame, BadPayloadLen }`
  - `pub fn words_to_le_bytes(&[u32]) -> Vec<u8>`, `pub fn le_bytes_to_words(&[u8]) -> Result<Vec<u32>, WireError>`
  - `pub fn frame_encode(&[u8]) -> Vec<u8>`, `pub fn frame_decode(&[u8]) -> Result<Vec<u8>, WireError>`
  - `pub fn encode_control(&ControlMsg) -> Vec<u8>`, `pub fn decode_result(&[u8]) -> Result<Vec<u32>, WireError>`

- [ ] **Step 1: Swap the placeholder module.** In `crates/tamal-abi/src/lib.rs`, delete the entire placeholder `pub mod control { ... }` block (the one whose docstring lists `LOAD_PROGRAM`, `SET_ROLE`, …) and replace it with:

```rust
/// The frame + message wire layer (spec §8) — a Rust mirror of the HDL `Tamal.Wire`.
///
/// Control plane (host → FPGA): `LOAD_PROGRAM` + `TRIGGER`. Result plane
/// (FPGA → host): the `TRACE_DRAIN` frame. Frame = `COBS(opcode ++ payload ++
/// crc8) ++ 0x00`, little-endian throughout.
pub mod wire;
```

- [ ] **Step 2: Write the failing tests.** Create `crates/tamal-abi/src/wire.rs` with only the tests:

```rust
//! Frame + message wire layer — a Rust mirror of the HDL `Tamal.Wire`.

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn le_round_trip_vector() {
        assert_eq!(words_to_le_bytes(&[0xAABB_CCDD]), vec![0xDD, 0xCC, 0xBB, 0xAA]);
        assert_eq!(le_bytes_to_words(&[0xDD, 0xCC, 0xBB, 0xAA]).unwrap(), vec![0xAABB_CCDD]);
    }

    #[test]
    fn le_bytes_len_must_be_multiple_of_four() {
        assert_eq!(le_bytes_to_words(&[0x01, 0x02, 0x03]), Err(WireError::BadPayloadLen));
    }

    #[test]
    fn trigger_encodes_to_pinned_bytes() {
        // logical [0x02, crc8([0x02])=0x0E] -> COBS [0x03,0x02,0x0E] -> +delim.
        assert_eq!(encode_control(&ControlMsg::Trigger), vec![0x03, 0x02, 0x0E, 0x00]);
    }

    #[test]
    fn load_program_round_trips_through_frame_decode() {
        let words = vec![0x0800_0064u32, 0x0000_0000u32];
        let wire = encode_control(&ControlMsg::LoadProgram(words.clone()));
        assert_eq!(*wire.last().unwrap(), 0x00, "ends in delimiter");
        assert_eq!(wire[..wire.len() - 1].iter().filter(|&&b| b == 0).count(), 0, "no interior zero");
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
        assert!(matches!(frame_decode(&bad), Err(WireError::BadCrc) | Err(WireError::BadCobs)));
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
            Err(WireError::WrongOpcode { expected: OP_TRACE_DRAIN, found: OP_TRIGGER })
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
```

- [ ] **Step 3: Run the tests to verify they fail.**

Run: `cargo test -p tamal-abi wire`
Expected: FAIL — `cannot find function encode_control` (and friends).

- [ ] **Step 4: Write the implementation.** Prepend above the `#[cfg(test)]` module:

```rust
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
```

- [ ] **Step 5: Run the tests to verify they pass.**

Run: `cargo test -p tamal-abi wire`
Expected: PASS (8 tests).

- [ ] **Step 6: Commit.**

```bash
git add crates/tamal-abi/src/wire.rs crates/tamal-abi/src/lib.rs
git commit -m "feat(abi): wire frame + control/result codec mirroring Tamal.Wire"
```

---

### Task 4: `tamal-abi::trace` — typed trace decode

**Files:**
- Create: `crates/tamal-abi/src/trace.rs`
- Modify: `crates/tamal-abi/src/lib.rs` (replace the placeholder `trace` module with `pub mod trace;`)

**Interfaces:**
- Produces:
  - `pub struct Revision { pub major: u8, pub minor: u8, pub patch: u16 }` with `pub const EXPECTED: Revision`
  - `pub enum Record { Capture { nbits: u8, byte: u8 }, Mark { label: u16, payload: u32 } }`
  - `pub enum TrapReason { None, Decode, Config, Rdsr, Illegal }`
  - `pub struct Halt { pub trap: bool, pub reason: TrapReason, pub ovf: bool, pub status: u8 }`
  - `pub struct Trace { pub revision: Revision, pub records: Vec<Record>, pub halt: Halt }`
  - `pub enum TraceError { Empty, UnknownRecordTag(u8), TruncatedMark, MissingTerminator }`
  - `pub fn decode_trace(words: &[u32]) -> Result<Trace, TraceError>`

- [ ] **Step 1: Swap the placeholder module.** In `crates/tamal-abi/src/lib.rs`, delete the entire placeholder `pub mod trace { ... }` block and replace it with:

```rust
/// Typed decode of the drained trace ring (engine §7.2 record encodings):
/// `REVISION` word, the CAPTURE/MARK record stream, and the HALT terminator.
pub mod trace;
```

- [ ] **Step 2: Write the failing tests.** Create `crates/tamal-abi/src/trace.rs` with only the tests:

```rust
//! Typed decode of the drained trace ring, mirroring the engine's `encodeRecord`
//! (design §7.2). Record tags live in bits [31:30]: `00`=CAPTURE, `10`=MARK,
//! `11`=HALT terminator.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_drain_revision_and_halt() {
        // REVISION 0.1.0, then HALT terminator (tag 11, status 0).
        let t = decode_trace(&[0x0001_0000, 0xC000_0000]).unwrap();
        assert_eq!(t.revision, Revision::EXPECTED);
        assert!(t.records.is_empty());
        assert_eq!(t.halt, Halt { trap: false, reason: TrapReason::None, ovf: false, status: 0 });
    }

    #[test]
    fn capture_and_mark_records() {
        // CAPTURE nbits=8 byte=0x5A ; MARK label=1 payload=0xDEADBEEF ; HALT.
        let words = [0x0001_0000, 0x0000_085A, 0x8000_0001, 0xDEAD_BEEF, 0xC000_0000];
        let t = decode_trace(&words).unwrap();
        assert_eq!(t.records[0], Record::Capture { nbits: 8, byte: 0x5A });
        assert_eq!(t.records[1], Record::Mark { label: 1, payload: 0xDEAD_BEEF });
        assert_eq!(t.records.len(), 2);
    }

    #[test]
    fn trap_terminator_fields() {
        // HALT tag 11, reason=1 (decode), trap=1, ovf=1, status=0x11.
        let w = (0b11u32 << 30) | (1 << 10) | (1 << 9) | (1 << 8) | 0x11;
        let t = decode_trace(&[0x0001_0000, w]).unwrap();
        assert_eq!(
            t.halt,
            Halt { trap: true, reason: TrapReason::Decode, ovf: true, status: 0x11 }
        );
    }

    #[test]
    fn error_cases() {
        assert_eq!(decode_trace(&[]), Err(TraceError::Empty));
        // No terminator after the revision word.
        assert_eq!(decode_trace(&[0x0001_0000, 0x0000_085A]), Err(TraceError::MissingTerminator));
        // MARK missing its payload word.
        assert_eq!(decode_trace(&[0x0001_0000, 0x8000_0001]), Err(TraceError::TruncatedMark));
        // Reserved record tag 01.
        assert_eq!(
            decode_trace(&[0x0001_0000, 0x4000_0000]),
            Err(TraceError::UnknownRecordTag(1))
        );
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail.**

Run: `cargo test -p tamal-abi trace`
Expected: FAIL — `cannot find function decode_trace`.

- [ ] **Step 4: Write the implementation.** Prepend above the `#[cfg(test)]` module:

```rust
/// The `REVISION` word (`word[0]`): `[major8 | minor8 | patch16]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Revision {
    /// Major version.
    pub major: u8,
    /// Minor version.
    pub minor: u8,
    /// Patch version.
    pub patch: u16,
}

impl Revision {
    /// The revision the current gateware advertises (`0x0001_0000` = 0.1.0).
    pub const EXPECTED: Revision = Revision { major: 0, minor: 1, patch: 0 };

    fn from_word(w: u32) -> Self {
        Revision {
            major: (w >> 24) as u8,
            minor: (w >> 16) as u8,
            patch: (w & 0xFFFF) as u16,
        }
    }
}

/// A single trace-ring record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Record {
    /// A sampled byte: `[00 | 18'0 | nbits(4) | byte(8)]`.
    Capture {
        /// Number of bits sampled (1..=8).
        nbits: u8,
        /// The sampled byte.
        byte: u8,
    },
    /// A program marker: `[10 | 16'0 | label(14)]` then a 32-bit payload word.
    Mark {
        /// The 14-bit label.
        label: u16,
        /// The payload word.
        payload: u32,
    },
}

/// Why the engine halted (the extended HALT terminator's `reason` field).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrapReason {
    /// No trap (clean HALT).
    None,
    /// Instruction decode error.
    Decode,
    /// Bad `SET_CONFIG` payload.
    Config,
    /// Bad `RDSR` selector.
    Rdsr,
    /// Illegal / reserved-group opcode.
    Illegal,
}

impl TrapReason {
    fn from_bits(v: u8) -> Self {
        match v {
            0 => TrapReason::None,
            1 => TrapReason::Decode,
            2 => TrapReason::Config,
            3 => TrapReason::Rdsr,
            _ => TrapReason::Illegal, // engine emits only 0..=4
        }
    }
}

/// The decoded HALT terminator: `[11 | 17'0 | reason(3) | trap(1) | ovf(1) | status(8)]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Halt {
    /// Whether this HALT was a trap.
    pub trap: bool,
    /// The trap reason (`None` for a clean HALT).
    pub reason: TrapReason,
    /// Whether the ring overflowed during the run.
    pub ovf: bool,
    /// The host-owned status byte from `HALT`.
    pub status: u8,
}

/// A fully decoded trace drain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trace {
    /// The gateware revision (`word[0]`).
    pub revision: Revision,
    /// The CAPTURE/MARK records, in order.
    pub records: Vec<Record>,
    /// The HALT terminator (verdict).
    pub halt: Halt,
}

/// Why [`decode_trace`] rejected the drained words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TraceError {
    /// No words at all (not even a REVISION).
    #[error("empty trace")]
    Empty,
    /// A record word carried an unrecognised tag.
    #[error("unknown record tag {0:#04b}")]
    UnknownRecordTag(u8),
    /// A MARK record was missing its payload word.
    #[error("truncated MARK record")]
    TruncatedMark,
    /// The word stream ended without a HALT terminator.
    #[error("missing HALT terminator")]
    MissingTerminator,
}

/// Decode drained ring words into a typed [`Trace`].
pub fn decode_trace(words: &[u32]) -> Result<Trace, TraceError> {
    if words.is_empty() {
        return Err(TraceError::Empty);
    }
    let revision = Revision::from_word(words[0]);
    let mut records = Vec::new();
    let mut i = 1;
    loop {
        if i >= words.len() {
            return Err(TraceError::MissingTerminator);
        }
        let w = words[i];
        match (w >> 30) & 0x3 {
            0b00 => {
                records.push(Record::Capture {
                    nbits: ((w >> 8) & 0xF) as u8,
                    byte: (w & 0xFF) as u8,
                });
                i += 1;
            }
            0b10 => {
                if i + 1 >= words.len() {
                    return Err(TraceError::TruncatedMark);
                }
                records.push(Record::Mark {
                    label: (w & 0x3FFF) as u16,
                    payload: words[i + 1],
                });
                i += 2;
            }
            0b11 => {
                return Ok(Trace {
                    revision,
                    records,
                    halt: Halt {
                        reason: TrapReason::from_bits(((w >> 10) & 0x7) as u8),
                        trap: (w >> 9) & 1 == 1,
                        ovf: (w >> 8) & 1 == 1,
                        status: (w & 0xFF) as u8,
                    },
                });
            }
            other => return Err(TraceError::UnknownRecordTag(other as u8)),
        }
    }
}
```

- [ ] **Step 5: Run the tests to verify they pass.**

Run: `cargo test -p tamal-abi trace`
Expected: PASS (4 tests).

- [ ] **Step 6: Verify the whole ABI crate + lint, then commit.**

Run: `cargo test -p tamal-abi && cargo clippy -p tamal-abi --all-targets`
Expected: all green, no clippy warnings.

```bash
git add crates/tamal-abi/src/trace.rs crates/tamal-abi/src/lib.rs
git commit -m "feat(abi): typed trace-ring decode (REVISION/CAPTURE/MARK/HALT)"
```

---

### Task 5: `tamal-loader::transport` — trait + UART backend

**Files:**
- Create: `crates/tamal-loader/src/transport.rs`
- Modify: `crates/tamal-loader/src/lib.rs` (replace the placeholder `transport` module with `pub mod transport;`)

**Interfaces:**
- Produces:
  - `pub trait Transport { fn send(&mut self, bytes: &[u8]) -> Result<(), TransportError>; fn read_frame(&mut self, timeout: Duration) -> Result<Vec<u8>, TransportError>; }`
  - `pub enum TransportError { Open(String), Io(std::io::Error), Timeout }`
  - `pub struct UartTransport` with `pub fn open(path: &str, baud: u32) -> Result<Self, TransportError>`
  - `pub(crate) fn push_frame_byte(acc: &mut Vec<u8>, byte: u8) -> Option<Vec<u8>>`

- [ ] **Step 1: Swap the placeholder module.** In `crates/tamal-loader/src/lib.rs`, delete the placeholder `pub mod transport { ... }` block and replace it with:

```rust
/// Pluggable link layers between host and device (UART today; FX3 later).
pub mod transport;
```

- [ ] **Step 2: Write the failing test.** Create `crates/tamal-loader/src/transport.rs` with the delimiter-accumulator test (the only pure, hardware-free unit here):

```rust
//! Pluggable transports. v1 ships a UART (`serialport`) backend; the FX3 USB
//! backend slots in later as another `Transport` impl.

use std::time::Duration;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accumulates_until_delimiter() {
        let mut acc = Vec::new();
        assert_eq!(push_frame_byte(&mut acc, 0x03), None);
        assert_eq!(push_frame_byte(&mut acc, 0x02), None);
        assert_eq!(push_frame_byte(&mut acc, 0x0E), None);
        assert_eq!(push_frame_byte(&mut acc, 0x00), Some(vec![0x03, 0x02, 0x0E, 0x00]));
        assert!(acc.is_empty(), "accumulator resets after a frame");
    }

    #[test]
    fn skips_leading_delimiters() {
        let mut acc = Vec::new();
        assert_eq!(push_frame_byte(&mut acc, 0x00), None, "leading delimiter is a resync no-op");
        assert_eq!(push_frame_byte(&mut acc, 0x00), None);
        assert_eq!(push_frame_byte(&mut acc, 0x01), None);
        assert_eq!(push_frame_byte(&mut acc, 0x00), Some(vec![0x01, 0x00]));
    }
}
```

- [ ] **Step 3: Run the test to verify it fails.**

Run: `cargo test -p tamal-loader transport`
Expected: FAIL — `cannot find function push_frame_byte`.

- [ ] **Step 4: Write the implementation.** Prepend above the `#[cfg(test)]` module (keep the `use std::time::Duration;` already present):

```rust
use std::io::{Read, Write};
use std::time::Instant;

use tamal_abi::wire::DELIMITER;

/// Why a transport operation failed.
#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    /// The port could not be opened.
    #[error("failed to open transport: {0}")]
    Open(String),
    /// An underlying I/O error.
    #[error("transport I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// No complete frame arrived within the deadline.
    #[error("timed out waiting for a frame")]
    Timeout,
}

/// A byte pipe to the rig. Backends frame their own reads (UART delimits on 0x00).
pub trait Transport {
    /// Send all bytes to the device.
    fn send(&mut self, bytes: &[u8]) -> Result<(), TransportError>;

    /// Read one frame — bytes up to and including the next `0x00` delimiter —
    /// or `Timeout` if none arrives within `timeout`.
    fn read_frame(&mut self, timeout: Duration) -> Result<Vec<u8>, TransportError>;
}

/// Feed one received byte into a frame accumulator. Returns the complete frame
/// (including the trailing delimiter) once a `0x00` closes a non-empty frame;
/// leading lone delimiters are skipped as resync anchors.
pub(crate) fn push_frame_byte(acc: &mut Vec<u8>, byte: u8) -> Option<Vec<u8>> {
    if byte == DELIMITER {
        if acc.is_empty() {
            None
        } else {
            acc.push(byte);
            Some(std::mem::take(acc))
        }
    } else {
        acc.push(byte);
        None
    }
}

/// A UART transport over a `serialport` link.
pub struct UartTransport {
    port: Box<dyn serialport::SerialPort>,
}

impl UartTransport {
    /// Open `path` at `baud` (8N1). A short per-read timeout lets `read_frame`
    /// poll its overall deadline.
    pub fn open(path: &str, baud: u32) -> Result<Self, TransportError> {
        let port = serialport::new(path, baud)
            .timeout(Duration::from_millis(50))
            .open()
            .map_err(|e| TransportError::Open(e.to_string()))?;
        Ok(Self { port })
    }
}

impl Transport for UartTransport {
    fn send(&mut self, bytes: &[u8]) -> Result<(), TransportError> {
        self.port.write_all(bytes)?;
        self.port.flush()?;
        Ok(())
    }

    fn read_frame(&mut self, timeout: Duration) -> Result<Vec<u8>, TransportError> {
        let deadline = Instant::now() + timeout;
        let mut acc = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            if Instant::now() >= deadline {
                return Err(TransportError::Timeout);
            }
            match self.port.read(&mut byte) {
                Ok(0) => continue,
                Ok(_) => {
                    if let Some(frame) = push_frame_byte(&mut acc, byte[0]) {
                        return Ok(frame);
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => continue,
                Err(e) => return Err(TransportError::Io(e)),
            }
        }
    }
}
```

- [ ] **Step 5: Run the test to verify it passes.**

Run: `cargo test -p tamal-loader transport`
Expected: PASS (2 tests).

- [ ] **Step 6: Commit.**

```bash
git add crates/tamal-loader/src/transport.rs crates/tamal-loader/src/lib.rs
git commit -m "feat(loader): Transport trait + UART backend + frame accumulator"
```

---

### Task 6: `tamal-loader::error` + `validate_program_bytes`

**Files:**
- Create: `crates/tamal-loader/src/error.rs`
- Modify: `crates/tamal-loader/src/lib.rs` (remove the placeholder `Device` struct/impl; add `error` module, `validate_program_bytes`, and re-exports)

**Interfaces:**
- Consumes: `transport::TransportError` (Task 5); `tamal_abi::wire::{WireError, le_bytes_to_words}`; `tamal_abi::trace::TraceError`
- Produces:
  - `pub enum Error { Transport(TransportError), Wire(WireError), Trace(TraceError), BadProgramLength(usize), ProgramTooLarge(usize), RetriesExhausted { attempts: u32 } }`
  - `pub const MAX_PROGRAM_WORDS: usize = 1024`
  - `pub fn validate_program_bytes(bytes: &[u8]) -> Result<Vec<u32>, Error>`

- [ ] **Step 1: Write the failing test.** Create `crates/tamal-loader/src/error.rs`:

```rust
//! The loader error taxonomy and program-input validation.

use crate::transport::TransportError;
use tamal_abi::trace::TraceError;
use tamal_abi::wire::WireError;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_multiple_of_four() {
        assert!(matches!(
            validate_program_bytes(&[0x00, 0x01, 0x02]),
            Err(Error::BadProgramLength(3))
        ));
    }

    #[test]
    fn accepts_the_1024_word_cap_and_rejects_above() {
        let ok = vec![0u8; MAX_PROGRAM_WORDS * 4];
        assert_eq!(validate_program_bytes(&ok).unwrap().len(), MAX_PROGRAM_WORDS);
        let too_big = vec![0u8; (MAX_PROGRAM_WORDS + 1) * 4];
        assert!(matches!(
            validate_program_bytes(&too_big),
            Err(Error::ProgramTooLarge(1025))
        ));
    }

    #[test]
    fn unpacks_little_endian_words() {
        assert_eq!(validate_program_bytes(&[0xDD, 0xCC, 0xBB, 0xAA]).unwrap(), vec![0xAABB_CCDD]);
    }
}
```

- [ ] **Step 2: Run the test to verify it fails.**

Run: `cargo test -p tamal-loader error`
Expected: FAIL — `cannot find function validate_program_bytes` / `Error`.

- [ ] **Step 3: Write the implementation.** Prepend above the `#[cfg(test)]` module:

```rust
/// The maximum program size the instruction store holds (HDL loader §7).
pub const MAX_PROGRAM_WORDS: usize = 1024;

/// A loader operation failure.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A transport-level failure.
    #[error(transparent)]
    Transport(#[from] TransportError),
    /// A wire-format decode failure on the drain.
    #[error("wire error: {0}")]
    Wire(#[from] WireError),
    /// A trace-decode failure on the drain.
    #[error("trace error: {0}")]
    Trace(#[from] TraceError),
    /// The program byte length was not a multiple of 4.
    #[error("program length {0} is not a multiple of 4")]
    BadProgramLength(usize),
    /// The program exceeded the instruction-store cap.
    #[error("program is {0} words (> {MAX_PROGRAM_WORDS} cap)")]
    ProgramTooLarge(usize),
    /// The run kept failing after every retry.
    #[error("no valid drain after {attempts} attempt(s)")]
    RetriesExhausted {
        /// How many attempts were made in total.
        attempts: u32,
    },
}

/// Read a raw `.bin` into little-endian words, enforcing the wire/loader limits.
pub fn validate_program_bytes(bytes: &[u8]) -> Result<Vec<u32>, Error> {
    if bytes.len() % 4 != 0 {
        return Err(Error::BadProgramLength(bytes.len()));
    }
    let words = bytes.len() / 4;
    if words > MAX_PROGRAM_WORDS {
        return Err(Error::ProgramTooLarge(words));
    }
    Ok(tamal_abi::wire::le_bytes_to_words(bytes).expect("length checked to be a multiple of 4"))
}
```

- [ ] **Step 4: Wire it into `lib.rs`.** In `crates/tamal-loader/src/lib.rs`, delete the placeholder `Device` struct and its `impl` block. Add, below the module docs:

```rust
pub mod error;

pub use error::{Error, MAX_PROGRAM_WORDS, validate_program_bytes};
```

- [ ] **Step 5: Run the test to verify it passes.**

Run: `cargo test -p tamal-loader error`
Expected: PASS (3 tests).

- [ ] **Step 6: Commit.**

```bash
git add crates/tamal-loader/src/error.rs crates/tamal-loader/src/lib.rs
git commit -m "feat(loader): Error taxonomy + program validation"
```

---

### Task 7: `tamal-loader::device` — `load_program` + `trigger`

**Files:**
- Create: `crates/tamal-loader/src/device.rs`
- Modify: `crates/tamal-loader/src/lib.rs` (add `pub mod device;` + re-exports)

**Interfaces:**
- Consumes: `transport::{Transport, TransportError}` (Task 5); `error::Error` (Task 6); `tamal_abi::wire::{ControlMsg, encode_control}`
- Produces:
  - `pub struct Device<T: Transport>` with `pub fn new(transport: T) -> Self`
  - `pub fn load_program(&mut self, words: &[u32]) -> Result<(), Error>`
  - `pub fn trigger(&mut self) -> Result<(), Error>`
  - Test-only helpers reused by Tasks 8–9: `struct MockTransport` and `fn drain_frame(words: &[u32]) -> Vec<u8>`

- [ ] **Step 1: Write the failing test.** Create `crates/tamal-loader/src/device.rs`:

```rust
//! The `Device<T>` session: frame + ship control messages, read + decode drains.

use std::time::Duration;

use tamal_abi::trace::{Trace, decode_trace};
use tamal_abi::wire::{ControlMsg, decode_result, encode_control};

use crate::error::Error;
use crate::transport::{Transport, TransportError};

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use tamal_abi::wire::{OP_TRACE_DRAIN, frame_encode, words_to_le_bytes};

    /// A scriptable in-memory "fake FPGA": records every `send`, and replays a
    /// queue of scripted `read_frame` results. Reused by Tasks 8 and 9.
    pub(super) struct MockTransport {
        pub sent: Vec<Vec<u8>>,
        pub responses: VecDeque<Result<Vec<u8>, TransportError>>,
    }

    impl MockTransport {
        pub fn new(responses: Vec<Result<Vec<u8>, TransportError>>) -> Self {
            Self { sent: Vec::new(), responses: responses.into() }
        }
    }

    impl Transport for MockTransport {
        fn send(&mut self, bytes: &[u8]) -> Result<(), TransportError> {
            self.sent.push(bytes.to_vec());
            Ok(())
        }
        fn read_frame(&mut self, _timeout: Duration) -> Result<Vec<u8>, TransportError> {
            self.responses.pop_front().unwrap_or(Err(TransportError::Timeout))
        }
    }

    /// Build a valid `TRACE_DRAIN` wire frame from ring words. Reused by Tasks 8–9.
    pub(super) fn drain_frame(words: &[u32]) -> Vec<u8> {
        let mut logical = vec![OP_TRACE_DRAIN];
        logical.extend_from_slice(&words_to_le_bytes(words));
        frame_encode(&logical)
    }

    #[test]
    fn load_and_trigger_emit_exact_wire_bytes() {
        let mut dev = Device::new(MockTransport::new(vec![]));
        let words = vec![0x0800_0064u32];
        dev.load_program(&words).unwrap();
        dev.trigger().unwrap();
        let sent = &dev.transport.sent;
        assert_eq!(sent[0], encode_control(&ControlMsg::LoadProgram(words)));
        assert_eq!(sent[1], vec![0x03, 0x02, 0x0E, 0x00]); // pinned TRIGGER frame
    }
}
```

- [ ] **Step 2: Run the test to verify it fails.**

Run: `cargo test -p tamal-loader device`
Expected: FAIL — `cannot find type Device`.

- [ ] **Step 3: Write the implementation.** Prepend above the `#[cfg(test)]` module:

```rust
/// A connected tamal rig over a transport `T`.
pub struct Device<T: Transport> {
    transport: T,
}

impl<T: Transport> Device<T> {
    /// Wrap a transport in a session.
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    /// Frame and send `LOAD_PROGRAM(words)`.
    pub fn load_program(&mut self, words: &[u32]) -> Result<(), Error> {
        let frame = encode_control(&ControlMsg::LoadProgram(words.to_vec()));
        self.transport.send(&frame)?;
        Ok(())
    }

    /// Frame and send `TRIGGER`.
    pub fn trigger(&mut self) -> Result<(), Error> {
        let frame = encode_control(&ControlMsg::Trigger);
        self.transport.send(&frame)?;
        Ok(())
    }
}
```

Note: the test reaches `dev.transport.sent`, so leave the `transport` field accessible to the module's tests (it is, since tests are a child module). Do not add a `pub` on the field.

- [ ] **Step 4: Wire it into `lib.rs`.** In `crates/tamal-loader/src/lib.rs`, add:

```rust
pub mod device;

pub use device::Device;
```

- [ ] **Step 5: Run the test to verify it passes.**

Run: `cargo test -p tamal-loader device`
Expected: PASS (1 test).

- [ ] **Step 6: Commit.**

```bash
git add crates/tamal-loader/src/device.rs crates/tamal-loader/src/lib.rs
git commit -m "feat(loader): Device load_program + trigger with MockTransport tests"
```

---

### Task 8: `tamal-loader::device` — `read_trace`

**Files:**
- Modify: `crates/tamal-loader/src/device.rs` (add the `read_trace` method + a test)

**Interfaces:**
- Consumes: `MockTransport` + `drain_frame` (defined in Task 7's test module); `decode_result`, `decode_trace`
- Produces: `pub fn read_trace(&mut self, timeout: Duration) -> Result<Trace, Error>`

- [ ] **Step 1: Write the failing test.** In `crates/tamal-loader/src/device.rs`, add inside the existing `#[cfg(test)] mod tests` block (reuses `MockTransport` and `drain_frame` from Task 7):

```rust
    #[test]
    fn read_trace_decodes_a_good_drain() {
        // REVISION 0.1.0, CAPTURE nbits=8 byte=0x5A, HALT status=0.
        let words = vec![0x0001_0000u32, 0x0000_085A, 0xC000_0000];
        let mut dev = Device::new(MockTransport::new(vec![Ok(drain_frame(&words))]));
        let trace = dev.read_trace(Duration::from_secs(1)).unwrap();
        assert_eq!(trace.revision, tamal_abi::trace::Revision::EXPECTED);
        assert_eq!(trace.records.len(), 1);
        assert_eq!(trace.halt.status, 0);
        assert!(!trace.halt.trap);
    }

    #[test]
    fn read_trace_surfaces_a_corrupt_drain_as_wire_error() {
        let words = vec![0x0001_0000u32, 0xC000_0000];
        let mut frame = drain_frame(&words);
        frame[2] ^= 0x01; // flip a payload byte -> CRC/COBS failure
        let mut dev = Device::new(MockTransport::new(vec![Ok(frame)]));
        assert!(matches!(
            dev.read_trace(Duration::from_secs(1)),
            Err(Error::Wire(_))
        ));
    }
```

- [ ] **Step 2: Run the tests to verify they fail.**

Run: `cargo test -p tamal-loader device`
Expected: FAIL — `no method named read_trace`.

- [ ] **Step 3: Write the implementation.** Add to the `impl<T: Transport> Device<T>` block in `crates/tamal-loader/src/device.rs`:

```rust
    /// Read the next frame, decode it as a `TRACE_DRAIN`, and parse the trace ring.
    pub fn read_trace(&mut self, timeout: Duration) -> Result<Trace, Error> {
        let wire = self.transport.read_frame(timeout)?;
        let words = decode_result(&wire)?;
        Ok(decode_trace(&words)?)
    }
```

- [ ] **Step 4: Run the tests to verify they pass.**

Run: `cargo test -p tamal-loader device`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit.**

```bash
git add crates/tamal-loader/src/device.rs
git commit -m "feat(loader): Device::read_trace (drain -> typed Trace)"
```

---

### Task 9: `tamal-loader::device` — `run` + `RunOptions` retry loop

**Files:**
- Modify: `crates/tamal-loader/src/device.rs` (add `RunOptions`, the `run` method, tests)
- Modify: `crates/tamal-loader/src/lib.rs` (re-export `RunOptions`)

**Interfaces:**
- Consumes: `load_program`, `trigger`, `read_trace` (Tasks 7–8); `error::Error`
- Produces:
  - `pub struct RunOptions { pub timeout: Duration, pub retries: u32 }`
  - `pub fn run(&mut self, words: &[u32], opts: RunOptions) -> Result<Trace, Error>`

- [ ] **Step 1: Write the failing tests.** In `crates/tamal-loader/src/device.rs`, add inside the `#[cfg(test)] mod tests` block:

```rust
    #[test]
    fn run_happy_path_returns_trace() {
        let words = vec![0x0001_0000u32, 0xC000_0000];
        let mut dev = Device::new(MockTransport::new(vec![Ok(drain_frame(&words))]));
        let opts = RunOptions { timeout: Duration::from_secs(1), retries: 3 };
        let trace = dev.run(&[0x0800_0064], opts).unwrap();
        assert_eq!(trace.halt.status, 0);
        // Two frames sent per attempt (LOAD + TRIGGER); one attempt here.
        assert_eq!(dev.transport.sent.len(), 2);
    }

    #[test]
    fn run_retries_on_timeout_then_succeeds() {
        let words = vec![0x0001_0000u32, 0xC000_0000];
        let mut dev = Device::new(MockTransport::new(vec![
            Err(TransportError::Timeout),
            Ok(drain_frame(&words)),
        ]));
        let opts = RunOptions { timeout: Duration::from_secs(1), retries: 3 };
        assert!(dev.run(&[0x0800_0064], opts).is_ok());
        // Re-sent LOAD + TRIGGER on the retry: 4 frames total.
        assert_eq!(dev.transport.sent.len(), 4);
    }

    #[test]
    fn run_retries_on_corrupt_drain_then_succeeds() {
        let words = vec![0x0001_0000u32, 0xC000_0000];
        let mut bad = drain_frame(&words);
        bad[2] ^= 0x01;
        let mut dev = Device::new(MockTransport::new(vec![Ok(bad), Ok(drain_frame(&words))]));
        let opts = RunOptions { timeout: Duration::from_secs(1), retries: 3 };
        assert!(dev.run(&[0x0800_0064], opts).is_ok());
    }

    #[test]
    fn run_exhausts_retries() {
        let mut dev = Device::new(MockTransport::new(vec![
            Err(TransportError::Timeout),
            Err(TransportError::Timeout),
        ]));
        let opts = RunOptions { timeout: Duration::from_secs(1), retries: 1 };
        assert!(matches!(
            dev.run(&[0x0800_0064], opts),
            Err(Error::RetriesExhausted { attempts: 2 })
        ));
    }
```

- [ ] **Step 2: Run the tests to verify they fail.**

Run: `cargo test -p tamal-loader device`
Expected: FAIL — `cannot find type RunOptions`.

- [ ] **Step 3: Write the implementation.** In `crates/tamal-loader/src/device.rs`, add the `RunOptions` struct above the `Device` struct:

```rust
/// Knobs for [`Device::run`].
#[derive(Debug, Clone, Copy)]
pub struct RunOptions {
    /// Per-drain read deadline.
    pub timeout: Duration,
    /// Extra attempts after the first (total attempts = `retries + 1`).
    pub retries: u32,
}
```

Then add the method to the `impl<T: Transport> Device<T>` block:

```rust
    /// Load, trigger, and read the drain, re-running on a timed-out or malformed
    /// drain up to `opts.retries` extra times. Re-sends BOTH `LOAD_PROGRAM` and
    /// `TRIGGER` each attempt (a partial LOAD is committed only by TRIGGER).
    /// Genuine transport I/O faults propagate immediately.
    pub fn run(&mut self, words: &[u32], opts: RunOptions) -> Result<Trace, Error> {
        let attempts = opts.retries + 1;
        for _ in 0..attempts {
            self.load_program(words)?;
            self.trigger()?;
            match self.read_trace(opts.timeout) {
                Ok(trace) => return Ok(trace),
                // Recoverable: a lost or malformed drain -> deterministic re-run.
                Err(Error::Transport(TransportError::Timeout))
                | Err(Error::Wire(_))
                | Err(Error::Trace(_)) => continue,
                // Non-recoverable (port/IO fault, etc.).
                Err(e) => return Err(e),
            }
        }
        Err(Error::RetriesExhausted { attempts })
    }
```

- [ ] **Step 4: Re-export `RunOptions`.** In `crates/tamal-loader/src/lib.rs`, change the device re-export line to:

```rust
pub use device::{Device, RunOptions};
```

- [ ] **Step 5: Run the tests to verify they pass.**

Run: `cargo test -p tamal-loader device`
Expected: PASS (7 tests).

- [ ] **Step 6: Verify the whole loader crate + lint, then commit.**

Run: `cargo test -p tamal-loader && cargo clippy -p tamal-loader --all-targets`
Expected: all green.

```bash
git add crates/tamal-loader/src/device.rs crates/tamal-loader/src/lib.rs
git commit -m "feat(loader): Device::run with timeout + auto-retry"
```

---

### Task 10: `tamal-loader-cli` — the `run` subcommand

**Files:**
- Modify: `crates/tamal-loader-cli/src/main.rs` (replace the scaffold with a `run` subcommand + pure formatting helpers)

**Interfaces:**
- Consumes: `tamal_loader::{Device, RunOptions, validate_program_bytes, transport::UartTransport}`; `tamal_abi::trace::{Trace, Record, Halt, TrapReason, Revision}`
- Produces (CLI-internal): `fn format_trace(&Trace) -> String`, `fn format_halt(&Halt) -> String`, `fn trace_exit_code(&Halt) -> u8`

- [ ] **Step 1: Write the failing test.** Replace the contents of `crates/tamal-loader-cli/src/main.rs` with the imports, a `#[cfg(test)]` block, and stubs so it compiles-then-fails:

```rust
//! `tamal-loader` — load a compiled program onto a rig, run it, and print the
//! drained trace with a HALT/TRAP verdict.

use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand};
use color_eyre::eyre::{Context, Result};

use tamal_abi::trace::{Halt, Record, Revision, Trace, TrapReason};
use tamal_loader::transport::UartTransport;
use tamal_loader::{Device, RunOptions, validate_program_bytes};

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Trace {
        Trace {
            revision: Revision::EXPECTED,
            records: vec![
                Record::Capture { nbits: 8, byte: 0x5A },
                Record::Mark { label: 1, payload: 0xDEAD_BEEF },
            ],
            halt: Halt { trap: false, reason: TrapReason::None, ovf: false, status: 0 },
        }
    }

    #[test]
    fn formats_records_and_ok_verdict() {
        let s = format_trace(&sample());
        assert!(s.contains("REVISION 0.1.0"), "{s}");
        assert!(s.contains("CAPTURE") && s.contains("byte=0x5A"), "{s}");
        assert!(s.contains("MARK") && s.contains("payload=0xDEADBEEF"), "{s}");
        assert!(s.contains("HALT  status=0x00  (ok)"), "{s}");
    }

    #[test]
    fn formats_trap_verdict() {
        let h = Halt { trap: true, reason: TrapReason::Decode, ovf: false, status: 0x11 };
        assert!(format_halt(&h).contains("TRAP  reason=decode"));
        assert_eq!(trace_exit_code(&h), 1);
        assert_eq!(trace_exit_code(&Halt { trap: false, reason: TrapReason::None, ovf: false, status: 0 }), 0);
    }
}
```

- [ ] **Step 2: Run the test to verify it fails.**

Run: `cargo test -p tamal-loader-cli`
Expected: FAIL — `cannot find function format_trace`.

- [ ] **Step 3: Write the implementation.** Add the CLI types, helpers, and `main` to `crates/tamal-loader-cli/src/main.rs` (above the `#[cfg(test)]` block):

```rust
/// Load tamal bytecode onto a rig, run it, and print the drained trace.
#[derive(Debug, Parser)]
#[command(name = "tamal-loader", version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Load a `.bin`, trigger a run, and print the drained trace + verdict.
    Run {
        /// The compiled program (`tamal-asm` `.bin`).
        program: PathBuf,
        /// The serial port (e.g. `/dev/tty.usbserial-XXXX`).
        #[arg(short, long)]
        port: String,
        /// UART baud rate.
        #[arg(long, default_value_t = 2_000_000)]
        baud: u32,
        /// Per-drain read timeout, in seconds.
        #[arg(long, default_value_t = 5)]
        timeout: u64,
        /// Extra attempts after the first on a lost/garbled drain.
        #[arg(long, default_value_t = 3)]
        retries: u32,
    },
}

fn reason_str(r: TrapReason) -> &'static str {
    match r {
        TrapReason::None => "none",
        TrapReason::Decode => "decode",
        TrapReason::Config => "config",
        TrapReason::Rdsr => "rdsr",
        TrapReason::Illegal => "illegal",
    }
}

fn format_halt(h: &Halt) -> String {
    if h.trap {
        format!(
            "TRAP  reason={}  ovf={}  status={:#04X}",
            reason_str(h.reason),
            h.ovf,
            h.status
        )
    } else {
        format!("HALT  status={:#04X}  (ok)", h.status)
    }
}

fn format_trace(t: &Trace) -> String {
    let r = &t.revision;
    let mut s = format!("REVISION {}.{}.{}\n", r.major, r.minor, r.patch);
    for (i, rec) in t.records.iter().enumerate() {
        match rec {
            Record::Capture { nbits, byte } => {
                s.push_str(&format!("[{i}] CAPTURE  nbits={nbits}  byte={byte:#04X}\n"));
            }
            Record::Mark { label, payload } => {
                s.push_str(&format!(
                    "[{i}] MARK     label={label:#06X}  payload={payload:#010X}\n"
                ));
            }
        }
    }
    s.push_str(&format_halt(&t.halt));
    s.push('\n');
    s
}

fn trace_exit_code(h: &Halt) -> u8 {
    if h.trap { 1 } else { 0 }
}

fn cmd_run(program: PathBuf, port: String, baud: u32, timeout: u64, retries: u32) -> Result<ExitCode> {
    let bytes = fs::read(&program).wrap_err_with(|| format!("reading {}", program.display()))?;
    let words = validate_program_bytes(&bytes)?;
    let transport = UartTransport::open(&port, baud)?;
    let mut device = Device::new(transport);
    let opts = RunOptions { timeout: Duration::from_secs(timeout), retries };
    let trace = device.run(&words, opts)?;
    if trace.revision != Revision::EXPECTED {
        eprintln!(
            "warning: gateware revision {}.{}.{} != expected {}.{}.{} (bitstream/CLI mismatch)",
            trace.revision.major, trace.revision.minor, trace.revision.patch,
            Revision::EXPECTED.major, Revision::EXPECTED.minor, Revision::EXPECTED.patch,
        );
    }
    print!("{}", format_trace(&trace));
    Ok(ExitCode::from(trace_exit_code(&trace.halt)))
}

fn main() -> Result<ExitCode> {
    color_eyre::install()?;
    let cli = Cli::parse();
    match cli.command {
        Command::Run { program, port, baud, timeout, retries } => {
            cmd_run(program, port, baud, timeout, retries)
        }
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass.**

Run: `cargo test -p tamal-loader-cli`
Expected: PASS (2 tests).

- [ ] **Step 5: Full-workspace verification.**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets && cargo fmt --check`
Expected: all tests pass, no clippy warnings, formatting clean.

- [ ] **Step 6: Smoke-check the CLI surface.**

Run: `cargo run -p tamal-loader-cli -- run --help`
Expected: usage text showing `<PROGRAM>`, `--port`, `--baud`, `--timeout`, `--retries`.

- [ ] **Step 7: Commit.**

```bash
git add crates/tamal-loader-cli/src/main.rs
git commit -m "feat(loader-cli): run subcommand — load, trigger, drain, print verdict"
```

---

## Manual hardware smoke test (post-merge, not CI)

With an Arty A7 flashed with the tamal bitstream and connected over USB-UART:

```bash
tamal-asm assemble examples/peripheral_io_read.s -o /tmp/prog.bin
tamal-loader run /tmp/prog.bin --port /dev/tty.usbserial-XXXX
echo "exit: $?"   # 0 = clean HALT, 1 = TRAP
```

Expect a `REVISION 0.1.0` line, the captured records, and a `HALT`/`TRAP` verdict.

