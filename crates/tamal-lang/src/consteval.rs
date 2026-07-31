//! Compile-time evaluation: fold a `parser::Expr` to a `Value` (an integer or a
//! byte string). `crc8` delegates to `tamal_abi::crc8`, so a folded CRC byte is
//! the exact value the wire and HDL use — it can never drift. Pure: no
//! wall-clock, no process environment, no randomness, so identical source folds
//! to identical bytes.

use crate::parser::{Arg, BinOp, Expr, Param, Type};
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
    /// Parameters bound by the innermost call: the callee's own scope, shadowing
    /// the module `const`s. Empty at module level; populated per call by
    /// [`Env::child_for_call_values`].
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

    /// This environment with no parameter bindings — the module scope.
    ///
    /// A parameter default is written at the callee's *definition* site, so it
    /// is evaluated here rather than in the caller's scope: a default must
    /// never be able to capture a caller local that happens to share its name.
    pub fn module_scope(&self) -> Env {
        let mut e = self.clone();
        e.locals.clear();
        e
    }

    /// This environment with `bindings` as its parameter scope, replacing any
    /// locals. The module `const`s survive; the caller's locals do not.
    pub fn child_for_call_values(&self, bindings: HashMap<String, Value>) -> Env {
        let mut e = self.clone();
        e.locals = bindings;
        e
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
        Expr::Call { func, args, span } => eval_call(func, args, span, env),
    }
}

fn eval_call(func: &str, args: &[Arg], span: &Span, env: &Env) -> Result<Value, Diagnostic> {
    match func {
        "crc8" => {
            let b = eval_bytes(builtin_arg(func, args, span)?, env)?;
            Ok(Value::Int(tamal_abi::crc8::crc8(&b) as i64))
        }
        "len" => Ok(Value::Int(
            eval_bytes(builtin_arg(func, args, span)?, env)?.len() as i64,
        )),
        "lo" => Ok(Value::Int(
            eval_int(builtin_arg(func, args, span)?, env)? & 0xff,
        )),
        "hi" => Ok(Value::Int(
            (eval_int(builtin_arg(func, args, span)?, env)? >> 8) & 0xff,
        )),
        _ => Err(
            Diagnostic::error(span.clone(), format!("unknown builtin `{func}`"))
                .with_help("the builtins are crc8, len, lo, hi"),
        ),
    }
}

/// The single positional argument of a builtin call. The builtins take exactly
/// one positional argument; anything else is a diagnostic (the arity is part of
/// their contract, not something a caller may vary).
fn builtin_arg<'a>(func: &str, args: &'a [Arg], call_span: &Span) -> Result<&'a Expr, Diagnostic> {
    if args.len() != 1 {
        return Err(Diagnostic::error(
            call_span.clone(),
            format!("`{func}` takes 1 argument, got {}", args.len()),
        ));
    }
    if let Some(name) = &args[0].name {
        return Err(Diagnostic::error(
            args[0].span.clone(),
            format!("`{func}` does not take named arguments (found `{name} = …`)"),
        ));
    }
    Ok(&args[0].value)
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

/// Bind a call's arguments to a callable's parameters.
///
/// Positional arguments bind parameters in declaration order; named
/// (`name = value`) arguments bind by name and must follow every positional
/// one; each parameter still unbound takes its default, or is an error if it
/// has none. Every bound value is checked against its declared type.
///
/// Shared by `fn` (evaluated here) and `proc` (expanded by the emitter) so the
/// two callables can never drift apart. Errors are produced by walking `params`
/// and `args` **in order**, so the same source always yields the same first
/// diagnostic — the determinism rule applies to error output too.
pub fn bind_args(
    callee: &str,
    params: &[Param],
    args: &[Arg],
    env: &Env,
    call_span: &Span,
) -> Result<HashMap<String, Value>, Diagnostic> {
    let mut bound: HashMap<String, Value> = HashMap::new();
    let mut seen_named = false;
    for (i, arg) in args.iter().enumerate() {
        let p = match &arg.name {
            None => {
                if seen_named {
                    return Err(Diagnostic::error(
                        arg.span.clone(),
                        "a positional argument cannot follow a named argument",
                    )
                    .with_help("pass the positional arguments first, or name them all"));
                }
                params.get(i).ok_or_else(|| {
                    Diagnostic::error(
                        arg.span.clone(),
                        format!(
                            "`{callee}` takes {} argument(s), got {}",
                            params.len(),
                            args.len()
                        ),
                    )
                })?
            }
            Some(name) => {
                seen_named = true;
                params.iter().find(|p| &p.name == name).ok_or_else(|| {
                    Diagnostic::error(
                        arg.span.clone(),
                        format!("`{callee}` has no parameter `{name}`"),
                    )
                })?
            }
        };
        if bound.contains_key(&p.name) {
            return Err(Diagnostic::error(
                arg.span.clone(),
                format!("argument `{}` of `{callee}` is bound twice", p.name),
            ));
        }
        let v = eval(&arg.value, env)?;
        check_type(
            &v,
            p.ty,
            &arg.span,
            &format!("argument `{}` of `{callee}`", p.name),
        )?;
        bound.insert(p.name.clone(), v);
    }
    // Fill the rest from defaults, in declaration order so that the first
    // missing parameter reported is stable across runs. Every default is
    // evaluated in the same module scope, so it is built once here rather than
    // cloned per defaulted parameter.
    let module = env.module_scope();
    for p in params {
        if bound.contains_key(&p.name) {
            continue;
        }
        let Some(default) = &p.default else {
            return Err(Diagnostic::error(
                call_span.clone(),
                format!("`{callee}` is missing a value for parameter `{}`", p.name),
            ));
        };
        let v = eval(default, &module)?;
        check_type(
            &v,
            p.ty,
            &p.span,
            &format!("the default for `{}` of `{callee}`", p.name),
        )?;
        bound.insert(p.name.clone(), v);
    }
    Ok(bound)
}

/// Check a value against a declared type; `what` names the position for the
/// diagnostic. A `byte` additionally range-checks `0..=255` — out of range is
/// an error, never a silent wrap.
fn check_type(v: &Value, ty: Type, span: &Span, what: &str) -> Result<(), Diagnostic> {
    let ok = match (ty, v) {
        (Type::Bytes, Value::Bytes(_)) => true,
        (Type::Int, Value::Int(_)) => true,
        (Type::Byte, Value::Int(n)) => (0..=255).contains(n),
        _ => false,
    };
    if ok {
        return Ok(());
    }
    let found = match v {
        Value::Int(n) => format!("the integer {n}"),
        Value::Bytes(b) => format!("{} byte(s)", b.len()),
    };
    Err(Diagnostic::error(
        span.clone(),
        format!("{what} expects `{}`, found {found}", ty.name()),
    ))
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
            args: vec![Arg {
                name: None,
                value: arg,
                span: 0..0,
            }],
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

    #[test]
    fn builtins_require_exactly_one_positional_argument() {
        // zero args
        let none = Expr::Call {
            func: "crc8".into(),
            args: vec![],
            span: 0..0,
        };
        assert!(
            eval(&none, &Env::new())
                .unwrap_err()
                .message
                .contains("takes 1 argument")
        );
        // two args
        let two = Expr::Call {
            func: "crc8".into(),
            args: vec![
                Arg {
                    name: None,
                    value: bytes(&[1]),
                    span: 0..0,
                },
                Arg {
                    name: None,
                    value: bytes(&[2]),
                    span: 0..0,
                },
            ],
            span: 0..0,
        };
        assert!(
            eval(&two, &Env::new())
                .unwrap_err()
                .message
                .contains("takes 1 argument")
        );
        // a named argument
        let named = Expr::Call {
            func: "crc8".into(),
            args: vec![Arg {
                name: Some("b".into()),
                value: bytes(&[1]),
                span: 0..0,
            }],
            span: 0..0,
        };
        assert!(
            eval(&named, &Env::new())
                .unwrap_err()
                .message
                .contains("does not take named arguments")
        );
    }

    /// Distinct, recognisable spans for the binder tests. A diagnostic's anchor
    /// is user-facing — `Emitter::remap` maps it back to `.tam` source, so a
    /// wrong anchor puts the user's caret on the wrong line. Every helper below
    /// stamps its own span from one of these, and none of them is `0..0`, so a
    /// mis-anchored diagnostic can never match by coincidence.
    fn call_span() -> Span {
        900..901
    }
    fn arg_span(i: usize) -> Span {
        100 + i..101 + i
    }
    fn param_span(i: usize) -> Span {
        200 + i..201 + i
    }

    fn param(name: &str, ty: Type, default: Option<Expr>, span: Span) -> Param {
        Param {
            name: name.into(),
            ty,
            default,
            span,
        }
    }

    fn pos(value: Expr) -> Arg {
        Arg {
            name: None,
            value,
            span: 0..0,
        }
    }

    fn named(name: &str, value: Expr) -> Arg {
        Arg {
            name: Some(name.into()),
            value,
            span: 0..0,
        }
    }

    /// Give each argument its own `arg_span(i)`, so a diagnostic anchored on an
    /// argument identifies *which* one.
    fn stamped(mut args: Vec<Arg>) -> Vec<Arg> {
        for (i, a) in args.iter_mut().enumerate() {
            a.span = arg_span(i);
        }
        args
    }

    /// `(pkt: bytes, ndata: int, err: byte = 0x11)` — the shape of the library
    /// `command` proc, which is what this binder exists to serve.
    fn command_params() -> Vec<Param> {
        vec![
            param("pkt", Type::Bytes, None, param_span(0)),
            param("ndata", Type::Int, None, param_span(1)),
            param("err", Type::Byte, Some(int(0x11)), param_span(2)),
        ]
    }

    fn bind_ok(args: Vec<Arg>) -> HashMap<String, Value> {
        bind_args(
            "command",
            &command_params(),
            &stamped(args),
            &Env::new(),
            &call_span(),
        )
        .unwrap()
    }

    fn bind_diag(args: Vec<Arg>) -> Diagnostic {
        bind_args(
            "command",
            &command_params(),
            &stamped(args),
            &Env::new(),
            &call_span(),
        )
        .unwrap_err()
    }

    fn bind_err(args: Vec<Arg>) -> String {
        bind_diag(args).message
    }

    #[test]
    fn binds_positional_arguments_in_order() {
        let b = bind_ok(vec![pos(bytes(&[0x44])), pos(int(1))]);
        assert_eq!(b["pkt"], Value::Bytes(vec![0x44]));
        assert_eq!(b["ndata"], Value::Int(1));
    }

    #[test]
    fn binds_named_arguments_by_name() {
        let b = bind_ok(vec![named("ndata", int(2)), named("pkt", bytes(&[0x06]))]);
        assert_eq!(b["pkt"], Value::Bytes(vec![0x06]));
        assert_eq!(b["ndata"], Value::Int(2));
    }

    #[test]
    fn unbound_parameters_take_their_default() {
        let b = bind_ok(vec![pos(bytes(&[0x44])), pos(int(0))]);
        assert_eq!(b["err"], Value::Int(0x11));
    }

    #[test]
    fn an_explicit_argument_overrides_a_default() {
        let b = bind_ok(vec![
            pos(bytes(&[0x44])),
            pos(int(0)),
            named("err", int(0x22)),
        ]);
        assert_eq!(b["err"], Value::Int(0x22));
    }

    #[test]
    fn a_positional_argument_may_not_follow_a_named_one() {
        let msg = bind_err(vec![named("pkt", bytes(&[0x44])), pos(int(0))]);
        assert!(msg.contains("cannot follow a named argument"), "got: {msg}");
    }

    #[test]
    fn rejects_unknown_duplicate_and_surplus_arguments() {
        let unknown = bind_err(vec![named("nope", int(0))]);
        assert!(unknown.contains("no parameter `nope`"), "got: {unknown}");

        let dup = bind_err(vec![pos(bytes(&[0x44])), named("pkt", bytes(&[0x06]))]);
        assert!(dup.contains("bound twice"), "got: {dup}");

        let surplus = bind_err(vec![
            pos(bytes(&[0x44])),
            pos(int(0)),
            pos(int(0x11)),
            pos(int(9)),
        ]);
        assert!(surplus.contains("takes 3 argument"), "got: {surplus}");
        assert!(surplus.contains("got 4"), "got: {surplus}");
    }

    #[test]
    fn a_missing_required_parameter_is_reported_in_declaration_order() {
        // Neither `pkt` nor `ndata` is bound; the FIRST declared one is named,
        // deterministically (params are a Vec, never a HashMap).
        let d = bind_diag(vec![]);
        assert!(d.message.contains("`pkt`"), "got: {}", d.message);
        // There is no argument to point at, so this one anchors on the call.
        assert_eq!(d.primary, call_span());
    }

    #[test]
    fn arguments_are_checked_against_their_declared_type() {
        let wrong = bind_diag(vec![pos(int(5)), pos(int(0))]); // pkt: bytes
        assert!(
            wrong.message.contains("expects `bytes`"),
            "got: {}",
            wrong.message
        );
        assert_eq!(wrong.primary, arg_span(0), "anchored on the bad argument");

        let too_big = bind_diag(vec![
            pos(bytes(&[0x44])),
            pos(int(0)),
            named("err", int(256)),
        ]);
        assert!(
            too_big.message.contains("expects `byte`"),
            "got: {}",
            too_big.message
        );
        assert_eq!(too_big.primary, arg_span(2), "anchored on the bad argument");
    }

    #[test]
    fn an_argument_is_evaluated_in_the_callers_scope() {
        // The mirror of the default rule below, and the more dangerous half: an
        // argument expression is written at the *call* site, so it must resolve
        // against the caller's locals. Evaluating it in module scope instead
        // would break every nested call — `proc outer(x: int) { command(ndata =
        // x) }` would fail with `unknown name `x`` — while leaving the rest of
        // the suite green.
        let caller = Env::new()
            .child_for_call_values([("x".to_string(), Value::Int(7))].into_iter().collect());
        let args = stamped(vec![
            pos(bytes(&[0x44])),
            pos(Expr::Name {
                name: "x".into(),
                span: 0..0,
            }),
        ]);
        let b = bind_args("command", &command_params(), &args, &caller, &call_span()).unwrap();
        assert_eq!(b["ndata"], Value::Int(7));
    }

    #[test]
    fn a_default_is_checked_against_the_declared_type() {
        // A default is as much a source of a wrong value as an argument is, so
        // it gets the same type check — anchored on the *parameter*, since that
        // is the declaration the author must fix.
        let params = vec![param("err", Type::Byte, Some(int(300)), param_span(2))];
        let d = bind_args("p", &params, &[], &Env::new(), &call_span()).unwrap_err();
        assert!(d.message.contains("the default for"), "got: {}", d.message);
        assert!(d.message.contains("expects `byte`"), "got: {}", d.message);
        assert_eq!(d.primary, param_span(2), "anchored on the parameter");
    }

    #[test]
    fn a_default_is_evaluated_in_module_scope_not_the_callers() {
        // The caller has a local `K`; the callee's default refers to `K` too.
        // Lexical scoping means the default must NOT see the caller's binding —
        // here there is no module `K`, so it is an "unknown name" error rather
        // than silently picking up 0x99.
        let mut caller = Env::new();
        caller.insert_const("BASE".into(), Value::Int(1));
        let caller = caller
            .child_for_call_values([("K".to_string(), Value::Int(0x99))].into_iter().collect());
        let params = vec![param(
            "err",
            Type::Byte,
            Some(Expr::Name {
                name: "K".into(),
                span: 0..0,
            }),
            param_span(0),
        )];
        let err = bind_args("p", &params, &[], &caller, &call_span()).unwrap_err();
        assert!(
            err.message.contains("unknown name `K`"),
            "got: {}",
            err.message
        );
    }

    #[test]
    fn a_parameter_shadows_a_module_const_of_the_same_name() {
        // `Env::get` is locals-first: inside a call body, a parameter named like
        // a module `const` wins. Reversing that lookup order would silently make
        // every same-named parameter invisible.
        let mut module = Env::new();
        module.insert_const("N".into(), Value::Int(1));
        let inside_call =
            module.child_for_call_values([("N".to_string(), Value::Int(2))].into_iter().collect());
        let n = Expr::Name {
            name: "N".into(),
            span: 0..0,
        };
        assert_eq!(eval(&n, &module).unwrap(), Value::Int(1));
        assert_eq!(eval(&n, &inside_call).unwrap(), Value::Int(2));
    }
}
