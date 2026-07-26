//! Emit: lower a `Module` to tamal-asm text, plus a source map from
//! generated-asm byte offsets back to the originating `.tam` spans. A `test`
//! becomes the entry label; `pass`/`fail`/raw become lines, and `send`/
//! `crc_region` evaluate to `put_byte` runs (with the compile-time CRC-8).

use crate::parser::{Module, Stmt};
use tamal_asm::{Diagnostic, Span};

use crate::consteval::{self, Consts};
use crate::regalloc::RegAlloc;
use tamal_abi::isa::Reg;

/// The eSPI WAIT_STATE response code the `wait_state` poll spins on.
const WAIT_STATE_CODE: u8 = 0x0F;

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

/// Lower a `Module` (exactly one test, enforced by the driver) to tamal-asm
/// text plus its source map.
pub fn emit(module: &Module, consts: &Consts) -> Result<Lowering, Vec<Diagnostic>> {
    let mut e = Emitter::new(consts);
    for test in &module.tests {
        e.push(".globl _start\n", &test.name_span);
        e.push("_start:\n", &test.name_span);
        for stmt in &test.stmts {
            e.top_stmt(stmt, &test.name_span)?;
        }
        e.flush_trailers();
    }
    Ok(e.finish())
}

/// The lowering state: the growing asm text + source map.
struct Emitter<'a> {
    consts: &'a Consts,
    asm: String,
    lines: Vec<(Span, Span)>,
    alloc: RegAlloc,
    gensym: u32,
    trailers: Vec<(Span, String)>,
}

/// A verdict branch an `expect` deferred to its enclosing `frame`: after CS
/// deasserts, emit `bnez <reg>, <label>`, and hoist `<label>: halt <code>` to a
/// per-test trailer.
struct Deferred {
    reg: Reg,
    label: String,
    code: u8,
    span: Span,
}

impl<'a> Emitter<'a> {
    fn new(consts: &'a Consts) -> Self {
        Emitter {
            consts,
            asm: String::new(),
            lines: Vec::new(),
            alloc: RegAlloc::new(),
            gensym: 0,
            trailers: Vec::new(),
        }
    }

    fn finish(self) -> Lowering {
        Lowering {
            asm: self.asm,
            lines: self.lines,
        }
    }

    fn flush_trailers(&mut self) {
        let trailers = std::mem::take(&mut self.trailers);
        for (span, block) in trailers {
            self.push(&block, &span);
        }
    }

    /// Append one asm line and record its `(asm byte range, .tam span)` mapping.
    fn push(&mut self, text: &str, span: &Span) {
        let start = self.asm.len();
        self.asm.push_str(text);
        self.lines.push((start..self.asm.len(), span.clone()));
    }

    /// A fresh, asm-safe label: `__<prefix><n>` (no leading dot — dots are
    /// directives in tamal-asm).
    fn gensym(&mut self, prefix: &str) -> String {
        let label = format!("__{prefix}{}", self.gensym);
        self.gensym += 1;
        label
    }

    /// Lower one top-level statement. `entry` is the test's name span, used for
    /// statements (like `pass`) that have no more specific span of their own.
    fn top_stmt(&mut self, stmt: &Stmt, entry: &Span) -> Result<(), Vec<Diagnostic>> {
        match stmt {
            Stmt::Pass => self.push("\thalt 0x00\n", entry),
            Stmt::Fail { code, span } => self.push(&format!("\thalt {code}\n"), span),
            Stmt::Raw { .. } => self.lower_raw(stmt)?,
            Stmt::Send { .. } => self.lower_send(stmt)?,
            Stmt::CrcRegion { .. } => self.lower_crc_region(stmt)?,
            Stmt::Config { .. } => self.lower_config(stmt)?,
            Stmt::Frame { body, span } => self.lower_frame(body, span)?,
            Stmt::Recv { targets, span } => self.lower_recv(targets, span)?,
            Stmt::WaitState { bind, span } => self.lower_wait_state(bind, span)?,
            Stmt::Expect { span, .. } => {
                return Err(vec![
                    Diagnostic::error(span.clone(), "`expect crc` must appear inside a `frame`")
                        .with_help("wrap the response phase in `frame { … }`"),
                ]);
            }
        }
        Ok(())
    }

    fn lower_raw(&mut self, stmt: &Stmt) -> Result<(), Vec<Diagnostic>> {
        let Stmt::Raw {
            mnemonic,
            operands,
            span,
        } = stmt
        else {
            unreachable!("lower_raw called with non-Raw statement")
        };
        let text = if operands.is_empty() {
            format!("\t{mnemonic}\n")
        } else {
            format!("\t{mnemonic} {}\n", operands.join(", "))
        };
        self.push(&text, span);
        Ok(())
    }

    fn lower_send(&mut self, stmt: &Stmt) -> Result<(), Vec<Diagnostic>> {
        let Stmt::Send {
            bytes,
            append_crc,
            span,
        } = stmt
        else {
            unreachable!("lower_send called with non-Send statement")
        };
        let mut bs = consteval::eval_bytes(bytes, self.consts).map_err(|d| vec![d])?;
        if *append_crc {
            bs.push(tamal_abi::crc8::crc8(&bs));
        }
        for b in bs {
            self.push(&format!("\tput_byte 0x{b:02X}\n"), span);
        }
        Ok(())
    }

    fn lower_crc_region(&mut self, stmt: &Stmt) -> Result<(), Vec<Diagnostic>> {
        let Stmt::CrcRegion { sends, span } = stmt else {
            unreachable!("lower_crc_region called with non-CrcRegion statement")
        };
        let mut total = Vec::new();
        for e in sends {
            total.extend(consteval::eval_bytes(e, self.consts).map_err(|d| vec![d])?);
        }
        total.push(tamal_abi::crc8::crc8(&total));
        for b in total {
            self.push(&format!("\tput_byte 0x{b:02X}\n"), span);
        }
        Ok(())
    }

    fn lower_config(&mut self, stmt: &Stmt) -> Result<(), Vec<Diagnostic>> {
        let Stmt::Config {
            role,
            io,
            sck,
            alert,
            span,
        } = stmt
        else {
            unreachable!("lower_config called with non-Config statement")
        };
        self.push(
            &format!("\tset_config {role}, {io}, {sck}, {alert}\n"),
            span,
        );
        Ok(())
    }

    fn lower_frame(&mut self, body: &[Stmt], span: &Span) -> Result<(), Vec<Diagnostic>> {
        self.push("\tcs_assert\n", span);
        self.alloc.enter_scope();
        let mut deferred: Vec<Deferred> = Vec::new();
        for stmt in body {
            self.frame_body_stmt(stmt, &mut deferred)?;
        }
        self.push("\tcs_deassert\n", span);
        for d in &deferred {
            self.push(
                &format!("\tbnez {}, {}\n", reg_name(d.reg), d.label),
                &d.span,
            );
            self.trailers.push((
                d.span.clone(),
                format!("{}:\n\thalt 0x{:02X}\n", d.label, d.code),
            ));
        }
        self.alloc.exit_scope();
        Ok(())
    }

    fn frame_body_stmt(
        &mut self,
        stmt: &Stmt,
        deferred: &mut Vec<Deferred>,
    ) -> Result<(), Vec<Diagnostic>> {
        match stmt {
            Stmt::Send { .. } => self.lower_send(stmt),
            Stmt::CrcRegion { .. } => self.lower_crc_region(stmt),
            Stmt::Raw { .. } => self.lower_raw(stmt),
            Stmt::Recv { targets, span } => self.lower_recv(targets, span),
            Stmt::WaitState { bind, span } => self.lower_wait_state(bind, span),
            Stmt::Expect { else_code, span } => {
                let code = consteval::eval_byte(else_code, self.consts).map_err(|d| vec![d])?;
                // Consume the trailing CRC byte (drives the RX residue to 0).
                let discard = self.alloc.temp(span).map_err(|d| vec![d])?;
                self.push(&format!("\tget_byte {}\n", reg_name(discard)), span);
                self.alloc.free(discard);
                // Latch the residue; keep it live until the deferred branch.
                let res = self.alloc.temp(span).map_err(|d| vec![d])?;
                self.push(&format!("\trdsr {}, crc\n", reg_name(res)), span);
                let label = self.gensym("fail");
                deferred.push(Deferred {
                    reg: res,
                    label,
                    code,
                    span: span.clone(),
                });
                Ok(())
            }
            // Not legal inside a frame. Listed explicitly (no wildcard) so a new
            // Stmt variant forces a decision here rather than silently erroring.
            Stmt::Pass | Stmt::Fail { .. } | Stmt::Config { .. } | Stmt::Frame { .. } => {
                Err(vec![Diagnostic::error(
                    stmt_span(stmt),
                    "this statement is not allowed inside a `frame`",
                )])
            }
        }
    }

    fn lower_recv(
        &mut self,
        targets: &[crate::parser::RecvTarget],
        span: &Span,
    ) -> Result<(), Vec<Diagnostic>> {
        use crate::parser::RecvTarget;
        for target in targets {
            match target {
                RecvTarget::Name(name) => {
                    let reg = self.alloc.bind(name.clone(), span).map_err(|d| vec![d])?;
                    self.push(&format!("\tget_byte {}\n", reg_name(reg)), span);
                }
                RecvTarget::Discard => {
                    let reg = self.alloc.temp(span).map_err(|d| vec![d])?;
                    self.push(&format!("\tget_byte {}\n", reg_name(reg)), span);
                    self.alloc.free(reg);
                }
            }
        }
        Ok(())
    }

    fn lower_wait_state(
        &mut self,
        bind: &Option<String>,
        span: &Span,
    ) -> Result<(), Vec<Diagnostic>> {
        let label = self.gensym("wait");
        self.push(&format!("{label}:\n"), span);
        self.push("\tcrc_reset\n", span);
        let resp = match bind {
            Some(name) => self.alloc.bind(name.clone(), span).map_err(|d| vec![d])?,
            None => self.alloc.temp(span).map_err(|d| vec![d])?,
        };
        self.push(&format!("\tget_byte {}\n", reg_name(resp)), span);
        let k = self.alloc.temp(span).map_err(|d| vec![d])?;
        self.push(
            &format!("\tli {}, 0x{WAIT_STATE_CODE:02X}\n", reg_name(k)),
            span,
        );
        self.push(
            &format!("\tbeq {}, {}, {label}\n", reg_name(resp), reg_name(k)),
            span,
        );
        self.alloc.free(k);
        if bind.is_none() {
            self.alloc.free(resp);
        }
        Ok(())
    }
}

/// The best source span for a statement, for diagnostics.
fn stmt_span(stmt: &Stmt) -> Span {
    match stmt {
        Stmt::Pass => 0..0,
        Stmt::Fail { span, .. }
        | Stmt::Raw { span, .. }
        | Stmt::Send { span, .. }
        | Stmt::CrcRegion { span, .. }
        | Stmt::Config { span, .. }
        | Stmt::Frame { span, .. }
        | Stmt::Recv { span, .. }
        | Stmt::WaitState { span, .. }
        | Stmt::Expect { span, .. } => span.clone(),
    }
}

/// Render a physical register as its `xN` asm operand.
fn reg_name(reg: Reg) -> String {
    format!("x{}", reg.bits())
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
