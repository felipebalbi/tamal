# tamal-lang Plan 1 — Walking Skeleton Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stand up the `tamal-lang` compiler crate and its `tamalc` CLI end-to-end for the smallest useful language subset — `test NAME { … }`, `pass`, `fail N`, and verbatim raw-instruction pass-through — proving the full pipeline (lex → parse → emit tamal-asm text → `tamal_asm::assemble` → bytecode) against the hand-written `examples/smoke_halt.s`.

**Architecture:** A structured front-end (lexer → parser → emit) lowers `.tam` source to annotated **tamal-asm text**, then calls the existing `tamal_asm::assemble` backend for encoding, `li`-tiling, label math, and the 1024-word cap (design decision D8). Diagnostics reuse `tamal_asm::{Diagnostic, Severity, Span}` with `.tam` byte spans. The CLI mirrors `tamal-asm-cli` (clap + ariadne). This is the first of six incremental plans; it deliberately supports only the subset needed to compile `smoke_halt` and to pass raw instructions through unchanged, establishing every module and the backend seam for later plans to grow.

**Tech Stack:** Rust (edition 2024, rust-version 1.85), `tamal-asm` + `tamal-abi` (workspace path deps), `clap` (derive), `color-eyre`, `ariadne`.

**Spec:** `docs/superpowers/specs/2026-07-20-tamal-lang-design.md` (esp. §4.1 conventions, §5 lowering table, §6 compiler architecture, D8).

---

## File Structure

Two new crates under `crates/` (MIT, auto-included by the workspace `members = ["crates/*"]`):

- `crates/tamal-lang/` — the compiler library.
  - `Cargo.toml` — package + deps (`tamal-asm`; dev-dep on nothing beyond `tamal-asm`).
  - `src/lib.rs` — public seam: `lower_to_asm`, `compile`; re-exports `Diagnostic`/`Severity`/`Span`; declares modules.
  - `src/lexer.rs` — `.tam` source → `Vec<Token>` (`//` + `/* */` comments, idents, numbers, `{`, `}`, `,`, newlines).
  - `src/parser.rs` — `Vec<Token>` → `Module { tests: Vec<Test> }` AST.
  - `src/emit.rs` — `Module` → tamal-asm text.
  - `tests/examples.rs` — end-to-end: `.tam` → bytecode == hand-written `.s`.
- `crates/tamal-lang-cli/` — the `tamalc` binary.
  - `Cargo.toml` — `[[bin]] name = "tamalc"`; deps `tamal-lang`, `tamal-asm`, `clap`, `color-eyre`, `ariadne`.
  - `src/main.rs` — `tamalc compile <input> --emit bin|asm|listing`.
- `crates/tamal-lang/../../../examples/smoke.tam` — a `.tam` smoke test for manual CLI verification.
- Root `Cargo.toml` — add `tamal-lang` to `[workspace.dependencies]`.

Each front-end module has one job and is small enough to hold in context. Later plans add `src/consteval.rs`, `src/resolve.rs`, `src/expand.rs` without disturbing these.

---

## Task 1: Scaffold both crates (compiling skeleton)

**Files:**
- Modify: `Cargo.toml` (root — add workspace dep)
- Create: `crates/tamal-lang/Cargo.toml`
- Create: `crates/tamal-lang/src/lib.rs`
- Create: `crates/tamal-lang-cli/Cargo.toml`
- Create: `crates/tamal-lang-cli/src/main.rs`

- [ ] **Step 1: Add the library crate to the workspace dependency table**

Modify `Cargo.toml` (root). Under `[workspace.dependencies]`, in the "Intra-workspace libs" block, add the `tamal-lang` line so other crates (the CLI) can depend on it:

```toml
# Intra-workspace libs.
tamal-abi = { path = "crates/tamal-abi" }
tamal-asm = { path = "crates/tamal-asm" }
tamal-lang = { path = "crates/tamal-lang" }
tamal-loader = { path = "crates/tamal-loader" }
```

- [ ] **Step 2: Create the library crate manifest**

Create `crates/tamal-lang/Cargo.toml`:

```toml
[package]
name = "tamal-lang"
version = "0.1.0"
description = "Compiler for tamal-lang: a high-level language that lowers to tamal assembly."
edition.workspace = true
rust-version.workspace = true
authors.workspace = true
repository.workspace = true
license.workspace = true

[dependencies]
tamal-asm.workspace = true
```

- [ ] **Step 3: Create the library root with the public seam**

Create `crates/tamal-lang/src/lib.rs`. In this task it only re-exports diagnostics and declares the (currently empty) module set is deferred — keep it minimal so the workspace builds. The real modules land in Tasks 2–5.

```rust
//! `tamal-lang` — the compiler for tamal-lang, a high-level language that lowers
//! to tamal assembly text and, through [`tamal_asm::assemble`], to tamal
//! bytecode. See `docs/superpowers/specs/2026-07-20-tamal-lang-design.md`.

#![forbid(unsafe_code)]

pub use tamal_asm::{Diagnostic, Severity, Span};
```

- [ ] **Step 4: Create the CLI crate manifest**

Create `crates/tamal-lang-cli/Cargo.toml`:

```toml
[package]
name = "tamal-lang-cli"
version = "0.1.0"
description = "Command-line front-end for the tamal-lang compiler (tamalc)."
edition.workspace = true
rust-version.workspace = true
authors.workspace = true
repository.workspace = true
license.workspace = true

[[bin]]
name = "tamalc"
path = "src/main.rs"

[dependencies]
tamal-lang.workspace = true
tamal-asm.workspace = true
clap.workspace = true
color-eyre.workspace = true
ariadne.workspace = true
```

- [ ] **Step 5: Create a trivial CLI entry point (replaced in Task 6)**

Create `crates/tamal-lang-cli/src/main.rs`:

```rust
//! `tamalc` — command-line front-end for the tamal-lang compiler.
//! (Skeleton; the real command lands in the CLI task.)

fn main() {
    eprintln!("tamalc: not yet implemented");
}
```

- [ ] **Step 6: Build the whole workspace**

Run: `cargo build`
Expected: compiles cleanly, including the two new crates (`Compiling tamal-lang v0.1.0`, `Compiling tamal-lang-cli v0.1.0`), 0 errors.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml crates/tamal-lang/Cargo.toml crates/tamal-lang/src/lib.rs crates/tamal-lang-cli/Cargo.toml crates/tamal-lang-cli/src/main.rs
git commit -m "feat(tamal-lang): scaffold compiler + tamalc CLI crates"
```

---

## Task 2: Lexer

**Files:**
- Create: `crates/tamal-lang/src/lexer.rs`
- Modify: `crates/tamal-lang/src/lib.rs` (declare `pub mod lexer;`)

- [ ] **Step 1: Write the failing tests**

Create `crates/tamal-lang/src/lexer.rs` with only the test module first (it will fail to compile — that is the "red"):

```rust
//! Lexer: `.tam` source text → tokens with byte spans. Skips `//` line and
//! `/* */` block comments; recognizes identifiers, numbers, `{`, `}`, `,`,
//! and newlines (statement separators).

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<Tok> {
        lex(src).unwrap().into_iter().map(|t| t.kind).collect()
    }

    #[test]
    fn lexes_test_block() {
        assert_eq!(
            kinds("test smoke {\n  pass\n}\n"),
            vec![
                Tok::Ident, Tok::Ident, Tok::LBrace, Tok::Newline,
                Tok::Ident, Tok::Newline,
                Tok::RBrace, Tok::Newline,
                Tok::Eof,
            ]
        );
    }

    #[test]
    fn skips_comments_and_lexes_numbers() {
        // a line comment, a block comment, then `fail 0x11`
        let src = "// hi\n/* block */ fail 0x11\n";
        assert_eq!(
            kinds(src),
            vec![Tok::Newline, Tok::Ident, Tok::Number, Tok::Newline, Tok::Eof]
        );
    }

    #[test]
    fn number_lexeme_is_verbatim() {
        let toks = lex("mark 1, x1\n").unwrap();
        // toks: Ident(mark) Number(1) Comma Ident(x1) Newline Eof
        assert_eq!(&"mark 1, x1\n"[toks[1].span.clone()], "1");
        assert_eq!(&"mark 1, x1\n"[toks[3].span.clone()], "x1");
    }

    #[test]
    fn rejects_unexpected_char() {
        let err = lex("test @\n").unwrap_err();
        assert!(err[0].message.contains("unexpected character"));
    }

    #[test]
    fn rejects_unterminated_block_comment() {
        let err = lex("/* nope").unwrap_err();
        assert!(err[0].message.contains("unterminated"));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p tamal-lang lexer`
Expected: FAIL to compile — `cannot find type Tok`, `cannot find function lex`.

- [ ] **Step 3: Implement the lexer**

Prepend the implementation above the test module in `crates/tamal-lang/src/lexer.rs`:

```rust
use tamal_asm::{Diagnostic, Span};

/// Token kinds for the Plan-1 subset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tok {
    /// An identifier: keyword, mnemonic, register, or name.
    Ident,
    /// A numeric literal (decimal / `0x` / `0b`, `_` separators allowed).
    Number,
    /// `{`
    LBrace,
    /// `}`
    RBrace,
    /// `,`
    Comma,
    /// A newline — the statement separator.
    Newline,
    /// End of input.
    Eof,
}

/// A lexed token: its kind and the byte span it covers in the source.
#[derive(Debug, Clone)]
pub struct Token {
    pub kind: Tok,
    pub span: Span,
}

/// Lex `src` into tokens, or return the first lexical error as a diagnostic.
pub fn lex(src: &str) -> Result<Vec<Token>, Vec<Diagnostic>> {
    let b = src.as_bytes();
    let mut i = 0usize;
    let mut toks = Vec::new();
    while i < b.len() {
        let c = b[i];
        match c {
            b' ' | b'\t' | b'\r' => i += 1,
            b'\n' => {
                toks.push(Token { kind: Tok::Newline, span: i..i + 1 });
                i += 1;
            }
            b'/' if i + 1 < b.len() && b[i + 1] == b'/' => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if i + 1 < b.len() && b[i + 1] == b'*' => {
                let start = i;
                i += 2;
                loop {
                    if i + 1 < b.len() && b[i] == b'*' && b[i + 1] == b'/' {
                        i += 2;
                        break;
                    }
                    if i >= b.len() {
                        return Err(vec![Diagnostic::error(
                            start..b.len(),
                            "unterminated block comment",
                        )]);
                    }
                    i += 1;
                }
            }
            b'{' => {
                toks.push(Token { kind: Tok::LBrace, span: i..i + 1 });
                i += 1;
            }
            b'}' => {
                toks.push(Token { kind: Tok::RBrace, span: i..i + 1 });
                i += 1;
            }
            b',' => {
                toks.push(Token { kind: Tok::Comma, span: i..i + 1 });
                i += 1;
            }
            _ if is_ident_start(c) => {
                let start = i;
                i += 1;
                while i < b.len() && is_ident_continue(b[i]) {
                    i += 1;
                }
                toks.push(Token { kind: Tok::Ident, span: start..i });
            }
            _ if c.is_ascii_digit() => {
                let start = i;
                i += 1;
                while i < b.len() && is_number_continue(b[i]) {
                    i += 1;
                }
                toks.push(Token { kind: Tok::Number, span: start..i });
            }
            _ => {
                return Err(vec![Diagnostic::error(
                    i..i + 1,
                    format!("unexpected character `{}`", c as char),
                )]);
            }
        }
    }
    toks.push(Token { kind: Tok::Eof, span: b.len()..b.len() });
    Ok(toks)
}

fn is_ident_start(c: u8) -> bool {
    c == b'_' || c.is_ascii_alphabetic()
}

fn is_ident_continue(c: u8) -> bool {
    c == b'_' || c.is_ascii_alphanumeric()
}

// Numbers start with a digit; the tail allows hex/bin digits and `_`
// separators (e.g. `0xDE_AD`, `0b0110`, `100`).
fn is_number_continue(c: u8) -> bool {
    c == b'_' || c.is_ascii_alphanumeric()
}
```

- [ ] **Step 4: Declare the module**

Modify `crates/tamal-lang/src/lib.rs` — add the module declaration after the `pub use`:

```rust
pub use tamal_asm::{Diagnostic, Severity, Span};

pub mod lexer;
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p tamal-lang lexer`
Expected: PASS — 5 tests (`lexes_test_block`, `skips_comments_and_lexes_numbers`, `number_lexeme_is_verbatim`, `rejects_unexpected_char`, `rejects_unterminated_block_comment`).

- [ ] **Step 6: Commit**

```bash
git add crates/tamal-lang/src/lexer.rs crates/tamal-lang/src/lib.rs
git commit -m "feat(tamal-lang): lexer (idents, numbers, comments, braces, newlines)"
```

---

## Task 3: Parser

**Files:**
- Create: `crates/tamal-lang/src/parser.rs`
- Modify: `crates/tamal-lang/src/lib.rs` (declare `pub mod parser;`)

- [ ] **Step 1: Write the failing tests**

Create `crates/tamal-lang/src/parser.rs` with the test module first:

```rust
//! Parser: tokens → the Plan-1 AST (`Module` of one `Test` of statements:
//! `pass`, `fail N`, or a verbatim raw instruction).

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lex;

    fn parse_ok(src: &str) -> Module {
        let toks = lex(src).unwrap();
        parse(src, &toks).unwrap()
    }

    #[test]
    fn parses_pass() {
        let m = parse_ok("test smoke {\n  pass\n}\n");
        assert_eq!(m.tests.len(), 1);
        assert_eq!(m.tests[0].name, "smoke");
        assert!(matches!(m.tests[0].stmts.as_slice(), [Stmt::Pass]));
    }

    #[test]
    fn parses_fail_with_code() {
        let m = parse_ok("test t {\n  fail 0x11\n}\n");
        match &m.tests[0].stmts[0] {
            Stmt::Fail { code, .. } => assert_eq!(code, "0x11"),
            other => panic!("expected Fail, got {other:?}"),
        }
    }

    #[test]
    fn parses_raw_instruction_with_operands() {
        let m = parse_ok("test probe {\n  mark 1, x1\n  pass\n}\n");
        match &m.tests[0].stmts[0] {
            Stmt::Raw { mnemonic, operands, .. } => {
                assert_eq!(mnemonic, "mark");
                assert_eq!(operands, &["1".to_string(), "x1".to_string()]);
            }
            other => panic!("expected Raw, got {other:?}"),
        }
        assert!(matches!(m.tests[0].stmts[1], Stmt::Pass));
    }

    #[test]
    fn parses_zero_operand_instruction() {
        let m = parse_ok("test t {\n  cs_assert\n  pass\n}\n");
        match &m.tests[0].stmts[0] {
            Stmt::Raw { mnemonic, operands, .. } => {
                assert_eq!(mnemonic, "cs_assert");
                assert!(operands.is_empty());
            }
            other => panic!("expected Raw, got {other:?}"),
        }
    }

    #[test]
    fn missing_close_brace_is_an_error() {
        let toks = lex("test t {\n  pass\n").unwrap();
        let err = parse("test t {\n  pass\n", &toks).unwrap_err();
        assert!(err[0].message.contains("missing `}`"));
    }

    #[test]
    fn missing_test_keyword_is_an_error() {
        let toks = lex("smoke {\n  pass\n}\n").unwrap();
        let err = parse("smoke {\n  pass\n}\n", &toks).unwrap_err();
        assert!(err[0].message.contains("expected `test`"));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p tamal-lang parser`
Expected: FAIL to compile — `cannot find type Module`, `parse`, `Stmt`.

- [ ] **Step 3: Implement the parser**

Prepend to `crates/tamal-lang/src/parser.rs`:

```rust
use crate::lexer::{Tok, Token};
use tamal_asm::{Diagnostic, Span};

/// A parsed `.tam` module: a list of tests (Plan 1 enforces exactly one at
/// lowering time; the AST permits many so later plans can relax it).
#[derive(Debug, Clone)]
pub struct Module {
    pub tests: Vec<Test>,
}

/// A single test = one program entry point.
#[derive(Debug, Clone)]
pub struct Test {
    pub name: String,
    pub name_span: Span,
    pub stmts: Vec<Stmt>,
}

/// A Plan-1 statement.
#[derive(Debug, Clone)]
pub enum Stmt {
    /// `pass` → `halt 0x00`.
    Pass,
    /// `fail N` → `halt N`. `code` is the verbatim number lexeme.
    Fail { code: String, span: Span },
    /// A verbatim instruction: `mnemonic op, op, …` emitted 1:1 to asm text.
    Raw {
        mnemonic: String,
        operands: Vec<String>,
        span: Span,
    },
}

/// Parse tokens into a [`Module`], or return diagnostics.
pub fn parse(src: &str, toks: &[Token]) -> Result<Module, Vec<Diagnostic>> {
    let mut p = P { src, toks, i: 0 };
    let mut tests = Vec::new();
    p.skip_newlines();
    while p.peek() != Tok::Eof {
        tests.push(p.parse_test()?);
        p.skip_newlines();
    }
    Ok(Module { tests })
}

struct P<'a> {
    src: &'a str,
    toks: &'a [Token],
    i: usize,
}

impl<'a> P<'a> {
    fn peek(&self) -> Tok {
        self.toks.get(self.i).map(|t| t.kind).unwrap_or(Tok::Eof)
    }

    fn span(&self) -> Span {
        self.toks
            .get(self.i)
            .map(|t| t.span.clone())
            .unwrap_or(self.src.len()..self.src.len())
    }

    fn lexeme(&self, span: &Span) -> &'a str {
        &self.src[span.clone()]
    }

    fn skip_newlines(&mut self) {
        while self.peek() == Tok::Newline {
            self.i += 1;
        }
    }

    fn expect(&mut self, kind: Tok, what: &str) -> Result<Token, Vec<Diagnostic>> {
        if self.peek() == kind {
            let t = self.toks[self.i].clone();
            self.i += 1;
            Ok(t)
        } else {
            Err(vec![Diagnostic::error(self.span(), format!("expected {what}"))])
        }
    }

    fn expect_ident(&mut self) -> Result<Span, Vec<Diagnostic>> {
        Ok(self.expect(Tok::Ident, "an identifier")?.span)
    }

    fn parse_test(&mut self) -> Result<Test, Vec<Diagnostic>> {
        let kw = self.expect_ident()?;
        if self.lexeme(&kw) != "test" {
            return Err(vec![Diagnostic::error(kw, "expected `test`")]);
        }
        let name_span = self.expect_ident()?;
        let name = self.lexeme(&name_span).to_string();
        self.expect(Tok::LBrace, "`{`")?;
        let mut stmts = Vec::new();
        loop {
            self.skip_newlines();
            match self.peek() {
                Tok::RBrace => {
                    self.i += 1;
                    break;
                }
                Tok::Eof => {
                    return Err(vec![Diagnostic::error(
                        self.span(),
                        "unexpected end of file: missing `}`",
                    )]);
                }
                _ => stmts.push(self.parse_stmt()?),
            }
        }
        Ok(Test { name, name_span, stmts })
    }

    fn parse_stmt(&mut self) -> Result<Stmt, Vec<Diagnostic>> {
        let head = self.expect_ident()?;
        let word = self.lexeme(&head).to_string();
        match word.as_str() {
            "pass" => {
                self.end_stmt()?;
                Ok(Stmt::Pass)
            }
            "fail" => {
                let n = self.expect(Tok::Number, "a verdict number")?;
                let code = self.lexeme(&n.span).to_string();
                let span = head.start..n.span.end;
                self.end_stmt()?;
                Ok(Stmt::Fail { code, span })
            }
            _ => {
                let mut operands = Vec::new();
                let mut end = head.end;
                if !self.at_stmt_end() {
                    loop {
                        let op = self.parse_operand()?;
                        end = op.end;
                        operands.push(self.lexeme(&op).to_string());
                        if self.peek() == Tok::Comma {
                            self.i += 1;
                            continue;
                        }
                        break;
                    }
                }
                self.end_stmt()?;
                Ok(Stmt::Raw {
                    mnemonic: word,
                    operands,
                    span: head.start..end,
                })
            }
        }
    }

    fn parse_operand(&mut self) -> Result<Span, Vec<Diagnostic>> {
        match self.peek() {
            Tok::Number | Tok::Ident => {
                let s = self.span();
                self.i += 1;
                Ok(s)
            }
            _ => Err(vec![Diagnostic::error(
                self.span(),
                "expected an operand (number or register/name)",
            )]),
        }
    }

    fn at_stmt_end(&self) -> bool {
        matches!(self.peek(), Tok::Newline | Tok::RBrace | Tok::Eof)
    }

    fn end_stmt(&mut self) -> Result<(), Vec<Diagnostic>> {
        match self.peek() {
            Tok::Newline => {
                self.i += 1;
                Ok(())
            }
            Tok::RBrace | Tok::Eof => Ok(()),
            _ => Err(vec![Diagnostic::error(
                self.span(),
                "expected end of statement (newline or `}`)",
            )]),
        }
    }
}
```

- [ ] **Step 4: Declare the module**

Modify `crates/tamal-lang/src/lib.rs`:

```rust
pub mod lexer;
pub mod parser;
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p tamal-lang parser`
Expected: PASS — 6 tests.

- [ ] **Step 6: Commit**

```bash
git add crates/tamal-lang/src/parser.rs crates/tamal-lang/src/lib.rs
git commit -m "feat(tamal-lang): parser (test/pass/fail/raw-instruction AST)"
```

---

## Task 4: Emit

**Files:**
- Create: `crates/tamal-lang/src/emit.rs`
- Modify: `crates/tamal-lang/src/lib.rs` (declare `pub mod emit;`)

- [ ] **Step 1: Write the failing tests**

Create `crates/tamal-lang/src/emit.rs` with the test module first:

```rust
//! Emit: lower a `Module` (Plan-1 subset) to tamal-asm text. A `test` becomes
//! the entry label; `pass`/`fail`/raw statements become instruction lines.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::{Module, Stmt, Test};

    fn one(stmts: Vec<Stmt>) -> Module {
        Module {
            tests: vec![Test { name: "t".into(), name_span: 0..1, stmts }],
        }
    }

    #[test]
    fn emits_entry_and_pass() {
        let asm = emit(&one(vec![Stmt::Pass]));
        assert_eq!(asm, ".globl _start\n_start:\n\thalt 0x00\n");
    }

    #[test]
    fn emits_fail_code_verbatim() {
        let asm = emit(&one(vec![Stmt::Fail { code: "0x11".into(), span: 0..1 }]));
        assert_eq!(asm, ".globl _start\n_start:\n\thalt 0x11\n");
    }

    #[test]
    fn emits_raw_instructions() {
        let asm = emit(&one(vec![
            Stmt::Raw { mnemonic: "cs_assert".into(), operands: vec![], span: 0..1 },
            Stmt::Raw {
                mnemonic: "mark".into(),
                operands: vec!["1".into(), "x1".into()],
                span: 0..1,
            },
            Stmt::Pass,
        ]));
        assert_eq!(
            asm,
            ".globl _start\n_start:\n\tcs_assert\n\tmark 1, x1\n\thalt 0x00\n"
        );
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p tamal-lang emit`
Expected: FAIL to compile — `cannot find function emit`.

- [ ] **Step 3: Implement emit**

Prepend to `crates/tamal-lang/src/emit.rs`:

```rust
use crate::parser::{Module, Stmt};

/// Lower a Plan-1 `Module` (exactly one test, enforced by the driver) to
/// tamal-asm text: the entry label followed by one line per statement.
pub fn emit(module: &Module) -> String {
    let mut out = String::new();
    for test in &module.tests {
        out.push_str(".globl _start\n");
        out.push_str("_start:\n");
        for stmt in &test.stmts {
            match stmt {
                Stmt::Pass => out.push_str("\thalt 0x00\n"),
                Stmt::Fail { code, .. } => {
                    out.push_str("\thalt ");
                    out.push_str(code);
                    out.push('\n');
                }
                Stmt::Raw { mnemonic, operands, .. } => {
                    out.push('\t');
                    out.push_str(mnemonic);
                    if !operands.is_empty() {
                        out.push(' ');
                        out.push_str(&operands.join(", "));
                    }
                    out.push('\n');
                }
            }
        }
    }
    out
}
```

- [ ] **Step 4: Declare the module**

Modify `crates/tamal-lang/src/lib.rs`:

```rust
pub mod emit;
pub mod lexer;
pub mod parser;
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p tamal-lang emit`
Expected: PASS — 3 tests.

- [ ] **Step 6: Commit**

```bash
git add crates/tamal-lang/src/emit.rs crates/tamal-lang/src/lib.rs
git commit -m "feat(tamal-lang): emit test/pass/fail/raw to tamal-asm text"
```

---

## Task 5: Driver (`lower_to_asm` + `compile`)

**Files:**
- Modify: `crates/tamal-lang/src/lib.rs` (add the two entry functions + unit tests)

- [ ] **Step 1: Write the failing tests**

Modify `crates/tamal-lang/src/lib.rs` — append a test module:

```rust
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
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p tamal-lang --lib tests`
Expected: FAIL to compile — `cannot find function lower_to_asm`, `compile`.

- [ ] **Step 3: Implement the driver**

Modify `crates/tamal-lang/src/lib.rs` — add module wiring and the two functions after the `pub mod` lines (before the test module):

```rust
pub mod emit;
pub mod lexer;
pub mod parser;

use tamal_asm::Program;

/// Lower tamal-lang source to tamal-asm text (the `--emit asm` artifact).
///
/// Plan 1 requires exactly one `test` per file (one program entry point);
/// zero or many is a diagnostic.
pub fn lower_to_asm(source: &str) -> Result<String, Vec<Diagnostic>> {
    let toks = lexer::lex(source)?;
    let module = parser::parse(source, &toks)?;
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
    Ok(emit::emit(&module))
}

/// Compile tamal-lang source to a tamal [`Program`] (bytecode), lowering to
/// asm text and handing it to the [`tamal_asm::assemble`] backend.
pub fn compile(source: &str) -> Result<Program, Vec<Diagnostic>> {
    let asm = lower_to_asm(source)?;
    tamal_asm::assemble(&asm)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p tamal-lang --lib`
Expected: PASS — the 4 new driver tests plus all lexer/parser/emit unit tests.

- [ ] **Step 5: Commit**

```bash
git add crates/tamal-lang/src/lib.rs
git commit -m "feat(tamal-lang): driver lower_to_asm + compile (one-test rule)"
```

---

## Task 6: CLI (`tamalc`)

**Files:**
- Modify: `crates/tamal-lang-cli/src/main.rs` (replace the stub)

- [ ] **Step 1: Replace the stub with the real command**

Overwrite `crates/tamal-lang-cli/src/main.rs`:

```rust
//! `tamalc` — command-line front-end for the tamal-lang compiler. Compiles
//! `.tam` source to bytecode, generated assembly, or a listing. Front-end
//! diagnostics point at `.tam` source; the rare backend error points at the
//! generated assembly (a compiler bug). Rendering uses ariadne, mirroring
//! `tamal-asm-cli`.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use ariadne::{Color, Label, Report, ReportKind, Source};
use clap::{Parser, Subcommand, ValueEnum};
use color_eyre::eyre::{Context, Result};

use tamal_lang::Diagnostic;

/// Compile tamal-lang source.
#[derive(Debug, Parser)]
#[command(name = "tamalc", version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Compile `.tam` source into bytecode, assembly, or a listing.
    Compile {
        /// Input `.tam` source file.
        input: PathBuf,
        /// Output path (default: `<input>.bin` for bin; stdout for asm/listing).
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Output format.
        #[arg(long, value_enum, default_value_t = Emit::Bin)]
        emit: Emit,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Emit {
    /// Raw little-endian words (loader-ready).
    Bin,
    /// The generated tamal assembly (the trust bridge).
    Asm,
    /// `addr  word  mnemonic ; source` table over the generated assembly.
    Listing,
}

fn main() -> Result<ExitCode> {
    color_eyre::install()?;
    let cli = Cli::parse();
    match cli.command {
        Command::Compile { input, output, emit } => cmd_compile(&input, output, emit),
    }
}

fn cmd_compile(input: &Path, output: Option<PathBuf>, emit: Emit) -> Result<ExitCode> {
    let source =
        fs::read_to_string(input).wrap_err_with(|| format!("reading {}", input.display()))?;
    let name = input.display().to_string();

    // Stage 1: lower to asm. Front-end diagnostics point at `.tam` source.
    let asm = match tamal_lang::lower_to_asm(&source) {
        Ok(a) => a,
        Err(diags) => {
            for d in &diags {
                report(&name, &source, d);
            }
            return Ok(ExitCode::FAILURE);
        }
    };

    if let Emit::Asm = emit {
        write_text(output, &asm)?;
        return Ok(ExitCode::SUCCESS);
    }

    // Stage 2: assemble. A backend error is a compiler bug; render it against
    // the generated assembly so it is still legible.
    let prog = match tamal_asm::assemble(&asm) {
        Ok(p) => p,
        Err(diags) => {
            let asm_name = format!("{name} (generated asm)");
            for d in &diags {
                report(&asm_name, &asm, d);
            }
            eprintln!("note: this error is in compiler-generated assembly — please report it");
            return Ok(ExitCode::FAILURE);
        }
    };

    match emit {
        Emit::Bin => {
            let out = output.unwrap_or_else(|| input.with_extension("bin"));
            fs::write(&out, prog.to_le_bytes())
                .wrap_err_with(|| format!("writing {}", out.display()))?;
        }
        Emit::Listing => write_text(output, &prog.listing(&asm))?,
        Emit::Asm => unreachable!("handled above"),
    }
    Ok(ExitCode::SUCCESS)
}

fn write_text(output: Option<PathBuf>, text: &str) -> Result<()> {
    match output {
        Some(path) => {
            fs::write(&path, text).wrap_err_with(|| format!("writing {}", path.display()))?;
        }
        None => std::io::stdout()
            .write_all(text.as_bytes())
            .wrap_err("writing stdout")?,
    }
    Ok(())
}

fn report(name: &str, source: &str, d: &Diagnostic) {
    let mut b = Report::build(ReportKind::Error, (name, d.primary.clone()))
        .with_message(&d.message)
        .with_label(
            Label::new((name, d.primary.clone()))
                .with_message(&d.message)
                .with_color(Color::Red),
        );
    for (span, text) in &d.labels {
        b = b.with_label(
            Label::new((name, span.clone()))
                .with_message(text)
                .with_color(Color::Yellow),
        );
    }
    if let Some(help) = &d.help {
        b = b.with_help(help);
    }
    let _ = b.finish().eprint((name, Source::from(source)));
}
```

- [ ] **Step 2: Build the CLI**

Run: `cargo build -p tamal-lang-cli`
Expected: compiles cleanly, 0 errors.

- [ ] **Step 3: Create a sample `.tam` for manual verification**

Create `examples/smoke.tam`:

```rust
// smoke.tam — the tamal-lang equivalent of examples/smoke_halt.s
test smoke {
    pass
}
```

- [ ] **Step 4: Verify `--emit asm` (the trust bridge)**

Run: `cargo run -q -p tamal-lang-cli -- compile examples/smoke.tam --emit asm`
Expected output (exactly):

```
.globl _start
_start:
	halt 0x00
```

- [ ] **Step 5: Verify `--emit listing`**

Run: `cargo run -q -p tamal-lang-cli -- compile examples/smoke.tam --emit listing`
Expected: a single listing row whose word is `40000000` and mnemonic `halt 0x00`, e.g.:

```
0000  40000000  halt 0x00                    ; halt 0x00
```

- [ ] **Step 6: Verify a diagnostic renders against `.tam` source**

Run: `cargo run -q -p tamal-lang-cli -- compile examples/smoke.tam --emit asm` after temporarily editing `examples/smoke.tam` to remove the closing `}` (then restore it).
Expected: an ariadne error citing `examples/smoke.tam` with message `unexpected end of file: missing \`}\``, and a non-zero exit. Restore the file afterward.

- [ ] **Step 7: Commit**

```bash
git add crates/tamal-lang-cli/src/main.rs examples/smoke.tam
git commit -m "feat(tamalc): compile CLI (--emit bin|asm|listing) with ariadne diagnostics"
```

---

## Task 7: End-to-end integration tests

**Files:**
- Create: `crates/tamal-lang/tests/examples.rs`

- [ ] **Step 1: Write the failing tests**

Create `crates/tamal-lang/tests/examples.rs`:

```rust
//! End-to-end: tamal-lang source compiles to the same bytecode as the
//! hand-written `.s` references, proving the full lex→parse→emit→assemble path.

#[test]
fn smoke_matches_hand_written_asm() {
    let prog = tamal_lang::compile("test smoke {\n    pass\n}\n").expect("compile smoke.tam");
    let reference =
        tamal_asm::assemble(include_str!("../../../examples/smoke_halt.s")).expect("assemble ref");
    assert_eq!(
        prog.to_le_bytes(),
        reference.to_le_bytes(),
        "smoke.tam must byte-match smoke_halt.s"
    );
    assert_eq!(prog.words().count(), 1);
}

#[test]
fn raw_instructions_pass_through_and_assemble() {
    // li x1, 0xDEAD (fits signed-21 -> 1 word) + mark 1, x1 + halt = 3 words.
    let tam = "test probe {\n    li x1, 0xDEAD\n    mark 1, x1\n    pass\n}\n";
    let prog = tamal_lang::compile(tam).expect("compile raw pass-through");
    assert_eq!(prog.words().count(), 3);
}
```

- [ ] **Step 2: Run tests to verify they pass**

Run: `cargo test -p tamal-lang --test examples`
Expected: PASS — 2 tests. (These exercise the real backend: `smoke` byte-matches `smoke_halt.s`, and raw instructions round-trip through `li`-tiling + encoding.)

Note: if `smoke_matches_hand_written_asm` fails, print both sides with
`cargo test -p tamal-lang --test examples -- --nocapture` and compare
`prog.listing(&tamal_lang::lower_to_asm(...).unwrap())` against
`reference.listing(include_str!(...))` to locate the divergence.

- [ ] **Step 3: Commit**

```bash
git add crates/tamal-lang/tests/examples.rs
git commit -m "test(tamal-lang): e2e byte-match vs smoke_halt.s + raw pass-through"
```

---

## Task 8: Polish — format, lint, full test

**Files:** none (verification only)

- [ ] **Step 1: Format**

Run: `cargo fmt`
Expected: no diff, or only the new files reformatted. Review with `git diff`.

- [ ] **Step 2: Lint the new crates**

Run: `cargo clippy -p tamal-lang -p tamal-lang-cli -- -D warnings`
Expected: 0 warnings. Fix any clippy findings, re-run.

- [ ] **Step 3: Full workspace test (no regressions elsewhere)**

Run: `cargo test`
Expected: all workspace tests pass, including the existing `tamal-asm` / `tamal-abi` suites and the new `tamal-lang` unit + integration tests.

- [ ] **Step 4: Commit any formatting/lint fixes**

```bash
git add -A
git commit -m "chore(tamal-lang): fmt + clippy clean for the walking skeleton"
```

(If Steps 1–2 produced no changes, skip this commit.)

---

## Definition of done

- `cargo build`, `cargo test`, and `cargo clippy -p tamal-lang -p tamal-lang-cli -- -D warnings` are all green.
- `tamalc compile examples/smoke.tam --emit bin` produces bytecode byte-identical to assembling `examples/smoke_halt.s`.
- `tamalc compile … --emit asm` prints the generated assembly (the trust bridge); `--emit listing` prints the word/mnemonic table.
- A malformed `.tam` yields an ariadne diagnostic pointing at the **`.tam`** source with a non-zero exit.
- The module set (`lexer`, `parser`, `emit`, driver) and the `tamal_asm::assemble` seam are in place for Plan 2 (values, const-eval, `crc8`, `send`) to build on.

## What this plan deliberately excludes (later plans)

Types/const-eval/`crc8`/`send`/`crc_region` (Plan 2); named variables + register allocation, `config`/`frame`/`recv`/`wait_state`/`expect`/verdict sugar (Plan 3); `proc`/`fn` + structured control flow (Plan 4); `import`/modules + bundled `espi` (Plan 5); `--lint` + determinism gates (Plan 6). Raw-instruction pass-through is the Plan-1 stand-in that lets real cycles be written now and keeps the language a strict superset of annotated asm.
