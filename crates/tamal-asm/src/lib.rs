//! `tamal-asm` — the assembler for the tamal ISA.
//!
//! Parses RISC-V-flavored tamal assembly and emits tamal bytecode in the
//! [`tamal_abi::isa`] encoding. The public API is [`assemble`] (source ->
//! [`Program`] or a `Vec<`[`Diagnostic`]`>`); the CLI renders diagnostics with
//! ariadne. See `docs/superpowers/specs/2026-07-04-tamal-asm-design.md`.

#![forbid(unsafe_code)]

pub mod diagnostics;
pub mod encoder;
pub mod lexer;
pub mod parser;
pub mod symbol;

pub use diagnostics::{Diagnostic, Severity, Span};

use parser::{LineKind, Operand, OperandKind};
use tamal_abi::isa::Instr;

/// A successfully assembled program: the instruction words plus, for each word,
/// the source span it came from (for listings).
#[derive(Debug, Clone)]
pub struct Program {
    instrs: Vec<Instr>,
    // `allow(dead_code)`: retained per-word source spans feed the assembler's
    // listing output, which lands in a later task; the field is populated now so
    // the shape is stable.
    #[allow(dead_code)]
    spans: Vec<Span>,
}

impl Program {
    /// The encoded 32-bit instruction words, in order.
    pub fn words(&self) -> impl Iterator<Item = u32> + '_ {
        self.instrs.iter().map(tamal_abi::isa::Instr::encode)
    }

    /// The program as little-endian bytes (loader-ready).
    pub fn to_le_bytes(&self) -> Vec<u8> {
        tamal_abi::isa::program_to_le_bytes(&self.instrs)
    }
}

/// The instruction-memory depth (words); programs may not exceed this.
const MAX_WORDS: usize = 1024;

/// Assemble tamal source into a [`Program`], or a batch of [`Diagnostic`]s.
pub fn assemble(source: &str) -> Result<Program, Vec<Diagnostic>> {
    let tokens = lexer::lex(source).map_err(|d| vec![d])?;
    let lines = parser::parse(&tokens).map_err(|d| vec![d])?;

    let (mut syms, d_equ) = symbol::collect_equs(&lines);
    let (total, d_addr) = symbol::assign_addresses(&lines, &mut syms);

    let mut diags = Vec::new();
    diags.extend(d_equ);
    diags.extend(d_addr);

    let mut instrs = Vec::new();
    let mut spans = Vec::new();
    let mut addr: u16 = 0;
    for line in &lines {
        match &line.kind {
            LineKind::Label(_) => {}
            LineKind::Directive { name, args } => {
                validate_directive(name, args, &line.span, &mut diags);
            }
            LineKind::Instr { mnemonic, operands } => {
                match encoder::encode_line(mnemonic, operands, &line.span, addr, &syms) {
                    Ok(seq) => {
                        addr = addr.wrapping_add(seq.len() as u16);
                        for ins in seq {
                            instrs.push(ins);
                            spans.push(line.span.clone());
                        }
                    }
                    Err(d) => {
                        // keep the address counter in step with pass 1 so later
                        // labels/branches stay consistent despite this error
                        addr =
                            addr.wrapping_add(encoder::instr_word_count(mnemonic, operands, &syms));
                        diags.push(d);
                    }
                }
            }
        }
    }

    if total > MAX_WORDS {
        let sp = lines.last().map(|l| l.span.clone()).unwrap_or(0..0);
        diags.push(Diagnostic::error(
            sp,
            format!("program is {total} words; the instruction memory holds at most {MAX_WORDS}"),
        ));
    }

    if diags.is_empty() {
        Ok(Program { instrs, spans })
    } else {
        Err(diags)
    }
}

/// Check a directive; push a diagnostic for anything unsupported in v1.
fn validate_directive(name: &str, args: &[Operand], span: &Span, diags: &mut Vec<Diagnostic>) {
    match name {
        "equ" => {}  // evaluated in collect_equs
        "text" => {} // the only section in v1
        "globl" => {
            if args.len() != 1 || !matches!(args[0].kind, OperandKind::Ident(_)) {
                diags.push(Diagnostic::error(
                    span.clone(),
                    "`.globl` takes a single symbol name",
                ));
            }
        }
        "macro" | "data" | "word" | "align" | "option" => {
            diags.push(
                Diagnostic::error(
                    span.clone(),
                    format!("`.{name}` is not supported in tamal-asm v1"),
                )
                .with_help("v1 supports .equ, .text, and .globl"),
            );
        }
        other => {
            diags.push(Diagnostic::error(
                span.clone(),
                format!("unknown directive `.{other}`"),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tamal_abi::isa::Instr;

    #[test]
    fn assembles_and_encodes_a_small_program() {
        let prog = assemble("_start:\n  cs_assert\n  put_byte 0x64\n  cs_deassert\n  halt 0x00\n")
            .unwrap();
        let words: Vec<u32> = prog.words().collect();
        assert_eq!(words.len(), 4);
        assert_eq!(words[0], Instr::CsAssert.encode());
        assert_eq!(words[1], Instr::PutByteImm(0x64).encode());
        assert_eq!(prog.to_le_bytes().len(), 16);
    }

    #[test]
    fn collects_multiple_diagnostics() {
        // undefined symbol + x16-31 register, both should be reported
        let errs = assemble("  put_byte NOPE\n  add x20, x0, x0\n").unwrap_err();
        assert_eq!(errs.len(), 2);
        assert!(errs.iter().any(|d| d.message.contains("undefined")));
        assert!(errs.iter().any(|d| d.message.contains("x20")));
    }

    #[test]
    fn unsupported_directive_diagnostic() {
        let errs = assemble(".macro foo\n").unwrap_err();
        assert!(errs[0].message.contains("not supported"));
    }

    #[test]
    fn assembles_all_examples_to_valid_words() {
        for (name, src) in [
            (
                "peripheral_io_read",
                include_str!("../../../examples/peripheral_io_read.s"),
            ),
            (
                "oob_smbus_msg",
                include_str!("../../../examples/oob_smbus_msg.s"),
            ),
            (
                "virtual_wire_pltrst",
                include_str!("../../../examples/virtual_wire_pltrst.s"),
            ),
            (
                "flash_completion",
                include_str!("../../../examples/flash_completion.s"),
            ),
        ] {
            let prog = assemble(src).unwrap_or_else(|d| panic!("{name} failed to assemble: {d:?}"));
            // every emitted word must decode back to a valid Instr
            for w in prog.words() {
                assert!(
                    Instr::decode(w).is_ok(),
                    "{name}: word {w:#010x} does not decode"
                );
            }
        }
        // anchor: the first instruction of peripheral_io_read is the SET_CONFIG
        let p = assemble(include_str!("../../../examples/peripheral_io_read.s")).unwrap();
        assert_eq!(p.words().next(), Some(0x5800_0000));
    }
}
