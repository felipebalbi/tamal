//! Typed decode of the drained trace ring, mirroring the engine's `encodeRecord`
//! (design §7.2). Record tags live in bits `[31:30]`: `00`=CAPTURE, `10`=MARK,
//! `11`=HALT terminator.

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
    pub const EXPECTED: Revision = Revision {
        major: 0,
        minor: 1,
        patch: 0,
    };

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_drain_revision_and_halt() {
        // REVISION 0.1.0, then HALT terminator (tag 11, status 0).
        let t = decode_trace(&[0x0001_0000, 0xC000_0000]).unwrap();
        assert_eq!(t.revision, Revision::EXPECTED);
        assert!(t.records.is_empty());
        assert_eq!(
            t.halt,
            Halt {
                trap: false,
                reason: TrapReason::None,
                ovf: false,
                status: 0
            }
        );
    }

    #[test]
    fn capture_and_mark_records() {
        // CAPTURE nbits=8 byte=0x5A ; MARK label=1 payload=0xDEADBEEF ; HALT.
        let words = [
            0x0001_0000,
            0x0000_085A,
            0x8000_0001,
            0xDEAD_BEEF,
            0xC000_0000,
        ];
        let t = decode_trace(&words).unwrap();
        assert_eq!(
            t.records[0],
            Record::Capture {
                nbits: 8,
                byte: 0x5A
            }
        );
        assert_eq!(
            t.records[1],
            Record::Mark {
                label: 1,
                payload: 0xDEAD_BEEF
            }
        );
        assert_eq!(t.records.len(), 2);
    }

    #[test]
    fn trap_terminator_fields() {
        // HALT tag 11, reason=1 (decode), trap=1, ovf=1, status=0x11.
        let w = (0b11u32 << 30) | (1 << 10) | (1 << 9) | (1 << 8) | 0x11;
        let t = decode_trace(&[0x0001_0000, w]).unwrap();
        assert_eq!(
            t.halt,
            Halt {
                trap: true,
                reason: TrapReason::Decode,
                ovf: true,
                status: 0x11
            }
        );
    }

    #[test]
    fn error_cases() {
        assert_eq!(decode_trace(&[]), Err(TraceError::Empty));
        // No terminator after the revision word.
        assert_eq!(
            decode_trace(&[0x0001_0000, 0x0000_085A]),
            Err(TraceError::MissingTerminator)
        );
        // MARK missing its payload word.
        assert_eq!(
            decode_trace(&[0x0001_0000, 0x8000_0001]),
            Err(TraceError::TruncatedMark)
        );
        // Reserved record tag 01.
        assert_eq!(
            decode_trace(&[0x0001_0000, 0x4000_0000]),
            Err(TraceError::UnknownRecordTag(1))
        );
    }
}
