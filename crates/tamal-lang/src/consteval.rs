//! Compile-time evaluation: fold a `parser::Expr` to a `Value` (an integer or a
//! byte string). `crc8` delegates to `tamal_abi::crc8`, so a folded CRC byte is
//! the exact value the wire and HDL use — it can never drift. Pure: no clock,
//! environment, or randomness, so identical source folds to identical bytes.

use crate::parser::{BinOp, Expr};
use std::collections::HashMap;
use tamal_asm::{Diagnostic, Span};

/// A folded compile-time value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    /// An integer (byte values are `Int`s range-checked at use sites).
    Int(i64),
    /// A byte string.
    Bytes(Vec<u8>),
}

/// The resolved `const` environment, keyed by name.
pub type Consts = HashMap<String, Value>;

/// Fold an expression to a [`Value`] under the given `const` environment.
pub fn eval(e: &Expr, consts: &Consts) -> Result<Value, Diagnostic> {
    match e {
        Expr::Int { value, .. } => Ok(Value::Int(*value)),
        Expr::Name { name, span } => consts
            .get(name)
            .cloned()
            .ok_or_else(|| Diagnostic::error(span.clone(), format!("unknown name `{name}`"))),
        Expr::Bytes { elems, .. } => {
            let mut out = Vec::with_capacity(elems.len());
            for el in elems {
                out.push(eval_byte(el, consts)?);
            }
            Ok(Value::Bytes(out))
        }
        Expr::Binary { op: BinOp::Concat, lhs, rhs, .. } => {
            let mut a = eval_bytes(lhs, consts)?;
            a.extend(eval_bytes(rhs, consts)?);
            Ok(Value::Bytes(a))
        }
        Expr::Binary { op: BinOp::Xor, lhs, rhs, .. } => {
            Ok(Value::Int(eval_int(lhs, consts)? ^ eval_int(rhs, consts)?))
        }
        Expr::Call { func, arg, span } => eval_call(func, arg, span, consts),
    }
}

fn eval_call(func: &str, arg: &Expr, span: &Span, consts: &Consts) -> Result<Value, Diagnostic> {
    match func {
        "crc8" => Ok(Value::Int(tamal_abi::crc8::crc8(&eval_bytes(arg, consts)?) as i64)),
        "len" => Ok(Value::Int(eval_bytes(arg, consts)?.len() as i64)),
        "lo" => Ok(Value::Int(eval_int(arg, consts)? & 0xff)),
        "hi" => Ok(Value::Int((eval_int(arg, consts)? >> 8) & 0xff)),
        _ => Err(Diagnostic::error(span.clone(), format!("unknown builtin `{func}`"))
            .with_help("the builtins are crc8, len, lo, hi")),
    }
}

/// Evaluate `e` to an integer.
pub fn eval_int(e: &Expr, consts: &Consts) -> Result<i64, Diagnostic> {
    match eval(e, consts)? {
        Value::Int(n) => Ok(n),
        Value::Bytes(_) => Err(Diagnostic::error(e.span(), "expected an integer, found bytes")),
    }
}

/// Evaluate `e` to a byte string.
pub fn eval_bytes(e: &Expr, consts: &Consts) -> Result<Vec<u8>, Diagnostic> {
    match eval(e, consts)? {
        Value::Bytes(b) => Ok(b),
        Value::Int(_) => Err(Diagnostic::error(e.span(), "expected bytes, found an integer")),
    }
}

/// Evaluate `e` to a single byte (an integer in `0..=255`).
pub fn eval_byte(e: &Expr, consts: &Consts) -> Result<u8, Diagnostic> {
    let n = eval_int(e, consts)?;
    u8::try_from(n)
        .map_err(|_| Diagnostic::error(e.span(), format!("byte value {n} is out of range 0..=255")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn int(v: i64) -> Expr {
        Expr::Int { value: v, span: 0..0 }
    }
    fn bytes(vs: &[i64]) -> Expr {
        Expr::Bytes {
            elems: vs.iter().map(|&v| int(v)).collect(),
            span: 0..0,
        }
    }
    fn call(func: &str, arg: Expr) -> Expr {
        Expr::Call {
            func: func.into(),
            arg: Box::new(arg),
            span: 0..0,
        }
    }

    #[test]
    fn folds_int_and_xor() {
        let e = Expr::Binary {
            op: BinOp::Xor,
            lhs: Box::new(int(0x16)),
            rhs: Box::new(int(0xFF)),
            span: 0..0,
        };
        assert_eq!(eval(&e, &Consts::new()).unwrap(), Value::Int(0xE9));
    }

    #[test]
    fn resolves_and_rejects_names() {
        let mut c = Consts::new();
        c.insert("X".into(), Value::Int(0x44));
        assert_eq!(
            eval(&Expr::Name { name: "X".into(), span: 0..0 }, &c).unwrap(),
            Value::Int(0x44)
        );
        assert!(eval(&Expr::Name { name: "NOPE".into(), span: 0..0 }, &c).is_err());
    }

    #[test]
    fn folds_bytes_and_concat() {
        let e = Expr::Binary {
            op: BinOp::Concat,
            lhs: Box::new(bytes(&[0x01, 0x02])),
            rhs: Box::new(bytes(&[0x03])),
            span: 0..0,
        };
        assert_eq!(eval(&e, &Consts::new()).unwrap(), Value::Bytes(vec![1, 2, 3]));
    }

    #[test]
    fn byte_out_of_range_errors() {
        assert!(eval(&bytes(&[0x100]), &Consts::new()).is_err());
    }

    #[test]
    fn crc8_folds_peripheral_command_bytes() {
        // crc8([0x44, 0x00, 0x64]) == 0x16 (matches examples/peripheral_io_read.s)
        let e = call("crc8", bytes(&[0x44, 0x00, 0x64]));
        assert_eq!(eval(&e, &Consts::new()).unwrap(), Value::Int(0x16));
    }

    #[test]
    fn len_lo_hi_builtins() {
        assert_eq!(eval(&call("len", bytes(&[1, 2, 3])), &Consts::new()).unwrap(), Value::Int(3));
        assert_eq!(eval(&call("lo", int(0xDEAD)), &Consts::new()).unwrap(), Value::Int(0xAD));
        assert_eq!(eval(&call("hi", int(0xDEAD)), &Consts::new()).unwrap(), Value::Int(0xDE));
    }

    #[test]
    fn type_mismatch_and_unknown_builtin_error() {
        assert!(eval(&call("crc8", int(5)), &Consts::new()).is_err()); // crc8 needs bytes
        assert!(eval(&call("nope", int(5)), &Consts::new()).is_err()); // unknown builtin
    }
}
