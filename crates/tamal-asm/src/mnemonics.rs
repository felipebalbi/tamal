//! The canonical `Instr` -> asm-text renderer (inverse of the parse table),
//! shared by disassembly and program listings.

use tamal_abi::isa::{Cfg6, Imm11, Imm21, Instr, Reg, ShiftOp};

fn rn(r: Reg) -> String {
    format!("x{}", r.bits())
}

fn sext11(imm: Imm11) -> i32 {
    let v = imm.bits() as i32;
    if v & 0x400 != 0 { v | !0x7FF } else { v }
}

fn sext21(imm: Imm21) -> i32 {
    let v = imm.bits() as i32;
    if v & (1 << 20) != 0 {
        v | !0x1F_FFFF
    } else {
        v
    }
}

fn render_set_config(p: Cfg6) -> String {
    let b = p.bits();
    let role = if (b >> 5) & 1 == 0 {
        "controller"
    } else {
        "target"
    };
    let io = match (b >> 3) & 3 {
        0 => "x1",
        1 => "x2",
        2 => "x4",
        _ => "x?",
    };
    let sck = match (b >> 1) & 3 {
        0 => "sck20",
        1 => "sck33",
        2 => "sck50",
        _ => "sck66",
    };
    let alert = if b & 1 == 0 { "alert_pin" } else { "alert_io1" };
    format!("set_config {role}, {io}, {sck}, {alert}")
}

/// Render an instruction as canonical asm text. Non-branch renderings re-parse
/// to the same word (round-trip); branches render a signed numeric offset
/// (disassembly is not required to re-assemble without labels).
pub fn render_instr(i: &Instr) -> String {
    use Instr::*;
    match *i {
        CsAssert => "cs_assert".into(),
        CsDeassert => "cs_deassert".into(),
        RstAssert => "rst_assert".into(),
        RstDeassert => "rst_deassert".into(),
        CrcReset => "crc_reset".into(),
        PutByteImm(b) => format!("put_byte {b}"),
        PutByteReg(r) => format!("put_byte {}", rn(r)),
        GetByte(r) => format!("get_byte {}", rn(r)),
        PutBitsImm(n, b) => format!("put_bits {}, {}", n.count(), b),
        PutBitsReg(r, n) => format!("put_bits {}, {}", rn(r), n.count()),
        GetBits(r, n) => format!("get_bits {}, {}", rn(r), n.count()),
        TarImm(t) => format!("tar {}", t.bits()),
        TarReg(r) => format!("tar {}", rn(r)),
        GetAlert(r) => format!("get_alert {}", rn(r)),
        Halt(s) => format!("halt {s}"),
        Beq(a, b, off) => format!("beq {}, {}, {}", rn(a), rn(b), sext11(off)),
        Bne(a, b, off) => format!("bne {}, {}, {}", rn(a), rn(b), sext11(off)),
        Bltu(a, b, off) => format!("bltu {}, {}, {}", rn(a), rn(b), sext11(off)),
        Bgeu(a, b, off) => format!("bgeu {}, {}, {}", rn(a), rn(b), sext11(off)),
        WaitOn(r, c, t) => format!("wait_on {}, {}, {}", rn(r), c.bits(), t.bits()),
        SetConfig(p) => render_set_config(p),
        Mark(tag, r) => format!("mark {}, {}", tag.bits(), rn(r)),
        LoadImm(r, imm) => format!("load_imm {}, {}", rn(r), sext21(imm)),
        Lui(r, imm) => format!("lui {}, {}", rn(r), imm.bits()),
        Mov(rd, rs) => format!("mov {}, {}", rn(rd), rn(rs)),
        Add(d, a, b) => format!("add {}, {}, {}", rn(d), rn(a), rn(b)),
        Sub(d, a, b) => format!("sub {}, {}, {}", rn(d), rn(a), rn(b)),
        And(d, a, b) => format!("and {}, {}, {}", rn(d), rn(a), rn(b)),
        Or(d, a, b) => format!("or {}, {}, {}", rn(d), rn(a), rn(b)),
        Xor(d, a, b) => format!("xor {}, {}, {}", rn(d), rn(a), rn(b)),
        Addi(d, a, imm) => format!("addi {}, {}, {}", rn(d), rn(a), sext11(imm)),
        Andi(d, a, imm) => format!("andi {}, {}, {}", rn(d), rn(a), sext11(imm)),
        Ori(d, a, imm) => format!("ori {}, {}, {}", rn(d), rn(a), sext11(imm)),
        Xori(d, a, imm) => format!("xori {}, {}, {}", rn(d), rn(a), sext11(imm)),
        Shift(d, a, op, amt) => {
            let m = match op {
                ShiftOp::Sll => "sll",
                ShiftOp::Srl => "srl",
                ShiftOp::Sra => "sra",
            };
            format!("{m} {}, {}, {}", rn(d), rn(a), amt.bits())
        }
        Rdsr(r, sr) => {
            if sr.bits() == 0 {
                format!("rdsr {}, crc", rn(r))
            } else {
                format!("rdsr {}, {}", rn(r), sr.bits())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tamal_abi::config::{AlertSource, Config, IoMode, Role, Sck};
    use tamal_abi::isa::{Amt5, BitCount, Imm11, Imm21, Instr, Reg, ShiftOp, Sr5, Tar4};

    fn r(n: u8) -> Reg {
        Reg::new(n).unwrap()
    }
    fn i11(v: i32) -> Imm11 {
        Imm11::new((v as u32 & 0x7FF) as u16).unwrap()
    }
    fn i21(v: i32) -> Imm21 {
        Imm21::new(v as u32 & 0x1F_FFFF).unwrap()
    }

    // Assemble the rendered text and confirm it re-encodes to the same word.
    fn roundtrip(i: Instr) {
        let text = render_instr(&i);
        let prog = crate::assemble(&format!("{text}\n"))
            .unwrap_or_else(|d| panic!("re-assemble `{text}` failed: {d:?}"));
        let words: Vec<u32> = prog.words().collect();
        assert_eq!(words, vec![i.encode()], "round-trip mismatch for `{text}`");
    }

    #[test]
    fn roundtrip_non_branch_mnemonics() {
        let cfg = Config {
            role: Role::Controller,
            io_mode: IoMode::X1,
            sck: Sck::Sck20,
            alert_source: AlertSource::AlertPin,
        };
        let cases = vec![
            Instr::CsAssert,
            Instr::CsDeassert,
            Instr::PutByteImm(0x64),
            Instr::PutByteReg(r(5)),
            Instr::GetByte(r(5)),
            Instr::PutBitsImm(BitCount::new(8).unwrap(), 0xAB),
            Instr::PutBitsReg(r(6), BitCount::new(3).unwrap()),
            Instr::GetBits(r(7), BitCount::new(2).unwrap()),
            Instr::TarImm(Tar4::new(2).unwrap()),
            Instr::TarReg(r(1)),
            Instr::GetAlert(r(5)),
            Instr::Halt(0x11),
            Instr::SetConfig(cfg.pack()),
            Instr::Mark(i11(42), r(5)),
            Instr::LoadImm(r(5), i21(-3)),
            Instr::LoadImm(r(5), i21(0x12345)),
            Instr::Lui(r(5), Imm21::new(0x12345).unwrap()),
            Instr::Lui(r(5), Imm21::new(0x1F_FFFF).unwrap()),
            Instr::Mov(r(5), r(6)),
            Instr::Add(r(5), r(6), r(7)),
            Instr::Addi(r(5), r(6), i11(-1)),
            Instr::Shift(r(5), r(6), ShiftOp::Sra, Amt5::new(3).unwrap()),
            Instr::Rdsr(r(7), Sr5::new(0).unwrap()),
            Instr::Rdsr(r(7), Sr5::new(5).unwrap()),
        ];
        for c in cases {
            roundtrip(c);
        }
    }

    #[test]
    fn branch_renders_signed_numeric_offset() {
        assert_eq!(
            render_instr(&Instr::Beq(r(5), r(6), i11(2))),
            "beq x5, x6, 2"
        );
        assert_eq!(
            render_instr(&Instr::Bne(r(7), r(0), i11(-2))),
            "bne x7, x0, -2"
        );
    }
}
