//! Lexer: `.tam` source text → tokens with byte spans. Skips `//` line and
//! `/* */` block comments; recognizes identifiers, numbers, punctuation
//! (`{} [] () , =`), the operators `+ ++ ^`, and newlines (statement
//! separators).

use tamal_asm::{Diagnostic, Span};

/// Token kinds.
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
                toks.push(Token {
                    kind: Tok::Newline,
                    span: i..i + 1,
                });
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
                toks.push(Token {
                    kind: Tok::LBrace,
                    span: i..i + 1,
                });
                i += 1;
            }
            b'}' => {
                toks.push(Token {
                    kind: Tok::RBrace,
                    span: i..i + 1,
                });
                i += 1;
            }
            b',' => {
                toks.push(Token {
                    kind: Tok::Comma,
                    span: i..i + 1,
                });
                i += 1;
            }
            b'[' => {
                toks.push(Token {
                    kind: Tok::LBracket,
                    span: i..i + 1,
                });
                i += 1;
            }
            b']' => {
                toks.push(Token {
                    kind: Tok::RBracket,
                    span: i..i + 1,
                });
                i += 1;
            }
            b'(' => {
                toks.push(Token {
                    kind: Tok::LParen,
                    span: i..i + 1,
                });
                i += 1;
            }
            b')' => {
                toks.push(Token {
                    kind: Tok::RParen,
                    span: i..i + 1,
                });
                i += 1;
            }
            b'=' => {
                toks.push(Token {
                    kind: Tok::Eq,
                    span: i..i + 1,
                });
                i += 1;
            }
            b'^' => {
                toks.push(Token {
                    kind: Tok::Caret,
                    span: i..i + 1,
                });
                i += 1;
            }
            b'+' if i + 1 < b.len() && b[i + 1] == b'+' => {
                toks.push(Token {
                    kind: Tok::PlusPlus,
                    span: i..i + 2,
                });
                i += 2;
            }
            b'+' => {
                toks.push(Token {
                    kind: Tok::Plus,
                    span: i..i + 1,
                });
                i += 1;
            }
            _ if is_ident_start(c) => {
                let start = i;
                i += 1;
                while i < b.len() && is_ident_continue(b[i]) {
                    i += 1;
                }
                toks.push(Token {
                    kind: Tok::Ident,
                    span: start..i,
                });
            }
            _ if c.is_ascii_digit() => {
                let start = i;
                i += 1;
                while i < b.len() && is_number_continue(b[i]) {
                    i += 1;
                }
                toks.push(Token {
                    kind: Tok::Number,
                    span: start..i,
                });
            }
            _ => {
                return Err(vec![Diagnostic::error(
                    i..i + 1,
                    format!("unexpected character `{}`", c as char),
                )]);
            }
        }
    }
    toks.push(Token {
        kind: Tok::Eof,
        span: b.len()..b.len(),
    });
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
                Tok::Ident,
                Tok::Ident,
                Tok::LBrace,
                Tok::Newline,
                Tok::Ident,
                Tok::Newline,
                Tok::RBrace,
                Tok::Newline,
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
            vec![
                Tok::Newline,
                Tok::Ident,
                Tok::Number,
                Tok::Newline,
                Tok::Eof
            ]
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

    #[test]
    fn lexes_bracket_paren_operator_tokens() {
        assert_eq!(
            kinds("send [a, b] + crc8\n"),
            vec![
                Tok::Ident, // send
                Tok::LBracket,
                Tok::Ident, // a
                Tok::Comma,
                Tok::Ident, // b
                Tok::RBracket,
                Tok::Plus,
                Tok::Ident, // crc8
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
}
