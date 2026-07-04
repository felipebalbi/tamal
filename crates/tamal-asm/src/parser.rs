//! Parser: tokens -> a flat line AST (labels, directives, instructions).

use crate::diagnostics::{Diagnostic, Span};
use crate::lexer::{TokKind, Token};

/// A parsed operand: a bare identifier or a numeric literal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperandKind {
    /// A register name, symbol, label, or `set_config`/`rdsr` keyword.
    Ident(String),
    /// A numeric immediate.
    Num(i64),
}

/// An operand with its source span.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Operand {
    /// The operand kind.
    pub kind: OperandKind,
    /// The byte span.
    pub span: Span,
}

/// One parsed source line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LineKind {
    /// `name:` — defines an address.
    Label(String),
    /// `.name args...`
    Directive {
        /// Directive name without the dot.
        name: String,
        /// Directive arguments.
        args: Vec<Operand>,
    },
    /// `mnemonic operands...`
    Instr {
        /// The mnemonic.
        mnemonic: String,
        /// The operands.
        operands: Vec<Operand>,
    },
}

/// A line with its source span.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    /// The line kind.
    pub kind: LineKind,
    /// The byte span.
    pub span: Span,
}

/// Parse a token stream into a flat line list. Fails fast on the first error.
pub fn parse(tokens: &[Token]) -> Result<Vec<Line>, Diagnostic> {
    let mut lines = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        match &tokens[i].kind {
            TokKind::Eof => break,
            TokKind::Newline => i += 1,
            TokKind::Ident(name)
                if matches!(tokens.get(i + 1).map(|t| &t.kind), Some(TokKind::Colon)) =>
            {
                let span = tokens[i].span.start..tokens[i + 1].span.end;
                lines.push(Line {
                    kind: LineKind::Label(name.clone()),
                    span,
                });
                i += 2;
            }
            TokKind::Ident(name) => {
                let start = tokens[i].span.start;
                let mnemonic = name.clone();
                i += 1;
                let (operands, next, end) = parse_operands(tokens, i)?;
                i = next;
                lines.push(Line {
                    kind: LineKind::Instr { mnemonic, operands },
                    span: start..end,
                });
            }
            TokKind::Directive(name) => {
                let start = tokens[i].span.start;
                let dname = name.clone();
                i += 1;
                let (args, next, end) = parse_operands(tokens, i)?;
                i = next;
                lines.push(Line {
                    kind: LineKind::Directive { name: dname, args },
                    span: start..end,
                });
            }
            _ => {
                return Err(Diagnostic::error(
                    tokens[i].span.clone(),
                    "expected a label, instruction, or directive",
                ));
            }
        }
    }
    Ok(lines)
}

/// Parse a comma-separated operand list from index `i` up to the next
/// Newline/Eof. Returns `(operands, index_after, end_offset)`.
fn parse_operands(
    tokens: &[Token],
    mut i: usize,
) -> Result<(Vec<Operand>, usize, usize), Diagnostic> {
    let mut ops = Vec::new();
    let mut end = tokens[i.saturating_sub(1)].span.end;
    if matches!(tokens[i].kind, TokKind::Newline | TokKind::Eof) {
        return Ok((ops, i, end));
    }
    loop {
        let op = parse_operand(&tokens[i])?;
        end = op.span.end;
        ops.push(op);
        i += 1;
        match &tokens[i].kind {
            TokKind::Comma => {
                i += 1;
                if matches!(tokens[i].kind, TokKind::Newline | TokKind::Eof) {
                    return Err(Diagnostic::error(
                        tokens[i].span.clone(),
                        "trailing comma: expected another operand",
                    ));
                }
            }
            TokKind::Newline | TokKind::Eof => break,
            _ => {
                return Err(Diagnostic::error(
                    tokens[i].span.clone(),
                    "expected `,` or end of line",
                ));
            }
        }
    }
    Ok((ops, i, end))
}

fn parse_operand(tok: &Token) -> Result<Operand, Diagnostic> {
    match &tok.kind {
        TokKind::Ident(s) => Ok(Operand {
            kind: OperandKind::Ident(s.clone()),
            span: tok.span.clone(),
        }),
        TokKind::Num(n) => Ok(Operand {
            kind: OperandKind::Num(*n),
            span: tok.span.clone(),
        }),
        _ => Err(Diagnostic::error(
            tok.span.clone(),
            "expected a register, immediate, or symbol",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lex;

    fn parse_src(src: &str) -> Vec<LineKind> {
        parse(&lex(src).unwrap())
            .unwrap()
            .into_iter()
            .map(|l| l.kind)
            .collect()
    }

    #[test]
    fn label_on_own_line_then_instr() {
        assert_eq!(
            parse_src("poll:\n  crc_reset\n"),
            vec![
                LineKind::Label("poll".into()),
                LineKind::Instr {
                    mnemonic: "crc_reset".into(),
                    operands: vec![]
                },
            ]
        );
    }

    #[test]
    fn label_and_instr_same_line() {
        let ls = parse_src("_start: cs_assert\n");
        assert_eq!(ls[0], LineKind::Label("_start".into()));
        assert_eq!(
            ls[1],
            LineKind::Instr {
                mnemonic: "cs_assert".into(),
                operands: vec![]
            }
        );
    }

    #[test]
    fn instr_with_operands() {
        let ls = parse_src("beq t0, t1, poll\n");
        match &ls[0] {
            LineKind::Instr { mnemonic, operands } => {
                assert_eq!(mnemonic, "beq");
                assert_eq!(operands.len(), 3);
                assert_eq!(operands[0].kind, OperandKind::Ident("t0".into()));
                assert_eq!(operands[2].kind, OperandKind::Ident("poll".into()));
            }
            other => panic!("expected instr, got {other:?}"),
        }
    }

    #[test]
    fn directive_equ() {
        let ls = parse_src(".equ PUT_IORD1, 0x44\n");
        match &ls[0] {
            LineKind::Directive { name, args } => {
                assert_eq!(name, "equ");
                assert_eq!(args[0].kind, OperandKind::Ident("PUT_IORD1".into()));
                assert_eq!(args[1].kind, OperandKind::Num(0x44));
            }
            other => panic!("expected directive, got {other:?}"),
        }
    }

    #[test]
    fn trailing_comma_is_error() {
        let err = parse(&lex("put_byte 0x64,\n").unwrap()).unwrap_err();
        assert!(err.message.contains("comma"));
    }

    #[test]
    fn number_as_mnemonic_is_error() {
        let err = parse(&lex("42\n").unwrap()).unwrap_err();
        assert!(err.message.contains("label, instruction, or directive"));
    }
}
