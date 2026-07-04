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

#[cfg(test)]
mod tests {
    use super::*;

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
}
