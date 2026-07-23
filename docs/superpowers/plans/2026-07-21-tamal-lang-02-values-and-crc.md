# tamal-lang Plan 2 — Values & Compile-Time CRC Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add named `const`s, a compile-time value/expression layer (`bytes` literals, `++` concat, `^` xor, the `crc8`/`len`/`lo`/`hi` builtins), and the `send` / `send … + crc8` / `crc_region` statements, so a test can build an eSPI command packet from named bytes and have its CRC-8 folded at **compile time** — killing the hand-computed CRC byte.

**Architecture:** Extend the existing lexer (bracket/paren/operator tokens) and parser (an `Expr` AST + top-level `const` items + `send`/`crc_region` statements). Add a pure `consteval` module that folds expressions to `Value::{Int, Bytes}`, with `crc8` delegating to `tamal_abi::crc8` (the same code the wire and HDL use, so the folded byte can never drift). Lowering resolves `const`s first, then `emit` evaluates each `send`/`crc_region` to a run of `put_byte 0xNN` lines (still text → `tamal_asm::assemble`, preserving the Plan-1 source map).

**Tech Stack:** Rust (edition 2024), `tamal-asm` + `tamal-abi` (`tamal_abi::crc8`), building on the Plan-1 walking skeleton (`crates/tamal-lang`, `crates/tamal-lang-cli`).

**Spec:** `docs/superpowers/specs/2026-07-20-tamal-lang-design.md` (§4.1 conventions, §4.4 deliberate-wrong, §5 value model + lowering table).

**Deferred to a later plan (NOT in scope here):** general arithmetic operators (`+ - * / % << >> & |`), `enum`, `reg`/named variables, `frame`/`recv`/`wait_state`/`expect`. Plan 2 includes only `^` (for deliberate-wrong) and `++` (concat) as binary operators, and `+` solely as the `send … + crc8` sugar — so there is no `+`-ambiguity to resolve.

---

## File Structure

- `crates/tamal-lang/src/lexer.rs` — **modify**: add `[ ] ( ) = + ++ ^` tokens.
- `crates/tamal-lang/src/parser.rs` — **modify**: add the `Expr` AST + `BinOp`; parse expressions; add top-level `const` items (`Module { consts, tests }`); add `Stmt::Send` and `Stmt::CrcRegion`.
- `crates/tamal-lang/src/consteval.rs` — **create**: `Value`, `Consts`, `eval` (pure compile-time folding + `crc8`/`len`/`lo`/`hi`).
- `crates/tamal-lang/src/emit.rs` — **modify**: `emit(module, consts) -> Result<Lowering, …>`; lower `send`/`crc_region` to `put_byte` runs.
- `crates/tamal-lang/src/lib.rs` — **modify**: resolve `const`s in `lower`, thread `consts` into `emit`, re-export `consteval`.
- `crates/tamal-lang/tests/values.rs` — **create**: end-to-end command-phase byte folding + deliberate-wrong.

Each `send`/`crc_region` lowers to concrete `put_byte 0xNN` lines, so everything downstream (encoding, the 1024-word cap, `--emit asm/listing`, the M1 source map) is unchanged.

---

## Task 1: Lexer — expression tokens

**Files:**
- Modify: `crates/tamal-lang/src/lexer.rs`

- [ ] **Step 1: Write the failing tests**

Add these tests inside the existing `mod tests` in `crates/tamal-lang/src/lexer.rs` (after `rejects_unterminated_block_comment`):

```rust
    #[test]
    fn lexes_bracket_paren_operator_tokens() {
        assert_eq!(
            kinds("send [a, b] + crc8\n"),
            vec![
                Tok::Ident,   // send
                Tok::LBracket,
                Tok::Ident,   // a
                Tok::Comma,
                Tok::Ident,   // b
                Tok::RBracket,
                Tok::Plus,
                Tok::Ident,   // crc8
                Tok::Newline,
                Tok::Eof,
            ]
        );
    }

    #[test]
    fn distinguishes_plus_from_plusplus() {
        assert_eq!(
            kinds("a ++ b ^ (c)\n"),
            vec![
                Tok::Ident,
                Tok::PlusPlus,
                Tok::Ident,
                Tok::Caret,
                Tok::LParen,
                Tok::Ident,
                Tok::RParen,
                Tok::Newline,
                Tok::Eof,
            ]
        );
    }

    #[test]
    fn lexes_equals() {
        assert_eq!(
            kinds("const X = 5\n"),
            vec![
                Tok::Ident, // const
                Tok::Ident, // X
                Tok::Eq,
                Tok::Number,
                Tok::Newline,
                Tok::Eof,
            ]
        );
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p tamal-lang --lib lexer`
Expected: FAIL to compile — `no variant named LBracket/RBracket/LParen/RParen/Plus/PlusPlus/Caret/Eq`.

- [ ] **Step 3: Add the token variants**

In `crates/tamal-lang/src/lexer.rs`, extend the `Tok` enum (add the new variants after `Comma`):

```rust
    /// `,`
    Comma,
    /// `[`
    LBracket,
    /// `]`
    RBracket,
    /// `(`
    LParen,
    /// `)`
    RParen,
    /// `=`
    Eq,
    /// `+` (the `send … + crc8` sugar)
    Plus,
    /// `++` (bytes concatenation)
    PlusPlus,
    /// `^` (bitwise xor, for deliberate-wrong CRCs)
    Caret,
```

- [ ] **Step 4: Lex the new characters**

In `crates/tamal-lang/src/lexer.rs`, in the `match c` inside `lex`, add these arms just before the `_ if is_ident_start(c)` arm. Note `+` needs one char of lookahead to decide `Plus` vs `PlusPlus`:

```rust
            b'[' => {
                toks.push(Token { kind: Tok::LBracket, span: i..i + 1 });
                i += 1;
            }
            b']' => {
                toks.push(Token { kind: Tok::RBracket, span: i..i + 1 });
                i += 1;
            }
            b'(' => {
                toks.push(Token { kind: Tok::LParen, span: i..i + 1 });
                i += 1;
            }
            b')' => {
                toks.push(Token { kind: Tok::RParen, span: i..i + 1 });
                i += 1;
            }
            b'=' => {
                toks.push(Token { kind: Tok::Eq, span: i..i + 1 });
                i += 1;
            }
            b'^' => {
                toks.push(Token { kind: Tok::Caret, span: i..i + 1 });
                i += 1;
            }
            b'+' if i + 1 < b.len() && b[i + 1] == b'+' => {
                toks.push(Token { kind: Tok::PlusPlus, span: i..i + 2 });
                i += 2;
            }
            b'+' => {
                toks.push(Token { kind: Tok::Plus, span: i..i + 1 });
                i += 1;
            }
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p tamal-lang --lib lexer`
Expected: PASS — the three new tests plus the five existing lexer tests.

- [ ] **Step 6: Commit**

```bash
git add crates/tamal-lang/src/lexer.rs
git commit -m "feat(tamal-lang): lexer tokens for [] () = + ++ ^"
```

## Task 2: Expr AST + primary parsing

**Files:**
- Modify: `crates/tamal-lang/src/parser.rs`

- [ ] **Step 1: Write the failing tests**

Add to the `mod tests` in `crates/tamal-lang/src/parser.rs` (after `missing_test_keyword_is_an_error`):

```rust
    fn parse_expr_ok(src: &str) -> Expr {
        let toks = lex(src).unwrap();
        let mut p = P {
            src,
            toks: &toks,
            i: 0,
        };
        p.parse_expr().unwrap()
    }

    #[test]
    fn parses_int_literal() {
        match parse_expr_ok("0x44") {
            Expr::Int { value, .. } => assert_eq!(value, 0x44),
            e => panic!("expected Int, got {e:?}"),
        }
    }

    #[test]
    fn parses_name() {
        match parse_expr_ok("PUT_IORD1") {
            Expr::Name { name, .. } => assert_eq!(name, "PUT_IORD1"),
            e => panic!("expected Name, got {e:?}"),
        }
    }

    #[test]
    fn parses_bytes_literal() {
        match parse_expr_ok("[0x44, 0x00, 0x64]") {
            Expr::Bytes { elems, .. } => assert_eq!(elems.len(), 3),
            e => panic!("expected Bytes, got {e:?}"),
        }
    }

    #[test]
    fn parses_call() {
        match parse_expr_ok("crc8(pkt)") {
            Expr::Call { func, .. } => assert_eq!(func, "crc8"),
            e => panic!("expected Call, got {e:?}"),
        }
    }

    #[test]
    fn parses_parenthesized() {
        match parse_expr_ok("(0x05)") {
            Expr::Int { value, .. } => assert_eq!(value, 5),
            e => panic!("expected Int, got {e:?}"),
        }
    }

    #[test]
    fn rejects_bad_number() {
        let toks = lex("0xZZ").unwrap();
        let mut p = P {
            src: "0xZZ",
            toks: &toks,
            i: 0,
        };
        assert!(p.parse_expr().is_err());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p tamal-lang --lib parser`
Expected: FAIL to compile — `cannot find type Expr`, `no method parse_expr`.

- [ ] **Step 3: Add the `Expr` AST + `parse_number`**

In `crates/tamal-lang/src/parser.rs`, add after the `Stmt` enum (before `parse`):

```rust
/// A compile-time expression.
#[derive(Debug, Clone)]
pub enum Expr {
    /// An integer literal (already parsed from its lexeme).
    Int { value: i64, span: Span },
    /// A reference to a `const` by name.
    Name { name: String, span: Span },
    /// A byte-string literal `[e, e, …]` (each element is a byte).
    Bytes { elems: Vec<Expr>, span: Span },
    /// A builtin call: `crc8(e)`, `len(e)`, `lo(e)`, `hi(e)`.
    Call {
        func: String,
        arg: Box<Expr>,
        span: Span,
    },
    /// A binary operation (`^` on ints, `++` on bytes).
    Binary {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
        span: Span,
    },
}

/// The binary operators available in the Plan-2 subset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    /// `^` — bitwise xor on integers (for deliberate-wrong CRCs).
    Xor,
    /// `++` — bytes concatenation.
    Concat,
}

impl Expr {
    /// The source span covering this expression.
    pub fn span(&self) -> Span {
        match self {
            Expr::Int { span, .. }
            | Expr::Name { span, .. }
            | Expr::Bytes { span, .. }
            | Expr::Call { span, .. }
            | Expr::Binary { span, .. } => span.clone(),
        }
    }
}

/// Parse a numeric lexeme (`0x..`, `0b..`, decimal, `_` separators allowed).
fn parse_number(lexeme: &str) -> Option<i64> {
    let s: String = lexeme.chars().filter(|&c| c != '_').collect();
    if let Some(h) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        i64::from_str_radix(h, 16).ok()
    } else if let Some(b) = s.strip_prefix("0b").or_else(|| s.strip_prefix("0B")) {
        i64::from_str_radix(b, 2).ok()
    } else {
        s.parse::<i64>().ok()
    }
}
```

- [ ] **Step 4: Add the primary-expression parser**

In `crates/tamal-lang/src/parser.rs`, add these methods inside `impl<'a> P<'a>` (e.g. after `parse_operand`):

```rust
    /// Parse an expression. Plan 2: primary only (binary ops added next task).
    fn parse_expr(&mut self) -> Result<Expr, Vec<Diagnostic>> {
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<Expr, Vec<Diagnostic>> {
        match self.peek() {
            Tok::Number => {
                let sp = self.span();
                self.i += 1;
                let value = parse_number(self.lexeme(&sp))
                    .ok_or_else(|| vec![Diagnostic::error(sp.clone(), "invalid number literal")])?;
                Ok(Expr::Int { value, span: sp })
            }
            Tok::Ident => {
                let sp = self.span();
                self.i += 1;
                let name = self.lexeme(&sp).to_string();
                if self.peek() == Tok::LParen {
                    self.i += 1;
                    let arg = self.parse_expr()?;
                    let close = self.expect(Tok::RParen, "`)`")?;
                    Ok(Expr::Call {
                        func: name,
                        arg: Box::new(arg),
                        span: sp.start..close.span.end,
                    })
                } else {
                    Ok(Expr::Name { name, span: sp })
                }
            }
            Tok::LBracket => {
                let start = self.span().start;
                self.i += 1;
                let mut elems = Vec::new();
                if self.peek() != Tok::RBracket {
                    loop {
                        elems.push(self.parse_expr()?);
                        if self.peek() == Tok::Comma {
                            self.i += 1;
                            continue;
                        }
                        break;
                    }
                }
                let close = self.expect(Tok::RBracket, "`]`")?;
                Ok(Expr::Bytes {
                    elems,
                    span: start..close.span.end,
                })
            }
            Tok::LParen => {
                self.i += 1;
                let e = self.parse_expr()?;
                self.expect(Tok::RParen, "`)`")?;
                Ok(e)
            }
            _ => Err(vec![Diagnostic::error(self.span(), "expected an expression")]),
        }
    }
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p tamal-lang --lib parser`
Expected: PASS — the 6 new expression tests plus the 6 existing parser tests.

- [ ] **Step 6: Commit**

```bash
git add crates/tamal-lang/src/parser.rs
git commit -m "feat(tamal-lang): Expr AST + primary-expression parsing"
```

## Task 3: Binary-expression parsing (`^`, `++`)

**Files:**
- Modify: `crates/tamal-lang/src/parser.rs`

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `crates/tamal-lang/src/parser.rs`:

```rust
    #[test]
    fn parses_concat() {
        match parse_expr_ok("a ++ b") {
            Expr::Binary {
                op: BinOp::Concat, ..
            } => {}
            e => panic!("expected Concat, got {e:?}"),
        }
    }

    #[test]
    fn parses_xor() {
        match parse_expr_ok("0x16 ^ 0xFF") {
            Expr::Binary {
                op: BinOp::Xor,
                lhs,
                rhs,
                ..
            } => {
                assert!(matches!(*lhs, Expr::Int { value: 0x16, .. }));
                assert!(matches!(*rhs, Expr::Int { value: 0xFF, .. }));
            }
            e => panic!("expected Xor, got {e:?}"),
        }
    }

    #[test]
    fn concat_is_lower_precedence_than_xor() {
        // `a ++ b ^ c` parses as `a ++ (b ^ c)`
        match parse_expr_ok("a ++ b ^ c") {
            Expr::Binary {
                op: BinOp::Concat,
                rhs,
                ..
            } => assert!(matches!(*rhs, Expr::Binary { op: BinOp::Xor, .. })),
            e => panic!("expected top-level Concat, got {e:?}"),
        }
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p tamal-lang --lib parser`
Expected: FAIL — `parses_concat`/`parses_xor`/`concat_is_lower_precedence_than_xor` panic (`parse_expr` still returns the bare primary, leaving the operator unconsumed).

- [ ] **Step 3: Replace `parse_expr` with two-level binary parsing**

In `crates/tamal-lang/src/parser.rs`, replace the Task-2 `parse_expr` method body and add the two helper levels (`++` is lower precedence than `^`):

```rust
    /// Parse an expression: `++`-concat (lowest) over `^`-xor over primaries.
    fn parse_expr(&mut self) -> Result<Expr, Vec<Diagnostic>> {
        self.parse_concat()
    }

    fn parse_concat(&mut self) -> Result<Expr, Vec<Diagnostic>> {
        let mut lhs = self.parse_xor()?;
        while self.peek() == Tok::PlusPlus {
            self.i += 1;
            let rhs = self.parse_xor()?;
            let span = lhs.span().start..rhs.span().end;
            lhs = Expr::Binary {
                op: BinOp::Concat,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                span,
            };
        }
        Ok(lhs)
    }

    fn parse_xor(&mut self) -> Result<Expr, Vec<Diagnostic>> {
        let mut lhs = self.parse_primary()?;
        while self.peek() == Tok::Caret {
            self.i += 1;
            let rhs = self.parse_primary()?;
            let span = lhs.span().start..rhs.span().end;
            lhs = Expr::Binary {
                op: BinOp::Xor,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                span,
            };
        }
        Ok(lhs)
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p tamal-lang --lib parser`
Expected: PASS — the 3 new tests plus all earlier parser/expr tests.

- [ ] **Step 5: Commit**

```bash
git add crates/tamal-lang/src/parser.rs
git commit -m "feat(tamal-lang): parse ^ and ++ binary expressions"
```

## Task 4: `consteval` — the pure compile-time fold

**Files:**
- Modify: `crates/tamal-lang/Cargo.toml` (add the `tamal-abi` dependency for `crc8`)
- Create: `crates/tamal-lang/src/consteval.rs`
- Modify: `crates/tamal-lang/src/lib.rs` (declare `pub mod consteval;`)

- [ ] **Step 1: Add the dependency, write the failing tests + declare the module**

First add `tamal-abi` to `crates/tamal-lang/Cargo.toml` (`consteval` calls `tamal_abi::crc8`) — the `[dependencies]` section becomes:

```toml
[dependencies]
tamal-abi.workspace = true
tamal-asm.workspace = true
```

Then create `crates/tamal-lang/src/consteval.rs` with ONLY this test module first, and add `pub mod consteval;` to `crates/tamal-lang/src/lib.rs` (alphabetically: `consteval`, `emit`, `lexer`, `parser`) so the failing compile surfaces:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn int(v: i64) -> Expr {
        Expr::Int { value: v, span: 0..0 }
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
            arg: Box::new(arg),
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
        assert_eq!(eval(&e, &Consts::new()).unwrap(), Value::Int(0xE9));
    }

    #[test]
    fn resolves_and_rejects_names() {
        let mut c = Consts::new();
        c.insert("X".into(), Value::Int(0x44));
        assert_eq!(
            eval(&Expr::Name { name: "X".into(), span: 0..0 }, &c).unwrap(),
            Value::Int(0x44)
        );
        assert!(eval(&Expr::Name { name: "NOPE".into(), span: 0..0 }, &c).is_err());
    }

    #[test]
    fn folds_bytes_and_concat() {
        let e = Expr::Binary {
            op: BinOp::Concat,
            lhs: Box::new(bytes(&[0x01, 0x02])),
            rhs: Box::new(bytes(&[0x03])),
            span: 0..0,
        };
        assert_eq!(eval(&e, &Consts::new()).unwrap(), Value::Bytes(vec![1, 2, 3]));
    }

    #[test]
    fn byte_out_of_range_errors() {
        assert!(eval(&bytes(&[0x100]), &Consts::new()).is_err());
    }

    #[test]
    fn crc8_folds_peripheral_command_bytes() {
        // crc8([0x44, 0x00, 0x64]) == 0x16 (matches examples/peripheral_io_read.s)
        let e = call("crc8", bytes(&[0x44, 0x00, 0x64]));
        assert_eq!(eval(&e, &Consts::new()).unwrap(), Value::Int(0x16));
    }

    #[test]
    fn len_lo_hi_builtins() {
        assert_eq!(eval(&call("len", bytes(&[1, 2, 3])), &Consts::new()).unwrap(), Value::Int(3));
        assert_eq!(eval(&call("lo", int(0xDEAD)), &Consts::new()).unwrap(), Value::Int(0xAD));
        assert_eq!(eval(&call("hi", int(0xDEAD)), &Consts::new()).unwrap(), Value::Int(0xDE));
    }

    #[test]
    fn type_mismatch_and_unknown_builtin_error() {
        assert!(eval(&call("crc8", int(5)), &Consts::new()).is_err()); // crc8 needs bytes
        assert!(eval(&call("nope", int(5)), &Consts::new()).is_err()); // unknown builtin
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p tamal-lang --lib consteval`
Expected: FAIL to compile — `cannot find value/type Value`, `eval`, `Consts`.

- [ ] **Step 3: Implement the evaluator**

Prepend to `crates/tamal-lang/src/consteval.rs` (above the test module):

```rust
//! Compile-time evaluation: fold a `parser::Expr` to a `Value` (an integer or a
//! byte string). `crc8` delegates to `tamal_abi::crc8`, so a folded CRC byte is
//! the exact value the wire and HDL use — it can never drift. Pure: no clock,
//! environment, or randomness, so identical source folds to identical bytes.

use crate::parser::{BinOp, Expr};
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

/// The resolved `const` environment, keyed by name.
pub type Consts = HashMap<String, Value>;

/// Fold an expression to a [`Value`] under the given `const` environment.
pub fn eval(e: &Expr, consts: &Consts) -> Result<Value, Diagnostic> {
    match e {
        Expr::Int { value, .. } => Ok(Value::Int(*value)),
        Expr::Name { name, span } => consts
            .get(name)
            .cloned()
            .ok_or_else(|| Diagnostic::error(span.clone(), format!("unknown name `{name}`"))),
        Expr::Bytes { elems, .. } => {
            let mut out = Vec::with_capacity(elems.len());
            for el in elems {
                out.push(eval_byte(el, consts)?);
            }
            Ok(Value::Bytes(out))
        }
        Expr::Binary { op: BinOp::Concat, lhs, rhs, .. } => {
            let mut a = eval_bytes(lhs, consts)?;
            a.extend(eval_bytes(rhs, consts)?);
            Ok(Value::Bytes(a))
        }
        Expr::Binary { op: BinOp::Xor, lhs, rhs, .. } => {
            Ok(Value::Int(eval_int(lhs, consts)? ^ eval_int(rhs, consts)?))
        }
        Expr::Call { func, arg, span } => eval_call(func, arg, span, consts),
    }
}

fn eval_call(func: &str, arg: &Expr, span: &Span, consts: &Consts) -> Result<Value, Diagnostic> {
    match func {
        "crc8" => Ok(Value::Int(tamal_abi::crc8(&eval_bytes(arg, consts)?) as i64)),
        "len" => Ok(Value::Int(eval_bytes(arg, consts)?.len() as i64)),
        "lo" => Ok(Value::Int(eval_int(arg, consts)? & 0xff)),
        "hi" => Ok(Value::Int((eval_int(arg, consts)? >> 8) & 0xff)),
        _ => Err(Diagnostic::error(span.clone(), format!("unknown builtin `{func}`"))
            .with_help("the builtins are crc8, len, lo, hi")),
    }
}

/// Evaluate `e` to an integer.
pub fn eval_int(e: &Expr, consts: &Consts) -> Result<i64, Diagnostic> {
    match eval(e, consts)? {
        Value::Int(n) => Ok(n),
        Value::Bytes(_) => Err(Diagnostic::error(e.span(), "expected an integer, found bytes")),
    }
}

/// Evaluate `e` to a byte string.
pub fn eval_bytes(e: &Expr, consts: &Consts) -> Result<Vec<u8>, Diagnostic> {
    match eval(e, consts)? {
        Value::Bytes(b) => Ok(b),
        Value::Int(_) => Err(Diagnostic::error(e.span(), "expected bytes, found an integer")),
    }
}

/// Evaluate `e` to a single byte (an integer in `0..=255`).
pub fn eval_byte(e: &Expr, consts: &Consts) -> Result<u8, Diagnostic> {
    let n = eval_int(e, consts)?;
    u8::try_from(n)
        .map_err(|_| Diagnostic::error(e.span(), format!("byte value {n} is out of range 0..=255")))
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p tamal-lang --lib consteval`
Expected: PASS — all 7 consteval tests (note `crc8_folds_peripheral_command_bytes` proves `crc8([0x44,0x00,0x64]) == 0x16`).

- [ ] **Step 5: Commit**

```bash
git add crates/tamal-lang/src/consteval.rs crates/tamal-lang/src/lib.rs
git commit -m "feat(tamal-lang): consteval fold (int/bytes/xor/concat + crc8/len/lo/hi)"
```

## Task 5: `const` items + resolution

**Files:**
- Modify: `crates/tamal-lang/src/parser.rs` (add `Const`, `Module.consts`, dispatch `const`/`test`)
- Modify: `crates/tamal-lang/src/lib.rs` (resolve consts in `lower`)
- Modify: `crates/tamal-lang/src/emit.rs` (fix `Module { … }` test constructions for the new field)

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `crates/tamal-lang/src/parser.rs`:

```rust
    #[test]
    fn parses_const_item() {
        let m = parse_ok("const PUT_IORD1 = 0x44\ntest t {\n  pass\n}\n");
        assert_eq!(m.consts.len(), 1);
        assert_eq!(m.consts[0].name, "PUT_IORD1");
        assert_eq!(m.tests.len(), 1);
    }
```

Add to `mod tests` in `crates/tamal-lang/src/lib.rs`:

```rust
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
```

Also UPDATE the existing `missing_test_keyword_is_an_error` test in `parser.rs` (its message changed) — replace its assertion line with:

```rust
        assert!(err[0].message.contains("expected `const` or `test`"));
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p tamal-lang --lib`
Expected: FAIL — the parser doesn't recognize `const` yet (`no field consts on Module`), the three lib tests fail, and the updated keyword test fails.

- [ ] **Step 3: Add `Const` + `Module.consts` and dispatch items**

In `crates/tamal-lang/src/parser.rs`, change the `Module` struct and add `Const` (replace the existing `Module` definition):

```rust
/// A parsed `.tam` module: `const` items and tests.
#[derive(Debug, Clone)]
pub struct Module {
    pub consts: Vec<Const>,
    pub tests: Vec<Test>,
}

/// A `const NAME = expr` item.
#[derive(Debug, Clone)]
pub struct Const {
    pub name: String,
    pub name_span: Span,
    pub value: Expr,
}
```

Replace the `parse` function so it dispatches on the leading keyword:

```rust
/// Parse tokens into a [`Module`], or return diagnostics.
pub fn parse(src: &str, toks: &[Token]) -> Result<Module, Vec<Diagnostic>> {
    let mut p = P { src, toks, i: 0 };
    let mut consts = Vec::new();
    let mut tests = Vec::new();
    p.skip_newlines();
    while p.peek() != Tok::Eof {
        let kw = p.expect_ident()?;
        match p.lexeme(&kw) {
            "const" => consts.push(p.parse_const()?),
            "test" => tests.push(p.parse_test()?),
            other => {
                return Err(vec![Diagnostic::error(
                    kw,
                    format!("expected `const` or `test`, found `{other}`"),
                )]);
            }
        }
        p.skip_newlines();
    }
    Ok(Module { consts, tests })
}
```

In `impl<'a> P<'a>`, change `parse_test` to NOT re-consume the keyword (the dispatcher already did) — replace its first three lines (the `let kw = …; if … != "test" …` block and the `let name_span`) so it starts at the name:

```rust
    fn parse_test(&mut self) -> Result<Test, Vec<Diagnostic>> {
        let name_span = self.expect_ident()?;
        let name = self.lexeme(&name_span).to_string();
        self.expect(Tok::LBrace, "`{`")?;
```

(keep the rest of `parse_test` unchanged). Then add `parse_const` in `impl<'a> P<'a>`:

```rust
    fn parse_const(&mut self) -> Result<Const, Vec<Diagnostic>> {
        let name_span = self.expect_ident()?;
        let name = self.lexeme(&name_span).to_string();
        self.expect(Tok::Eq, "`=`")?;
        let value = self.parse_expr()?;
        self.end_stmt()?;
        Ok(Const {
            name,
            name_span,
            value,
        })
    }
```

- [ ] **Step 4: Resolve consts in `lower`**

In `crates/tamal-lang/src/lib.rs`, inside `lower`, right after `let module = parser::parse(source, &toks)?;` and BEFORE the one-test check, insert const resolution:

```rust
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
```

(The `consts` map is validated here and threaded into `emit` in the next task.)

- [ ] **Step 5: Fix `Module { … }` constructions in emit tests**

In `crates/tamal-lang/src/emit.rs` `mod tests`, both `Module { … }` literals need the new `consts` field. In the `one` helper, change `Module { tests: … }` to include `consts: vec![]`:

```rust
    fn one(stmts: Vec<Stmt>) -> Module {
        Module {
            consts: vec![],
            tests: vec![Test {
                name: "t".into(),
                name_span: 0..1,
                stmts,
            }],
        }
    }
```

In `remap_points_asm_span_at_originating_tam_span`, change its `Module {` literal to start with `consts: vec![],`:

```rust
        let m = Module {
            consts: vec![],
            tests: vec![Test {
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p tamal-lang --lib`
Expected: PASS — all lexer/parser/consteval/emit/driver tests, including the 4 new const tests.

- [ ] **Step 7: Commit**

```bash
git add crates/tamal-lang/src/parser.rs crates/tamal-lang/src/lib.rs crates/tamal-lang/src/emit.rs
git commit -m "feat(tamal-lang): const items + source-order resolution"
```

## Task 6: `send` and `send … + crc8`

**Files:**
- Modify: `crates/tamal-lang/src/parser.rs` (add `Stmt::Send` + the `send` statement)
- Modify: `crates/tamal-lang/src/emit.rs` (evaluate `send` to a `put_byte` run; new signature)
- Modify: `crates/tamal-lang/src/lib.rs` (thread `consts` into `emit`)

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `crates/tamal-lang/src/parser.rs`:

```rust
    #[test]
    fn parses_send_with_crc() {
        let m = parse_ok("test t {\n  send [0x44] + crc8\n  pass\n}\n");
        match &m.tests[0].stmts[0] {
            Stmt::Send { append_crc, .. } => assert!(append_crc),
            s => panic!("expected Send, got {s:?}"),
        }
    }
```

Add to `mod tests` in `crates/tamal-lang/src/lib.rs`:

```rust
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p tamal-lang --lib`
Expected: FAIL — `send [..]` currently hits the `Raw` arm and errors on the `[` operand; the new tests fail to parse/lower.

- [ ] **Step 3: Add the `Stmt::Send` variant**

In `crates/tamal-lang/src/parser.rs`, add a variant to the `Stmt` enum (after `Raw`):

```rust
    /// `send <bytes-expr>` optionally followed by `+ crc8`.
    Send {
        bytes: Expr,
        append_crc: bool,
        span: Span,
    },
```

- [ ] **Step 4: Parse the `send` statement**

In `crates/tamal-lang/src/parser.rs` `parse_stmt`, add a `"send"` arm to the `match word.as_str()` (before the `_ =>` Raw arm):

```rust
            "send" => {
                let bytes = self.parse_expr()?;
                let mut end = bytes.span().end;
                let mut append_crc = false;
                if self.peek() == Tok::Plus {
                    self.i += 1;
                    let kw = self.expect_ident()?;
                    if self.lexeme(&kw) != "crc8" {
                        return Err(vec![Diagnostic::error(
                            kw,
                            "expected `crc8` after `+` in a `send`",
                        )]);
                    }
                    append_crc = true;
                    end = kw.end;
                }
                self.end_stmt()?;
                Ok(Stmt::Send {
                    bytes,
                    append_crc,
                    span: head.start..end,
                })
            }
```

- [ ] **Step 5: Replace `emit` to evaluate `send` and thread `consts`**

In `crates/tamal-lang/src/emit.rs`, add imports at the top (after the existing `use` lines):

```rust
use crate::consteval::{self, Consts};
```

Replace the entire `emit` function with this (new signature returning `Result`, plus the `Send` arm; the other arms push directly instead of via a shared `(text, span)`):

```rust
/// Lower a Plan-2 `Module` (exactly one test, enforced by the driver) to
/// tamal-asm text plus its source map. `send` statements are evaluated to a run
/// of `put_byte 0xNN` lines under the resolved `const` environment.
pub fn emit(module: &Module, consts: &Consts) -> Result<Lowering, Vec<Diagnostic>> {
    let mut asm = String::new();
    let mut lines = Vec::new();
    for test in &module.tests {
        push(&mut asm, &mut lines, ".globl _start\n", &test.name_span);
        push(&mut asm, &mut lines, "_start:\n", &test.name_span);
        for stmt in &test.stmts {
            match stmt {
                Stmt::Pass => push(&mut asm, &mut lines, "\thalt 0x00\n", &test.name_span),
                Stmt::Fail { code, span } => {
                    push(&mut asm, &mut lines, &format!("\thalt {code}\n"), span)
                }
                Stmt::Raw {
                    mnemonic,
                    operands,
                    span,
                } => {
                    let text = if operands.is_empty() {
                        format!("\t{mnemonic}\n")
                    } else {
                        format!("\t{mnemonic} {}\n", operands.join(", "))
                    };
                    push(&mut asm, &mut lines, &text, span);
                }
                Stmt::Send {
                    bytes,
                    append_crc,
                    span,
                } => {
                    let mut bs = consteval::eval_bytes(bytes, consts).map_err(|d| vec![d])?;
                    if *append_crc {
                        bs.push(tamal_abi::crc8(&bs));
                    }
                    for b in bs {
                        push(&mut asm, &mut lines, &format!("\tput_byte 0x{b:02X}\n"), span);
                    }
                }
            }
        }
    }
    Ok(Lowering { asm, lines })
}
```

- [ ] **Step 6: Update the emit unit tests for the new signature**

In `crates/tamal-lang/src/emit.rs` `mod tests`, every `emit(&…)` call now takes `&Consts::new()` and returns `Result`. Update the three `emits_*` tests to `emit(&one(vec![…]), &Consts::new()).unwrap().asm` and the remap test's `let low = emit(&m);` to:

```rust
        let low = emit(&m, &Consts::new()).unwrap();
```

(`Consts` is in scope via `use super::*` because `emit` now imports it.)

- [ ] **Step 7: Thread `consts` into `emit` from `lower`**

In `crates/tamal-lang/src/lib.rs`, change the final line of `lower` from `Ok(emit::emit(&module))` to (note: `emit` now returns `Result`, so no `Ok(...)` wrapper):

```rust
    emit::emit(&module, &consts)
```

- [ ] **Step 8: Run tests to verify they pass**

Run: `cargo test -p tamal-lang --lib`
Expected: PASS — including `send_lowers_to_put_byte_run` and `send_plus_crc8_appends_folded_byte` (the `0x16` fold).

- [ ] **Step 9: Commit**

```bash
git add crates/tamal-lang/src/parser.rs crates/tamal-lang/src/emit.rs crates/tamal-lang/src/lib.rs
git commit -m "feat(tamal-lang): send + send…+crc8 lowering to put_byte runs"
```

## Task 7: `crc_region { … }`

**Files:**
- Modify: `crates/tamal-lang/src/parser.rs` (add `Stmt::CrcRegion` + the block)
- Modify: `crates/tamal-lang/src/emit.rs` (fold CRC over all emitted bytes)

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `crates/tamal-lang/src/parser.rs`:

```rust
    #[test]
    fn parses_crc_region() {
        let m = parse_ok(
            "test t {\n crc_region {\n  send [0x44]\n  send [0x00, 0x64]\n }\n pass\n}\n",
        );
        match &m.tests[0].stmts[0] {
            Stmt::CrcRegion { sends, .. } => assert_eq!(sends.len(), 2),
            s => panic!("expected CrcRegion, got {s:?}"),
        }
    }
```

Add to `mod tests` in `crates/tamal-lang/src/lib.rs`:

```rust
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p tamal-lang --lib`
Expected: FAIL — `crc_region` is not parsed (hits the `Raw` arm and errors on `{`).

- [ ] **Step 3: Add the `Stmt::CrcRegion` variant**

In `crates/tamal-lang/src/parser.rs`, add to the `Stmt` enum (after `Send`):

```rust
    /// `crc_region { send <expr> … }` — appends a CRC-8 over exactly the bytes
    /// the block emits, in order.
    CrcRegion { sends: Vec<Expr>, span: Span },
```

- [ ] **Step 4: Parse the block**

In `crates/tamal-lang/src/parser.rs` `parse_stmt`, add a `"crc_region"` arm (before the `_ =>` Raw arm):

```rust
            "crc_region" => {
                self.expect(Tok::LBrace, "`{`")?;
                let mut sends = Vec::new();
                let end = loop {
                    self.skip_newlines();
                    match self.peek() {
                        Tok::RBrace => {
                            let e = self.span().end;
                            self.i += 1;
                            break e;
                        }
                        Tok::Eof => {
                            return Err(vec![Diagnostic::error(
                                self.span(),
                                "unexpected end of file: missing `}` for `crc_region`",
                            )]);
                        }
                        _ => {
                            let kw = self.expect_ident()?;
                            if self.lexeme(&kw) != "send" {
                                return Err(vec![Diagnostic::error(
                                    kw,
                                    "only `send` statements are allowed in a `crc_region`",
                                )]);
                            }
                            sends.push(self.parse_expr()?);
                            self.end_stmt()?;
                        }
                    }
                };
                self.end_stmt()?;
                Ok(Stmt::CrcRegion {
                    sends,
                    span: head.start..end,
                })
            }
```

- [ ] **Step 5: Emit the region — accumulate bytes, then append the CRC**

In `crates/tamal-lang/src/emit.rs`, add a `CrcRegion` arm to the `match stmt` in `emit` (after the `Send` arm):

```rust
                Stmt::CrcRegion { sends, span } => {
                    let mut total = Vec::new();
                    for e in sends {
                        total.extend(consteval::eval_bytes(e, consts).map_err(|d| vec![d])?);
                    }
                    total.push(tamal_abi::crc8(&total));
                    for b in total {
                        push(&mut asm, &mut lines, &format!("\tput_byte 0x{b:02X}\n"), span);
                    }
                }
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p tamal-lang --lib`
Expected: PASS — `parses_crc_region` and `crc_region_folds_over_all_emitted_bytes` (the block folds `[0x44,0x00,0x64]` to `0x16`).

- [ ] **Step 7: Commit**

```bash
git add crates/tamal-lang/src/parser.rs crates/tamal-lang/src/emit.rs
git commit -m "feat(tamal-lang): crc_region structural CRC block"
```

## Task 8: End-to-end integration + polish

**Files:**
- Create: `crates/tamal-lang/tests/values.rs`

- [ ] **Step 1: Write the acceptance tests**

These compose the pieces from Tasks 4–7 and should pass green (they are acceptance guards; a failure reveals an integration gap). Create `crates/tamal-lang/tests/values.rs`:

```rust
//! End-to-end: `const` + `bytes` + `crc8` fold an eSPI command phase at compile
//! time, byte-identically to hand-written assembly.

#[test]
fn command_phase_byte_matches_hand_written_asm() {
    // `send [..] + crc8` folds crc8([0x44,0x00,0x64]) = 0x16 at compile time.
    let tam = "const PUT_IORD1 = 0x44\n\
               test io {\n\
               \x20   cs_assert\n\
               \x20   send [PUT_IORD1, 0x00, 0x64] + crc8\n\
               \x20   tar 2\n\
               \x20   cs_deassert\n\
               \x20   halt 0x00\n\
               }\n";
    let reference = ".globl _start\n_start:\n\
                     \tcs_assert\n\
                     \tput_byte 0x44\n\tput_byte 0x00\n\tput_byte 0x64\n\tput_byte 0x16\n\
                     \ttar 2\n\tcs_deassert\n\thalt 0x00\n";
    let got = tamal_lang::compile(tam).expect("compile .tam").to_le_bytes();
    let want = tamal_asm::assemble(reference).expect("assemble ref").to_le_bytes();
    assert_eq!(got, want, "send+crc8 command phase must byte-match hand-written asm");
}

#[test]
fn crc_region_byte_matches_the_same_command_phase() {
    let tam = "test io {\n\
               \x20   cs_assert\n\
               \x20   crc_region {\n\
               \x20       send [0x44]\n\
               \x20       send [0x00, 0x64]\n\
               \x20   }\n\
               \x20   tar 2\n\
               \x20   cs_deassert\n\
               \x20   halt 0x00\n\
               }\n";
    let reference = ".globl _start\n_start:\n\
                     \tcs_assert\n\
                     \tput_byte 0x44\n\tput_byte 0x00\n\tput_byte 0x64\n\tput_byte 0x16\n\
                     \ttar 2\n\tcs_deassert\n\thalt 0x00\n";
    let got = tamal_lang::compile(tam).expect("compile .tam").to_le_bytes();
    let want = tamal_asm::assemble(reference).expect("assemble ref").to_le_bytes();
    assert_eq!(got, want, "crc_region must fold the same 0x16 as `+ crc8`");
}

#[test]
fn deliberate_wrong_crc_folds_to_xored_byte() {
    // A deliberately-corrupted CRC is greppable and folds to 0x16 ^ 0xFF = 0xE9.
    let tam = "test bad_crc {\n\
               \x20   send [0x44, 0x00, 0x64] ++ [crc8([0x44, 0x00, 0x64]) ^ 0xFF]\n\
               \x20   halt 0x00\n\
               }\n";
    let asm = tamal_lang::lower_to_asm(tam).expect("lower");
    assert!(asm.contains("put_byte 0xE9"), "wrong CRC must fold to 0xE9:\n{asm}");
    assert!(!asm.contains("put_byte 0x16"), "the correct CRC must not appear");
}
```

- [ ] **Step 2: Run the acceptance tests**

Run: `cargo test -p tamal-lang --test values`
Expected: PASS — 3 tests. The byte-match tests prove `send + crc8` and `crc_region` fold the exact `0x16` a human hand-wrote in `examples/peripheral_io_read.s`.

Note: if `command_phase_byte_matches_hand_written_asm` fails, compare `tamal_lang::lower_to_asm(tam).unwrap()` against `reference` to find the divergent line.

- [ ] **Step 3: Commit**

```bash
git add crates/tamal-lang/tests/values.rs
git commit -m "test(tamal-lang): e2e command-phase CRC fold byte-matches hand-written asm"
```

- [ ] **Step 4: Format, lint, full test**

Run each and confirm clean:

```bash
cargo fmt -p tamal-lang -p tamal-lang-cli
cargo fmt -p tamal-lang -p tamal-lang-cli --check
cargo clippy -p tamal-lang -p tamal-lang-cli --all-targets -- -D warnings
cargo test
```
Expected: `fmt --check` clean; clippy 0 warnings; `cargo test` all green (the full workspace, including the new consteval/const/send/crc_region unit tests and the `values` integration tests). Fix any clippy findings minimally and re-run.

- [ ] **Step 5: Commit any formatting/lint fixes**

```bash
git add -A
git commit -m "chore(tamal-lang): fmt + clippy clean for values & crc"
```

(If Step 4 produced no changes, skip this commit.)

---

## Definition of done

- `cargo test`, `cargo clippy -p tamal-lang -p tamal-lang-cli -- -D warnings`, and `cargo fmt … --check` are all green.
- `const NAME = <expr>` works, referencing earlier consts; duplicate/undefined names are `.tam`-pointed diagnostics.
- `send [a, b, c]` lowers to a `put_byte` run; `send pkt + crc8` and `crc_region { … }` append the compile-time CRC-8; `crc8([0x44,0x00,0x64])` folds to `0x16` and byte-matches the hand-written command phase of `examples/peripheral_io_read.s`.
- Deliberate-wrong (`crc8(pkt) ^ 0xFF`) folds to `0xE9` and is greppable in source.
- The value/`consteval` layer (`Value`, `eval`, `crc8`/`len`/`lo`/`hi`) is in place for Plan 3 (frames/verdicts) and beyond.

## Deferred to a follow-on (not in this plan)

General arithmetic operators (`+ - * / % << >> & |`), `enum`, indexing (`bytes[i]`), and typed `const NAME: byte = …` annotations. None are needed for byte-match parity with the examples; add them when a test needs them.







