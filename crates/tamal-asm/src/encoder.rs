//! Lowering: mnemonics + pseudo-ops -> tamal_abi::isa::Instr, with resolution,
//! `li` tiling, branch offsets, and range checks.

use tamal_abi::isa::Reg;

use crate::diagnostics::Diagnostic;
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
