//! The loader error taxonomy and program-input validation.

use crate::transport::TransportError;
use tamal_abi::trace::TraceError;
use tamal_abi::wire::WireError;

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
        assert_eq!(
            validate_program_bytes(&ok).unwrap().len(),
            MAX_PROGRAM_WORDS
        );
        let too_big = vec![0u8; (MAX_PROGRAM_WORDS + 1) * 4];
        assert!(matches!(
            validate_program_bytes(&too_big),
            Err(Error::ProgramTooLarge(1025))
        ));
    }

    #[test]
    fn unpacks_little_endian_words() {
        assert_eq!(
            validate_program_bytes(&[0xDD, 0xCC, 0xBB, 0xAA]).unwrap(),
            vec![0xAABB_CCDD]
        );
    }
}
