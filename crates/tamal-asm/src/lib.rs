//! `tamal-asm` — the assembler for the tamal ISA.
//!
//! It parses **RISC-V-flavored** tamal assembly and emits tamal bytecode in the
//! [`tamal_abi::isa`] encoding. The accepted surface mirrors familiar RISC-V
//! asm conventions (see the [riscv-asm-manual]): ABI register names
//! (`zero`/`ra`/`sp`/`t0`../`a0`../`s0`..), directives (`.text`, `.data`,
//! `.word`, `.globl`, `.equ`, `.align`, `.macro`, `.option`), numeric local
//! labels (`1f`/`1b`), and pseudo-instructions (`li`, `la`, `mv`, `nop`, `j`,
//! `call`, `ret`, `beqz`). The bus-domain opcodes are tamal-specific, so the
//! output is **not** compatible with a stock RISC-V toolchain.
//!
//! Nothing here is implemented yet — these are placeholders for the structure.
//!
//! [riscv-asm-manual]: https://github.com/riscv-non-isa/riscv-asm-manual

#![forbid(unsafe_code)]

use thiserror::Error;

/// Tokeniser: source text → tokens.
pub mod lexer {
    //! Placeholder — the token stream and lexer land here.
}

/// Parser: tokens → an intermediate representation (labels, directives, instrs).
pub mod parser {
    //! Placeholder — the AST/IR and parser land here.
}

/// Encoder: IR → tamal bytecode words (via [`tamal_abi::isa`]).
pub mod encoder {
    //! Placeholder — instruction encoding + relocation/label fixups land here.
}

/// Errors that can arise while assembling tamal source.
#[derive(Debug, Error)]
pub enum AssembleError {
    /// The assembler is a scaffold; real assembly is not wired up yet.
    #[error("tamal-asm is a scaffold: assembling is not implemented yet")]
    NotImplemented,
}

/// Assemble tamal source into bytecode.
///
/// Placeholder: returns [`AssembleError::NotImplemented`] until the
/// lexer/parser/encoder pipeline lands.
pub fn assemble(_source: &str) -> Result<Vec<u8>, AssembleError> {
    Err(AssembleError::NotImplemented)
}
