//! `tamal-asm` — the assembler for the tamal ISA.
//!
//! Parses RISC-V-flavored tamal assembly and emits tamal bytecode in the
//! [`tamal_abi::isa`] encoding. The public API is [`assemble`] (source ->
//! [`Program`] or a `Vec<`[`Diagnostic`]`>`); the CLI renders diagnostics with
//! ariadne. See `docs/superpowers/specs/2026-07-04-tamal-asm-design.md`.

#![forbid(unsafe_code)]

pub mod diagnostics;
pub mod lexer;

pub use diagnostics::{Diagnostic, Severity, Span};
