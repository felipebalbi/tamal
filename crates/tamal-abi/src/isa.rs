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
    pub(crate) const fn stored(self) -> u8 {
        self.0
    }

    /// Construct from a stored `n-1` field (masks to 3 bits).
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
    pub(crate) const fn bits(self) -> u8 {
        match self {
            ShiftOp::Sll => 0b00,
            ShiftOp::Srl => 0b01,
            ShiftOp::Sra => 0b10,
        }
    }

    /// Decode a 2-bit field; the reserved `0b11` yields `None`.
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
pub(crate) struct Fields {
    pub(crate) group: u8, // [31:30]
    pub(crate) sub: u8,   // [29:26]
    pub(crate) rd: u8,    // [25:21]
    pub(crate) rs1: u8,   // [20:16]
    pub(crate) rs2: u8,   // [15:11]
    pub(crate) imm: u16,  // [10:0]
}

/// Split a 32-bit word into its raw fields (the Rust analog of `bitCoerce`).
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
pub(crate) fn pack_bits_imm(n_minus_1: u8, byte: u8) -> u16 {
    ((n_minus_1 as u16 & 0x7) << 8) | byte as u16
}

/// Inverse of [`pack_bits_imm`].
pub(crate) fn unpack_bits_imm(imm: u16) -> (u8, u8) {
    (((imm >> 8) & 0x7) as u8, (imm & 0xFF) as u8)
}

/// `LUI`: spread a 20-bit immediate across `rs1 ++ rs2 ++ imm` (bit 20 = 0).
pub(crate) fn split_imm20(i20: u32) -> (u8, u8, u16) {
    (
        ((i20 >> 16) & 0x1F) as u8,
        ((i20 >> 11) & 0x1F) as u8,
        (i20 & 0x7FF) as u16,
    )
}

/// Inverse of [`split_imm20`]; `hi` is the reserved bit 20.
pub(crate) fn join_imm20(rs1: u8, rs2: u8, imm: u16) -> (u8, u32) {
    let temp = ((rs1 as u32 & 0x1F) << 16) | ((rs2 as u32 & 0x1F) << 11) | (imm as u32 & 0x7FF);
    (((temp >> 20) & 0x1) as u8, temp & 0xF_FFFF)
}

/// `WAIT_ON`: `imm = cond[10:9] ++ timeout[8:0]`.
pub(crate) fn wait_pack(cond: u8, timeout: u16) -> u16 {
    ((cond as u16 & 0x3) << 9) | (timeout & 0x1FF)
}

/// Inverse of [`wait_pack`].
pub(crate) fn wait_unpack(imm: u16) -> (u8, u16) {
    (((imm >> 9) & 0x3) as u8, imm & 0x1FF)
}

/// `SHIFT`: `imm = op[10:9] ++ 0[8:5] ++ amt[4:0]` (mid nibble reserved-zero).
pub(crate) fn shift_pack(op: u8, amt: u8) -> u16 {
    ((op as u16 & 0x3) << 9) | (amt as u16 & 0x1F)
}

/// Inverse of [`shift_pack`]; returns `(op, mid, amt)` so `decode` can check `mid == 0`.
pub(crate) fn shift_unpack(imm: u16) -> (u8, u8, u8) {
    (
        ((imm >> 9) & 0x3) as u8,
        ((imm >> 5) & 0xF) as u8,
        (imm & 0x1F) as u8,
    )
}

/// A decoded tamal instruction — one variant per opcode across the three groups
/// (BUS `00` / CTRL `01` / DATA `10`). A 1:1 mirror of the HDL `Tamal.Isa.Instr`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Instr {
    // BUS group (00)
    /// Assert `CS#` (begin a frame).
    CsAssert,
    /// Deassert `CS#` (end a frame).
    CsDeassert,
    /// Shift an 8-bit immediate onto the bus, MSB-first.
    PutByteImm(u8),
    /// Shift `rs1[7:0]` onto the bus.
    PutByteReg(Reg),
    /// Sample a byte into `rd` (updates the RX CRC).
    GetByte(Reg),
    /// Shift `n` bits of an immediate byte.
    PutBitsImm(BitCount, u8),
    /// Shift `n` bits from `rs1`.
    PutBitsReg(Reg, BitCount),
    /// Sample `n` bits into `rd` (CRC-neutral).
    GetBits(Reg, BitCount),
    /// Turn the bus around for an immediate count of clocks.
    TarImm(Tar4),
    /// Turn the bus around for `rs1` clocks.
    TarReg(Reg),
    /// Drive `RESET#` asserted.
    RstAssert,
    /// Drive `RESET#` deasserted.
    RstDeassert,
    /// Sample `ALERT#` into `rd[0]`.
    GetAlert(Reg),
    // CTRL group (01)
    /// End the program, writing the status byte to the trace ring.
    Halt(u8),
    /// Branch if `rs1 == rs2`.
    Beq(Reg, Reg, Imm11),
    /// Branch if `rs1 != rs2`.
    Bne(Reg, Reg, Imm11),
    /// Branch if `rs1 < rs2` (unsigned).
    Bltu(Reg, Reg, Imm11),
    /// Branch if `rs1 >= rs2` (unsigned).
    Bgeu(Reg, Reg, Imm11),
    /// Block until a condition or timeout; `rd` records met/timed-out.
    WaitOn(Reg, WaitCond, WaitTimeout),
    /// Set the engine configuration (raw 6-bit payload; see [`crate::config`]).
    SetConfig(Cfg6),
    /// Write a MARK record (label + `rs1` payload) to the trace ring.
    Mark(Imm11, Reg),
    /// Reset the RX CRC-8 accumulator.
    CrcReset,
    // DATA group (10)
    /// `rd <- sext(imm)`.
    LoadImm(Reg, Imm11),
    /// `rd <- imm << 12`.
    Lui(Reg, Imm20),
    /// `rd <- rs1`.
    Mov(Reg, Reg),
    /// `rd <- rs1 + rs2`.
    Add(Reg, Reg, Reg),
    /// `rd <- rs1 + sext(imm)`.
    Addi(Reg, Reg, Imm11),
    /// `rd <- rs1 - rs2`.
    Sub(Reg, Reg, Reg),
    /// `rd <- rs1 & rs2`.
    And(Reg, Reg, Reg),
    /// `rd <- rs1 & sext(imm)`.
    Andi(Reg, Reg, Imm11),
    /// `rd <- rs1 | rs2`.
    Or(Reg, Reg, Reg),
    /// `rd <- rs1 | sext(imm)`.
    Ori(Reg, Reg, Imm11),
    /// `rd <- rs1 ^ rs2`.
    Xor(Reg, Reg, Reg),
    /// `rd <- rs1 ^ sext(imm)`.
    Xori(Reg, Reg, Imm11),
    /// `rd <- rs1` shifted by `amt` per [`ShiftOp`].
    Shift(Reg, Reg, ShiftOp, Amt5),
    /// Read special register `sr#` into `rd` (`sr=0` is the RX CRC-8).
    Rdsr(Reg, Sr5),
}

/// Why [`Instr::decode`] rejected a 32-bit word. Mirrors the HDL `DecodeError`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DecodeError {
    /// A reserved instruction field held a non-zero value.
    #[error("a reserved instruction field held a non-zero value")]
    ReservedFieldNonZero,
    /// The opcode is recognised but not implemented (kept for HDL parity; not
    /// produced by the current decoder).
    #[error("opcode recognised but not implemented")]
    OpcodeUnimplemented,
    /// The opcode is illegal: a reserved group or an unknown sub-opcode.
    #[error("illegal opcode (reserved group or unknown sub-opcode)")]
    IllegalOpcode,
}

impl Instr {
    /// Encode this instruction to its 32-bit word. Total and infallible — the
    /// exact inverse of a successful [`Instr::decode`].
    pub fn encode(&self) -> u32 {
        use Instr::*;
        match *self {
            // BUS group (00)
            CsAssert => join_word(0b00, 0x0, 0, 0, 0, 0),
            CsDeassert => join_word(0b00, 0x1, 0, 0, 0, 0),
            PutByteImm(b) => join_word(0b00, 0x2, 0, 0, 0, b as u16),
            PutByteReg(rs) => join_word(0b00, 0x3, 0, rs.bits(), 0, 0),
            GetByte(rd) => join_word(0b00, 0x4, rd.bits(), 0, 0, 0),
            PutBitsImm(n, b) => join_word(0b00, 0x5, 0, 0, 0, pack_bits_imm(n.stored(), b)),
            PutBitsReg(rs, n) => {
                join_word(0b00, 0x6, 0, rs.bits(), 0, pack_bits_imm(n.stored(), 0))
            }
            GetBits(rd, n) => join_word(0b00, 0x7, rd.bits(), 0, 0, pack_bits_imm(n.stored(), 0)),
            TarImm(n) => join_word(0b00, 0x8, 0, 0, 0, n.bits() as u16),
            TarReg(rs) => join_word(0b00, 0x9, 0, rs.bits(), 0, 0),
            RstAssert => join_word(0b00, 0xA, 0, 0, 0, 0),
            RstDeassert => join_word(0b00, 0xB, 0, 0, 0, 0),
            GetAlert(rd) => join_word(0b00, 0xC, rd.bits(), 0, 0, 0),
            // CTRL group (01)
            Halt(s) => join_word(0b01, 0x0, 0, 0, 0, s as u16),
            Beq(a, b, off) => join_word(0b01, 0x1, 0, a.bits(), b.bits(), off.bits()),
            Bne(a, b, off) => join_word(0b01, 0x2, 0, a.bits(), b.bits(), off.bits()),
            Bltu(a, b, off) => join_word(0b01, 0x3, 0, a.bits(), b.bits(), off.bits()),
            Bgeu(a, b, off) => join_word(0b01, 0x4, 0, a.bits(), b.bits(), off.bits()),
            WaitOn(rd, c, t) => {
                join_word(0b01, 0x5, rd.bits(), 0, 0, wait_pack(c.bits(), t.bits()))
            }
            SetConfig(p) => join_word(0b01, 0x6, 0, 0, 0, p.bits() as u16),
            Mark(lbl, rs) => join_word(0b01, 0x7, 0, rs.bits(), 0, lbl.bits()),
            CrcReset => join_word(0b01, 0x8, 0, 0, 0, 0),
            // DATA group (10)
            LoadImm(rd, i) => join_word(0b10, 0x0, rd.bits(), 0, 0, i.bits()),
            Lui(rd, i20) => {
                let (rs1, rs2, imm) = split_imm20(i20.bits());
                join_word(0b10, 0x1, rd.bits(), rs1, rs2, imm)
            }
            Mov(rd, rs) => join_word(0b10, 0x2, rd.bits(), rs.bits(), 0, 0),
            Add(rd, a, b) => join_word(0b10, 0x3, rd.bits(), a.bits(), b.bits(), 0),
            Addi(rd, a, i) => join_word(0b10, 0x4, rd.bits(), a.bits(), 0, i.bits()),
            Sub(rd, a, b) => join_word(0b10, 0x5, rd.bits(), a.bits(), b.bits(), 0),
            And(rd, a, b) => join_word(0b10, 0x6, rd.bits(), a.bits(), b.bits(), 0),
            Andi(rd, a, i) => join_word(0b10, 0x7, rd.bits(), a.bits(), 0, i.bits()),
            Or(rd, a, b) => join_word(0b10, 0x8, rd.bits(), a.bits(), b.bits(), 0),
            Ori(rd, a, i) => join_word(0b10, 0x9, rd.bits(), a.bits(), 0, i.bits()),
            Xor(rd, a, b) => join_word(0b10, 0xA, rd.bits(), a.bits(), b.bits(), 0),
            Xori(rd, a, i) => join_word(0b10, 0xB, rd.bits(), a.bits(), 0, i.bits()),
            Shift(rd, a, op, amt) => join_word(
                0b10,
                0xC,
                rd.bits(),
                a.bits(),
                0,
                shift_pack(op.bits(), amt.bits()),
            ),
            Rdsr(rd, sr) => join_word(0b10, 0xD, rd.bits(), 0, 0, sr.bits() as u16),
        }
    }
}

impl Instr {
    /// Decode a 32-bit word: dispatch on the group, rebuild the instruction, and
    /// verify every reserved field is zero. Total — any word yields `Ok(Instr)`
    /// or `Err(DecodeError)`. Does **not** reject register fields >= 16.
    pub fn decode(w: u32) -> Result<Instr, DecodeError> {
        let f = split_word(w);
        match f.group {
            0b00 => decode_bus(f),
            0b01 => decode_ctrl(f),
            0b10 => decode_data(f),
            _ => Err(DecodeError::IllegalOpcode), // group 0b11 reserved
        }
    }
}

/// Accept `i` only when every reserved field is zero, else `ReservedFieldNonZero`.
fn only(reserved_zero: bool, i: Instr) -> Result<Instr, DecodeError> {
    if reserved_zero {
        Ok(i)
    } else {
        Err(DecodeError::ReservedFieldNonZero)
    }
}

fn decode_bus(f: Fields) -> Result<Instr, DecodeError> {
    use Instr::*;
    let Fields {
        sub,
        rd,
        rs1,
        rs2,
        imm,
        ..
    } = f;
    let (n3, byte) = unpack_bits_imm(imm);
    let imm_hi8 = (imm >> 8) & 0x7; // imm[10:8]
    let imm_hi4 = (imm >> 4) & 0x7F; // imm[10:4]
    match sub {
        0x0 => only(rd == 0 && rs1 == 0 && rs2 == 0 && imm == 0, CsAssert),
        0x1 => only(rd == 0 && rs1 == 0 && rs2 == 0 && imm == 0, CsDeassert),
        0x2 => only(
            rd == 0 && rs1 == 0 && rs2 == 0 && imm_hi8 == 0,
            PutByteImm(byte),
        ),
        0x3 => only(
            rd == 0 && rs2 == 0 && imm == 0,
            PutByteReg(Reg::from_bits(rs1)),
        ),
        0x4 => only(
            rs1 == 0 && rs2 == 0 && imm == 0,
            GetByte(Reg::from_bits(rd)),
        ),
        0x5 => only(
            rd == 0 && rs1 == 0 && rs2 == 0,
            PutBitsImm(BitCount::from_stored(n3), byte),
        ),
        0x6 => only(
            rd == 0 && rs2 == 0 && byte == 0,
            PutBitsReg(Reg::from_bits(rs1), BitCount::from_stored(n3)),
        ),
        0x7 => only(
            rs1 == 0 && rs2 == 0 && byte == 0,
            GetBits(Reg::from_bits(rd), BitCount::from_stored(n3)),
        ),
        0x8 => only(
            rd == 0 && rs1 == 0 && rs2 == 0 && imm_hi4 == 0,
            TarImm(Tar4::from_bits((imm & 0xF) as u8)),
        ),
        0x9 => only(rd == 0 && rs2 == 0 && imm == 0, TarReg(Reg::from_bits(rs1))),
        0xA => only(rd == 0 && rs1 == 0 && rs2 == 0 && imm == 0, RstAssert),
        0xB => only(rd == 0 && rs1 == 0 && rs2 == 0 && imm == 0, RstDeassert),
        0xC => only(
            rs1 == 0 && rs2 == 0 && imm == 0,
            GetAlert(Reg::from_bits(rd)),
        ),
        _ => Err(DecodeError::IllegalOpcode),
    }
}

fn decode_ctrl(f: Fields) -> Result<Instr, DecodeError> {
    use Instr::*;
    let Fields {
        sub,
        rd,
        rs1,
        rs2,
        imm,
        ..
    } = f;
    let (cond, timeout) = wait_unpack(imm);
    let imm_hi8 = (imm >> 8) & 0x7; // imm[10:8]
    let imm_hi6 = (imm >> 6) & 0x1F; // imm[10:6]
    match sub {
        0x0 => only(
            rd == 0 && rs1 == 0 && rs2 == 0 && imm_hi8 == 0,
            Halt((imm & 0xFF) as u8),
        ),
        0x1 => only(
            rd == 0,
            Beq(
                Reg::from_bits(rs1),
                Reg::from_bits(rs2),
                Imm11::from_bits(imm),
            ),
        ),
        0x2 => only(
            rd == 0,
            Bne(
                Reg::from_bits(rs1),
                Reg::from_bits(rs2),
                Imm11::from_bits(imm),
            ),
        ),
        0x3 => only(
            rd == 0,
            Bltu(
                Reg::from_bits(rs1),
                Reg::from_bits(rs2),
                Imm11::from_bits(imm),
            ),
        ),
        0x4 => only(
            rd == 0,
            Bgeu(
                Reg::from_bits(rs1),
                Reg::from_bits(rs2),
                Imm11::from_bits(imm),
            ),
        ),
        0x5 => only(
            rs1 == 0 && rs2 == 0,
            WaitOn(
                Reg::from_bits(rd),
                WaitCond::from_bits(cond),
                WaitTimeout::from_bits(timeout),
            ),
        ),
        0x6 => only(
            rd == 0 && rs1 == 0 && rs2 == 0 && imm_hi6 == 0,
            SetConfig(Cfg6::from_bits((imm & 0x3F) as u8)),
        ),
        0x7 => only(
            rd == 0 && rs2 == 0,
            Mark(Imm11::from_bits(imm), Reg::from_bits(rs1)),
        ),
        0x8 => only(rd == 0 && rs1 == 0 && rs2 == 0 && imm == 0, CrcReset),
        _ => Err(DecodeError::IllegalOpcode),
    }
}

fn decode_data(f: Fields) -> Result<Instr, DecodeError> {
    use Instr::*;
    let Fields {
        sub,
        rd,
        rs1,
        rs2,
        imm,
        ..
    } = f;
    let (hi, i20) = join_imm20(rs1, rs2, imm);
    let (sh_op, sh_mid, sh_amt) = shift_unpack(imm);
    let imm_hi5 = (imm >> 5) & 0x3F; // imm[10:5]
    match sub {
        0x0 => only(
            rs1 == 0 && rs2 == 0,
            LoadImm(Reg::from_bits(rd), Imm11::from_bits(imm)),
        ),
        0x1 => only(hi == 0, Lui(Reg::from_bits(rd), Imm20::from_bits(i20))),
        0x2 => only(
            rs2 == 0 && imm == 0,
            Mov(Reg::from_bits(rd), Reg::from_bits(rs1)),
        ),
        0x3 => only(
            imm == 0,
            Add(Reg::from_bits(rd), Reg::from_bits(rs1), Reg::from_bits(rs2)),
        ),
        0x4 => only(
            rs2 == 0,
            Addi(
                Reg::from_bits(rd),
                Reg::from_bits(rs1),
                Imm11::from_bits(imm),
            ),
        ),
        0x5 => only(
            imm == 0,
            Sub(Reg::from_bits(rd), Reg::from_bits(rs1), Reg::from_bits(rs2)),
        ),
        0x6 => only(
            imm == 0,
            And(Reg::from_bits(rd), Reg::from_bits(rs1), Reg::from_bits(rs2)),
        ),
        0x7 => only(
            rs2 == 0,
            Andi(
                Reg::from_bits(rd),
                Reg::from_bits(rs1),
                Imm11::from_bits(imm),
            ),
        ),
        0x8 => only(
            imm == 0,
            Or(Reg::from_bits(rd), Reg::from_bits(rs1), Reg::from_bits(rs2)),
        ),
        0x9 => only(
            rs2 == 0,
            Ori(
                Reg::from_bits(rd),
                Reg::from_bits(rs1),
                Imm11::from_bits(imm),
            ),
        ),
        0xA => only(
            imm == 0,
            Xor(Reg::from_bits(rd), Reg::from_bits(rs1), Reg::from_bits(rs2)),
        ),
        0xB => only(
            rs2 == 0,
            Xori(
                Reg::from_bits(rd),
                Reg::from_bits(rs1),
                Imm11::from_bits(imm),
            ),
        ),
        0xC => match ShiftOp::from_bits(sh_op) {
            Some(op) if rs2 == 0 && sh_mid == 0 => Ok(Shift(
                Reg::from_bits(rd),
                Reg::from_bits(rs1),
                op,
                Amt5::from_bits(sh_amt),
            )),
            _ => Err(DecodeError::ReservedFieldNonZero),
        },
        0xD => only(
            rs1 == 0 && rs2 == 0 && imm_hi5 == 0,
            Rdsr(Reg::from_bits(rd), Sr5::from_bits((imm & 0x1F) as u8)),
        ),
        _ => Err(DecodeError::IllegalOpcode),
    }
}

/// Encode a program to little-endian bytes (`0xAABBCCDD → [DD, CC, BB, AA]`),
/// ready for a `LOAD_PROGRAM` frame or a bytecode file (ISA §4 / wire §7).
pub fn program_to_le_bytes(program: &[Instr]) -> Vec<u8> {
    let mut out = Vec::with_capacity(program.len() * 4);
    for instr in program {
        out.extend_from_slice(&instr.encode().to_le_bytes());
    }
    out
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

    #[test]
    fn instr_and_error_types_exist() {
        // Constructing representative variants proves the field types line up.
        let _ = Instr::CsAssert;
        let _ = Instr::PutByteImm(0x64);
        let _ = Instr::GetByte(Reg::new(5).unwrap());
        let _ = Instr::PutBitsImm(BitCount::new(8).unwrap(), 0xAB);
        let _ = Instr::TarImm(Tar4::new(2).unwrap());
        let _ = Instr::Beq(
            Reg::new(5).unwrap(),
            Reg::new(6).unwrap(),
            Imm11::new(4).unwrap(),
        );
        let _ = Instr::WaitOn(
            Reg::new(5).unwrap(),
            WaitCond::new(1).unwrap(),
            WaitTimeout::new(0x64).unwrap(),
        );
        let _ = Instr::SetConfig(Cfg6::new(0).unwrap());
        let _ = Instr::Lui(Reg::new(5).unwrap(), Imm20::new(0x12345).unwrap());
        let _ = Instr::Shift(
            Reg::new(5).unwrap(),
            Reg::new(6).unwrap(),
            ShiftOp::Sra,
            Amt5::new(3).unwrap(),
        );
        let _ = Instr::Rdsr(Reg::new(7).unwrap(), Sr5::new(0).unwrap());
        assert_ne!(
            DecodeError::ReservedFieldNonZero,
            DecodeError::IllegalOpcode
        );
        assert_ne!(DecodeError::OpcodeUnimplemented, DecodeError::IllegalOpcode);
    }

    fn golden_encode_vectors() -> Vec<(Instr, u32)> {
        vec![
            (Instr::CsAssert, 0x0000_0000),
            (Instr::CsDeassert, 0x0400_0000),
            (Instr::PutByteImm(0x64), 0x0800_0064),
            (Instr::GetByte(Reg::new(5).unwrap()), 0x10A0_0000),
            (
                Instr::PutBitsImm(BitCount::new(8).unwrap(), 0xAB),
                0x1400_07AB,
            ),
            (Instr::TarImm(Tar4::new(2).unwrap()), 0x2000_0002),
            (Instr::Halt(0x11), 0x4000_0011),
            (
                Instr::Beq(
                    Reg::new(5).unwrap(),
                    Reg::new(6).unwrap(),
                    Imm11::new(4).unwrap(),
                ),
                0x4405_3004,
            ),
            (
                Instr::WaitOn(
                    Reg::new(5).unwrap(),
                    WaitCond::new(1).unwrap(),
                    WaitTimeout::new(0x64).unwrap(),
                ),
                0x54A0_0264,
            ),
            (Instr::SetConfig(Cfg6::new(0).unwrap()), 0x5800_0000),
            (Instr::CrcReset, 0x6000_0000),
            (
                Instr::LoadImm(Reg::new(5).unwrap(), Imm11::new(0x0F).unwrap()),
                0x80A0_000F,
            ),
            (
                Instr::Lui(Reg::new(5).unwrap(), Imm20::new(0x12345).unwrap()),
                0x84A1_2345,
            ),
            (
                Instr::Shift(
                    Reg::new(5).unwrap(),
                    Reg::new(6).unwrap(),
                    ShiftOp::Sra,
                    Amt5::new(3).unwrap(),
                ),
                0xB0A6_0403,
            ),
            (
                Instr::Rdsr(Reg::new(7).unwrap(), Sr5::new(0).unwrap()),
                0xB4E0_0000,
            ),
        ]
    }

    #[test]
    fn encode_matches_golden_words() {
        for (instr, word) in golden_encode_vectors() {
            assert_eq!(instr.encode(), word, "encode mismatch for {instr:?}");
        }
    }

    #[test]
    fn program_to_le_bytes_is_little_endian() {
        // 0x0800_0064 -> [0x64, 0x00, 0x00, 0x08]
        let bytes = program_to_le_bytes(&[Instr::PutByteImm(0x64)]);
        assert_eq!(bytes, vec![0x64, 0x00, 0x00, 0x08]);
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

    use proptest::strategy::Union;

    fn arb_reg() -> impl Strategy<Value = Reg> {
        (0u8..=31).prop_map(Reg::from_bits)
    }
    fn arb_imm11() -> impl Strategy<Value = Imm11> {
        (0u16..=0x7FF).prop_map(Imm11::from_bits)
    }
    fn arb_imm20() -> impl Strategy<Value = Imm20> {
        (0u32..=0xF_FFFF).prop_map(Imm20::from_bits)
    }
    fn arb_tar4() -> impl Strategy<Value = Tar4> {
        (0u8..=0xF).prop_map(Tar4::from_bits)
    }
    fn arb_cfg6() -> impl Strategy<Value = Cfg6> {
        (0u8..=0x3F).prop_map(Cfg6::from_bits)
    }
    fn arb_sr5() -> impl Strategy<Value = Sr5> {
        (0u8..=0x1F).prop_map(Sr5::from_bits)
    }
    fn arb_amt5() -> impl Strategy<Value = Amt5> {
        (0u8..=0x1F).prop_map(Amt5::from_bits)
    }
    fn arb_bit_count() -> impl Strategy<Value = BitCount> {
        (1u8..=8).prop_map(|n| BitCount::new(n).unwrap())
    }
    fn arb_shift_op() -> impl Strategy<Value = ShiftOp> {
        prop_oneof![Just(ShiftOp::Sll), Just(ShiftOp::Srl), Just(ShiftOp::Sra)]
    }
    fn arb_wait_cond() -> impl Strategy<Value = WaitCond> {
        (0u8..=0x3).prop_map(WaitCond::from_bits)
    }
    fn arb_wait_timeout() -> impl Strategy<Value = WaitTimeout> {
        (0u16..=0x1FF).prop_map(WaitTimeout::from_bits)
    }

    fn arb_bus_instr() -> impl Strategy<Value = Instr> {
        // `Union::new` (not `prop_oneof!`) because there are more than 10 arms.
        Union::new(vec![
            Just(Instr::CsAssert).boxed(),
            Just(Instr::CsDeassert).boxed(),
            any::<u8>().prop_map(Instr::PutByteImm).boxed(),
            arb_reg().prop_map(Instr::PutByteReg).boxed(),
            arb_reg().prop_map(Instr::GetByte).boxed(),
            (arb_bit_count(), any::<u8>())
                .prop_map(|(n, b)| Instr::PutBitsImm(n, b))
                .boxed(),
            (arb_reg(), arb_bit_count())
                .prop_map(|(r, n)| Instr::PutBitsReg(r, n))
                .boxed(),
            (arb_reg(), arb_bit_count())
                .prop_map(|(r, n)| Instr::GetBits(r, n))
                .boxed(),
            arb_tar4().prop_map(Instr::TarImm).boxed(),
            arb_reg().prop_map(Instr::TarReg).boxed(),
            Just(Instr::RstAssert).boxed(),
            Just(Instr::RstDeassert).boxed(),
            arb_reg().prop_map(Instr::GetAlert).boxed(),
        ])
    }

    fn arb_ctrl_instr() -> impl Strategy<Value = Instr> {
        Union::new(vec![
            any::<u8>().prop_map(Instr::Halt).boxed(),
            (arb_reg(), arb_reg(), arb_imm11())
                .prop_map(|(a, b, o)| Instr::Beq(a, b, o))
                .boxed(),
            (arb_reg(), arb_reg(), arb_imm11())
                .prop_map(|(a, b, o)| Instr::Bne(a, b, o))
                .boxed(),
            (arb_reg(), arb_reg(), arb_imm11())
                .prop_map(|(a, b, o)| Instr::Bltu(a, b, o))
                .boxed(),
            (arb_reg(), arb_reg(), arb_imm11())
                .prop_map(|(a, b, o)| Instr::Bgeu(a, b, o))
                .boxed(),
            (arb_reg(), arb_wait_cond(), arb_wait_timeout())
                .prop_map(|(r, c, t)| Instr::WaitOn(r, c, t))
                .boxed(),
            arb_cfg6().prop_map(Instr::SetConfig).boxed(),
            (arb_imm11(), arb_reg())
                .prop_map(|(l, r)| Instr::Mark(l, r))
                .boxed(),
            Just(Instr::CrcReset).boxed(),
        ])
    }

    fn arb_data_instr() -> impl Strategy<Value = Instr> {
        // `Union::new` (not `prop_oneof!`) because there are more than 10 arms.
        Union::new(vec![
            (arb_reg(), arb_imm11())
                .prop_map(|(r, i)| Instr::LoadImm(r, i))
                .boxed(),
            (arb_reg(), arb_imm20())
                .prop_map(|(r, i)| Instr::Lui(r, i))
                .boxed(),
            (arb_reg(), arb_reg())
                .prop_map(|(d, s)| Instr::Mov(d, s))
                .boxed(),
            (arb_reg(), arb_reg(), arb_reg())
                .prop_map(|(d, a, b)| Instr::Add(d, a, b))
                .boxed(),
            (arb_reg(), arb_reg(), arb_imm11())
                .prop_map(|(d, a, i)| Instr::Addi(d, a, i))
                .boxed(),
            (arb_reg(), arb_reg(), arb_reg())
                .prop_map(|(d, a, b)| Instr::Sub(d, a, b))
                .boxed(),
            (arb_reg(), arb_reg(), arb_reg())
                .prop_map(|(d, a, b)| Instr::And(d, a, b))
                .boxed(),
            (arb_reg(), arb_reg(), arb_imm11())
                .prop_map(|(d, a, i)| Instr::Andi(d, a, i))
                .boxed(),
            (arb_reg(), arb_reg(), arb_reg())
                .prop_map(|(d, a, b)| Instr::Or(d, a, b))
                .boxed(),
            (arb_reg(), arb_reg(), arb_imm11())
                .prop_map(|(d, a, i)| Instr::Ori(d, a, i))
                .boxed(),
            (arb_reg(), arb_reg(), arb_reg())
                .prop_map(|(d, a, b)| Instr::Xor(d, a, b))
                .boxed(),
            (arb_reg(), arb_reg(), arb_imm11())
                .prop_map(|(d, a, i)| Instr::Xori(d, a, i))
                .boxed(),
            (arb_reg(), arb_reg(), arb_shift_op(), arb_amt5())
                .prop_map(|(d, a, op, amt)| Instr::Shift(d, a, op, amt))
                .boxed(),
            (arb_reg(), arb_sr5())
                .prop_map(|(r, s)| Instr::Rdsr(r, s))
                .boxed(),
        ])
    }

    fn arb_instr() -> impl Strategy<Value = Instr> {
        prop_oneof![arb_bus_instr(), arb_ctrl_instr(), arb_data_instr()]
    }

    proptest! {
        #[test]
        fn bus_round_trip(i in arb_bus_instr()) {
            prop_assert_eq!(Instr::decode(i.encode()), Ok(i));
        }
        #[test]
        fn ctrl_round_trip(i in arb_ctrl_instr()) {
            prop_assert_eq!(Instr::decode(i.encode()), Ok(i));
        }
        #[test]
        fn data_round_trip(i in arb_data_instr()) {
            prop_assert_eq!(Instr::decode(i.encode()), Ok(i));
        }
    }

    #[test]
    fn reserved_nonzero_field_traps() {
        // CS_ASSERT is all-reserved; setting imm bit 0 must trap.
        let cs_assert = Instr::CsAssert.encode();
        assert_eq!(
            Instr::decode(cs_assert + 1),
            Err(DecodeError::ReservedFieldNonZero)
        );
    }

    #[test]
    fn reserved_shift_op_traps() {
        // DATA sub 0xC with op field 0b11 (imm[10:9]) is the reserved SHIFT op.
        let w = join_word(0b10, 0xC, 0, 0, 0, 0b11 << 9);
        assert_eq!(Instr::decode(w), Err(DecodeError::ReservedFieldNonZero));
    }

    #[test]
    fn reserved_group_is_illegal() {
        let w = 0b11u32 << 30;
        assert_eq!(Instr::decode(w), Err(DecodeError::IllegalOpcode));
    }

    #[test]
    fn unknown_sub_opcode_is_illegal() {
        // BUS sub 0xD is unassigned.
        let w = join_word(0b00, 0xD, 0, 0, 0, 0);
        assert_eq!(Instr::decode(w), Err(DecodeError::IllegalOpcode));
    }

    proptest! {
        #[test]
        fn any_word_decodes_canonical_or_traps(w in any::<u32>()) {
            // A trap (`Err`) is always acceptable; a successful decode must be canonical.
            if let Ok(i) = Instr::decode(w) {
                prop_assert_eq!(i.encode(), w);
            }
        }

        #[test]
        fn combined_round_trip(i in arb_instr()) {
            prop_assert_eq!(Instr::decode(i.encode()), Ok(i));
        }
    }
}
