//! Compile-time evaluation: fold a `parser::Expr` to a `Value` (an integer or a
//! byte string). `crc8` delegates to `tamal_abi::crc8`, so a folded CRC byte is
//! the exact value the wire and HDL use — it can never drift. Pure: no
//! wall-clock, no process environment, no randomness, so identical source folds
//! to identical bytes.

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

/// The compile-time environment threaded through evaluation: the module's
/// `const`s (the base scope) plus the parameters bound by the innermost
/// `fn`/`proc` expansion.
///
/// Lookup is **locals first, then consts**, and a call body is evaluated in a
/// child built from the module base — never from the caller's locals — so
/// scoping is lexical: a callee sees its own parameters and the module's
/// constants, and nothing else.
///
/// A plain [`Clone`] copies `locals` verbatim, so it is *not* how you build a
/// callee scope — use the call-scope constructors for that.
#[derive(Debug, Clone, Default)]
pub struct Env {
    /// Module-level `const`s: the base scope every call body starts from.
    consts: HashMap<String, Value>,
    /// Parameters bound by the innermost call. Empty at module level; filled by
    /// `fn`/`proc` expansion. Nothing populates it yet — `fn`/`proc` expansion
    /// will.
    locals: HashMap<String, Value>,
}

impl Env {
    /// An empty environment.
    pub fn new() -> Self {
        Self::default()
    }

    /// Define a module-level `const`.
    ///
    /// Overwrites silently, like the [`HashMap::insert`] it wraps: the
    /// no-duplicate-`const` rule is a *caller's* obligation, enforced by
    /// checking [`Env::has_const`] first (see `lower` in `lib.rs`). Checking
    /// before inserting is load-bearing — it keeps the duplicate diagnostic
    /// winning over any error from evaluating the duplicate's value.
    pub fn insert_const(&mut self, name: String, value: Value) {
        self.consts.insert(name, value);
    }

    /// Is `name` already defined as a module-level `const`?
    pub fn has_const(&self, name: &str) -> bool {
        self.consts.contains_key(name)
    }

    /// The value bound to `name`: the innermost parameter binding if there is
    /// one, else a module `const`.
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.locals.get(name).or_else(|| self.consts.get(name))
    }
}

/// Fold an expression to a [`Value`] under the given environment.
pub fn eval(e: &Expr, env: &Env) -> Result<Value, Diagnostic> {
    match e {
        Expr::Int { value, .. } => Ok(Value::Int(*value)),
        Expr::Name { name, span } => env
            .get(name)
            .cloned()
            .ok_or_else(|| Diagnostic::error(span.clone(), format!("unknown name `{name}`"))),
        Expr::Bytes { elems, .. } => {
            let mut out = Vec::with_capacity(elems.len());
            for el in elems {
                out.push(eval_byte(el, env)?);
            }
            Ok(Value::Bytes(out))
        }
        Expr::Binary {
            op: BinOp::Concat,
            lhs,
            rhs,
            ..
        } => {
            let mut a = eval_bytes(lhs, env)?;
            a.extend(eval_bytes(rhs, env)?);
            Ok(Value::Bytes(a))
        }
        Expr::Binary {
            op: BinOp::Xor,
            lhs,
            rhs,
            ..
        } => Ok(Value::Int(eval_int(lhs, env)? ^ eval_int(rhs, env)?)),
        Expr::Call { func, arg, span } => eval_call(func, arg, span, env),
    }
}

fn eval_call(func: &str, arg: &Expr, span: &Span, env: &Env) -> Result<Value, Diagnostic> {
    match func {
        "crc8" => Ok(Value::Int(
            tamal_abi::crc8::crc8(&eval_bytes(arg, env)?) as i64
        )),
        "len" => Ok(Value::Int(eval_bytes(arg, env)?.len() as i64)),
        "lo" => Ok(Value::Int(eval_int(arg, env)? & 0xff)),
        "hi" => Ok(Value::Int((eval_int(arg, env)? >> 8) & 0xff)),
        _ => Err(
            Diagnostic::error(span.clone(), format!("unknown builtin `{func}`"))
                .with_help("the builtins are crc8, len, lo, hi"),
        ),
    }
}

/// Evaluate `e` to an integer.
pub fn eval_int(e: &Expr, env: &Env) -> Result<i64, Diagnostic> {
    match eval(e, env)? {
        Value::Int(n) => Ok(n),
        Value::Bytes(_) => Err(Diagnostic::error(
            e.span(),
            "expected an integer, found bytes",
        )),
    }
}

/// Evaluate `e` to a byte string.
pub fn eval_bytes(e: &Expr, env: &Env) -> Result<Vec<u8>, Diagnostic> {
    match eval(e, env)? {
        Value::Bytes(b) => Ok(b),
        Value::Int(_) => Err(Diagnostic::error(
            e.span(),
            "expected bytes, found an integer",
        )),
    }
}

/// Evaluate `e` to a single byte (an integer in `0..=255`).
pub fn eval_byte(e: &Expr, env: &Env) -> Result<u8, Diagnostic> {
    let n = eval_int(e, env)?;
    u8::try_from(n)
        .map_err(|_| Diagnostic::error(e.span(), format!("byte value {n} is out of range 0..=255")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn int(v: i64) -> Expr {
        Expr::Int {
            value: v,
            span: 0..0,
        }
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
        assert_eq!(eval(&e, &Env::new()).unwrap(), Value::Int(0xE9));
    }

    #[test]
    fn resolves_and_rejects_names() {
        let mut env = Env::new();
        env.insert_const("X".into(), Value::Int(0x44));
        assert_eq!(
            eval(
                &Expr::Name {
                    name: "X".into(),
                    span: 0..0
                },
                &env
            )
            .unwrap(),
            Value::Int(0x44)
        );
        assert!(
            eval(
                &Expr::Name {
                    name: "NOPE".into(),
                    span: 0..0
                },
                &env
            )
            .is_err()
        );
    }

    #[test]
    fn folds_bytes_and_concat() {
        let e = Expr::Binary {
            op: BinOp::Concat,
            lhs: Box::new(bytes(&[0x01, 0x02])),
            rhs: Box::new(bytes(&[0x03])),
            span: 0..0,
        };
        assert_eq!(eval(&e, &Env::new()).unwrap(), Value::Bytes(vec![1, 2, 3]));
    }

    #[test]
    fn byte_out_of_range_errors() {
        assert!(eval(&bytes(&[0x100]), &Env::new()).is_err());
    }

    #[test]
    fn crc8_folds_peripheral_command_bytes() {
        // crc8([0x44, 0x00, 0x64]) == 0x16 (matches examples/peripheral_io_read.s)
        let e = call("crc8", bytes(&[0x44, 0x00, 0x64]));
        assert_eq!(eval(&e, &Env::new()).unwrap(), Value::Int(0x16));
    }

    #[test]
    fn len_lo_hi_builtins() {
        assert_eq!(
            eval(&call("len", bytes(&[1, 2, 3])), &Env::new()).unwrap(),
            Value::Int(3)
        );
        assert_eq!(
            eval(&call("lo", int(0xDEAD)), &Env::new()).unwrap(),
            Value::Int(0xAD)
        );
        assert_eq!(
            eval(&call("hi", int(0xDEAD)), &Env::new()).unwrap(),
            Value::Int(0xDE)
        );
    }

    #[test]
    fn type_mismatch_and_unknown_builtin_error() {
        assert!(eval(&call("crc8", int(5)), &Env::new()).is_err()); // crc8 needs bytes
        assert!(eval(&call("nope", int(5)), &Env::new()).is_err()); // unknown builtin
    }
}
