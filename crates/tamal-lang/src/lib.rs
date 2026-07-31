//! `tamal-lang` — the compiler for tamal-lang, a high-level language that lowers
//! to tamal assembly text and, through [`tamal_asm::assemble`], to tamal
//! bytecode. See `docs/superpowers/specs/2026-07-20-tamal-lang-design.md`.

#![forbid(unsafe_code)]

pub use tamal_asm::{Diagnostic, Severity, Span};

pub mod consteval;
pub mod emit;
pub mod lexer;
pub mod parser;
pub mod regalloc;

pub use emit::Lowering;

/// The largest compile-time unroll (`repeat N`, `recv N`) the compiler accepts.
///
/// A tamal program is capped at 1024 words, so a larger unroll can never
/// assemble; rejecting it up front turns what would be an out-of-memory into a
/// diagnostic. Note this is a *per-construct* bound and does not by itself
/// bound composition — see `MAX_EMITTED_LINES` in `emit.rs`.
pub const MAX_UNROLL: i64 = 1024;

use tamal_asm::Program;

/// Lower tamal-lang source to tamal-asm text plus its source map.
///
/// Validates the callable names (`fn` and `proc`) and installs the `fn` table,
/// resolves `const`s, then requires exactly one `test` per file (one program
/// entry point); zero or many is a diagnostic, and a test that can never halt
/// is rejected.
pub fn lower(source: &str) -> Result<Lowering, Vec<Diagnostic>> {
    let toks = lexer::lex(source)?;
    let module = parser::parse(source, &toks)?;
    let mut env = consteval::Env::new();
    // Install the `fn` table first: a `const` may be built by a compile-time
    // helper, so every `fn` must be callable before any `const` resolves. The
    // `const`s themselves still resolve in source order, and a `fn` called from
    // a `const` is expanded at that point — so its body sees only the `const`s
    // defined ABOVE the calling `const`, not the whole module.
    for f in &module.fns {
        check_callable_name(&f.name, &f.name_span, "fn")?;
        if !env.define_fn(f.clone()) {
            return Err(vec![Diagnostic::error(
                f.name_span.clone(),
                format!("duplicate fn `{}`", f.name),
            )]);
        }
    }
    // `proc`s are expanded by the emitter, but their names are validated here,
    // beside the `fn`s, so every callable-name collision is caught in one place.
    for (i, p) in module.procs.iter().enumerate() {
        check_callable_name(&p.name, &p.name_span, "proc")?;
        if env.get_fn(&p.name).is_some() {
            return Err(vec![Diagnostic::error(
                p.name_span.clone(),
                format!("`{}` is already defined as a `fn`", p.name),
            )]);
        }
        if module.procs[..i].iter().any(|q| q.name == p.name) {
            return Err(vec![Diagnostic::error(
                p.name_span.clone(),
                format!("duplicate proc `{}`", p.name),
            )]);
        }
    }
    // Resolve `const`s in source order; each may reference earlier ones.
    // Duplicate names and references to undefined names are hard errors.
    for c in &module.consts {
        if env.has_const(&c.name) {
            return Err(vec![Diagnostic::error(
                c.name_span.clone(),
                format!("duplicate const `{}`", c.name),
            )]);
        }
        let v = consteval::eval(&c.value, &env).map_err(|d| vec![d])?;
        env.insert_const(c.name.clone(), v);
    }
    if module.tests.len() != 1 {
        let span = module
            .tests
            .get(1)
            .map(|t| t.name_span.clone())
            .unwrap_or(0..0);
        return Err(vec![Diagnostic::error(
            span,
            format!(
                "a .tam file must contain exactly one `test` (found {})",
                module.tests.len()
            ),
        )]);
    }
    // M2: every test must reach a terminator (`pass`, `fail`, or a raw `halt`).
    // `wait_state`/`expect` add branches (a poll loop, a verdict `bnez`), but
    // none of them is a program terminator, so this scan for at least one
    // terminator statement still soundly rejects a body that would run the
    // engine off the end (e.g. a `frame { expect … }` with no following `pass`).
    let test = &module.tests[0];
    let halts = test.stmts.iter().any(|s| match s {
        parser::Stmt::Pass | parser::Stmt::Fail { .. } => true,
        parser::Stmt::Raw { mnemonic, .. } => mnemonic == "halt",
        parser::Stmt::Send { .. } => false,
        parser::Stmt::CrcRegion { .. } => false,
        parser::Stmt::Config { .. } => false,
        parser::Stmt::Frame { .. } => false,
        parser::Stmt::Recv { .. } => false,
        parser::Stmt::WaitState { .. } => false,
        parser::Stmt::Expect { .. } => false,
        parser::Stmt::Call { .. } => false,
        // Conservative, like `Frame` and `Call`: a `repeat` body may execute
        // zero times, so a verdict inside one is never counted as reaching a
        // halt. A verdict belongs at the top level of the test.
        parser::Stmt::Repeat { .. } => false,
    });
    if !halts {
        return Err(vec![
            Diagnostic::error(
                test.name_span.clone(),
                format!("test `{}` never halts", test.name),
            )
            .with_help(
                "a test must reach `pass`, `fail`, or a `halt` at the top level of the test — a verdict inside a `frame`, `proc` body or `repeat` is not counted",
            ),
        ]);
    }
    emit::emit(&module, env)
}

/// Reject a callable name that is already spoken for. `kind` is `"fn"` or
/// `"proc"`, for the diagnostic.
///
/// Two classes of reserved name, shared by both callables because they share
/// one namespace (a `proc` may not collide with a `fn`, and vice versa), so the
/// set of legal names must not depend on the kind:
///
/// * a compile-time **builtin** — it must always mean the same thing;
/// * a **statement keyword** — `parse_stmt` matches those before it would ever
///   see a call, so `proc send() { … }` is definable but uncallable. A `fn` of
///   that name *is* reachable (calls appear in expression position, e.g.
///   `send send(1)`), but it is rejected all the same: converting a `fn` to a
///   `proc` must never turn a legal name illegal.
fn check_callable_name(name: &str, span: &Span, kind: &str) -> Result<(), Vec<Diagnostic>> {
    if consteval::BUILTINS.contains(&name) {
        return Err(vec![
            Diagnostic::error(
                span.clone(),
                format!("`{name}` is a builtin and cannot be redefined"),
            )
            .with_help(format!(
                "the builtins are {}",
                consteval::BUILTINS.join(", ")
            )),
        ]);
    }
    if parser::STMT_KEYWORDS.contains(&name) {
        return Err(vec![
            Diagnostic::error(
                span.clone(),
                format!("`{name}` is a statement keyword and cannot be used as a `{kind}` name"),
            )
            .with_help(format!(
                "the statement keywords are {}",
                parser::STMT_KEYWORDS.join(", ")
            )),
        ]);
    }
    Ok(())
}

/// Lower to just the tamal-asm text (the `--emit asm` artifact).
pub fn lower_to_asm(source: &str) -> Result<String, Vec<Diagnostic>> {
    Ok(lower(source)?.asm)
}

/// Compile tamal-lang source to a tamal [`Program`] (bytecode), lowering to
/// asm text and handing it to the [`tamal_asm::assemble`] backend. Backend
/// diagnostics are re-pointed at the `.tam` source via the lowering's map.
pub fn compile(source: &str) -> Result<Program, Vec<Diagnostic>> {
    let lowering = lower(source)?;
    tamal_asm::assemble(&lowering.asm).map_err(|diags| lowering.remap(diags))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lowers_smoke_to_asm() {
        let asm = lower_to_asm("test smoke {\n    pass\n}\n").unwrap();
        assert_eq!(asm, ".globl _start\n_start:\n\thalt 0x00\n");
    }

    #[test]
    fn compile_pass_is_one_halt_word() {
        let prog = compile("test smoke {\n    pass\n}\n").unwrap();
        let words: Vec<u32> = prog.words().collect();
        assert_eq!(words, vec![0x4000_0000]); // CTRL group, HALT sub, imm 0
    }

    #[test]
    fn rejects_zero_tests() {
        let err = lower_to_asm("// just a comment\n").unwrap_err();
        assert!(err[0].message.contains("exactly one `test`"));
    }

    #[test]
    fn rejects_multiple_tests() {
        let src = "test a {\n pass\n}\ntest b {\n pass\n}\n";
        let err = lower_to_asm(src).unwrap_err();
        assert!(err[0].message.contains("exactly one `test`"));
    }

    // M2: a test whose body cannot reach a terminator would run the engine off
    // the end of the program. Such a test must be rejected, not compiled to an
    // empty/haltless program.
    #[test]
    fn rejects_test_with_no_statements() {
        let err = lower_to_asm("test empty {\n}\n").unwrap_err();
        assert!(err[0].message.contains("never halts"));
    }

    #[test]
    fn rejects_test_that_never_halts() {
        let err = lower_to_asm("test t {\n cs_assert\n}\n").unwrap_err();
        assert!(err[0].message.contains("never halts"));
    }

    #[test]
    fn accepts_raw_halt_as_terminator() {
        // a raw `halt` instruction is a valid terminator on its own
        let prog = compile("test t {\n halt 0x05\n}\n").unwrap();
        assert_eq!(prog.words().count(), 1);
    }

    // M1: a backend error on user-authored raw/fail content must be re-pointed
    // at the `.tam` source (via the source map), not left indexing generated asm.
    #[test]
    fn compile_error_points_at_tam_for_bad_raw() {
        let source = "test t {\n bogus_op\n pass\n}\n";
        let err = compile(source).unwrap_err();
        assert_eq!(
            source.get(err[0].primary.clone()),
            Some("bogus_op"),
            "diagnostic should point at the .tam token, not generated asm"
        );
    }

    #[test]
    fn compile_error_points_at_tam_for_out_of_range_fail() {
        let source = "test t {\n fail 300\n}\n";
        let err = compile(source).unwrap_err();
        let pointed = source.get(err[0].primary.clone()).unwrap_or("");
        assert!(
            pointed.contains("300"),
            "diagnostic should point at the .tam `fail 300`, got {pointed:?}"
        );
    }

    #[test]
    fn resolves_const_referencing_earlier_const() {
        assert!(lower_to_asm("const A = 0x40\nconst B = A\ntest t {\n pass\n}\n").is_ok());
    }

    #[test]
    fn rejects_duplicate_const() {
        let err = lower_to_asm("const A = 1\nconst A = 2\ntest t {\n pass\n}\n").unwrap_err();
        assert!(err[0].message.contains("duplicate const"));
    }

    #[test]
    fn rejects_const_with_unknown_name() {
        let err = lower_to_asm("const A = NOPE\ntest t {\n pass\n}\n").unwrap_err();
        assert!(err[0].message.contains("unknown name"));
    }

    #[test]
    fn send_lowers_to_put_byte_run() {
        let asm =
            lower_to_asm("const OP = 0x44\ntest t {\n send [OP, 0x00, 0x64]\n pass\n}\n").unwrap();
        assert!(asm.contains("put_byte 0x44"));
        assert!(asm.contains("put_byte 0x00"));
        assert!(asm.contains("put_byte 0x64"));
    }

    #[test]
    fn send_plus_crc8_appends_folded_byte() {
        let asm = lower_to_asm("test t {\n send [0x44, 0x00, 0x64] + crc8\n pass\n}\n").unwrap();
        assert!(asm.contains("put_byte 0x16")); // compile-time CRC-8, poly 0x07
    }

    #[test]
    fn config_lowers_to_set_config() {
        let asm =
            lower_to_asm("test t {\n config controller, x1, sck20, alert_pin\n pass\n}\n").unwrap();
        assert!(
            asm.contains("\tset_config controller, x1, sck20, alert_pin\n"),
            "got:\n{asm}"
        );
    }

    #[test]
    fn config_assembles_to_the_v1_config_word() {
        // controller,x1,sck20,alert_pin packs to 0x00 -> SET_CONFIG word 0x5800_0000
        let prog =
            compile("test t {\n config controller, x1, sck20, alert_pin\n pass\n}\n").unwrap();
        let words: Vec<u32> = prog.words().collect();
        assert_eq!(words[0], 0x5800_0000);
    }

    #[test]
    fn crc_region_folds_over_all_emitted_bytes() {
        // the same three bytes as the peripheral command phase → the same 0x16
        let asm = lower_to_asm(
            "test t {\n crc_region {\n  send [0x44]\n  send [0x00, 0x64]\n }\n pass\n}\n",
        )
        .unwrap();
        assert!(asm.contains("put_byte 0x44"));
        assert!(asm.contains("put_byte 0x64"));
        assert!(asm.contains("put_byte 0x16"));
    }

    #[test]
    fn frame_wraps_body_in_cs_assert_deassert() {
        let asm = lower_to_asm("test t {\n frame {\n  tar 2\n }\n pass\n}\n").unwrap();
        assert_eq!(
            asm,
            ".globl _start\n_start:\n\tcs_assert\n\ttar 2\n\tcs_deassert\n\thalt 0x00\n"
        );
    }

    #[test]
    fn recv_names_allocate_ascending_registers() {
        let asm = lower_to_asm("test t {\n recv a, b, c\n pass\n}\n").unwrap();
        assert!(asm.contains("\tget_byte x1\n"), "got:\n{asm}");
        assert!(asm.contains("\tget_byte x2\n"), "got:\n{asm}");
        assert!(asm.contains("\tget_byte x3\n"), "got:\n{asm}");
    }

    #[test]
    fn recv_discards_reuse_the_same_scratch_register() {
        // `_` is freed immediately, so three discards all reuse x1.
        let asm = lower_to_asm("test t {\n recv _, _, _\n pass\n}\n").unwrap();
        assert_eq!(
            asm,
            ".globl _start\n_start:\n\tget_byte x1\n\tget_byte x1\n\tget_byte x1\n\thalt 0x00\n"
        );
    }

    #[test]
    fn frame_scope_frees_recv_registers_on_exit() {
        // recv inside a frame binds x1,x2; after the frame a second recv reuses
        // x1,x2 (the frame scope released them).
        let asm =
            lower_to_asm("test t {\n frame {\n  recv a, b\n }\n recv c, d\n pass\n}\n").unwrap();
        let gets: Vec<&str> = asm.lines().filter(|l| l.contains("get_byte")).collect();
        assert_eq!(
            gets,
            vec![
                "\tget_byte x1",
                "\tget_byte x2",
                "\tget_byte x1",
                "\tget_byte x2"
            ]
        );
    }

    #[test]
    fn wait_state_lowers_to_the_poll_idiom() {
        let asm = lower_to_asm("test t {\n wait_state\n pass\n}\n").unwrap();
        assert_eq!(
            asm,
            ".globl _start\n_start:\n\
             __wait0:\n\tcrc_reset\n\tget_byte x1\n\tli x2, 0x0F\n\tbeq x1, x2, __wait0\n\
             \thalt 0x00\n"
        );
    }

    #[test]
    fn named_wait_state_keeps_the_terminal_byte_bound() {
        // `wait_state term` keeps the response register live, so a following
        // recv allocates the NEXT register (x2), not x1.
        let asm = lower_to_asm("test t {\n wait_state term\n recv d\n pass\n}\n").unwrap();
        // poll uses x1 (resp, kept) and x2 (constant, freed); recv d -> x2 reused.
        let gets: Vec<&str> = asm.lines().filter(|l| l.contains("get_byte")).collect();
        assert_eq!(gets, vec!["\tget_byte x1", "\tget_byte x2"]);
    }

    #[test]
    fn expect_crc_defers_the_verdict_past_cs_deassert() {
        let asm =
            lower_to_asm("test t {\n frame {\n  expect crc else 0x11\n }\n pass\n}\n").unwrap();
        assert_eq!(
            asm,
            ".globl _start\n_start:\n\
             \tcs_assert\n\
             \tget_byte x1\n\trdsr x1, crc\n\
             \tcs_deassert\n\
             \tbnez x1, __fail0\n\
             \thalt 0x00\n\
             __fail0:\n\thalt 0x11\n"
        );
    }

    #[test]
    fn expect_crc_outside_a_frame_is_an_error() {
        let err = lower_to_asm("test t {\n expect crc else 0x11\n pass\n}\n").unwrap_err();
        assert!(
            err[0].message.contains("inside a `frame`"),
            "got: {:?}",
            err[0].message
        );
    }

    #[test]
    fn expect_requires_the_crc_keyword() {
        let err =
            lower_to_asm("test t {\n frame {\n  expect foo else 0x11\n }\n pass\n}\n").unwrap_err();
        assert!(
            err[0].message.contains("expected `crc`"),
            "got: {:?}",
            err[0].message
        );
    }

    #[test]
    fn expect_requires_the_else_keyword() {
        // `then` is a valid identifier but not `else`, so it reaches the guard.
        let err =
            lower_to_asm("test t {\n frame {\n  expect crc then 0x11\n }\n pass\n}\n").unwrap_err();
        assert!(
            err[0].message.contains("`else <byte>`"),
            "got: {:?}",
            err[0].message
        );
    }

    #[test]
    fn config_inside_a_frame_is_rejected() {
        let err = lower_to_asm(
            "test t {\n frame {\n  config controller, x1, sck20, alert_pin\n }\n pass\n}\n",
        )
        .unwrap_err();
        assert!(
            err[0].message.contains("not allowed inside a `frame`"),
            "got: {:?}",
            err[0].message
        );
    }

    #[test]
    fn fn_call_folds_to_its_returned_value() {
        // A `fn` disappears entirely: the call is replaced by its bytes, and
        // `+ crc8` folds the CRC over exactly those bytes (0x16).
        let asm = lower_to_asm(
            "fn iord_hdr(op: byte, addr: int) -> bytes { [op, hi(addr), lo(addr)] }\n\
             test t {\n send iord_hdr(0x44, 0x0064) + crc8\n pass\n}\n",
        )
        .unwrap();
        assert_eq!(
            asm,
            ".globl _start\n_start:\n\
             \tput_byte 0x44\n\tput_byte 0x00\n\tput_byte 0x64\n\tput_byte 0x16\n\
             \thalt 0x00\n"
        );
    }

    #[test]
    fn a_const_can_call_a_fn() {
        // The `fn` table is installed before `const`s resolve, so a `const` may
        // be built by a compile-time helper.
        let asm = lower_to_asm(
            "fn iord_hdr(op: byte, addr: int) -> bytes { [op, hi(addr), lo(addr)] }\n\
             const HDR = iord_hdr(0x44, 0x0064)\n\
             test t {\n send HDR + crc8\n pass\n}\n",
        )
        .unwrap();
        assert!(asm.contains("\tput_byte 0x16\n"), "got:\n{asm}");
    }

    #[test]
    fn a_recursive_fn_is_rejected() {
        let err = lower_to_asm("fn f(n: int) -> int { f(n) }\ntest t {\n send [f(1)]\n pass\n}\n")
            .unwrap_err();
        assert!(
            err[0].message.contains("is already being expanded"),
            "got: {:?}",
            err[0]
        );
    }

    #[test]
    fn mutual_recursion_between_fns_is_rejected() {
        // `f` → `g` → `f`. The guard must scan the WHOLE in-progress chain, not
        // just its innermost entry: at the inner call the chain is ["f", "g"]
        // and it is the *outer* `f` that makes this recursion. Checking only
        // the last entry would loop until the host stack dies.
        let err = lower_to_asm(
            "fn f(n: int) -> int { g(n) }\n\
             fn g(n: int) -> int { f(n) }\n\
             test t {\n send [f(1)]\n pass\n}\n",
        )
        .unwrap_err();
        assert!(
            err[0].message.contains("is already being expanded"),
            "got: {:?}",
            err[0]
        );
    }

    #[test]
    fn a_self_referential_parameter_default_is_rejected() {
        // A default belongs to the callee's own declaration, so the callee is
        // already being expanded when the default runs. Before this was
        // handled, the compiler overflowed its stack and aborted with no
        // diagnostic at all.
        let src = "fn f(n: int = f()) -> int { n }\ntest t {\n send [f()]\n pass\n}\n";
        let err = lower_to_asm(src).unwrap_err();
        assert!(
            err[0].message.contains("is already being expanded"),
            "got: {:?}",
            err[0]
        );
        // The anchor is the non-obvious part: it points at the `f()` INSIDE the
        // default, not at the `fn` item and not at the `f()` in the test body —
        // the default is where the author has to break the cycle.
        let in_default = src.find("= f()").unwrap() + 2;
        assert_eq!(
            err[0].primary,
            in_default..in_default + 3,
            "anchored on the `f()` inside the default"
        );
    }

    #[test]
    fn a_default_cycle_between_two_fns_is_rejected() {
        // The default cycle need not be direct: `f`'s default calls `g`, whose
        // default calls `f`.
        let err = lower_to_asm(
            "fn f(n: int = g()) -> int { n }\n\
             fn g(m: int = f()) -> int { m }\n\
             test t {\n send [f()]\n pass\n}\n",
        )
        .unwrap_err();
        assert!(
            err[0].message.contains("is already being expanded"),
            "got: {:?}",
            err[0]
        );
    }

    #[test]
    fn nested_calls_to_the_same_fn_are_legal() {
        // The regression guard for the recursion fix. An ARGUMENT is written at
        // the call site and is evaluated in the caller's scope with the callee
        // NOT yet on the chain, so `f(f(1))` is ordinary nesting, not recursion.
        // Pushing the callee before binding arguments would "fix" the default
        // cycle above while silently rejecting this — hence the exact bytes.
        let asm =
            lower_to_asm("fn f(n: int) -> int { n ^ 0x10 }\ntest t {\n send [f(f(1))]\n pass\n}\n")
                .unwrap();
        // f(1) = 1 ^ 0x10 = 0x11; f(0x11) = 0x11 ^ 0x10 = 0x01.
        assert_eq!(
            asm,
            ".globl _start\n_start:\n\tput_byte 0x01\n\thalt 0x00\n"
        );
    }

    #[test]
    fn a_default_may_call_a_different_fn() {
        // The guard must reject cycles without outlawing a default that simply
        // delegates to another compile-time helper.
        let asm = lower_to_asm(
            "fn base() -> int { 0x40 }\n\
             fn f(n: int = base()) -> int { n }\n\
             test t {\n send [f()]\n pass\n}\n",
        )
        .unwrap();
        assert_eq!(
            asm,
            ".globl _start\n_start:\n\tput_byte 0x40\n\thalt 0x00\n"
        );
    }

    #[test]
    fn a_fn_body_must_produce_its_declared_type() {
        let err = lower_to_asm("fn bad(n: int) -> bytes { n }\ntest t {\n send bad(5)\n pass\n}\n")
            .unwrap_err();
        assert!(
            err[0].message.contains("expects `bytes`"),
            "got: {:?}",
            err[0]
        );
    }

    #[test]
    fn a_fn_may_not_shadow_a_builtin() {
        let err =
            lower_to_asm("fn crc8(b: bytes) -> byte { 0 }\ntest t {\n pass\n}\n").unwrap_err();
        assert!(err[0].message.contains("builtin"), "got: {:?}", err[0]);
    }

    #[test]
    fn a_duplicate_fn_is_rejected() {
        let err = lower_to_asm(
            "fn f(n: int) -> int { n }\nfn f(n: int) -> int { n }\ntest t {\n pass\n}\n",
        )
        .unwrap_err();
        assert!(err[0].message.contains("duplicate fn"), "got: {:?}", err[0]);
    }

    #[test]
    fn a_fn_argument_resolves_in_the_calling_fns_scope() {
        // An argument expression is written at the CALL site, so it must be
        // evaluated in the caller's scope — here `addr` is `hdr`'s parameter,
        // passed on to `lo_byte`. `bind_args` is handed the caller's `env` for
        // exactly this reason; passing `env.default_scope_for(…)` instead would
        // make this `unknown name `addr``. Task 3 pins the rule inside the
        // binder; this pins the `eval_fn_call` call site that feeds it.
        let asm = lower_to_asm(
            "fn lo_byte(n: int) -> byte { lo(n) }\n\
             fn hdr(addr: int) -> bytes { [0x44, hi(addr), lo_byte(addr)] }\n\
             test t {\n send hdr(0x0064)\n pass\n}\n",
        )
        .unwrap();
        assert_eq!(
            asm,
            ".globl _start\n_start:\n\
             \tput_byte 0x44\n\tput_byte 0x00\n\tput_byte 0x64\n\
             \thalt 0x00\n"
        );
    }

    #[test]
    fn a_proc_call_inlines_its_body() {
        let asm = lower_to_asm(
            "proc cfg() { config controller, x1, sck20, alert_pin }\n\
             test t {\n cfg()\n pass\n}\n",
        )
        .unwrap();
        assert_eq!(
            asm,
            ".globl _start\n_start:\n\tset_config controller, x1, sck20, alert_pin\n\thalt 0x00\n"
        );
    }

    #[test]
    fn proc_parameters_are_visible_in_its_body() {
        let asm = lower_to_asm(
            "proc emit_pkt(pkt: bytes) { send pkt + crc8 }\n\
             test t {\n emit_pkt([0x44, 0x00, 0x64])\n pass\n}\n",
        )
        .unwrap();
        assert_eq!(
            asm,
            ".globl _start\n_start:\n\
             \tput_byte 0x44\n\tput_byte 0x00\n\tput_byte 0x64\n\tput_byte 0x16\n\
             \thalt 0x00\n"
        );
    }

    #[test]
    fn two_expansions_get_fresh_registers_and_labels() {
        // Hygiene (D2/D5): each expansion re-uses the same registers (the
        // previous one released them) but never the same label.
        let asm =
            lower_to_asm("proc poll() { wait_state }\ntest t {\n poll()\n poll()\n pass\n}\n")
                .unwrap();
        assert_eq!(
            asm,
            ".globl _start\n_start:\n\
             __wait0:\n\tcrc_reset\n\tget_byte x1\n\tli x2, 0x0F\n\tbeq x1, x2, __wait0\n\
             __wait1:\n\tcrc_reset\n\tget_byte x1\n\tli x2, 0x0F\n\tbeq x1, x2, __wait1\n\
             \thalt 0x00\n"
        );
    }

    #[test]
    fn a_proc_expansion_cannot_clobber_a_live_caller_register() {
        let asm = lower_to_asm(
            "proc read() { recv inner }\n\
             test t {\n recv keep\n read()\n recv after\n pass\n}\n",
        )
        .unwrap();
        let gets: Vec<&str> = asm.lines().filter(|l| l.contains("get_byte")).collect();
        // `keep` holds x1 across the call, so the callee is handed x2 — and
        // releases it on exit, so the next binding reuses x2.
        assert_eq!(
            gets,
            vec!["\tget_byte x1", "\tget_byte x2", "\tget_byte x2"]
        );
    }

    #[test]
    fn a_proc_call_inside_a_frame_defers_its_expect_to_that_frame() {
        let asm = lower_to_asm(
            "proc verify() { expect crc else 0x11 }\n\
             test t {\n frame {\n  verify()\n }\n pass\n}\n",
        )
        .unwrap();
        assert_eq!(
            asm,
            ".globl _start\n_start:\n\
             \tcs_assert\n\
             \tget_byte x1\n\trdsr x1, crc\n\
             \tcs_deassert\n\
             \tbnez x1, __fail0\n\
             \thalt 0x00\n\
             __fail0:\n\thalt 0x11\n"
        );
    }

    #[test]
    fn a_verdict_latched_inside_a_proc_survives_later_allocations() {
        // The residue lives in the proc's scope but is branched on at the
        // frame's exit: the `recv` after the call must NOT be handed x1.
        let asm = lower_to_asm(
            "proc verify() { expect crc else 0x11 }\n\
             test t {\n frame {\n  verify()\n  recv extra\n }\n pass\n}\n",
        )
        .unwrap();
        assert_eq!(
            asm,
            ".globl _start\n_start:\n\
             \tcs_assert\n\
             \tget_byte x1\n\trdsr x1, crc\n\
             \tget_byte x2\n\
             \tcs_deassert\n\
             \tbnez x1, __fail0\n\
             \thalt 0x00\n\
             __fail0:\n\thalt 0x11\n"
        );
    }

    #[test]
    fn a_verdict_latched_two_expansions_deep_survives_every_scope_exit() {
        // The hard case for `exit_expansion_scope`: `inner` latches the
        // residue, `outer` allocates after it, and the frame allocates after
        // that. Ownership of the residue register has to walk outward one scope
        // at a time — inner -> outer -> frame — so a single `reserve` at the
        // innermost exit is not enough. `mid` and `late` both get x2 (each is
        // released by the scope that owned it); neither may be handed x1.
        let asm = lower_to_asm(
            "proc inner() { expect crc else 0x11 }\n\
             proc outer() { inner()\n recv mid\n }\n\
             test t {\n frame {\n  outer()\n  recv late\n }\n pass\n}\n",
        )
        .unwrap();
        assert_eq!(
            asm,
            ".globl _start\n_start:\n\
             \tcs_assert\n\
             \tget_byte x1\n\trdsr x1, crc\n\
             \tget_byte x2\n\
             \tget_byte x2\n\
             \tcs_deassert\n\
             \tbnez x1, __fail0\n\
             \thalt 0x00\n\
             __fail0:\n\thalt 0x11\n"
        );
    }

    #[test]
    fn a_recursive_proc_is_rejected() {
        let err = lower_to_asm("proc p() { p() }\ntest t {\n p()\n pass\n}\n").unwrap_err();
        assert!(
            err[0].message.contains("already being expanded"),
            "got: {:?}",
            err[0]
        );
    }

    #[test]
    fn mutual_recursion_between_procs_is_rejected() {
        // The guard must scan the WHOLE in-progress chain, not just its
        // innermost entry: at the inner call the chain is ["a", "b"] and it is
        // the *outer* `a` that makes this recursion. Checking only the last
        // entry does not merely mis-report — it inlines until the host stack
        // dies (a SIGABRT with no diagnostic at all).
        let err = lower_to_asm(
            "proc a() { b() }\n\
             proc b() { a() }\n\
             test t {\n a()\n pass\n}\n",
        )
        .unwrap_err();
        assert!(
            err[0].message.contains("already being expanded"),
            "got: {:?}",
            err[0]
        );
    }

    #[test]
    fn proc_parameters_do_not_leak_past_the_call() {
        // D5 hygiene, the caller's half: the expansion swaps in the callee's
        // value scope and must swap the caller's back. Here `N` is both a
        // module `const` and a parameter, so a leaked scope is visible in the
        // emitted bytes — the second `send` would re-emit the argument.
        let asm = lower_to_asm(
            "const N = 0x40\n\
             proc p(N: int) { send [N] }\n\
             test t {\n p(1)\n send [N]\n pass\n}\n",
        )
        .unwrap();
        assert_eq!(
            asm,
            ".globl _start\n_start:\n\tput_byte 0x01\n\tput_byte 0x40\n\thalt 0x00\n"
        );
    }

    #[test]
    fn a_nested_proc_call_forwards_an_argument_from_the_callers_scope() {
        // Two properties in one program. First, a `proc` may call another
        // `proc` — the expansion is recursive over the body. Second, an
        // ARGUMENT is written at the call site, so `inner(x)` must resolve `x`
        // against `outer`'s parameters: `bind_args` is handed the *caller's*
        // env for exactly that reason. Passing a bare module scope instead
        // would make this `unknown name `x``, while leaving the rest green.
        let asm = lower_to_asm(
            "proc inner(n: int) { send [n] }\n\
             proc outer(x: int) { inner(x) }\n\
             test t {\n outer(0x40)\n pass\n}\n",
        )
        .unwrap();
        assert_eq!(
            asm,
            ".globl _start\n_start:\n\tput_byte 0x40\n\thalt 0x00\n"
        );
    }

    #[test]
    fn a_proc_may_not_be_named_after_a_statement_keyword() {
        // `proc send() {…}` is definable but uncallable: `send()` parses as the
        // `send` statement and dies on the empty expression, with nothing to
        // suggest the definition is unreachable. Reject it at the definition.
        let err = lower_to_asm("proc send() { tar 2 }\ntest t {\n pass\n}\n").unwrap_err();
        assert!(
            err[0].message.contains("is a statement keyword"),
            "got: {:?}",
            err[0]
        );
        // Anchored on the name being defined, and the help lists the words.
        assert_eq!(err[0].primary, 5..9, "anchored on the `proc` name");
        assert!(
            err[0]
                .help
                .as_deref()
                .is_some_and(|h| h.contains("crc_region")),
            "got: {:?}",
            err[0].help
        );
    }

    #[test]
    fn a_fn_may_not_be_named_after_a_statement_keyword_either() {
        // A `fn` of that name IS reachable (`send send(1)` works, because calls
        // appear in expression position). It is rejected anyway: `fn` and `proc`
        // share one namespace, so converting one to the other must never turn a
        // legal name illegal.
        let err = lower_to_asm("fn recv(n: int) -> int { n }\ntest t {\n pass\n}\n").unwrap_err();
        assert!(
            err[0].message.contains("is a statement keyword"),
            "got: {:?}",
            err[0]
        );
        assert!(err[0].message.contains("`fn` name"), "got: {:?}", err[0]);
    }

    #[test]
    fn a_proc_may_not_shadow_a_builtin() {
        // The `fn` rule (`a_fn_may_not_shadow_a_builtin`) applies to `proc`s
        // too — one namespace, one set of legal names.
        let err = lower_to_asm("proc crc8() { tar 2 }\ntest t {\n pass\n}\n").unwrap_err();
        assert!(err[0].message.contains("builtin"), "got: {:?}", err[0]);
    }

    #[test]
    fn a_duplicate_proc_is_rejected() {
        // The emitter's `proc` table is a HashMap, so without this the second
        // definition would silently win.
        let err = lower_to_asm("proc p() { cs_assert }\nproc p() { tar 2 }\ntest t {\n pass\n}\n")
            .unwrap_err();
        assert!(
            err[0].message.contains("duplicate proc"),
            "got: {:?}",
            err[0]
        );
    }

    #[test]
    fn a_terminator_inside_a_proc_body_does_not_satisfy_the_halt_scan() {
        // The M2 scan is a *top-level* scan of the test's own statements, so a
        // `pass` reachable only through an expansion does not count. That is
        // deliberately conservative — this program would in fact halt — and the
        // help text says so, so both are pinned together.
        let err = lower_to_asm("proc p() { pass }\ntest t {\n p()\n}\n").unwrap_err();
        assert!(err[0].message.contains("never halts"), "got: {:?}", err[0]);
        assert!(
            err[0]
                .help
                .as_deref()
                .is_some_and(|h| h.contains("`proc` body")),
            "got: {:?}",
            err[0].help
        );
    }

    #[test]
    fn a_fn_cannot_be_called_as_a_statement() {
        let err =
            lower_to_asm("fn f(n: int) -> int { n }\ntest t {\n f(1)\n pass\n}\n").unwrap_err();
        assert!(err[0].message.contains("is a `fn`"), "got: {:?}", err[0]);
    }

    #[test]
    fn an_unknown_call_is_rejected() {
        let err = lower_to_asm("test t {\n nope()\n pass\n}\n").unwrap_err();
        assert!(
            err[0].message.contains("unknown `proc`"),
            "got: {:?}",
            err[0]
        );
    }

    #[test]
    fn a_proc_may_not_collide_with_a_fn() {
        let err = lower_to_asm(
            "fn dup(n: int) -> int { n }\nproc dup() { cs_assert }\ntest t {\n pass\n}\n",
        )
        .unwrap_err();
        assert!(
            err[0].message.contains("already defined as a `fn`"),
            "got: {:?}",
            err[0]
        );
    }

    #[test]
    fn a_verdict_latched_in_a_plain_frame_survives_later_allocations() {
        // The same residue-liveness property as the proc case, without a proc:
        // the register holding the residue must stay live until the `bnez`.
        let asm = lower_to_asm(
            "test t {\n frame {\n  recv a\n  expect crc else 0x11\n  recv b\n }\n pass\n}\n",
        )
        .unwrap();
        assert_eq!(
            asm,
            ".globl _start\n_start:\n\
             \tcs_assert\n\
             \tget_byte x1\n\
             \tget_byte x2\n\trdsr x2, crc\n\
             \tget_byte x3\n\
             \tcs_deassert\n\
             \tbnez x2, __fail0\n\
             \thalt 0x00\n\
             __fail0:\n\thalt 0x11\n"
        );
    }

    #[test]
    fn pass_inside_a_frame_points_at_the_test_not_the_file_start() {
        // Task 5 gave `pass` a real span; without it the caret lands at 0..0,
        // i.e. the start of the file.
        let err = lower_to_asm("test t {\n frame {\n  pass\n }\n pass\n}\n").unwrap_err();
        assert!(
            err[0].message.contains("not allowed inside a `frame`"),
            "got: {:?}",
            err[0]
        );
        assert_eq!(err[0].primary, 5..6, "the caret must land on `t`, not 0..0");
    }

    #[test]
    fn fail_inside_a_frame_is_rejected() {
        let err = lower_to_asm("test t {\n frame {\n  fail 0x22\n }\n pass\n}\n").unwrap_err();
        assert!(
            err[0].message.contains("not allowed inside a `frame`"),
            "got: {:?}",
            err[0]
        );
    }

    #[test]
    fn a_nested_frame_is_rejected() {
        let err = lower_to_asm("test t {\n frame {\n  frame {\n   tar 2\n  }\n }\n pass\n}\n")
            .unwrap_err();
        assert!(
            err[0].message.contains("not allowed inside a `frame`"),
            "got: {:?}",
            err[0]
        );
    }

    #[test]
    fn repeat_unrolls_at_compile_time() {
        let asm = lower_to_asm("test t {\n repeat 3 {\n  recv _\n }\n pass\n}\n").unwrap();
        assert_eq!(
            asm,
            ".globl _start\n_start:\n\tget_byte x1\n\tget_byte x1\n\tget_byte x1\n\thalt 0x00\n"
        );
    }

    #[test]
    fn repeat_zero_emits_nothing() {
        // The OOB write-completion case: `ndata = 0` contributes no reads.
        let asm = lower_to_asm("test t {\n repeat 0 {\n  recv _\n }\n pass\n}\n").unwrap();
        assert_eq!(asm, ".globl _start\n_start:\n\thalt 0x00\n");
    }

    #[test]
    fn a_repeat_count_may_come_from_a_proc_parameter() {
        let asm = lower_to_asm(
            "proc payload(n: int) { repeat n { recv _ } }\n\
             test t {\n payload(2)\n pass\n}\n",
        )
        .unwrap();
        assert_eq!(
            asm,
            ".globl _start\n_start:\n\tget_byte x1\n\tget_byte x1\n\thalt 0x00\n"
        );
    }

    #[test]
    fn each_repeat_iteration_gets_its_own_register_scope() {
        // `a` is released at the end of each iteration, so the next one reuses
        // the same register instead of leaking a fresh one.
        let asm = lower_to_asm("test t {\n repeat 2 {\n  recv a\n }\n pass\n}\n").unwrap();
        let gets: Vec<&str> = asm.lines().filter(|l| l.contains("get_byte")).collect();
        assert_eq!(gets, vec!["\tget_byte x1", "\tget_byte x1"]);
    }

    #[test]
    fn an_oversized_repeat_is_rejected() {
        let err =
            lower_to_asm("test t {\n repeat 99999999 {\n  cs_assert\n }\n pass\n}\n").unwrap_err();
        assert!(
            err[0].message.contains("not in 0..=1024"),
            "got: {:?}",
            err[0]
        );
    }

    #[test]
    fn a_repeat_of_exactly_max_unroll_is_accepted() {
        // The bound is inclusive: `MAX_UNROLL` itself is legal.
        let src = format!("test t {{\n repeat {MAX_UNROLL} {{\n  cs_assert\n }}\n pass\n}}\n");
        let asm = lower_to_asm(&src).unwrap();
        assert_eq!(asm.matches("cs_assert").count(), MAX_UNROLL as usize);
    }

    #[test]
    fn a_repeat_one_over_max_unroll_is_rejected() {
        let src = format!(
            "test t {{\n repeat {} {{\n  cs_assert\n }}\n pass\n}}\n",
            MAX_UNROLL + 1
        );
        let err = lower_to_asm(&src).unwrap_err();
        assert!(
            err[0].message.contains("not in 0..=1024"),
            "got: {:?}",
            err[0]
        );
    }

    #[test]
    fn a_terminator_inside_a_repeat_does_not_satisfy_the_halt_scan() {
        // The same conservative rule as `proc` and `frame`: a `repeat` body may
        // run zero times, so a `pass` reachable only through one is not counted.
        let err = lower_to_asm("test t {\n repeat 1 {\n  pass\n }\n}\n").unwrap_err();
        assert!(err[0].message.contains("never halts"), "got: {:?}", err[0]);
    }

    #[test]
    fn a_verdict_latched_inside_a_repeat_survives_later_allocations() {
        // The residue is latched in the repeat iteration's scope but branched
        // on at the frame's exit, so `recv late` must not be handed x1.
        let asm = lower_to_asm(
            "test t {\n frame {\n  repeat 1 {\n   expect crc else 0x11\n  }\n  recv late\n }\n pass\n}\n",
        )
        .unwrap();
        assert_eq!(
            asm,
            ".globl _start\n_start:\n\
             \tcs_assert\n\
             \tget_byte x1\n\trdsr x1, crc\n\
             \tget_byte x2\n\
             \tcs_deassert\n\
             \tbnez x1, __fail0\n\
             \thalt 0x00\n\
             __fail0:\n\thalt 0x11\n"
        );
    }

    #[test]
    fn a_verdict_latched_in_a_repeat_in_a_proc_still_reaches_the_frame() {
        // Two nested expansion scopes between the `expect` and the `frame` that
        // consumes it: the residue register must be re-taken at *each* exit, so
        // `recv late` still gets x2 rather than clobbering x1.
        let asm = lower_to_asm(
            "proc p() { repeat 1 { expect crc else 0x11 } }\n\
             test t {\n frame {\n  p()\n  recv late\n }\n pass\n}\n",
        )
        .unwrap();
        assert_eq!(
            asm,
            ".globl _start\n_start:\n\
             \tcs_assert\n\
             \tget_byte x1\n\trdsr x1, crc\n\
             \tget_byte x2\n\
             \tcs_deassert\n\
             \tbnez x1, __fail0\n\
             \thalt 0x00\n\
             __fail0:\n\thalt 0x11\n"
        );
    }
}
