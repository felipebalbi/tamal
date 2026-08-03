//! Compile-time evaluation: fold a `parser::Expr` to a `Value` (an integer or a
//! byte string). `crc8` delegates to `tamal_abi::crc8`, so a folded CRC byte is
//! the exact value the wire and HDL use — it can never drift. Pure: no
//! wall-clock, no process environment, no randomness, so identical source folds
//! to identical bytes.
//!
//! This module also owns the semantics both callables share: [`Env`], the
//! lexical scope model (a module's `const`s plus the innermost call's
//! parameters, looked up locals-first so a parameter shadows a same-named
//! `const`, with defaults evaluated in module scope rather than the caller's
//! *and* with the callee counted as already being expanded, so a
//! self-referential default is reported instead of looped on), and
//! [`bind_args`], the argument binder that `fn` and `proc` share so the two can
//! never drift apart.

use crate::parser::{Arg, BinOp, Expr, FnDef, Param, Type};
use std::collections::HashMap;
use std::rc::Rc;
use tamal_asm::{Diagnostic, Span};

/// A folded compile-time value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    /// An integer (byte values are `Int`s range-checked at use sites).
    Int(i64),
    /// A byte string.
    Bytes(Vec<u8>),
}

/// The compile-time builtins. A user `fn` may not take one of these names — a
/// builtin must always mean the same thing.
pub const BUILTINS: &[&str] = &["crc8", "len", "lo", "hi"];

/// The compile-time environment threaded through evaluation: the module's
/// `const`s (the base scope) plus the parameters bound by the innermost
/// `fn`/`proc` expansion. It also carries the module's `fn` table and the chain
/// of expansions currently in progress, which is what makes recursion
/// detectable.
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
    ///
    /// Behind an [`Rc`] for the same reason as `fns`, and it matters more: a
    /// `const` may hold a `Value::Bytes`, so a deep clone reallocates every
    /// byte string in the module on every call.
    consts: Rc<HashMap<String, Value>>,
    /// Parameters bound by the innermost call: the callee's own scope, shadowing
    /// the module `const`s. Empty at module level; populated per call by
    /// [`Env::child_for_call_values`].
    locals: HashMap<String, Value>,
    /// The module's `fn` table; calls resolve against it.
    ///
    /// Behind an [`Rc`] because `Env` is cloned twice per call (once to bind
    /// the arguments, once for the body scope) and the table is immutable after
    /// the driver installs it — so the clone is a refcount bump instead of a
    /// deep copy of every `fn` AST.
    fns: Rc<HashMap<String, FnDef>>,
    /// The callables whose expansion is in progress, innermost last. Every call
    /// is inlined, so a name that appears twice is recursion — rejected,
    /// because the ISA has no stack to recurse on.
    active: Vec<String>,
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
        Rc::make_mut(&mut self.consts).insert(name, value);
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

    /// The scope a parameter **default** is evaluated in: module scope (no
    /// parameter bindings) with `callee` pushed onto the in-progress chain.
    ///
    /// Two rules meet here, and they pull in opposite directions:
    ///
    /// * **No locals**, because a default is written at the callee's
    ///   *definition* site: it must never capture a caller local that happens
    ///   to share its name.
    /// * **`callee` on the chain**, because a default is part of the callee's
    ///   own declaration, so the callee already counts as being expanded:
    ///   `fn f(n: int = f())` is recursion and must be reported, not looped on
    ///   until the host stack runs out.
    ///
    /// An **argument** is the mirror image: written at the *call* site, it is
    /// evaluated in the caller's scope with the callee **not** yet on the
    /// chain — which is exactly what lets legal nesting like `f(f(1))` compile.
    /// That asymmetry is why the callee is pushed here and in
    /// [`Env::child_for_call`], but never around [`bind_args`]' argument loop.
    ///
    /// Deliberately private: [`bind_args`]' defaults loop is the only place
    /// this scope is correct, and a neutral-looking accessor is exactly how the
    /// argument path would acquire it by mistake.
    fn default_scope_for(&self, callee: &str) -> Env {
        let mut e = self.clone();
        e.locals.clear();
        e.active.push(callee.to_string());
        e
    }

    /// This environment with `bindings` as its parameter scope, replacing any
    /// locals. The module `const`s survive; the caller's locals do not.
    ///
    /// The `_values` suffix marks this as the value-only half of the call-scope
    /// constructor: [`Env::child_for_call`] delegates here and additionally
    /// pushes the callee onto the recursion chain.
    pub fn child_for_call_values(&self, bindings: HashMap<String, Value>) -> Env {
        let mut e = self.clone();
        e.locals = bindings;
        e
    }

    /// Define a `fn`. Returns `false` if one of that name already exists.
    pub fn define_fn(&mut self, f: FnDef) -> bool {
        if self.fns.contains_key(&f.name) {
            return false;
        }
        Rc::make_mut(&mut self.fns).insert(f.name.clone(), f);
        true
    }

    /// The `fn` bound to `name`, if any.
    pub fn get_fn(&self, name: &str) -> Option<&FnDef> {
        self.fns.get(name)
    }

    /// Is a call to `name` already in progress (i.e. would this recurse)?
    pub fn is_active(&self, name: &str) -> bool {
        self.active.iter().any(|n| n == name)
    }

    /// The environment a call body is evaluated in: the module scope plus
    /// `bindings`, with `callee` pushed onto the in-progress chain.
    pub fn child_for_call(&self, callee: &str, bindings: HashMap<String, Value>) -> Env {
        let mut e = self.child_for_call_values(bindings);
        e.active.push(callee.to_string());
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
        _ => match env.get_fn(func) {
            Some(f) => eval_fn_call(f, args, span, env),
            None => Err(
                Diagnostic::error(span.clone(), format!("unknown function `{func}`")).with_help(
                    "the builtins are crc8, len, lo, hi; a `proc` emits instructions and is called as a statement, not inside an expression",
                ),
            ),
        },
    }
}

/// Evaluate a `fn` call: bind the arguments, evaluate the body in the callee's
/// own scope, and check the result against the declared return type. There is
/// no runtime call — the value simply replaces the call site.
fn eval_fn_call(f: &FnDef, args: &[Arg], span: &Span, env: &Env) -> Result<Value, Diagnostic> {
    if env.is_active(&f.name) {
        return Err(Diagnostic::error(
            span.clone(),
            // Not "calls itself": the chain may be mutual (`f` → `g` → `f`),
            // in which case this fires inside `g`'s body and `f` never calls
            // itself directly. "Already being expanded" is true of both.
            format!(
                "`{}` is already being expanded: recursion is not possible",
                f.name
            ),
        )
        .with_help("every call is inlined — the tamal ISA has no call/ret and no stack"));
    }
    let bindings = bind_args(&f.name, &f.params, args, env, span)?;
    let child = env.child_for_call(&f.name, bindings);
    let v = eval(&f.body, &child)?;
    check_type(
        &v,
        f.ret,
        &f.body.span(),
        &format!("the body of `{}`", f.name),
    )?;
    Ok(v)
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
///
/// The returned map is keyed **for lookup only and must never be iterated**:
/// `HashMap` order is randomised per process, so iterating it to emit, list or
/// report anything would put that randomness into the output. Both consumers
/// move it straight into [`Env::locals`], which is only ever read by name. If
/// you need an order, walk `params` — that is the declaration order.
///
/// # Preconditions
///
/// `params` must have unique names; the parser is responsible for rejecting a
/// declaration that repeats one. With a duplicate, a named argument binds the
/// first of the pair and the defaults loop then treats *both* as bound, so the
/// second is never type-checked and never reported as missing. The
/// `debug_assert!` below makes that obligation executable.
pub fn bind_args(
    callee: &str,
    params: &[Param],
    args: &[Arg],
    env: &Env,
    call_span: &Span,
) -> Result<HashMap<String, Value>, Diagnostic> {
    debug_assert!(
        params
            .iter()
            .enumerate()
            .all(|(i, p)| !params[i + 1..].iter().any(|q| q.name == p.name)),
        "`{callee}` declares a duplicate parameter name; the parser must reject that"
    );
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
                // The check above makes the positional arguments a prefix of
                // `args`, so `i` is this argument's parameter index directly.
                params.get(i).ok_or_else(|| {
                    Diagnostic::error(
                        arg.span.clone(),
                        format!(
                            "`{callee}` takes {} argument{}, got {}",
                            params.len(),
                            if params.len() == 1 { "" } else { "s" },
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
    // missing parameter reported is stable across runs. Every default shares
    // one scope, built at most once and only when some parameter actually needs
    // it — in the common case every parameter is bound and no scope is built at
    // all, which is why this is lazy rather than hoisted.
    //
    // Note `default_scope_for`, not a bare module scope: a default belongs to
    // the callee's declaration, so the callee counts as already being expanded
    // and `fn f(n: int = f())` is caught by the guard in `eval_fn_call` instead
    // of recursing until the host stack dies. The argument loop above
    // deliberately does NOT do this — see `Env::default_scope_for`.
    let mut default_scope: Option<Env> = None;
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
        let scope = default_scope.get_or_insert_with(|| env.default_scope_for(callee));
        let v = eval(default, scope)?;
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
/// an error, never a silent wrap, and it is reported *as* a range error: an
/// integer that will not fit in a byte has the right type and the wrong
/// magnitude, so saying "expects `byte`, found the integer 300" would hide the
/// actual problem. The wording matches `eval_byte`'s.
fn check_type(v: &Value, ty: Type, span: &Span, what: &str) -> Result<(), Diagnostic> {
    let problem = match (ty, v) {
        (Type::Bytes, Value::Bytes(_)) | (Type::Int, Value::Int(_)) => return Ok(()),
        (Type::Byte, Value::Int(n)) if (0..=255).contains(n) => return Ok(()),
        (Type::Byte, Value::Int(n)) => format!("expects `byte`: {n} is out of range 0..=255"),
        (_, Value::Int(n)) => format!("expects `{}`, found the integer {n}", ty.name()),
        (_, Value::Bytes(b)) => format!("expects `{}`, found {} byte(s)", ty.name(), b.len()),
    };
    Err(Diagnostic::error(span.clone(), format!("{what} {problem}")))
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
            // Placeholder: `stamped` assigns every argument its real span.
            span: 0..0,
            value,
        }
    }

    fn named(name: &str, value: Expr) -> Arg {
        Arg {
            name: Some(name.into()),
            // Placeholder: `stamped` assigns every argument its real span.
            span: 0..0,
            value,
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
    fn rejects_a_named_argument_with_no_matching_parameter() {
        let unknown = bind_err(vec![named("nope", int(0))]);
        assert!(unknown.contains("no parameter `nope`"), "got: {unknown}");
    }

    #[test]
    fn rejects_a_parameter_bound_twice() {
        let dup = bind_diag(vec![pos(bytes(&[0x44])), named("pkt", bytes(&[0x06]))]);
        assert!(dup.message.contains("bound twice"), "got: {}", dup.message);
        assert_eq!(
            dup.primary,
            arg_span(1),
            "anchored on the second, offending argument"
        );
    }

    #[test]
    fn rejects_more_arguments_than_parameters() {
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
    fn the_arity_message_pluralises_like_the_builtins_do() {
        // The user-callable arity message used to read "takes 1 argument(s)"
        // while the builtin one (`builtin_arg`) read "takes 1 argument" — the
        // `(s)` form was the odd one out in an otherwise carefully-voiced
        // diagnostic set. Both halves need pinning: `contains("takes 3
        // argument")` above is satisfied by "argument(s)" too, so it cannot
        // tell the forms apart.
        let plural = bind_err(vec![
            pos(bytes(&[0x44])),
            pos(int(0)),
            pos(int(0x11)),
            pos(int(9)),
        ]);
        assert!(plural.contains("takes 3 arguments, got 4"), "got: {plural}");

        // …and the singular, which needs a one-parameter callee.
        let one = vec![param("pkt", Type::Bytes, None, param_span(0))];
        let singular = bind_args(
            "one_arg",
            &one,
            &stamped(vec![pos(bytes(&[0x44])), pos(int(0))]),
            &Env::new(),
            &call_span(),
        )
        .unwrap_err()
        .message;
        assert!(
            singular.contains("takes 1 argument, got 2"),
            "got: {singular}"
        );
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
    fn an_argument_of_the_wrong_type_is_rejected() {
        let wrong = bind_diag(vec![pos(int(5)), pos(int(0))]); // pkt: bytes
        assert!(
            wrong.message.contains("expects `bytes`"),
            "got: {}",
            wrong.message
        );
        assert_eq!(wrong.primary, arg_span(0), "anchored on the bad argument");
    }

    #[test]
    fn a_byte_argument_above_the_range_is_rejected() {
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
        // A range error says so, rather than reading as a type error.
        assert!(
            too_big.message.contains("out of range 0..=255"),
            "got: {}",
            too_big.message
        );
        assert_eq!(too_big.primary, arg_span(2), "anchored on the bad argument");
    }

    #[test]
    fn a_byte_argument_below_the_range_is_rejected() {
        // Both ends of the `byte` range are errors, never a silent wrap. There
        // is no unary minus in the grammar yet, so the negative is built
        // straight from the AST — Plan 4b's arithmetic makes it reachable from
        // source.
        let negative = bind_diag(vec![
            pos(bytes(&[0x44])),
            pos(int(0)),
            named("err", int(-1)),
        ]);
        assert!(
            negative.message.contains("expects `byte`"),
            "got: {}",
            negative.message
        );
    }

    #[test]
    fn a_child_scope_replaces_the_callers_locals() {
        // A call body sees its own parameters and the module `const`s, and
        // nothing else. Merging the caller's locals in instead of replacing
        // them would leak an outer `proc`'s parameters into an inner one's body
        // once Task 6 inlines nested calls.
        let mut module = Env::new();
        module.insert_const("BASE".into(), Value::Int(1));
        let caller = module
            .child_for_call_values([("outer".to_string(), Value::Int(7))].into_iter().collect());
        let callee = caller
            .child_for_call_values([("inner".to_string(), Value::Int(9))].into_iter().collect());

        assert_eq!(callee.get("inner"), Some(&Value::Int(9)));
        // The load-bearing half: the caller's local is *gone*, not merged in.
        assert_eq!(callee.get("outer"), None);
        // The module scope, by contrast, survives every call.
        assert_eq!(callee.get("BASE"), Some(&Value::Int(1)));
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
        //
        // The lone parameter deliberately carries `param_span(2)` while sitting
        // at index 0: the anchor must come from `p.span` itself, not from any
        // computation over the parameter's position.
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
