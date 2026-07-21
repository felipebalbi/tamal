//! Parser: tokens → the Plan-1 AST (`Module` of one `Test` of statements:
//! `pass`, `fail N`, or a verbatim raw instruction).

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
