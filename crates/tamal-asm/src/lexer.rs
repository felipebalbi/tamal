//! Tokeniser: source text -> tokens with byte spans. Skips `#` and `;` comments.

use crate::diagnostics::{Diagnostic, Span};

/// A lexical token kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokKind {
    /// An identifier (mnemonic, register, label, or symbol name).
    Ident(String),
    /// A numeric literal (decimal, `0x`, or `0b`; may be negative).
    Num(i64),
    /// A directive name without the leading dot (`.equ` -> `equ`).
    Directive(String),
    /// `,`
    Comma,
    /// `:`
    Colon,
    /// End of a logical line.
    Newline,
    /// End of input.
    Eof,
}

/// A token with its source span.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    /// The token kind.
    pub kind: TokKind,
    /// The byte span in the source.
    pub span: Span,
}

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

fn is_ident_continue(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Tokenise `source`. Fails fast on the first lexical error.
pub fn lex(source: &str) -> Result<Vec<Token>, Diagnostic> {
    let bytes = source.as_bytes();
    let mut toks = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        match c {
            ' ' | '\t' | '\r' => i += 1,
            '#' | ';' => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            '\n' => {
                toks.push(Token {
                    kind: TokKind::Newline,
                    span: i..i + 1,
                });
                i += 1;
            }
            ',' => {
                toks.push(Token {
                    kind: TokKind::Comma,
                    span: i..i + 1,
                });
                i += 1;
            }
            ':' => {
                toks.push(Token {
                    kind: TokKind::Colon,
                    span: i..i + 1,
                });
                i += 1;
            }
            '.' => {
                let start = i;
                i += 1;
                let name_start = i;
                while i < bytes.len() && is_ident_continue(bytes[i] as char) {
                    i += 1;
                }
                if i == name_start {
                    return Err(Diagnostic::error(start..i, "empty directive after `.`"));
                }
                toks.push(Token {
                    kind: TokKind::Directive(source[name_start..i].to_string()),
                    span: start..i,
                });
            }
            '-' | '0'..='9' => {
                let start = i;
                if c == '-' {
                    i += 1;
                }
                let num_start = i;
                // hex / binary prefixes
                let mut radix = 10;
                if source[i..].starts_with("0x") || source[i..].starts_with("0X") {
                    radix = 16;
                    i += 2;
                } else if source[i..].starts_with("0b") || source[i..].starts_with("0B") {
                    radix = 2;
                    i += 2;
                }
                let digits_start = i;
                while i < bytes.len() && (is_ident_continue(bytes[i] as char)) {
                    i += 1;
                }
                let digits = &source[digits_start..i];
                if digits.is_empty() {
                    return Err(Diagnostic::error(start..i, "malformed number literal"));
                }
                let mag = i64::from_str_radix(digits, radix).map_err(|_| {
                    Diagnostic::error(
                        start..i,
                        format!("invalid number literal `{}`", &source[num_start..i]),
                    )
                })?;
                let val = if c == '-' { -mag } else { mag };
                toks.push(Token {
                    kind: TokKind::Num(val),
                    span: start..i,
                });
            }
            _ if is_ident_start(c) => {
                let start = i;
                while i < bytes.len() && is_ident_continue(bytes[i] as char) {
                    i += 1;
                }
                toks.push(Token {
                    kind: TokKind::Ident(source[start..i].to_string()),
                    span: start..i,
                });
            }
            _ => {
                return Err(Diagnostic::error(
                    i..i + 1,
                    format!("unexpected character `{c}`"),
                ));
            }
        }
    }
    toks.push(Token {
        kind: TokKind::Eof,
        span: source.len()..source.len(),
    });
    Ok(toks)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<TokKind> {
        lex(src).unwrap().into_iter().map(|t| t.kind).collect()
    }

    #[test]
    fn numbers_dec_hex_bin_neg() {
        assert_eq!(
            kinds("42 0x64 0b1010 -5"),
            vec![
                TokKind::Num(42),
                TokKind::Num(0x64),
                TokKind::Num(0b1010),
                TokKind::Num(-5),
                TokKind::Eof,
            ]
        );
    }

    #[test]
    fn idents_directives_punct() {
        assert_eq!(
            kinds("_start: set_config, .equ x5"),
            vec![
                TokKind::Ident("_start".into()),
                TokKind::Colon,
                TokKind::Ident("set_config".into()),
                TokKind::Comma,
                TokKind::Directive("equ".into()),
                TokKind::Ident("x5".into()),
                TokKind::Eof,
            ]
        );
    }

    #[test]
    fn both_comment_chars_and_newlines() {
        assert_eq!(
            kinds("cs_assert # hash comment\nhalt ; semi comment\n"),
            vec![
                TokKind::Ident("cs_assert".into()),
                TokKind::Newline,
                TokKind::Ident("halt".into()),
                TokKind::Newline,
                TokKind::Eof,
            ]
        );
    }

    #[test]
    fn spans_point_at_source() {
        let toks = lex("  put_byte 0x64").unwrap();
        assert_eq!(toks[0].span, 2..10); // put_byte
        assert_eq!(toks[1].span, 11..15); // 0x64
    }

    #[test]
    fn bad_number_is_error() {
        let err = lex("0xZZ").unwrap_err();
        assert!(err.message.contains("number"));
    }
}
