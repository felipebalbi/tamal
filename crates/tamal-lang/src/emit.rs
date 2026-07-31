//! Emit: lower a `Module` to tamal-asm text, plus a source map from
//! generated-asm byte offsets back to the originating `.tam` spans. A `test`
//! becomes the entry label; `pass`/`fail`/raw become lines, and `send`/
//! `crc_region` evaluate to `put_byte` runs (with the compile-time CRC-8).

use crate::parser::{Arg, Expr, Module, ProcDef, Stmt};
use std::collections::HashMap;
use tamal_asm::{Diagnostic, Span};

use crate::consteval::{self, Env};
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
pub fn emit(module: &Module, env: Env) -> Result<Lowering, Vec<Diagnostic>> {
    let mut e = Emitter::new(env, module);
    for test in &module.tests {
        e.push(".globl _start\n", &test.name_span);
        e.push("_start:\n", &test.name_span);
        for stmt in &test.stmts {
            e.stmt(stmt, &test.name_span)?;
        }
        e.flush_trailers();
    }
    Ok(e.finish())
}

/// The lowering state: the growing asm text + source map.
struct Emitter {
    /// Owned, not borrowed: a `proc` expansion swaps this around the callee body.
    env: Env,
    asm: String,
    lines: Vec<(Span, Span)>,
    alloc: RegAlloc,
    gensym: u32,
    trailers: Vec<(Span, String)>,
    /// One entry per `frame` currently being lowered, innermost last; each
    /// holds the verdicts its `expect`s deferred to the frame exit. A stack
    /// (rather than a parameter) so an inlined `proc` body reaches the
    /// enclosing frame's list without threading it through the expansion.
    frames: Vec<Vec<Deferred>>,
    /// The module's `proc` table. `proc`s live here rather than in the `Env`
    /// because only emit expands them (a `proc` call is a statement); `fn`s
    /// live in the `Env` because consteval resolves them (a `fn` call is an
    /// expression). Duplicates were already rejected by the driver.
    procs: HashMap<String, ProcDef>,
    /// The `proc`s whose expansion is in progress, innermost last — a name that
    /// appears twice is recursion, which cannot be inlined.
    active_procs: Vec<String>,
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

impl Emitter {
    fn new(env: Env, module: &Module) -> Self {
        Emitter {
            env,
            asm: String::new(),
            lines: Vec::new(),
            alloc: RegAlloc::new(),
            gensym: 0,
            trailers: Vec::new(),
            frames: Vec::new(),
            procs: module
                .procs
                .iter()
                .map(|p| (p.name.clone(), p.clone()))
                .collect(),
            active_procs: Vec::new(),
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

    /// Is a `frame` currently open?
    fn in_frame(&self) -> bool {
        !self.frames.is_empty()
    }

    /// Lower one statement.
    ///
    /// `entry` is the test's name span, used by statements (like `pass`) that
    /// have no more specific span of their own. Legality is context-dependent:
    /// `pass`/`fail`/`config`/`frame` are rejected inside a `frame`, and
    /// `expect` is rejected outside one. The dispatch match is wildcard-free by
    /// design, so a new `Stmt` variant forces a decision *about lowering* here.
    ///
    /// The in-frame legality decision, though, lives in the `matches!` guard
    /// below — which lists only the rejected variants, so a new one silently
    /// defaults to legal inside a frame. That is deliberate for `Stmt::Call`
    /// (and, in Task 7, `Stmt::Repeat`): both are legal in either context. A
    /// new variant that is *not* must be added to the guard by hand; the
    /// compiler will not prompt for it.
    fn stmt(&mut self, stmt: &Stmt, entry: &Span) -> Result<(), Vec<Diagnostic>> {
        if self.in_frame()
            && matches!(
                stmt,
                Stmt::Pass | Stmt::Fail { .. } | Stmt::Config { .. } | Stmt::Frame { .. }
            )
        {
            return Err(vec![Diagnostic::error(
                stmt_span(stmt, entry),
                "this statement is not allowed inside a `frame`",
            )]);
        }
        match stmt {
            Stmt::Pass => self.push("\thalt 0x00\n", entry),
            Stmt::Fail { code, span } => self.push(&format!("\thalt {code}\n"), span),
            Stmt::Raw { .. } => self.lower_raw(stmt)?,
            Stmt::Send { .. } => self.lower_send(stmt)?,
            Stmt::CrcRegion { .. } => self.lower_crc_region(stmt)?,
            Stmt::Config { .. } => self.lower_config(stmt)?,
            Stmt::Frame { body, span } => self.lower_frame(body, span, entry)?,
            Stmt::Recv { targets, span } => self.lower_recv(targets, span)?,
            Stmt::WaitState { bind, span } => self.lower_wait_state(bind, span)?,
            Stmt::Expect { else_code, span } => self.lower_expect(else_code, span)?,
            Stmt::Call {
                name,
                name_span,
                args,
                span,
            } => self.lower_call(name, name_span, args, span, entry)?,
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
        let mut bs = consteval::eval_bytes(bytes, &self.env).map_err(|d| vec![d])?;
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
            total.extend(consteval::eval_bytes(e, &self.env).map_err(|d| vec![d])?);
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

    fn lower_frame(
        &mut self,
        body: &[Stmt],
        span: &Span,
        entry: &Span,
    ) -> Result<(), Vec<Diagnostic>> {
        self.push("\tcs_assert\n", span);
        self.alloc.enter_scope();
        self.frames.push(Vec::new());
        let mut result = Ok(());
        for s in body {
            if let Err(e) = self.stmt(s, entry) {
                result = Err(e);
                break;
            }
        }
        // Pop on the error path too, so the frame stack stays balanced.
        let deferred = self.frames.pop().expect("lower_frame pushed this frame");
        // `alloc` is deliberately NOT unwound here: on the error path this `?`
        // aborts the whole compilation, so the leaked scope is never observed —
        // and balancing it would mean either duplicating `exit_scope` or moving
        // it above the verdict emission, which reads the frame's registers.
        result?;
        // D9 (load-bearing): CS deasserts UNCONDITIONALLY, before any verdict
        // branch. Each `expect` already latched its residue inside the frame;
        // we deassert here, THEN emit the deferred `bnez` verdict(s), and hoist
        // each fail-`halt` to a trailer so the `pass` path falls through. Do not
        // reorder cs_deassert after the bnez — that would strand CS# on a fail.
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

    /// `expect crc else <byte>` (D12 + D9): consume the trailing CRC byte,
    /// latch the RX residue, and defer the verdict branch to the enclosing
    /// frame — it must run *after* `cs_deassert`.
    fn lower_expect(&mut self, else_code: &Expr, span: &Span) -> Result<(), Vec<Diagnostic>> {
        if !self.in_frame() {
            return Err(vec![
                Diagnostic::error(span.clone(), "`expect crc` must appear inside a `frame`")
                    .with_help("wrap the response phase in `frame { … }`"),
            ]);
        }
        let code = consteval::eval_byte(else_code, &self.env).map_err(|d| vec![d])?;
        // Consume the trailing CRC byte (drives the RX residue to 0).
        let discard = self.alloc.temp(span).map_err(|d| vec![d])?;
        self.push(&format!("\tget_byte {}\n", reg_name(discard)), span);
        self.alloc.free(discard);
        // Latch the residue; keep it live until the deferred branch.
        let res = self.alloc.temp(span).map_err(|d| vec![d])?;
        self.push(&format!("\trdsr {}, crc\n", reg_name(res)), span);
        let label = self.gensym("fail");
        let frame = self.frames.last_mut().expect("in_frame() was just checked");
        frame.push(Deferred {
            reg: res,
            label,
            code,
            span: span.clone(),
        });
        Ok(())
    }

    /// Expand a `proc` call **inline** (D2) — the ISA has no `call`/`ret` and no
    /// stack, so there is no runtime linkage to invent.
    ///
    /// The expansion is hygienic (D5): the body is lowered in the callee's own
    /// value scope (module `const`s + this call's parameters, never the
    /// caller's locals) and inside a fresh `RegAlloc` scope, so it cannot alias
    /// a register that is live in the caller, and everything it binds is
    /// released on exit. Labels come from the shared gensym counter, so two
    /// expansions of the same `proc` never collide.
    fn lower_call(
        &mut self,
        name: &str,
        name_span: &Span,
        args: &[Arg],
        span: &Span,
        entry: &Span,
    ) -> Result<(), Vec<Diagnostic>> {
        if self.env.get_fn(name).is_some() {
            return Err(vec![
                Diagnostic::error(
                    span.clone(),
                    format!(
                        "`{name}` is a `fn`: it returns a value and cannot be called as a statement"
                    ),
                )
                .with_help("use a `fn` inside an expression, e.g. `send iord_hdr(0x44, 0x64)`"),
            ]);
        }
        let Some(p) = self.procs.get(name).cloned() else {
            return Err(vec![Diagnostic::error(
                name_span.clone(),
                format!("unknown `proc` `{name}`"),
            )]);
        };
        if self.active_procs.iter().any(|n| n == name) {
            return Err(vec![
                Diagnostic::error(
                    span.clone(),
                    format!("`{name}` is already being expanded: recursion is not possible"),
                )
                .with_help("every `proc` is inlined — the tamal ISA has no call/ret and no stack"),
            ]);
        }
        let bindings =
            consteval::bind_args(name, &p.params, args, &self.env, span).map_err(|d| vec![d])?;
        let child = self.env.child_for_call(name, bindings);
        let saved = std::mem::replace(&mut self.env, child);
        self.active_procs.push(name.to_string());
        self.alloc.enter_scope();
        let mut result = Ok(());
        for s in &p.body {
            if let Err(e) = self.stmt(s, entry) {
                result = Err(e);
                break;
            }
        }
        // Unwind in reverse, on the error path too, so the emitter is never
        // left inside a half-expanded call.
        self.alloc.exit_scope();
        self.reserve_deferred();
        self.active_procs.pop();
        self.env = saved;
        result
    }

    /// Re-take the registers the open frame's pending verdicts depend on.
    ///
    /// An `expect` inside a nested scope (an inlined `proc`, a `repeat`
    /// iteration) latches its residue into a register owned by that scope, but
    /// the branch on it runs at the **frame's** exit. Closing the nested scope
    /// would release the register, so re-take it here — otherwise a later
    /// statement in the frame could clobber the value before the verdict reads
    /// it. (Nested frames are rejected, so the innermost frame is the one that
    /// will consume these.)
    fn reserve_deferred(&mut self) {
        let Some(frame) = self.frames.last() else {
            return;
        };
        let regs: Vec<Reg> = frame.iter().map(|d| d.reg).collect();
        for r in regs {
            self.alloc.reserve(r);
        }
    }
}

/// The best source span for a statement, for diagnostics. `pass` carries no
/// span of its own, so it borrows the test's entry span.
fn stmt_span(stmt: &Stmt, entry: &Span) -> Span {
    match stmt {
        Stmt::Pass => entry.clone(),
        Stmt::Fail { span, .. }
        | Stmt::Raw { span, .. }
        | Stmt::Send { span, .. }
        | Stmt::CrcRegion { span, .. }
        | Stmt::Config { span, .. }
        | Stmt::Frame { span, .. }
        | Stmt::Recv { span, .. }
        | Stmt::WaitState { span, .. }
        | Stmt::Expect { span, .. }
        | Stmt::Call { span, .. } => span.clone(),
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
            fns: vec![],
            procs: vec![],
            tests: vec![Test {
                name: "t".into(),
                name_span: 0..1,
                stmts,
            }],
        }
    }

    #[test]
    fn emits_entry_and_pass() {
        let asm = emit(&one(vec![Stmt::Pass]), Env::new()).unwrap().asm;
        assert_eq!(asm, ".globl _start\n_start:\n\thalt 0x00\n");
    }

    #[test]
    fn emits_fail_code_verbatim() {
        let asm = emit(
            &one(vec![Stmt::Fail {
                code: "0x11".into(),
                span: 0..1,
            }]),
            Env::new(),
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
            Env::new(),
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
            fns: vec![],
            procs: vec![],
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
        let low = emit(&m, Env::new()).unwrap();
        let idx = low.asm.find("bogus").expect("emitted the raw mnemonic");
        let d = Diagnostic::error(idx..idx + 5, "unknown instruction");
        let remapped = low.remap(vec![d]);
        assert_eq!(remapped[0].primary, 40..45);
    }
}
