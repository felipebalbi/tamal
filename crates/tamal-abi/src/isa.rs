//! Rust mirror of the HDL `Tamal.Isa` instruction encoding.

/// Generate a width-checked operand newtype over `$repr` holding `$bits` bits.
///
/// `new` validates the range and returns `Option`; `from_bits` masks (infallible,
/// for `decode`, whose fields are already width-limited); `bits` reads the raw
/// value. Kept crate-private constructors (`from_bits`) so `decode` and `config`
/// can build values without re-checking.
macro_rules! bounded {
    ($(#[$doc:meta])* $name:ident, $repr:ty, $bits:expr) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name($repr);

        impl $name {
            /// The largest representable value (`2^BITS - 1`).
            pub const MAX: $repr = (1 << $bits) - 1;

            /// Construct if `v` fits in the field width, else `None`.
            pub const fn new(v: $repr) -> Option<Self> {
                if v <= Self::MAX { Some(Self(v)) } else { None }
            }

            /// The raw field value.
            pub const fn bits(self) -> $repr {
                self.0
            }

            /// Construct from an already width-limited field value (masks defensively).
            #[allow(dead_code)] // consumed by `decode`/`config` in later work
            pub(crate) const fn from_bits(v: $repr) -> Self {
                Self(v & Self::MAX)
            }
        }
    };
}

bounded!(
    /// A 5-bit register selector, `x0..=x31`. `decode` does not window this to
    /// `x0..=x15`; that is an assembler/engine concern.
    Reg, u8, 5
);
bounded!(/// An 11-bit immediate / branch offset / label field.
    Imm11, u16, 11);
bounded!(/// A 20-bit `LUI` immediate.
    Imm20, u32, 20);
bounded!(/// A 4-bit `TAR` turnaround count.
    Tar4, u8, 4);
bounded!(/// A 6-bit `SET_CONFIG` payload.
    Cfg6, u8, 6);
bounded!(/// A 5-bit `RDSR` special-register selector.
    Sr5, u8, 5);
bounded!(/// A 5-bit shift amount.
    Amt5, u8, 5);
bounded!(/// A 2-bit `WAIT_ON` condition selector.
    WaitCond, u8, 2);
bounded!(/// A 9-bit `WAIT_ON` timeout.
    WaitTimeout, u16, 9);

/// A `PUT_BITS`/`GET_BITS` bit count, `1..=8`. Stored as `n-1` (a 3-bit field)
/// to match the HDL `Index 8`, but constructed and read as the semantic count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BitCount(u8);

impl BitCount {
    /// Construct from a count in `1..=8`, else `None`.
    ///
    /// (Not `const`: the idiomatic `(1..=8).contains(&count)` is not a `const fn`,
    /// and a hand-written two-sided comparison trips `clippy::manual_range_contains`.)
    pub fn new(count: u8) -> Option<Self> {
        if (1..=8).contains(&count) {
            Some(Self(count - 1))
        } else {
            None
        }
    }

    /// The semantic count, `1..=8`.
    pub const fn count(self) -> u8 {
        self.0 + 1
    }

    /// The stored `n-1` field value, `0..=7`.
    #[allow(dead_code)] // consumed by encode/decode in later work
    pub(crate) const fn stored(self) -> u8 {
        self.0
    }

    /// Construct from a stored `n-1` field (masks to 3 bits).
    #[allow(dead_code)] // consumed by encode/decode in later work
    pub(crate) const fn from_stored(s: u8) -> Self {
        Self(s & 0x7)
    }
}

/// A `SHIFT` operation. The reserved `0b11` encoding is unrepresentable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShiftOp {
    /// Logical shift left (`0b00`).
    Sll,
    /// Logical shift right (`0b01`).
    Srl,
    /// Arithmetic shift right (`0b10`).
    Sra,
}

impl ShiftOp {
    /// The 2-bit field encoding.
    #[allow(dead_code)] // consumed by encode/decode in later work
    pub(crate) const fn bits(self) -> u8 {
        match self {
            ShiftOp::Sll => 0b00,
            ShiftOp::Srl => 0b01,
            ShiftOp::Sra => 0b10,
        }
    }

    /// Decode a 2-bit field; the reserved `0b11` yields `None`.
    #[allow(dead_code)] // consumed by encode/decode in later work
    pub(crate) const fn from_bits(v: u8) -> Option<Self> {
        match v & 0x3 {
            0b00 => Some(ShiftOp::Sll),
            0b01 => Some(ShiftOp::Srl),
            0b10 => Some(ShiftOp::Sra),
            _ => None,
        }
    }
}

/// The six raw instruction fields, before per-opcode interpretation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // consumed by encode/decode in later work
pub(crate) struct Fields {
    pub(crate) group: u8, // [31:30]
    pub(crate) sub: u8,   // [29:26]
    pub(crate) rd: u8,    // [25:21]
    pub(crate) rs1: u8,   // [20:16]
    pub(crate) rs2: u8,   // [15:11]
    pub(crate) imm: u16,  // [10:0]
}

/// Split a 32-bit word into its raw fields (the Rust analog of `bitCoerce`).
#[allow(dead_code)] // consumed by encode/decode in later work
pub(crate) fn split_word(w: u32) -> Fields {
    Fields {
        group: ((w >> 30) & 0x3) as u8,
        sub: ((w >> 26) & 0xF) as u8,
        rd: ((w >> 21) & 0x1F) as u8,
        rs1: ((w >> 16) & 0x1F) as u8,
        rs2: ((w >> 11) & 0x1F) as u8,
        imm: (w & 0x7FF) as u16,
    }
}

/// Join raw fields into a 32-bit word (masking each to its width).
#[allow(dead_code)] // consumed by encode/decode in later work
pub(crate) fn join_word(group: u8, sub: u8, rd: u8, rs1: u8, rs2: u8, imm: u16) -> u32 {
    ((group as u32 & 0x3) << 30)
        | ((sub as u32 & 0xF) << 26)
        | ((rd as u32 & 0x1F) << 21)
        | ((rs1 as u32 & 0x1F) << 16)
        | ((rs2 as u32 & 0x1F) << 11)
        | (imm as u32 & 0x7FF)
}

// --- Sub-field packers (imm interpretation per opcode). ---

/// `PUT_BITS`/`GET_BITS`: `imm[10:8] = n-1`, `imm[7:0] = byte`.
#[allow(dead_code)] // consumed by encode/decode in later work
pub(crate) fn pack_bits_imm(n_minus_1: u8, byte: u8) -> u16 {
    ((n_minus_1 as u16 & 0x7) << 8) | byte as u16
}

/// Inverse of [`pack_bits_imm`].
#[allow(dead_code)] // consumed by encode/decode in later work
pub(crate) fn unpack_bits_imm(imm: u16) -> (u8, u8) {
    (((imm >> 8) & 0x7) as u8, (imm & 0xFF) as u8)
}

/// `LUI`: spread a 20-bit immediate across `rs1 ++ rs2 ++ imm` (bit 20 = 0).
#[allow(dead_code)] // consumed by encode/decode in later work
pub(crate) fn split_imm20(i20: u32) -> (u8, u8, u16) {
    (
        ((i20 >> 16) & 0x1F) as u8,
        ((i20 >> 11) & 0x1F) as u8,
        (i20 & 0x7FF) as u16,
    )
}

/// Inverse of [`split_imm20`]; `hi` is the reserved bit 20.
#[allow(dead_code)] // consumed by encode/decode in later work
pub(crate) fn join_imm20(rs1: u8, rs2: u8, imm: u16) -> (u8, u32) {
    let temp = ((rs1 as u32 & 0x1F) << 16) | ((rs2 as u32 & 0x1F) << 11) | (imm as u32 & 0x7FF);
    (((temp >> 20) & 0x1) as u8, temp & 0xF_FFFF)
}

/// `WAIT_ON`: `imm = cond[10:9] ++ timeout[8:0]`.
#[allow(dead_code)] // consumed by encode/decode in later work
pub(crate) fn wait_pack(cond: u8, timeout: u16) -> u16 {
    ((cond as u16 & 0x3) << 9) | (timeout & 0x1FF)
}

/// Inverse of [`wait_pack`].
#[allow(dead_code)] // consumed by encode/decode in later work
pub(crate) fn wait_unpack(imm: u16) -> (u8, u16) {
    (((imm >> 9) & 0x3) as u8, imm & 0x1FF)
}

/// `SHIFT`: `imm = op[10:9] ++ 0[8:5] ++ amt[4:0]` (mid nibble reserved-zero).
#[allow(dead_code)] // consumed by encode/decode in later work
pub(crate) fn shift_pack(op: u8, amt: u8) -> u16 {
    ((op as u16 & 0x3) << 9) | (amt as u16 & 0x1F)
}

/// Inverse of [`shift_pack`]; returns `(op, mid, amt)` so `decode` can check `mid == 0`.
#[allow(dead_code)] // consumed by encode/decode in later work
pub(crate) fn shift_unpack(imm: u16) -> (u8, u8, u8) {
    (
        ((imm >> 9) & 0x3) as u8,
        ((imm >> 5) & 0xF) as u8,
        (imm & 0x1F) as u8,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn newtype_new_accepts_in_range_rejects_out_of_range() {
        assert_eq!(Reg::new(31).map(|r| r.bits()), Some(31));
        assert_eq!(Reg::new(32), None);
        assert_eq!(Imm11::new(0x7FF).map(|i| i.bits()), Some(0x7FF));
        assert_eq!(Imm11::new(0x800), None);
        assert_eq!(Imm20::new(0xF_FFFF).map(|i| i.bits()), Some(0xF_FFFF));
        assert_eq!(Imm20::new(0x10_0000), None);
        assert_eq!(Tar4::new(15).map(|t| t.bits()), Some(15));
        assert_eq!(Tar4::new(16), None);
        assert_eq!(Cfg6::new(0x3F).map(|c| c.bits()), Some(0x3F));
        assert_eq!(Cfg6::new(0x40), None);
        assert_eq!(WaitTimeout::new(0x1FF).map(|t| t.bits()), Some(0x1FF));
        assert_eq!(WaitTimeout::new(0x200), None);
    }

    #[test]
    fn from_bits_round_trips_masked_values() {
        assert_eq!(Reg::from_bits(5).bits(), 5);
        assert_eq!(Amt5::from_bits(0x1F).bits(), 0x1F);
        assert_eq!(WaitCond::from_bits(0x3).bits(), 0x3);
        assert_eq!(Sr5::from_bits(0).bits(), 0);
    }

    #[test]
    fn bit_count_is_one_to_eight_stored_as_n_minus_one() {
        assert_eq!(BitCount::new(0), None);
        assert_eq!(BitCount::new(9), None);
        let one = BitCount::new(1).unwrap();
        let eight = BitCount::new(8).unwrap();
        assert_eq!(one.count(), 1);
        assert_eq!(one.stored(), 0);
        assert_eq!(eight.count(), 8);
        assert_eq!(eight.stored(), 7);
        assert_eq!(BitCount::from_stored(7).count(), 8);
    }

    #[test]
    fn shift_op_maps_to_two_bits_and_rejects_reserved() {
        assert_eq!(ShiftOp::Sll.bits(), 0b00);
        assert_eq!(ShiftOp::Srl.bits(), 0b01);
        assert_eq!(ShiftOp::Sra.bits(), 0b10);
        assert_eq!(ShiftOp::from_bits(0b00), Some(ShiftOp::Sll));
        assert_eq!(ShiftOp::from_bits(0b01), Some(ShiftOp::Srl));
        assert_eq!(ShiftOp::from_bits(0b10), Some(ShiftOp::Sra));
        assert_eq!(ShiftOp::from_bits(0b11), None);
    }

    #[test]
    fn split_word_golden_bit_positions() {
        // group=0b10, sub=0xC, rd=0x1F, rs1=0x15, rs2=0x0A, imm=0x555
        let w = (0b10 << 30) | (0xC << 26) | (0x1F << 21) | (0x15 << 16) | (0x0A << 11) | 0x555;
        let f = split_word(w);
        assert_eq!(f.group, 0b10);
        assert_eq!(f.sub, 0xC);
        assert_eq!(f.rd, 0x1F);
        assert_eq!(f.rs1, 0x15);
        assert_eq!(f.rs2, 0x0A);
        assert_eq!(f.imm, 0x555);
    }

    #[test]
    fn bits_imm_layout() {
        // n-1 in imm[10:8], byte in imm[7:0]
        assert_eq!(pack_bits_imm(0x7, 0xAB), 0x7AB);
        assert_eq!(unpack_bits_imm(0x7AB), (0x7, 0xAB));
    }

    #[test]
    fn imm20_spread_reconstructs() {
        // i20 = 0x12345 -> rs1=0x01, rs2=0x04, imm=0x345
        let (rs1, rs2, imm) = split_imm20(0x12345);
        assert_eq!((rs1, rs2, imm), (0x01, 0x04, 0x345));
        let (hi, i20) = join_imm20(rs1, rs2, imm);
        assert_eq!((hi, i20), (0, 0x12345));
    }

    #[test]
    fn imm20_join_reports_reserved_bit20() {
        // rs1 bit4 set -> hi = 1 (the reserved bit)
        let (hi, _) = join_imm20(0b1_0000, 0, 0);
        assert_eq!(hi, 1);
    }

    #[test]
    fn wait_layout() {
        // cond in imm[10:9], timeout in imm[8:0]
        assert_eq!(wait_pack(0b1, 0x64), 0x264);
        assert_eq!(wait_unpack(0x264), (0b1, 0x64));
    }

    #[test]
    fn shift_layout() {
        // op in imm[10:9], reserved mid imm[8:5], amt imm[4:0]
        assert_eq!(shift_pack(0b10, 0x03), 0x403);
        assert_eq!(shift_unpack(0x403), (0b10, 0, 0x03));
        // a set mid bit is surfaced for the reserved-field check
        assert_eq!(shift_unpack(0x020), (0, 0b0001, 0));
    }

    proptest! {
        #[test]
        fn join_split_round_trip(w in any::<u32>()) {
            let f = split_word(w);
            prop_assert_eq!(join_word(f.group, f.sub, f.rd, f.rs1, f.rs2, f.imm), w);
        }

        #[test]
        fn imm20_round_trip(i20 in 0u32..=0xF_FFFF) {
            let (rs1, rs2, imm) = split_imm20(i20);
            let (hi, back) = join_imm20(rs1, rs2, imm);
            prop_assert_eq!(hi, 0);
            prop_assert_eq!(back, i20);
        }

        #[test]
        fn wait_round_trip(cond in 0u8..=0x3, timeout in 0u16..=0x1FF) {
            prop_assert_eq!(wait_unpack(wait_pack(cond, timeout)), (cond, timeout));
        }
    }
}
