//! Lowering: mnemonics + pseudo-ops -> tamal_abi::isa::Instr, with resolution,
//! `li` tiling, branch offsets, and range checks.

use tamal_abi::config::{AlertSource, Config, ConfigError, IoMode, Role, Sck, decode_config};
use tamal_abi::isa::{
    Amt5, BitCount, Imm11, Imm20, Instr, Reg, ShiftOp, Sr5, Tar4, WaitCond, WaitTimeout,
};

use crate::diagnostics::{Diagnostic, Span};
use crate::parser::{Operand, OperandKind};
use crate::symbol::{Sym, SymbolTable};

/// Map a register name (`xN` or an ABI name) to its x-number, or `None`.
fn reg_number(name: &str) -> Option<u8> {
    if let Some(rest) = name.strip_prefix('x') {
        if let Ok(n) = rest.parse::<u16>() {
            return if n <= 31 { Some(n as u8) } else { None };
        }
    }
    Some(match name {
        "zero" => 0,
        "ra" => 1,
        "sp" => 2,
        "gp" => 3,
        "tp" => 4,
        "t0" => 5,
        "t1" => 6,
        "t2" => 7,
        "s0" | "fp" => 8,
        "s1" => 9,
        "a0" => 10,
        "a1" => 11,
        "a2" => 12,
        "a3" => 13,
        "a4" => 14,
        "a5" => 15,
        "a6" => 16,
        "a7" => 17,
        "s2" => 18,
        "s3" => 19,
        "s4" => 20,
        "s5" => 21,
        "s6" => 22,
        "s7" => 23,
        "s8" => 24,
        "s9" => 25,
        "s10" => 26,
        "s11" => 27,
        "t3" => 28,
        "t4" => 29,
        "t5" => 30,
        "t6" => 31,
        _ => return None,
    })
}

/// Resolve a register operand to a v1 `Reg` (x0-x15), else a diagnostic.
pub(crate) fn resolve_reg(op: &Operand) -> Result<Reg, Diagnostic> {
    let name = match &op.kind {
        OperandKind::Ident(s) => s.as_str(),
        OperandKind::Num(_) => {
            return Err(Diagnostic::error(
                op.span.clone(),
                "expected a register, found a number",
            ));
        }
    };
    let num = reg_number(name)
        .ok_or_else(|| Diagnostic::error(op.span.clone(), format!("unknown register `{name}`")))?;
    if num >= 16 {
        return Err(Diagnostic::error(
            op.span.clone(),
            format!("register `{name}` maps to x{num}"),
        )
        .with_help("tamal v1 has a 16-register file (x0-x15); x16-x31 are unavailable"));
    }
    Ok(Reg::new(num).expect("num < 16 is a valid Reg"))
}

/// Resolve an immediate operand (number or `.equ`) to an `i64`, else a diagnostic.
pub(crate) fn resolve_imm(op: &Operand, syms: &SymbolTable) -> Result<i64, Diagnostic> {
    match &op.kind {
        OperandKind::Num(n) => Ok(*n),
        OperandKind::Ident(name) => match syms.get(name) {
            Some(Sym::Equ(v)) => Ok(v),
            Some(Sym::Label(_)) => Err(Diagnostic::error(
                op.span.clone(),
                format!("`{name}` is a label; expected a constant"),
            )),
            None => Err(Diagnostic::error(
                op.span.clone(),
                format!("undefined symbol `{name}`"),
            )),
        },
    }
}

/// Range-check `value` into `[lo, hi]`, else a diagnostic naming `what`.
pub(crate) fn checked(
    value: i64,
    lo: i64,
    hi: i64,
    span: &Span,
    what: &str,
) -> Result<i64, Diagnostic> {
    if value < lo || value > hi {
        Err(Diagnostic::error(
            span.clone(),
            format!("{what} out of range: {value} is not in [{lo}, {hi}]"),
        ))
    } else {
        Ok(value)
    }
}

/// An 11-bit immediate from a signed value in `[-1024, 1023]` (two's complement).
fn imm11(v: i32) -> Imm11 {
    Imm11::new((v as u32 & 0x7FF) as u16).expect("11-bit pattern always fits")
}

/// Build the `li` instruction sequence for any 32-bit `value` (1–4 instructions),
/// minimal — one `load_imm` for signed-11 values, and never a dead `lui rd, 0`.
pub(crate) fn tile_li(rd: Reg, value: i32) -> Vec<Instr> {
    // 1 instruction when the value fits the sign-extended 11-bit immediate.
    if (-1024..=1023).contains(&value) {
        return vec![Instr::LoadImm(rd, imm11(value))];
    }
    let w = value as u32;
    let low12 = (w & 0xFFF) as i32; // 0..4095
    let mut hi = (w >> 12) & 0xF_FFFF; // top 20 bits
    let mut resid = low12;
    if resid > 2047 {
        resid -= 4096; // fold into [-2048, -1]
        hi = hi.wrapping_add(1) & 0xF_FFFF;
    }
    // Seed the register: `lui` only when the high 20 bits are set; otherwise start
    // from a `load_imm` of the first residual chunk (avoids a wasteful `lui rd, 0`).
    let mut out = Vec::new();
    if hi == 0 {
        let seed = resid.clamp(-1024, 1023);
        out.push(Instr::LoadImm(rd, imm11(seed)));
        resid -= seed;
    } else {
        out.push(Instr::Lui(rd, Imm20::new(hi).expect("20-bit pattern fits")));
    }
    while resid != 0 {
        let step = resid.clamp(-1024, 1023);
        out.push(Instr::Addi(rd, rd, imm11(step)));
        resid -= step;
    }
    out
}

/// Resolve + range-check an `li` value into its 32-bit pattern (as `i32`).
pub(crate) fn li_value(op: &Operand, syms: &SymbolTable) -> Result<i32, Diagnostic> {
    let v = resolve_imm(op, syms)?;
    if !(i64::from(i32::MIN)..=i64::from(u32::MAX)).contains(&v) {
        return Err(Diagnostic::error(
            op.span.clone(),
            format!("`li` value {v} does not fit in 32 bits"),
        ));
    }
    Ok(v as u32 as i32) // reinterpret the low-32 bit pattern
}

/// Number of instruction words an `li` of `value` expands to (1–4).
pub(crate) fn li_word_count(value: i32) -> u16 {
    tile_li(Reg::new(0).expect("x0"), value).len() as u16
}

/// Word count of one instruction line, for pass-1 addressing. Infallible: only
/// `li` varies (1–4 words, from its constant); everything else is 1 word. When a
/// `li` value can't be sized (bad arity / undefined symbol), assume 1 — pass 2
/// reports the real error and assembly fails anyway.
pub(crate) fn instr_word_count(mnemonic: &str, operands: &[Operand], syms: &SymbolTable) -> u16 {
    if mnemonic == "li" && operands.len() == 2 {
        if let Ok(v) = li_value(&operands[1], syms) {
            return li_word_count(v);
        }
    }
    1
}

fn is_register(op: &Operand) -> bool {
    matches!(&op.kind, OperandKind::Ident(s) if reg_number(s).is_some())
}

fn args_exact(operands: &[Operand], n: usize, span: &Span, mnem: &str) -> Result<(), Diagnostic> {
    if operands.len() == n {
        Ok(())
    } else {
        Err(Diagnostic::error(
            span.clone(),
            format!("`{mnem}` takes {n} operand(s), got {}", operands.len()),
        ))
    }
}

fn field_u8(op: &Operand, syms: &SymbolTable, what: &str) -> Result<u8, Diagnostic> {
    Ok(checked(resolve_imm(op, syms)?, 0, 255, &op.span, what)? as u8)
}
fn field_imm11(op: &Operand, syms: &SymbolTable) -> Result<Imm11, Diagnostic> {
    let v = checked(resolve_imm(op, syms)?, -1024, 1023, &op.span, "immediate")?;
    Ok(Imm11::new((v as u32 & 0x7FF) as u16).unwrap())
}
fn field_bitcount(op: &Operand, syms: &SymbolTable) -> Result<BitCount, Diagnostic> {
    let v = checked(resolve_imm(op, syms)?, 1, 8, &op.span, "bit count")?;
    Ok(BitCount::new(v as u8).unwrap())
}
fn field_tar4(op: &Operand, syms: &SymbolTable) -> Result<Tar4, Diagnostic> {
    let v = checked(resolve_imm(op, syms)?, 0, 15, &op.span, "TAR count")?;
    Ok(Tar4::new(v as u8).unwrap())
}
fn field_amt5(op: &Operand, syms: &SymbolTable) -> Result<Amt5, Diagnostic> {
    let v = checked(resolve_imm(op, syms)?, 0, 31, &op.span, "shift amount")?;
    Ok(Amt5::new(v as u8).unwrap())
}
fn field_imm20(op: &Operand, syms: &SymbolTable) -> Result<Imm20, Diagnostic> {
    let v = checked(resolve_imm(op, syms)?, 0, 0xF_FFFF, &op.span, "immediate")?;
    Ok(Imm20::new(v as u32).unwrap())
}
fn field_mark_tag(op: &Operand, syms: &SymbolTable) -> Result<Imm11, Diagnostic> {
    let v = checked(resolve_imm(op, syms)?, 0, 2047, &op.span, "mark tag")?;
    Ok(Imm11::new(v as u16).unwrap())
}
fn field_wait_cond(op: &Operand, syms: &SymbolTable) -> Result<WaitCond, Diagnostic> {
    let v = checked(resolve_imm(op, syms)?, 0, 3, &op.span, "wait condition")?;
    Ok(WaitCond::new(v as u8).unwrap())
}
fn field_wait_timeout(op: &Operand, syms: &SymbolTable) -> Result<WaitTimeout, Diagnostic> {
    let v = checked(resolve_imm(op, syms)?, 0, 511, &op.span, "wait timeout")?;
    Ok(WaitTimeout::new(v as u16).unwrap())
}

fn resolve_sr(op: &Operand, syms: &SymbolTable) -> Result<Sr5, Diagnostic> {
    if let OperandKind::Ident(s) = &op.kind {
        if s.eq_ignore_ascii_case("crc") {
            return Ok(Sr5::new(0).unwrap());
        }
    }
    let v = checked(resolve_imm(op, syms)?, 0, 31, &op.span, "special register")?;
    Ok(Sr5::new(v as u8).unwrap())
}

fn resolve_branch_target(op: &Operand, addr: u16, syms: &SymbolTable) -> Result<Imm11, Diagnostic> {
    let name = match &op.kind {
        OperandKind::Ident(s) => s,
        OperandKind::Num(_) => {
            return Err(Diagnostic::error(
                op.span.clone(),
                "branch target must be a label",
            ));
        }
    };
    let target = match syms.get(name) {
        Some(Sym::Label(t)) => i32::from(t),
        Some(Sym::Equ(_)) => {
            return Err(Diagnostic::error(
                op.span.clone(),
                format!("`{name}` is a constant; a branch target must be a label"),
            ));
        }
        None => {
            return Err(Diagnostic::error(
                op.span.clone(),
                format!("undefined label `{name}`"),
            ));
        }
    };
    let off = target - i32::from(addr);
    checked(i64::from(off), -1024, 1023, &op.span, "branch offset")?;
    Ok(Imm11::new((off as u32 & 0x7FF) as u16).unwrap())
}

fn encode_set_config(operands: &[Operand], span: &Span) -> Result<Instr, Diagnostic> {
    args_exact(operands, 4, span, "set_config")?;
    let kw = |op: &Operand| -> Result<String, Diagnostic> {
        match &op.kind {
            OperandKind::Ident(s) => Ok(s.to_ascii_lowercase()),
            OperandKind::Num(_) => Err(Diagnostic::error(op.span.clone(), "expected a keyword")),
        }
    };
    let role = match kw(&operands[0])?.as_str() {
        "controller" => Role::Controller,
        "target" => Role::Target,
        other => {
            return Err(Diagnostic::error(
                operands[0].span.clone(),
                format!("unknown role `{other}` (controller|target)"),
            ));
        }
    };
    let io_mode = match kw(&operands[1])?.as_str() {
        "x1" => IoMode::X1,
        "x2" => IoMode::X2,
        "x4" => IoMode::X4,
        other => {
            return Err(Diagnostic::error(
                operands[1].span.clone(),
                format!("unknown IO mode `{other}` (x1|x2|x4)"),
            ));
        }
    };
    let sck = match kw(&operands[2])?.as_str() {
        "sck20" => Sck::Sck20,
        "sck33" => Sck::Sck33,
        "sck50" => Sck::Sck50,
        "sck66" => Sck::Sck66,
        other => {
            return Err(Diagnostic::error(
                operands[2].span.clone(),
                format!("unknown SCK `{other}` (sck20|sck33|sck50|sck66)"),
            ));
        }
    };
    let alert_source = match kw(&operands[3])?.as_str() {
        "alert_pin" => AlertSource::AlertPin,
        "alert_io1" => AlertSource::AlertIo1,
        other => {
            return Err(Diagnostic::error(
                operands[3].span.clone(),
                format!("unknown alert source `{other}` (alert_pin|alert_io1)"),
            ));
        }
    };
    let cfg = Config {
        role,
        io_mode,
        sck,
        alert_source,
    };
    let payload = cfg.pack();
    if let Err(e) = decode_config(payload) {
        let (sp, msg): (Span, &str) = match e {
            ConfigError::UnsupportedRole => (
                operands[0].span.clone(),
                "role `target` is not available in v1 (controller only)",
            ),
            ConfigError::UnsupportedIoMode => (
                operands[1].span.clone(),
                "IO mode `x2`/`x4` is not available in v1 (x1 only)",
            ),
            ConfigError::UnsupportedSck => (
                operands[2].span.clone(),
                "SCK `sck33`/`sck50`/`sck66` is not available in v1 (sck20 only)",
            ),
        };
        return Err(Diagnostic::error(sp, msg));
    }
    Ok(Instr::SetConfig(payload))
}

fn rrr(
    operands: &[Operand],
    span: &Span,
    mnem: &str,
    ctor: fn(Reg, Reg, Reg) -> Instr,
) -> Result<Vec<Instr>, Diagnostic> {
    args_exact(operands, 3, span, mnem)?;
    Ok(vec![ctor(
        resolve_reg(&operands[0])?,
        resolve_reg(&operands[1])?,
        resolve_reg(&operands[2])?,
    )])
}

fn rri(
    operands: &[Operand],
    span: &Span,
    mnem: &str,
    syms: &SymbolTable,
    ctor: fn(Reg, Reg, Imm11) -> Instr,
) -> Result<Vec<Instr>, Diagnostic> {
    args_exact(operands, 3, span, mnem)?;
    Ok(vec![ctor(
        resolve_reg(&operands[0])?,
        resolve_reg(&operands[1])?,
        field_imm11(&operands[2], syms)?,
    )])
}

fn shift(
    operands: &[Operand],
    span: &Span,
    mnem: &str,
    syms: &SymbolTable,
    op: ShiftOp,
) -> Result<Vec<Instr>, Diagnostic> {
    args_exact(operands, 3, span, mnem)?;
    Ok(vec![Instr::Shift(
        resolve_reg(&operands[0])?,
        resolve_reg(&operands[1])?,
        op,
        field_amt5(&operands[2], syms)?,
    )])
}

fn branch(
    operands: &[Operand],
    span: &Span,
    mnem: &str,
    addr: u16,
    syms: &SymbolTable,
    ctor: fn(Reg, Reg, Imm11) -> Instr,
) -> Result<Vec<Instr>, Diagnostic> {
    args_exact(operands, 3, span, mnem)?;
    let rs1 = resolve_reg(&operands[0])?;
    let rs2 = resolve_reg(&operands[1])?;
    let off = resolve_branch_target(&operands[2], addr, syms)?;
    Ok(vec![ctor(rs1, rs2, off)])
}

fn zero() -> Reg {
    Reg::new(0).expect("x0")
}

/// Lower one instruction line to its `Instr` sequence (most = 1; `li` = 1–4).
pub(crate) fn encode_line(
    mnemonic: &str,
    operands: &[Operand],
    span: &Span,
    addr: u16,
    syms: &SymbolTable,
) -> Result<Vec<Instr>, Diagnostic> {
    match mnemonic {
        "cs_assert" => {
            args_exact(operands, 0, span, mnemonic)?;
            Ok(vec![Instr::CsAssert])
        }
        "cs_deassert" => {
            args_exact(operands, 0, span, mnemonic)?;
            Ok(vec![Instr::CsDeassert])
        }
        "rst_assert" => {
            args_exact(operands, 0, span, mnemonic)?;
            Ok(vec![Instr::RstAssert])
        }
        "rst_deassert" => {
            args_exact(operands, 0, span, mnemonic)?;
            Ok(vec![Instr::RstDeassert])
        }
        "crc_reset" => {
            args_exact(operands, 0, span, mnemonic)?;
            Ok(vec![Instr::CrcReset])
        }
        "put_byte" => {
            args_exact(operands, 1, span, mnemonic)?;
            if is_register(&operands[0]) {
                Ok(vec![Instr::PutByteReg(resolve_reg(&operands[0])?)])
            } else {
                Ok(vec![Instr::PutByteImm(field_u8(
                    &operands[0],
                    syms,
                    "byte",
                )?)])
            }
        }
        "get_byte" => {
            args_exact(operands, 1, span, mnemonic)?;
            Ok(vec![Instr::GetByte(resolve_reg(&operands[0])?)])
        }
        "put_bits" => {
            args_exact(operands, 2, span, mnemonic)?;
            if is_register(&operands[0]) {
                Ok(vec![Instr::PutBitsReg(
                    resolve_reg(&operands[0])?,
                    field_bitcount(&operands[1], syms)?,
                )])
            } else {
                Ok(vec![Instr::PutBitsImm(
                    field_bitcount(&operands[0], syms)?,
                    field_u8(&operands[1], syms, "byte")?,
                )])
            }
        }
        "get_bits" => {
            args_exact(operands, 2, span, mnemonic)?;
            Ok(vec![Instr::GetBits(
                resolve_reg(&operands[0])?,
                field_bitcount(&operands[1], syms)?,
            )])
        }
        "tar" => {
            args_exact(operands, 1, span, mnemonic)?;
            if is_register(&operands[0]) {
                Ok(vec![Instr::TarReg(resolve_reg(&operands[0])?)])
            } else {
                Ok(vec![Instr::TarImm(field_tar4(&operands[0], syms)?)])
            }
        }
        "get_alert" => {
            args_exact(operands, 1, span, mnemonic)?;
            Ok(vec![Instr::GetAlert(resolve_reg(&operands[0])?)])
        }
        "halt" => {
            args_exact(operands, 1, span, mnemonic)?;
            Ok(vec![Instr::Halt(field_u8(
                &operands[0],
                syms,
                "halt status",
            )?)])
        }
        "wait_on" => {
            args_exact(operands, 3, span, mnemonic)?;
            Ok(vec![Instr::WaitOn(
                resolve_reg(&operands[0])?,
                field_wait_cond(&operands[1], syms)?,
                field_wait_timeout(&operands[2], syms)?,
            )])
        }
        "set_config" => Ok(vec![encode_set_config(operands, span)?]),
        "mark" => {
            args_exact(operands, 2, span, mnemonic)?;
            Ok(vec![Instr::Mark(
                field_mark_tag(&operands[0], syms)?,
                resolve_reg(&operands[1])?,
            )])
        }
        "load_imm" => {
            args_exact(operands, 2, span, mnemonic)?;
            Ok(vec![Instr::LoadImm(
                resolve_reg(&operands[0])?,
                field_imm11(&operands[1], syms)?,
            )])
        }
        "lui" => {
            args_exact(operands, 2, span, mnemonic)?;
            Ok(vec![Instr::Lui(
                resolve_reg(&operands[0])?,
                field_imm20(&operands[1], syms)?,
            )])
        }
        "mov" | "mv" => {
            args_exact(operands, 2, span, mnemonic)?;
            Ok(vec![Instr::Mov(
                resolve_reg(&operands[0])?,
                resolve_reg(&operands[1])?,
            )])
        }
        "add" => rrr(operands, span, mnemonic, Instr::Add),
        "sub" => rrr(operands, span, mnemonic, Instr::Sub),
        "and" => rrr(operands, span, mnemonic, Instr::And),
        "or" => rrr(operands, span, mnemonic, Instr::Or),
        "xor" => rrr(operands, span, mnemonic, Instr::Xor),
        "addi" => rri(operands, span, mnemonic, syms, Instr::Addi),
        "andi" => rri(operands, span, mnemonic, syms, Instr::Andi),
        "ori" => rri(operands, span, mnemonic, syms, Instr::Ori),
        "xori" => rri(operands, span, mnemonic, syms, Instr::Xori),
        "sll" => shift(operands, span, mnemonic, syms, ShiftOp::Sll),
        "srl" => shift(operands, span, mnemonic, syms, ShiftOp::Srl),
        "sra" => shift(operands, span, mnemonic, syms, ShiftOp::Sra),
        "rdsr" => {
            args_exact(operands, 2, span, mnemonic)?;
            Ok(vec![Instr::Rdsr(
                resolve_reg(&operands[0])?,
                resolve_sr(&operands[1], syms)?,
            )])
        }
        "beq" => branch(operands, span, mnemonic, addr, syms, Instr::Beq),
        "bne" => branch(operands, span, mnemonic, addr, syms, Instr::Bne),
        "bltu" => branch(operands, span, mnemonic, addr, syms, Instr::Bltu),
        "bgeu" => branch(operands, span, mnemonic, addr, syms, Instr::Bgeu),
        "nop" => {
            args_exact(operands, 0, span, mnemonic)?;
            Ok(vec![Instr::Addi(zero(), zero(), Imm11::new(0).unwrap())])
        }
        "li" => {
            args_exact(operands, 2, span, mnemonic)?;
            let rd = resolve_reg(&operands[0])?;
            let v = li_value(&operands[1], syms)?;
            Ok(tile_li(rd, v))
        }
        "j" => {
            args_exact(operands, 1, span, mnemonic)?;
            let off = resolve_branch_target(&operands[0], addr, syms)?;
            Ok(vec![Instr::Beq(zero(), zero(), off)])
        }
        "beqz" => {
            args_exact(operands, 2, span, mnemonic)?;
            let rs = resolve_reg(&operands[0])?;
            let off = resolve_branch_target(&operands[1], addr, syms)?;
            Ok(vec![Instr::Beq(rs, zero(), off)])
        }
        "bnez" => {
            args_exact(operands, 2, span, mnemonic)?;
            let rs = resolve_reg(&operands[0])?;
            let off = resolve_branch_target(&operands[1], addr, syms)?;
            Ok(vec![Instr::Bne(rs, zero(), off)])
        }
        "la" | "call" | "ret" => Err(Diagnostic::error(
            span.clone(),
            format!("`{mnemonic}` is not supported in tamal-asm v1"),
        )
        .with_help("v1 has no `la` or subroutine linkage (call/ret)")),
        other => Err(Diagnostic::error(
            span.clone(),
            format!("unknown instruction `{other}`"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lex;
    use crate::parser::{LineKind, Operand, OperandKind, parse};
    use crate::symbol::{Sym, SymbolTable};

    fn ident(s: &str) -> Operand {
        Operand {
            kind: OperandKind::Ident(s.into()),
            span: 0..s.len(),
        }
    }
    fn num(n: i64) -> Operand {
        Operand {
            kind: OperandKind::Num(n),
            span: 0..1,
        }
    }

    use tamal_abi::isa::{Imm11, Imm20, Instr, Reg};

    fn r(n: u8) -> Reg {
        Reg::new(n).unwrap()
    }
    fn i11(v: i32) -> Imm11 {
        Imm11::new((v as u32 & 0x7FF) as u16).unwrap()
    }

    #[test]
    fn tile_li_one_word_loadimm() {
        assert_eq!(tile_li(r(5), 0x0F), vec![Instr::LoadImm(r(5), i11(15))]);
        assert_eq!(tile_li(r(5), -1), vec![Instr::LoadImm(r(5), i11(-1))]);
    }

    #[test]
    fn tile_li_two_words_lui_addi() {
        // 0x1234: low12=0x234=564, hi=1, one addi
        assert_eq!(
            tile_li(r(5), 0x1234),
            vec![
                Instr::Lui(r(5), Imm20::new(1).unwrap()),
                Instr::Addi(r(5), r(5), i11(0x234))
            ]
        );
    }

    #[test]
    fn tile_li_near_range_seeds_load_imm_no_lui() {
        // 1024 is just past signed-11: load_imm 1023 + addi 1, and NO lui rd, 0
        assert_eq!(
            tile_li(r(5), 1024),
            vec![
                Instr::LoadImm(r(5), i11(1023)),
                Instr::Addi(r(5), r(5), i11(1))
            ]
        );
    }

    #[test]
    fn tile_li_gap_band_three_words() {
        // 0x1800: low12=0x800=2048 -> resid=-2048, hi=2, two addis of -1024
        assert_eq!(
            tile_li(r(5), 0x1800),
            vec![
                Instr::Lui(r(5), Imm20::new(2).unwrap()),
                Instr::Addi(r(5), r(5), i11(-1024)),
                Instr::Addi(r(5), r(5), i11(-1024)),
            ]
        );
    }

    #[test]
    fn tile_li_worst_case_four_words_reconstructs() {
        // low12 == 0x7FF at large magnitude -> lui + 3 addi
        let seq = tile_li(r(5), 0x0012_47FF);
        assert_eq!(seq.len(), 4);
        // reconstruct the value the engine would compute
        let mut acc: i64 = 0;
        for ins in &seq {
            match *ins {
                Instr::Lui(_, imm) => acc = (i64::from(imm.bits())) << 12,
                Instr::Addi(_, _, imm) => {
                    let s = imm.bits() as i32;
                    let sext = if s & 0x400 != 0 { s | !0x7FF } else { s };
                    acc = (acc + i64::from(sext)) & 0xFFFF_FFFF;
                }
                _ => panic!("unexpected instr in li tiling"),
            }
        }
        assert_eq!(acc as u32, 0x0012_47FF);
    }

    #[test]
    fn li_word_count_matches() {
        assert_eq!(li_word_count(0x0F), 1);
        assert_eq!(li_word_count(1024), 2);
        assert_eq!(li_word_count(0x1234), 2);
        assert_eq!(li_word_count(0x1800), 3);
        assert_eq!(li_word_count(0x0012_47FF), 4);
    }

    #[test]
    fn checked_range() {
        assert!(checked(255, 0, 255, &(0..1), "byte").is_ok());
        assert!(checked(256, 0, 255, &(0..1), "byte").is_err());
        assert!(checked(-1, 0, 255, &(0..1), "byte").is_err());
    }

    #[test]
    fn resolve_reg_abi_and_numeric() {
        assert_eq!(resolve_reg(&ident("t0")).unwrap().bits(), 5);
        assert_eq!(resolve_reg(&ident("x5")).unwrap().bits(), 5);
        assert_eq!(resolve_reg(&ident("zero")).unwrap().bits(), 0);
        assert_eq!(resolve_reg(&ident("a5")).unwrap().bits(), 15);
    }

    #[test]
    fn resolve_reg_rejects_x16_plus_with_help() {
        let e = resolve_reg(&ident("a6")).unwrap_err();
        assert!(e.message.contains("x16"));
        assert!(e.help.as_deref().unwrap().contains("x0-x15"));
        assert!(resolve_reg(&ident("x20")).is_err());
        assert!(resolve_reg(&num(5)).is_err());
    }

    #[test]
    fn resolve_imm_num_and_equ_and_errors() {
        let mut t = SymbolTable::new();
        t.insert("K", Sym::Equ(0x0F), 0..1).unwrap();
        t.insert("L", Sym::Label(2), 0..1).unwrap();
        assert_eq!(resolve_imm(&num(42), &t).unwrap(), 42);
        assert_eq!(resolve_imm(&ident("K"), &t).unwrap(), 0x0F);
        assert!(
            resolve_imm(&ident("L"), &t)
                .unwrap_err()
                .message
                .contains("label")
        );
        assert!(
            resolve_imm(&ident("nope"), &t)
                .unwrap_err()
                .message
                .contains("undefined")
        );
    }

    fn enc(src: &str, addr: u16, syms: &SymbolTable) -> Result<Vec<Instr>, Diagnostic> {
        let ls = parse(&lex(src).unwrap()).unwrap();
        for l in &ls {
            if let LineKind::Instr { mnemonic, operands } = &l.kind {
                return encode_line(mnemonic, operands, &l.span, addr, syms);
            }
        }
        panic!("no instruction in `{src}`");
    }

    fn empty() -> SymbolTable {
        SymbolTable::new()
    }

    #[test]
    fn bus_and_data_ops() {
        assert_eq!(
            enc("cs_assert\n", 0, &empty()).unwrap(),
            vec![Instr::CsAssert]
        );
        assert_eq!(
            enc("put_byte 0x64\n", 0, &empty()).unwrap(),
            vec![Instr::PutByteImm(0x64)]
        );
        assert_eq!(
            enc("put_byte t0\n", 0, &empty()).unwrap(),
            vec![Instr::PutByteReg(r(5))]
        );
        assert_eq!(
            enc("get_byte t0\n", 0, &empty()).unwrap(),
            vec![Instr::GetByte(r(5))]
        );
        assert_eq!(
            enc("tar 2\n", 0, &empty()).unwrap(),
            vec![Instr::TarImm(Tar4::new(2).unwrap())]
        );
        assert_eq!(
            enc("halt 0x11\n", 0, &empty()).unwrap(),
            vec![Instr::Halt(0x11)]
        );
        assert_eq!(
            enc("addi t0, t0, 0x0F\n", 0, &empty()).unwrap(),
            vec![Instr::Addi(r(5), r(5), i11(15))]
        );
        assert_eq!(
            enc("sra x5, x6, 3\n", 0, &empty()).unwrap(),
            vec![Instr::Shift(
                r(5),
                r(6),
                ShiftOp::Sra,
                Amt5::new(3).unwrap()
            )]
        );
        assert_eq!(
            enc("nop\n", 0, &empty()).unwrap(),
            vec![Instr::Addi(r(0), r(0), i11(0))]
        );
        assert_eq!(
            enc("mv t0, t1\n", 0, &empty()).unwrap(),
            vec![Instr::Mov(r(5), r(6))]
        );
    }

    #[test]
    fn set_config_v1_and_rejects_non_v1() {
        // controller / x1 / sck20 / alert_pin packs to 0x00 -> word 0x58000000
        assert_eq!(
            enc("set_config controller, x1, sck20, alert_pin\n", 0, &empty()).unwrap()[0].encode(),
            0x5800_0000
        );
        let e = enc("set_config controller, x2, sck20, alert_pin\n", 0, &empty()).unwrap_err();
        assert!(e.message.contains("x2"));
    }

    #[test]
    fn rdsr_crc() {
        assert_eq!(
            enc("rdsr t2, CRC\n", 0, &empty()).unwrap(),
            vec![Instr::Rdsr(r(7), Sr5::new(0).unwrap())]
        );
    }

    #[test]
    fn branches_and_pseudo_offsets() {
        let mut t = empty();
        t.insert("here", Sym::Label(2), 0..1).unwrap();
        // beq at addr 0 -> off = 2 - 0 = 2
        assert_eq!(
            enc("beq t0, t1, here\n", 0, &t).unwrap(),
            vec![Instr::Beq(r(5), r(6), i11(2))]
        );
        // j at addr 5 -> off = 2 - 5 = -3
        assert_eq!(
            enc("j here\n", 5, &t).unwrap(),
            vec![Instr::Beq(r(0), r(0), i11(-3))]
        );
        // bnez at addr 4 -> off = 2 - 4 = -2
        assert_eq!(
            enc("bnez t2, here\n", 4, &t).unwrap(),
            vec![Instr::Bne(r(7), r(0), i11(-2))]
        );
    }

    #[test]
    fn error_cases() {
        assert!(
            enc("add x20, x0, x0\n", 0, &empty())
                .unwrap_err()
                .message
                .contains("x20")
        );
        assert!(
            enc("put_byte 0x100\n", 0, &empty())
                .unwrap_err()
                .message
                .contains("range")
        );
        assert!(
            enc("frobnicate\n", 0, &empty())
                .unwrap_err()
                .message
                .contains("unknown instruction")
        );
        assert!(
            enc("beq t0, t1, missing\n", 0, &empty())
                .unwrap_err()
                .message
                .contains("undefined label")
        );
    }
}
