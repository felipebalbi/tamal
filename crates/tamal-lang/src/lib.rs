//! `tamal-lang` — the compiler for tamal-lang, a high-level language that lowers
//! to tamal assembly text and, through [`tamal_asm::assemble`], to tamal
//! bytecode. See `docs/superpowers/specs/2026-07-20-tamal-lang-design.md`.

#![forbid(unsafe_code)]

pub use tamal_asm::{Diagnostic, Severity, Span};

pub mod emit;
pub mod lexer;
pub mod parser;
