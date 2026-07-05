//! CRC-8 (poly 0x07, init 0x00, MSB-first, no reflection, no final XOR) — a Rust
//! mirror of the HDL `Tamal.Crc.crc8Update`. The residue of a message followed by
//! its CRC byte is 0x00.

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
