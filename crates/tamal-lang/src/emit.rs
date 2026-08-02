//! Emit: lower a `Module` to tamal-asm text, plus a source map from
//! generated-asm byte offsets back to the originating `.tam` spans. A `test`
//! becomes the entry label; `pass`/`fail`/raw become lines, and `send`/
//! `crc_region` evaluate to `put_byte` runs (with the compile-time CRC-8).

use crate::parser::{Arg, Expr, Module, ProcDef, Stmt};
use std::collections::HashMap;
use std::rc::Rc;
use tamal_asm::{Diagnostic, Span};

use crate::consteval::{self, Env};
use crate::regalloc::RegAlloc;
use tamal_abi::isa::Reg;

/// The eSPI WAIT_STATE response code the `wait_state` poll spins on.
const WAIT_STATE_CODE: u8 = 0x0F;

/// The largest number of asm lines a single program may emit.
///
/// Exactly that: every [`Emitter::push`] appends one line, `flush_trailers`
/// included, so the count and the line total never diverge.
///
/// A tamal program is capped at 1024 words, so anything beyond this can never
/// assemble. The per-construct [`crate::MAX_UNROLL`] gives a *better message*
/// for the obvious case (`repeat 99999999`), but only a central budget bounds
/// *composition* — nested `repeat`s, or a `proc` that fans out to two calls per
/// level. Without it those hang the compiler with no diagnostic.
///
/// Deliberately looser than 1024 because labels and directives (`.globl`,
/// `_start:`, `__waitN:`) emit lines that are not words at all, so lines can
/// outnumber words. (`li` tiling to two words pushes the ratio the *safe* way —
/// more words per line — so it is not a reason to loosen anything.) Measured:
/// the line-densest legal program, all `wait_state` (5 pushes per 4 words), is
/// 1021 words / 1278 lines, so 4096 carries 3.2x headroom. Its job is to stop
/// unbounded growth, not to duplicate the assembler's exact cap.
///
/// `pub(crate)` only so the driver's tests can place a program exactly on the
/// boundary; nothing outside this module reads it.
pub(crate) const MAX_EMITTED_LINES: usize = 4096;

/// The largest number of *expansions* — `proc` inlines plus `repeat`
/// iterations — a single program may perform.
///
/// [`MAX_EMITTED_LINES`] bounds the *output*, which is only the same thing as
/// bounding the *work* while every unit of expansion emits something. A body
/// that emits nothing — `{ }`, `send []`, `recv 0`, `repeat 0 { … }`, a call to
/// an empty `proc` — still costs a full scope enter/exit per iteration, so
/// `repeat 1024 { repeat 1024 { repeat 1024 { } } }` does 2^30 units of work,
/// emits three lines, and never reaches `push` at all. This bounds that work
/// directly, so the two budgets together cover both halves.
///
/// This is a **policy**, not a derivation: it rejects a program whose average
/// expansion nesting depth exceeds ~64. Nothing about [`MAX_EMITTED_LINES`]
/// bounds the expansion-per-line ratio, because a single emitting statement can
/// be nested arbitrarily deep. Measured, the exact boundary:
///
/// ```text
/// test t { repeat 1023 { repeat 1 { … repeat 1 { cs_assert } … } } pass }
/// ```
///
/// With 63 inner levels that is **exactly 1024 words** — the assembler's own
/// hard cap, so as legal as a program gets — and costs 1023 * 64 = 65472
/// expansions, sitting just under the bar. One level more is rejected. So the
/// ratio this admits is **64 expansions per emitted word**, not 16 per emitted
/// line; an earlier version of this comment claimed the latter and was wrong by
/// ~64x.
///
/// 64 deep is far past anything real. Measured on the shape Plan 5's `espi`
/// stdlib produces — `repeat n { command(…) }` over a `frame` `proc` that calls
/// two more and unrolls `recv` 64 times — a complete ~950-word program costs
/// 816 expansions (80x headroom), and pushing the same shape up to the *line*
/// ceiling costs 3400, so the emission budget binds first at every size. The
/// degenerate-but-legal zero-emitting `repeat 1024 { send [] }` costs 1024 (64x).
///
/// Deliberately **not** written as a multiple of [`MAX_EMITTED_LINES`]. It was,
/// and the formula implied a derivation that does not exist: retuning the line
/// budget down to its own honest 3.2x headroom (~1400) would mechanically drag
/// this to 22400, where a depth-20 program of 1027 lines starts to sit on the
/// edge. The two budgets bound different things and move independently.
///
/// ## What this does *not* bound
///
/// Only *expansion* work — `proc` inlining and `repeat` unrolling. Consteval is
/// outside both budgets: a `fn` chain that fans out (`fn f0() -> int { f1() ^
/// f1() }`, …) costs 2^depth calls, and neither the line count nor the
/// expansion count moves while it runs. `repeat` reaches it — `lower_repeat`
/// evaluates its count expression once per invocation, so
/// `repeat 1024 { repeat f0() { } }` multiplies the fan-out by the unroll and
/// takes 13 s for three emitted lines. See the plan's tracked follow-up; the
/// remedy is a consteval work budget, which is `consteval`-shaped work.
const MAX_EXPANSIONS: usize = 65536;

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
        e.push(".globl _start\n", &test.name_span)?;
        e.push("_start:\n", &test.name_span)?;
        for stmt in &test.stmts {
            e.stmt(stmt, &test.name_span)?;
        }
        e.flush_trailers()?;
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
    /// Per-test verdict trailers hoisted out of a `frame`, as
    /// `(originating .tam span, label, halt code)`. Kept as parts rather than
    /// pre-rendered text so `flush_trailers` can emit one `push` per asm line.
    trailers: Vec<(Span, String, u8)>,
    /// One entry per `frame` currently being lowered, innermost last; each
    /// holds the verdicts its `expect`s deferred to the frame exit. A stack
    /// (rather than a parameter) so an inlined `proc` body reaches the
    /// enclosing frame's list without threading it through the expansion.
    frames: Vec<Vec<Deferred>>,
    /// The module's `proc` table. `proc`s live here rather than in the `Env`
    /// because only emit expands them (a `proc` call is a statement); `fn`s
    /// live in the `Env` because consteval resolves them (a `fn` call is an
    /// expression). Duplicates were already rejected by the driver.
    ///
    /// Behind an [`Rc`] for the same reason as [`Env`]'s tables: every
    /// expansion clones the entry to release the `&self` borrow, and a bare
    /// `ProcDef` would deep-copy the parameters *and the whole body AST* each
    /// time. With the `Rc` that clone is a refcount bump.
    procs: HashMap<String, Rc<ProcDef>>,
    /// The `proc`s whose expansion is in progress, innermost last — a name that
    /// appears twice is recursion, which cannot be inlined.
    ///
    /// Deliberately independent of [`Env`]'s `active` chain: that one tracks
    /// `fn`s, which consteval resolves. See `lower_call` for why the two are
    /// kept apart rather than unified.
    active_procs: Vec<String>,
    /// How many expansions (`proc` inlines + `repeat` iterations) have been
    /// performed so far, for [`MAX_EXPANSIONS`]. A **whole-program** running
    /// total: never reset per construct, or composition would escape it the
    /// same way it escapes the per-construct [`crate::MAX_UNROLL`].
    expansions: usize,
    /// The source span of each expansion currently in progress, outermost
    /// first — the caret [`Emitter::budget_error`] anchors on.
    ///
    /// A `Vec`, never a map: the *order* is the whole point (`first()` is the
    /// outermost), and a map's iteration order would put nondeterminism into a
    /// diagnostic. Pushed and popped by the [`Emitter::enter_expansion_scope`]
    /// / [`Emitter::exit_expansion_scope`] pair, so it stays balanced on the
    /// error path too.
    expansion_sites: Vec<Span>,
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
                .map(|p| (p.name.clone(), Rc::new(p.clone())))
                .collect(),
            active_procs: Vec::new(),
            expansions: 0,
            expansion_sites: Vec::new(),
        }
    }

    fn finish(self) -> Lowering {
        Lowering {
            asm: self.asm,
            lines: self.lines,
        }
    }

    fn flush_trailers(&mut self) -> Result<(), Vec<Diagnostic>> {
        let trailers = std::mem::take(&mut self.trailers);
        for (span, label, code) in trailers {
            // Two pushes, not one block: it keeps `MAX_EMITTED_LINES` an exact
            // line count rather than an approximate one, and gives the source
            // map per-line granularity over the trailer.
            self.push(&format!("{label}:\n"), &span)?;
            self.push(&format!("\thalt 0x{code:02X}\n"), &span)?;
        }
        Ok(())
    }

    /// Append one asm line and record its `(asm byte range, .tam span)` mapping.
    ///
    /// Fallible **only** to enforce [`MAX_EMITTED_LINES`]. Every emitted line
    /// funnels through here, so this is the one place that can bound total
    /// output — and because it returns a `Result`, the `?` at each call site
    /// unwinds every enclosing expansion loop for free. That is the point of
    /// the fallible shape over a latch flag: a latch would record the overflow
    /// but leave each expansion loop to notice it, so a loop that forgot to
    /// check would still run a 2^22 expansion to completion. Here it is not
    /// possible to forget.
    ///
    /// Implementation note, not an invariant anything relies on: the budget is
    /// checked before the append rather than after, so `asm` never transiently
    /// holds an over-budget line. Nothing can observe the difference —
    /// [`Lowering`] is only built on the `Ok` path — so moving the check would
    /// be a wash. See [`Emitter::budget_error`] for where the diagnostic points.
    ///
    /// **Invariant the callers depend on: an emit error is always fatal.**
    /// Every `?` here is an early return that skips cleanup — `lower_recv`,
    /// `lower_wait_state` and `lower_expect` return before their `alloc.free`,
    /// and `lower_frame` before its `alloc.exit_scope()` — which is sound only
    /// because the whole compilation is abandoned, so the leaked scope is never
    /// observed. [`emit`] returns `Vec<Diagnostic>`, so recovering and
    /// continuing is a plausible future direction; it would turn every one of
    /// those into a real leak.
    fn push(&mut self, text: &str, span: &Span) -> Result<(), Vec<Diagnostic>> {
        if self.lines.len() >= MAX_EMITTED_LINES {
            return Err(self.budget_error(
                span,
                format!("the program emits more than {MAX_EMITTED_LINES} asm lines"),
                "a tamal program is at most 1024 words — reduce a `repeat` count, \
                 or a `proc` that expands to more calls than it looks like",
            ));
        }
        let start = self.asm.len();
        self.asm.push_str(text);
        self.lines.push((start..self.asm.len(), span.clone()));
        Ok(())
    }

    /// Build the diagnostic for either compile-time budget, so both report the
    /// same way. `site` is where the budget actually ran out — the line that
    /// overflowed, or the expansion that was being entered.
    ///
    /// The caret goes on the **outermost** expansion still in progress, not on
    /// `site`. `site` is specific but neither relevant nor stable: which
    /// statement it lands on depends on where the overflowing line happens to
    /// fall, so adding an unrelated earlier statement moves the caret, and on
    /// `repeat 1024 { cs_assert cs_deassert tar 2 crc_reset }` it accuses
    /// `tar 2` when the culprit is the `repeat`. The outermost expansion is the
    /// construct the author has to shrink.
    ///
    /// But the caret alone is not enough either, because the culprit is often
    /// **neither end**. When the runaway lives in a callee — the shape a
    /// bundled stdlib produces, where the author writes one small call and the
    /// unroll is library-internal — the outermost site is an innocent
    /// unshrinkable wrapper and `site` is an innocent leaf:
    ///
    /// ```text
    /// proc big() { repeat 1024 { repeat 1024 { cs_assert } } }
    /// test t { repeat 1 { big() } pass }
    /// ```
    ///
    /// So label the **whole chain**: every expansion between the caret and
    /// `site`, outermost first, then `site` itself. It is ordered (the labels
    /// come straight off the ordered site stack, never a map) and bounded by
    /// source nesting depth — tens of entries, not thousands. `site` is skipped
    /// when nothing is expanding (a straight-line program, or a trailer flushed
    /// after every expansion has closed): there it *is* the caret, and the
    /// label would only duplicate it.
    fn budget_error(&self, site: &Span, message: String, help: &str) -> Vec<Diagnostic> {
        let primary = self.expansion_sites.first().unwrap_or(site).clone();
        let mut d = Diagnostic::error(primary.clone(), message).with_help(help);
        for nested in self.expansion_sites.iter().skip(1) {
            d = d.with_label(nested.clone(), "nested expansion");
        }
        if primary != *site {
            d = d.with_label(site.clone(), "the budget ran out here");
        }
        vec![d]
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
    /// and `Stmt::Repeat`: both are legal at the top level, inside a `frame`
    /// and inside a `proc`. A new variant that is *not* must be added to the
    /// guard by hand; the compiler will not prompt for it.
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
            Stmt::Pass => self.push("\thalt 0x00\n", entry)?,
            Stmt::Fail { code, span } => self.push(&format!("\thalt {code}\n"), span)?,
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
            Stmt::Repeat { count, body, span } => self.lower_repeat(count, body, span, entry)?,
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
        self.push(&text, span)?;
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
            self.push(&format!("\tput_byte 0x{b:02X}\n"), span)?;
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
            self.push(&format!("\tput_byte 0x{b:02X}\n"), span)?;
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
        )?;
        Ok(())
    }

    fn lower_frame(
        &mut self,
        body: &[Stmt],
        span: &Span,
        entry: &Span,
    ) -> Result<(), Vec<Diagnostic>> {
        self.push("\tcs_assert\n", span)?;
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
        self.push("\tcs_deassert\n", span)?;
        for d in &deferred {
            self.push(
                &format!("\tbnez {}, {}\n", reg_name(d.reg), d.label),
                &d.span,
            )?;
            self.trailers
                .push((d.span.clone(), d.label.clone(), d.code));
        }
        // The one `exit_scope` that is NOT routed through
        // `exit_expansion_scope`: this frame's verdicts have just been emitted,
        // so the registers they held are dead and there is nothing left to keep
        // alive. Every *nested* expansion scope must use the helper instead.
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
                    self.push(&format!("\tget_byte {}\n", reg_name(reg)), span)?;
                }
                RecvTarget::Discard => {
                    let reg = self.alloc.temp(span).map_err(|d| vec![d])?;
                    self.push(&format!("\tget_byte {}\n", reg_name(reg)), span)?;
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
        self.push(&format!("{label}:\n"), span)?;
        self.push("\tcrc_reset\n", span)?;
        let resp = match bind {
            Some(name) => self.alloc.bind(name.clone(), span).map_err(|d| vec![d])?,
            None => self.alloc.temp(span).map_err(|d| vec![d])?,
        };
        self.push(&format!("\tget_byte {}\n", reg_name(resp)), span)?;
        let k = self.alloc.temp(span).map_err(|d| vec![d])?;
        self.push(
            &format!("\tli {}, 0x{WAIT_STATE_CODE:02X}\n", reg_name(k)),
            span,
        )?;
        self.push(
            &format!("\tbeq {}, {}, {label}\n", reg_name(resp), reg_name(k)),
            span,
        )?;
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
        self.push(&format!("\tget_byte {}\n", reg_name(discard)), span)?;
        self.alloc.free(discard);
        // Latch the residue; keep it live until the deferred branch.
        let res = self.alloc.temp(span).map_err(|d| vec![d])?;
        self.push(&format!("\trdsr {}, crc\n", reg_name(res)), span)?;
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
        // Charged before anything is swapped, so the over-budget path has
        // nothing to unwind.
        self.enter_expansion_scope(span)?;
        // `child_for_call_values`, NOT `child_for_call`: the callee is pushed
        // onto `active_procs` below instead of onto `Env`'s chain. The two
        // chains are kept separate because the `Env` one tracks `fn`s (read
        // only by consteval's `eval_fn_call`) while this one tracks `proc`s
        // (read only here). Sharing one chain would work today only because the
        // driver forbids a `fn` and a `proc` sharing a name — a guarantee that
        // qualified names (`espi.command`) may weaken. Do not unify them.
        let child = self.env.child_for_call_values(bindings);
        let saved = std::mem::replace(&mut self.env, child);
        self.active_procs.push(name.to_string());
        let mut result = Ok(());
        for s in &p.body {
            if let Err(e) = self.stmt(s, entry) {
                result = Err(e);
                break;
            }
        }
        // Unwind in reverse, on the error path too, so the emitter is never
        // left inside a half-expanded call.
        self.env = saved;
        self.active_procs.pop();
        self.exit_expansion_scope();
        result
    }

    /// `repeat N { … }` — a compile-time unroll (spec §5): emit the body `N`
    /// times. There is no loop counter and no branch.
    ///
    /// Each iteration gets its own register scope, so a binding made in the
    /// body is released before the next iteration reuses the register — closed
    /// through `exit_expansion_scope` so a verdict latched inside the body
    /// stays alive for the enclosing frame.
    fn lower_repeat(
        &mut self,
        count: &Expr,
        body: &[Stmt],
        span: &Span,
        entry: &Span,
    ) -> Result<(), Vec<Diagnostic>> {
        let n = consteval::eval_int(count, &self.env).map_err(|d| vec![d])?;
        if !(0..=crate::MAX_UNROLL).contains(&n) {
            return Err(vec![
                Diagnostic::error(
                    span.clone(),
                    format!("`repeat` count {n} is not in 0..={}", crate::MAX_UNROLL),
                )
                .with_help(
                    "a tamal program is at most 1024 words, so a larger unroll can never assemble",
                ),
            ]);
        }
        for _ in 0..n {
            self.enter_expansion_scope(span)?;
            let mut result = Ok(());
            for s in body {
                if let Err(e) = self.stmt(s, entry) {
                    result = Err(e);
                    break;
                }
            }
            self.exit_expansion_scope();
            result?;
        }
        Ok(())
    }

    /// Open one expansion's register scope, charging it to [`MAX_EXPANSIONS`].
    ///
    /// The entry twin of [`Emitter::exit_expansion_scope`]: every expansion
    /// scope (a `proc` inline, one `repeat` iteration) must be opened through
    /// here rather than by calling `enter_scope` directly, because this is the
    /// one place that counts expansion *work*. `push` cannot: a body that emits
    /// nothing never reaches it, so the emission budget is blind to exactly the
    /// shapes that cost the most per emitted line. Nor can `Emitter::stmt`: an
    /// empty body never calls it, so a counter there charges nothing for the
    /// innermost level's iterations.
    ///
    /// Fallible for the same reason `push` is: the `?` at each call site
    /// unwinds every enclosing expansion loop for free, so a loop cannot forget
    /// to check. On the error path this is a no-op — nothing is pushed and no
    /// scope is opened — so the caller returns without *needing* a matching
    /// exit (the stack stays balanced), and the diagnostic still sees the
    /// enclosing expansions it should point at.
    fn enter_expansion_scope(&mut self, site: &Span) -> Result<(), Vec<Diagnostic>> {
        if self.expansions >= MAX_EXPANSIONS {
            return Err(self.budget_error(
                site,
                format!("the program performs more than {MAX_EXPANSIONS} expansions"),
                "every `repeat` iteration and every `proc` call is expanded at compile time, \
                 even when its body emits nothing — reduce a `repeat` count, or a `proc` that \
                 expands to more calls than it looks like",
            ));
        }
        self.expansions += 1;
        self.expansion_sites.push(site.clone());
        self.alloc.enter_scope();
        Ok(())
    }

    /// Close a nested expansion's register scope, keeping any pending verdict
    /// registers alive.
    ///
    /// Every expansion scope (`proc` inline, `repeat` iteration) must be closed
    /// through here rather than by calling `exit_scope` directly: the enclosing
    /// frame's deferred verdicts may hold registers this scope owned, and
    /// releasing them would let a later statement clobber the value the verdict
    /// branches on. The frame's *own* exit is the exception — it closes its
    /// scope after its verdicts are already emitted.
    fn exit_expansion_scope(&mut self) {
        self.expansion_sites.pop();
        self.alloc.exit_scope();
        self.reserve_deferred();
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
        | Stmt::Call { span, .. }
        | Stmt::Repeat { span, .. } => span.clone(),
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
    fn a_negative_repeat_count_is_rejected() {
        // The grammar cannot spell a negative literal today, but the AST is a
        // public type and `Expr::Int` can hold one. Without the range check's
        // lower bound `for _ in 0..-1` is simply an empty range, so a negative
        // count would silently emit nothing instead of pointing at the mistake.
        let err = emit(
            &one(vec![Stmt::Repeat {
                count: Expr::Int {
                    value: -1,
                    span: 0..1,
                },
                body: vec![Stmt::Raw {
                    mnemonic: "cs_assert".into(),
                    operands: vec![],
                    span: 0..1,
                }],
                span: 0..1,
            }]),
            Env::new(),
        )
        .err()
        .expect("a negative count is a diagnostic, not a lowering");
        assert!(
            err[0].message.contains("not in 0..=1024"),
            "got: {:?}",
            err[0]
        );
    }

    #[test]
    fn a_program_of_exactly_the_budget_compiles_and_one_line_more_does_not() {
        // Pins where the cap actually falls. The `.globl`/`_start:` prologue is
        // two lines, so `MAX_EMITTED_LINES - 2` raw statements sit exactly on
        // it; one more is over. Driven off the constant so the boundary stays
        // pinned if the budget is ever retuned.
        let raws = |n: usize| -> Vec<Stmt> {
            (0..n)
                .map(|_| Stmt::Raw {
                    mnemonic: "cs_assert".into(),
                    operands: vec![],
                    span: 0..1,
                })
                .collect()
        };
        assert!(
            emit(&one(raws(MAX_EMITTED_LINES - 2)), Env::new()).is_ok(),
            "exactly {MAX_EMITTED_LINES} emitted lines must compile"
        );
        assert!(
            emit(&one(raws(MAX_EMITTED_LINES - 1)), Env::new()).is_err(),
            "one line over the budget must be a diagnostic"
        );
    }

    #[test]
    fn exactly_the_expansion_budget_compiles_and_one_expansion_more_does_not() {
        // The expansion budget's twin of the test above. `repeat N { }` emits
        // nothing at all, so this isolates it from the emission budget
        // completely. `MAX_UNROLL` forbids reaching `MAX_EXPANSIONS` with one
        // `repeat`, so nest two: `outer * (1 + inner)` iterations, both counts
        // legal. Driven off the constants so the boundary stays pinned if
        // either budget is ever retuned.
        let rep = |n: i64, body: Vec<Stmt>| Stmt::Repeat {
            count: Expr::Int {
                value: n,
                span: 0..1,
            },
            body,
            span: 0..1,
        };
        let outer = crate::MAX_UNROLL;
        let inner = MAX_EXPANSIONS as i64 / outer - 1;
        assert_eq!(
            outer * (1 + inner),
            MAX_EXPANSIONS as i64,
            "the budget must factor over MAX_UNROLL for this construction to be exact"
        );
        assert!(
            emit(&one(vec![rep(outer, vec![rep(inner, vec![])])]), Env::new()).is_ok(),
            "exactly {MAX_EXPANSIONS} expansions must compile"
        );
        assert!(
            emit(
                &one(vec![rep(outer, vec![rep(inner, vec![])]), rep(1, vec![])]),
                Env::new()
            )
            .is_err(),
            "one expansion over the budget must be a diagnostic"
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
