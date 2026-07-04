//! Lowering: mnemonics + pseudo-ops -> tamal_abi::isa::Instr, with resolution,
//! `li` tiling, branch offsets, and range checks.

use tamal_abi::isa::{Imm11, Imm20, Instr, Reg};

use crate::diagnostics::{Diagnostic, Span};
use crate::parser::{Operand, OperandKind};
use crate::symbol::{Sym, SymbolTable};

/// Map a register name (`xN` or an ABI name) to its x-number, or `None`.
// `allow(dead_code)`: the lowering pass that calls these lands in a later task;
// remove the attributes once `resolve_*` are wired into instruction encoding.
#[allow(dead_code)]
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
#[allow(dead_code)]
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
#[allow(dead_code)]
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
// `allow(dead_code)`: the lowering pass that calls the `li`/range helpers lands
// in a later task; remove the attributes once they are wired into encoding.
#[allow(dead_code)]
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
#[allow(dead_code)]
fn imm11(v: i32) -> Imm11 {
    Imm11::new((v as u32 & 0x7FF) as u16).expect("11-bit pattern always fits")
}

/// Build the `li` instruction sequence for any 32-bit `value` (1–4 instructions),
/// minimal — one `load_imm` for signed-11 values, and never a dead `lui rd, 0`.
#[allow(dead_code)]
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
#[allow(dead_code)]
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
#[allow(dead_code)]
pub(crate) fn li_word_count(value: i32) -> u16 {
    tile_li(Reg::new(0).expect("x0"), value).len() as u16
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::{Operand, OperandKind};
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
}
