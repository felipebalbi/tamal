//! `tamal-lang` — the compiler for tamal-lang, a high-level language that lowers
//! to tamal assembly text and, through [`tamal_asm::assemble`], to tamal
//! bytecode. See `docs/superpowers/specs/2026-07-20-tamal-lang-design.md`.

#![forbid(unsafe_code)]

pub use tamal_asm::{Diagnostic, Severity, Span};

pub mod consteval;
pub mod emit;
pub mod lexer;
pub mod parser;

pub use emit::Lowering;

use tamal_asm::Program;

/// Lower tamal-lang source to tamal-asm text plus its source map.
///
/// Resolves `const`s, then requires exactly one `test` per file (one program
/// entry point); zero or many is a diagnostic, and a test that can never halt
/// is rejected.
pub fn lower(source: &str) -> Result<Lowering, Vec<Diagnostic>> {
    let toks = lexer::lex(source)?;
    let module = parser::parse(source, &toks)?;
    // Resolve `const`s in source order; each may reference earlier ones.
    // Duplicate names and references to undefined names are hard errors.
    let mut consts = consteval::Consts::new();
    for c in &module.consts {
        if consts.contains_key(&c.name) {
            return Err(vec![Diagnostic::error(
                c.name_span.clone(),
                format!("duplicate const `{}`", c.name),
            )]);
        }
        let v = consteval::eval(&c.value, &consts).map_err(|d| vec![d])?;
        consts.insert(c.name.clone(), v);
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
    // M2: every test must be able to halt. With no branches in the Plan-1
    // subset, execution is linear, so a body containing no terminator (`pass`,
    // `fail`, or a raw `halt`) would run the engine off the end of the program.
    let test = &module.tests[0];
    let halts = test.stmts.iter().any(|s| match s {
        parser::Stmt::Pass | parser::Stmt::Fail { .. } => true,
        parser::Stmt::Raw { mnemonic, .. } => mnemonic == "halt",
        parser::Stmt::Send { .. } => false,
        parser::Stmt::CrcRegion { .. } => false,
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
    emit::emit(&module, &consts)
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
}
