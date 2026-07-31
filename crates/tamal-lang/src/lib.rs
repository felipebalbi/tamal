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

use tamal_asm::Program;

/// Lower tamal-lang source to tamal-asm text plus its source map.
///
/// Installs the module's `fn` table, resolves `const`s, then requires exactly
/// one `test` per file (one program entry point); zero or many is a diagnostic,
/// and a test that can never halt is rejected.
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
        if consteval::BUILTINS.contains(&f.name.as_str()) {
            return Err(vec![
                Diagnostic::error(
                    f.name_span.clone(),
                    format!("`{}` is a builtin and cannot be redefined", f.name),
                )
                .with_help("the builtins are crc8, len, lo, hi"),
            ]);
        }
        if !env.define_fn(f.clone()) {
            return Err(vec![Diagnostic::error(
                f.name_span.clone(),
                format!("duplicate fn `{}`", f.name),
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
    });
    if !halts {
        return Err(vec![
            Diagnostic::error(
                test.name_span.clone(),
                format!("test `{}` never halts", test.name),
            )
            .with_help("a test must reach `pass`, `fail`, or a `halt` instruction"),
        ]);
    }
    emit::emit(&module, env)
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
        let err =
            lower_to_asm("fn f(n: int = f()) -> int { n }\ntest t {\n send [f()]\n pass\n}\n")
                .unwrap_err();
        assert!(
            err[0].message.contains("is already being expanded"),
            "got: {:?}",
            err[0]
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
    fn a_fn_may_not_shadow_a_builtin_or_repeat_a_name() {
        let shadow =
            lower_to_asm("fn crc8(b: bytes) -> byte { 0 }\ntest t {\n pass\n}\n").unwrap_err();
        assert!(
            shadow[0].message.contains("builtin"),
            "got: {:?}",
            shadow[0]
        );

        let dup = lower_to_asm(
            "fn f(n: int) -> int { n }\nfn f(n: int) -> int { n }\ntest t {\n pass\n}\n",
        )
        .unwrap_err();
        assert!(dup[0].message.contains("duplicate fn"), "got: {:?}", dup[0]);
    }

    #[test]
    fn a_fn_argument_resolves_in_the_calling_fns_scope() {
        // An argument expression is written at the CALL site, so it must be
        // evaluated in the caller's scope — here `addr` is `hdr`'s parameter,
        // passed on to `lo_byte`. `bind_args` is handed the caller's `env` for
        // exactly this reason; passing `env.module_scope()` instead would make
        // this `unknown name `addr``. Task 3 pins the rule inside the binder;
        // this pins the `eval_fn_call` call site that feeds it.
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
}
