//! Parser: tokens → the AST — a `Module` of `const` items and a `Test` of
//! statements (`pass`, `fail N`, `send`/`crc_region`, or a verbatim raw
//! instruction), plus the compile-time `Expr` grammar (`[bytes]`, `++`, `^`,
//! builtin calls).

use crate::lexer::{Tok, Token};
use tamal_asm::{Diagnostic, Span};

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
    /// `send <bytes-expr>` optionally followed by `+ crc8`.
    Send {
        bytes: Expr,
        append_crc: bool,
        span: Span,
    },
    /// `crc_region { send <expr> … }` — appends a CRC-8 over exactly the bytes
    /// the block emits, in order.
    CrcRegion { sends: Vec<Expr>, span: Span },
}

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
            Err(vec![Diagnostic::error(
                self.span(),
                format!("expected {what}"),
            )])
        }
    }

    fn expect_ident(&mut self) -> Result<Span, Vec<Diagnostic>> {
        Ok(self.expect(Tok::Ident, "an identifier")?.span)
    }

    fn parse_test(&mut self) -> Result<Test, Vec<Diagnostic>> {
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
        Ok(Test {
            name,
            name_span,
            stmts,
        })
    }

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
                    self.skip_newlines();
                    let arg = self.parse_expr()?;
                    self.skip_newlines();
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
                // Newlines inside `[...]` are not statement terminators, and a
                // trailing comma before `]` is allowed.
                let mut elems = Vec::new();
                self.skip_newlines();
                while self.peek() != Tok::RBracket {
                    elems.push(self.parse_expr()?);
                    self.skip_newlines();
                    if self.peek() == Tok::Comma {
                        self.i += 1;
                        self.skip_newlines();
                    } else {
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
                self.skip_newlines();
                let e = self.parse_expr()?;
                self.skip_newlines();
                self.expect(Tok::RParen, "`)`")?;
                Ok(e)
            }
            _ => Err(vec![Diagnostic::error(
                self.span(),
                "expected an expression",
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
            Stmt::Raw {
                mnemonic, operands, ..
            } => {
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
            Stmt::Raw {
                mnemonic, operands, ..
            } => {
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
        assert!(err[0].message.contains("expected `const` or `test`"));
    }

    #[test]
    fn parses_const_item() {
        let m = parse_ok("const PUT_IORD1 = 0x44\ntest t {\n  pass\n}\n");
        assert_eq!(m.consts.len(), 1);
        assert_eq!(m.consts[0].name, "PUT_IORD1");
        assert_eq!(m.tests.len(), 1);
    }

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

    #[test]
    fn parses_send_with_crc() {
        let m = parse_ok("test t {\n  send [0x44] + crc8\n  pass\n}\n");
        match &m.tests[0].stmts[0] {
            Stmt::Send { append_crc, .. } => assert!(append_crc),
            s => panic!("expected Send, got {s:?}"),
        }
    }

    #[test]
    fn parses_crc_region() {
        let m =
            parse_ok("test t {\n crc_region {\n  send [0x44]\n  send [0x00, 0x64]\n }\n pass\n}\n");
        match &m.tests[0].stmts[0] {
            Stmt::CrcRegion { sends, .. } => assert_eq!(sends.len(), 2),
            s => panic!("expected CrcRegion, got {s:?}"),
        }
    }

    #[test]
    fn parses_multiline_bytes_literal() {
        // Newlines inside `[...]` are not statement terminators; a trailing
        // comma is allowed. (The motivating snippet from the Plan-3 prep.)
        match parse_expr_ok("[\n  0x44,\n  0x00,\n  0x64,\n]") {
            Expr::Bytes { elems, .. } => assert_eq!(elems.len(), 3),
            e => panic!("expected Bytes, got {e:?}"),
        }
    }

    #[test]
    fn parses_bytes_with_trailing_comma() {
        // A trailing comma before the closing `]` is allowed even on one line.
        match parse_expr_ok("[0x44, 0x00,]") {
            Expr::Bytes { elems, .. } => assert_eq!(elems.len(), 2),
            e => panic!("expected Bytes, got {e:?}"),
        }
    }

    #[test]
    fn parses_multiline_call_arg() {
        // Newlines after `(` and before `)` are not statement terminators.
        match parse_expr_ok("crc8(\n  pkt\n)") {
            Expr::Call { func, .. } => assert_eq!(func, "crc8"),
            e => panic!("expected Call, got {e:?}"),
        }
    }

    #[test]
    fn parses_multiline_parenthesized() {
        match parse_expr_ok("(\n  0x05\n)") {
            Expr::Int { value, .. } => assert_eq!(value, 5),
            e => panic!("expected Int, got {e:?}"),
        }
    }

    #[test]
    fn parses_multiline_send_statement() {
        // A `send` whose bytes literal spans multiple lines (with a trailing
        // comma) still parses, and `+ crc8` after the `]` is picked up.
        let m = parse_ok(
            "test t {\n  send [\n    0x44,\n    0x00,\n    0x64,\n  ] + crc8\n  pass\n}\n",
        );
        match &m.tests[0].stmts[0] {
            Stmt::Send {
                bytes, append_crc, ..
            } => {
                assert!(append_crc);
                match bytes {
                    Expr::Bytes { elems, .. } => assert_eq!(elems.len(), 3),
                    e => panic!("expected Bytes, got {e:?}"),
                }
            }
            s => panic!("expected Send, got {s:?}"),
        }
    }
}
