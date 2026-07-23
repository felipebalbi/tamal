//! Emit: lower a `Module` (Plan-1 subset) to tamal-asm text, plus a source map
//! from generated-asm byte offsets back to the originating `.tam` spans. A
//! `test` becomes the entry label; `pass`/`fail`/raw statements become lines.

use crate::parser::{Module, Stmt};
use tamal_asm::{Diagnostic, Span};

use crate::consteval::{self, Consts};

/// The product of lowering: the tamal-asm text and a per-line source map so a
/// backend diagnostic (whose spans index the generated asm) can be re-pointed
/// at the `.tam` span that produced the offending line.
pub struct Lowering {
    /// The generated tamal-asm text.
    pub asm: String,
    /// `(asm byte range, originating .tam span)` per emitted line, in order.
    lines: Vec<(Span, Span)>,
}

impl Lowering {
    /// Re-point a batch of backend diagnostics from generated-asm offsets back
    /// to the `.tam` source spans that produced them.
    pub fn remap(&self, diags: Vec<Diagnostic>) -> Vec<Diagnostic> {
        diags.into_iter().map(|d| self.remap_one(d)).collect()
    }

    fn remap_one(&self, mut d: Diagnostic) -> Diagnostic {
        d.primary = self.tam_span(&d.primary);
        for (span, _) in &mut d.labels {
            *span = self.tam_span(span);
        }
        d
    }

    /// Map a generated-asm byte span to the `.tam` span of the line containing
    /// its start; falls back to the last line, then to an empty span.
    fn tam_span(&self, asm: &Span) -> Span {
        self.lines
            .iter()
            .find(|(range, _)| range.contains(&asm.start))
            .or_else(|| self.lines.last())
            .map(|(_, tam)| tam.clone())
            .unwrap_or(0..0)
    }
}

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
                        bs.push(tamal_abi::crc8::crc8(&bs));
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

/// Append one asm line and record its `(asm byte range, .tam span)` mapping.
fn push(asm: &mut String, lines: &mut Vec<(Span, Span)>, text: &str, span: &Span) {
    let start = asm.len();
    asm.push_str(text);
    lines.push((start..asm.len(), span.clone()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::{Module, Stmt, Test};

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

    #[test]
    fn emits_entry_and_pass() {
        let asm = emit(&one(vec![Stmt::Pass]), &Consts::new()).unwrap().asm;
        assert_eq!(asm, ".globl _start\n_start:\n\thalt 0x00\n");
    }

    #[test]
    fn emits_fail_code_verbatim() {
        let asm = emit(
            &one(vec![Stmt::Fail {
                code: "0x11".into(),
                span: 0..1,
            }]),
            &Consts::new(),
        )
        .unwrap()
        .asm;
        assert_eq!(asm, ".globl _start\n_start:\n\thalt 0x11\n");
    }

    #[test]
    fn emits_raw_instructions() {
        let asm = emit(
            &one(vec![
                Stmt::Raw {
                    mnemonic: "cs_assert".into(),
                    operands: vec![],
                    span: 0..1,
                },
                Stmt::Raw {
                    mnemonic: "mark".into(),
                    operands: vec!["1".into(), "x1".into()],
                    span: 0..1,
                },
                Stmt::Pass,
            ]),
            &Consts::new(),
        )
        .unwrap()
        .asm;
        assert_eq!(
            asm,
            ".globl _start\n_start:\n\tcs_assert\n\tmark 1, x1\n\thalt 0x00\n"
        );
    }

    #[test]
    fn remap_points_asm_span_at_originating_tam_span() {
        // a raw statement whose .tam span is 40..45; its emitted asm line's
        // offset must remap back to that span.
        let m = Module {
            consts: vec![],
            tests: vec![Test {
                name: "t".into(),
                name_span: 0..1,
                stmts: vec![
                    Stmt::Raw {
                        mnemonic: "bogus".into(),
                        operands: vec![],
                        span: 40..45,
                    },
                    Stmt::Pass,
                ],
            }],
        };
        let low = emit(&m, &Consts::new()).unwrap();
        let idx = low.asm.find("bogus").expect("emitted the raw mnemonic");
        let d = Diagnostic::error(idx..idx + 5, "unknown instruction");
        let remapped = low.remap(vec![d]);
        assert_eq!(remapped[0].primary, 40..45);
    }
}
