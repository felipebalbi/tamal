# tamal-lang Plan 3 — Frames, Verdicts & the Register Model Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Compile a *full* eSPI channel test end-to-end — not just the command phase. Add the domain sugar `config`, `frame { … }`, `recv`, `wait_state [name]`, and `expect crc else <byte>`, backed by a compiler-managed **register allocator** (the ≤15-physical-register model, `x0`=zero, no spill), so that an HLL `peripheral_io_read.tam` lowers to bytecode **byte-identical modulo register allocation** to the hand-written `examples/peripheral_io_read.s`.

**Architecture:** Add a scoped, lowest-free-first **register allocator** (`regalloc.rs`) over `x1`..`x15`; entering/leaving a scope (a `frame`) frees everything bound inside it (hygiene by construction, D5). Refactor `emit.rs` into an `Emitter` struct that threads the allocator, a gensym label counter, and a per-test **trailer** accumulator. `frame { }` emits `cs_assert` · body · `cs_deassert` on every exit and hoists each `expect`'s fail-verdict to a trailer *after* the deassert (D9). `wait_state` reproduces the examples' hand-rolled `crc_reset`/`get_byte`/`li`/`beq` poll (resolving spec open-Q #1); `expect crc` consumes the trailing CRC byte, latches the residue with `rdsr … crc`, and defers a `bnez` branch to run after CS deasserts (D12 + D9). Everything is still text → `tamal_asm::assemble`.

**Tech Stack:** Rust (edition 2024), building on Plans 1–2 (`crates/tamal-lang`, `crates/tamal-lang-cli`); `tamal-asm` (`assemble`, `Program`) and `tamal-abi` (`isa::{Instr, Reg}`, `crc8`).

**Spec:** `docs/superpowers/specs/2026-07-20-tamal-lang-design.md` — §1.1 (ISA constraints), §4.2 (the target test), §5 (semantic model + lowering table), §7 (register-safety invariants), and decisions **D5** (named vars / hygiene / no spill), **D9** (frame deasserts before the verdict), **D12** (`wait_state` consumes the response-code byte, `expect crc` consumes the trailing CRC byte).

**Prerequisite (already landed):** the newline-in-brackets PREP fix (`fix(tamal-lang): allow newlines and trailing commas inside brackets/parens`) — multi-line `recv`/packet literals depend on it.

**Deferred to a later plan (NOT in scope here):** `import`/modules + the bundled `espi` stdlib (Plan 5 — so this plan's `peripheral_io_read.tam` uses a local `const`, not `import espi`); `proc`/`fn` + `if`/`while`/`repeat` (Plan 4); `enum`; explicit standalone `let`/`reg` declaration statements (Plan 3 introduces named variables only through `recv` and `wait_state <name>` bindings, which is all the byte-match needs); `--lint` + error injection (Plan 6). `expect`'s only form here is `expect crc else <byte>`.

---

## Architecture & key design decisions

Read this before starting; the tasks assume it.

1. **Register allocator (D5).** A `RegAlloc` owns a pool of `x1`..`x15` (`x0` is the wired zero, never allocated — spec §5, §7). Allocation is **lowest-free-first**. It keeps a **scope stack**: `enter_scope()`/`exit_scope()` bracket a `frame`; `exit_scope` frees every register allocated in that scope and drops its name bindings, so an inner scope can never clobber an outer live value. There is **no spill target**, so running out is a hard `Diagnostic` (never a silent spill). Named bindings come from `recv <name>` and `wait_state <name>`; scratch temporaries (`recv _`, `recv N`, the `wait_state` poll temporaries, `expect`'s trailing-CRC read) are allocated and freed immediately.

2. **`frame { }` deasserts before the verdict (D9).** Lowering: `cs_assert`, `enter_scope`, body, `cs_deassert`, then for each verdict deferred by an `expect` inside the frame emit `bnez <residue>, <fail_label>` (the residue was latched *inside* the frame, before deassert), then `exit_scope`. The `fail_label: halt <code>` block is pushed to a **per-test trailer list** flushed at the end of the program — exactly mirroring the `.s`'s `bad_crc:` tail after `halt VERDICT_OK`.

3. **`wait_state` reproduces the hand-rolled poll (open-Q #1 → hand-rolled).** `<label>:` · `crc_reset` · `get_byte <resp>` · `li <k>, 0x0F` · `beq <resp>, <k>, <label>`. `0x0F` is the eSPI `RSP_WAIT_STATE` code, a constant of the `wait_state` lowering. It **consumes the response-code byte** (D12); a bare `wait_state` discards it, `wait_state name` binds the terminal (non-WAIT_STATE) byte.

4. **`expect crc else X` (D12 + D9).** `get_byte <discard>` (the trailing CRC byte that drives the RX residue to 0) · `rdsr <res>, crc` · defer `(res, gensym fail label, X)` to the enclosing frame. Only legal **inside a frame** (it needs the frame's deassert-then-branch ordering); outside a frame it is a compile error.

5. **Verification: "byte-identical modulo register allocation" (spec §9).** The capstone decodes both programs to `tamal_abi::isa::Instr`, zeroes every `Reg` operand, and compares. This tolerates the allocator's register choices while proving the opcodes, immediates, CRC bytes, and branch offsets all match the hand-written `.s`. Per-construct tasks additionally assert exact generated-asm text (goldens), which pins the allocator's lowest-free-first numbering (`x1`, `x2`, …) and the gensym label names (`__wait0`, `__fail0`, …).

**Label/gensym note:** the tamal-asm label grammar is `[A-Za-z_][A-Za-z0-9_]*` — **no leading dot** (a `.`-prefixed token lexes as a directive). Gensym labels therefore use an underscore prefix: `__wait0`, `__fail0`.

---

## File Structure

- `crates/tamal-lang/src/regalloc.rs` — **create**: `RegAlloc` (scoped pool over `x1`..`x15`), `alloc`/`bind`/`temp`/`free`/`lookup`, `enter_scope`/`exit_scope`, exhaustion diagnostic.
- `crates/tamal-lang/src/parser.rs` — **modify**: add `Stmt::{Config, Frame, Recv, WaitState, Expect}` + `RecvTarget`; parse them (frame bodies reuse `parse_stmt`).
- `crates/tamal-lang/src/emit.rs` — **modify**: refactor to an `Emitter` struct; add the allocator, a gensym counter, a trailer list; lower the five new statements; keep the M1 source map.
- `crates/tamal-lang/src/lib.rs` — **modify**: register `pub mod regalloc`; extend the M2 `halts` match for the new variants.
- `crates/tamal-lang/tests/frames.rs` — **create**: the `frame`/`recv`/`wait_state`/`expect` golden-asm tests + the capstone byte-match-modulo-registers against `examples/peripheral_io_read.s`.
- `crates/tamal-lang/README.md` — **modify** (Task 9): note the new statements + the register model.

---

## Task 1: Register allocator (`regalloc.rs`)

**Files:**
- Create: `crates/tamal-lang/src/regalloc.rs`
- Modify: `crates/tamal-lang/src/lib.rs` (add `pub mod regalloc;`)

- [ ] **Step 1: Register the module**

In `crates/tamal-lang/src/lib.rs`, add to the module list (after `pub mod parser;`):

```rust
pub mod regalloc;
```

- [ ] **Step 2: Write the failing tests**

Create `crates/tamal-lang/src/regalloc.rs` with ONLY the tests first (the types come in Step 4). Paste this whole file:

```rust
//! Register allocation: bind named runtime variables and scratch temporaries to
//! the tamal ISA's 15 usable physical registers (`x1`..`x15`; `x0` is the wired
//! zero and is never allocated). A scoped, lowest-free-first pool: entering a
//! scope (a `frame`, or later a `proc`) and leaving it frees everything bound
//! inside it, so a nested scope can never clobber an outer live value. There is
//! no spill target, so exhaustion is a hard error (spec D5, §7).

use std::collections::HashMap;
use tamal_abi::isa::Reg;
use tamal_asm::{Diagnostic, Span};

#[cfg(test)]
mod tests {
    use super::*;

    fn r(n: u8) -> Reg {
        Reg::new(n).unwrap()
    }

    #[test]
    fn binds_lowest_free_first_and_reuses_after_free() {
        let mut a = RegAlloc::new();
        assert_eq!(a.bind("first".into(), &(0..0)).unwrap(), r(1));
        assert_eq!(a.bind("second".into(), &(0..0)).unwrap(), r(2));
        a.free(r(1));
        // x1 is free again, so the next temp takes it.
        assert_eq!(a.temp(&(0..0)).unwrap(), r(1));
    }

    #[test]
    fn lookup_finds_bound_names() {
        let mut a = RegAlloc::new();
        let reg = a.bind("data".into(), &(0..0)).unwrap();
        assert_eq!(a.lookup("data"), Some(reg));
        assert_eq!(a.lookup("nope"), None);
    }

    #[test]
    fn exit_scope_frees_regs_and_names() {
        let mut a = RegAlloc::new();
        a.enter_scope();
        a.bind("inner".into(), &(0..0)).unwrap(); // x1
        a.bind("inner2".into(), &(0..0)).unwrap(); // x2
        a.exit_scope();
        assert_eq!(a.lookup("inner"), None);
        // both x1 and x2 are free again
        assert_eq!(a.temp(&(0..0)).unwrap(), r(1));
        assert_eq!(a.temp(&(0..0)).unwrap(), r(2));
    }

    #[test]
    fn never_allocates_x0_and_caps_at_15() {
        let mut a = RegAlloc::new();
        let mut regs = Vec::new();
        for _ in 0..15 {
            regs.push(a.temp(&(0..0)).unwrap());
        }
        // x1..x15, never x0
        assert_eq!(
            regs.iter().map(|r| r.bits()).collect::<Vec<_>>(),
            (1..=15).collect::<Vec<u8>>()
        );
        // the 16th allocation has no register left: a hard error, no spill.
        let err = a.temp(&(0..0)).unwrap_err();
        assert!(err.message.contains("out of registers"));
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail (do not compile yet)**

Run: `cargo test -p tamal-lang --lib regalloc`
Expected: **compile error** — `cannot find type RegAlloc` (the impl is Step 4). This is the RED state.

- [ ] **Step 4: Write the minimal implementation**

In `crates/tamal-lang/src/regalloc.rs`, insert the implementation **between** the `use` lines and the `#[cfg(test)] mod tests` block:

```rust
/// The number of physical registers (`x0`..`x15`); `x0` is reserved (zero).
const NUM_REGS: usize = 16;

/// One open lexical scope: the registers allocated in it (freed on exit) and
/// the names bound in it (dropped from the environment on exit).
#[derive(Default)]
struct Scope {
    regs: Vec<Reg>,
    names: Vec<String>,
}

/// A scoped, lowest-free-first register allocator over `x1`..`x15`.
///
/// `x0` is never allocated. `exit_scope` frees everything allocated since the
/// matching `enter_scope`. Exhaustion is a hard error — there is no spill.
pub struct RegAlloc {
    /// `busy[n]` is true when physical register `xn` is in use.
    busy: [bool; NUM_REGS],
    /// The scope stack; index 0 is the root (test) scope.
    scopes: Vec<Scope>,
    /// Every live name binding → its physical register.
    env: HashMap<String, Reg>,
}

impl RegAlloc {
    /// A fresh allocator with a single root scope and nothing allocated.
    pub fn new() -> Self {
        RegAlloc {
            busy: [false; NUM_REGS],
            scopes: vec![Scope::default()],
            env: HashMap::new(),
        }
    }

    /// Open a nested scope (a `frame`); its allocations are released by the
    /// matching [`RegAlloc::exit_scope`].
    pub fn enter_scope(&mut self) {
        self.scopes.push(Scope::default());
    }

    /// Close the innermost scope, freeing its registers and dropping its names.
    pub fn exit_scope(&mut self) {
        if let Some(scope) = self.scopes.pop() {
            for reg in scope.regs {
                self.busy[reg.bits() as usize] = false;
            }
            for name in scope.names {
                self.env.remove(&name);
            }
        }
    }

    /// Allocate the lowest free physical register, recording it in the current
    /// scope. Errors (no spill) when `x1`..`x15` are all in use.
    fn alloc(&mut self, span: &Span) -> Result<Reg, Diagnostic> {
        for n in 1..NUM_REGS {
            if !self.busy[n] {
                self.busy[n] = true;
                let reg = Reg::new(n as u8).expect("n < 16 fits a 5-bit register");
                self.scopes
                    .last_mut()
                    .expect("there is always a root scope")
                    .regs
                    .push(reg);
                return Ok(reg);
            }
        }
        Err(Diagnostic::error(
            span.clone(),
            "out of registers (the tamal ISA has only x1..x15 and no spill)",
        )
        .with_help("free a value by ending its scope, or use fewer live variables"))
    }

    /// Allocate an anonymous scratch register (freed by [`RegAlloc::free`] or at
    /// scope exit).
    pub fn temp(&mut self, span: &Span) -> Result<Reg, Diagnostic> {
        self.alloc(span)
    }

    /// Allocate a register and bind `name` to it for the current scope.
    pub fn bind(&mut self, name: String, span: &Span) -> Result<Reg, Diagnostic> {
        let reg = self.alloc(span)?;
        self.env.insert(name.clone(), reg);
        self.scopes
            .last_mut()
            .expect("there is always a root scope")
            .names
            .push(name);
        Ok(reg)
    }

    /// The register a name is currently bound to, if any.
    pub fn lookup(&self, name: &str) -> Option<Reg> {
        self.env.get(name).copied()
    }

    /// Release a register early (before its scope ends). Idempotent: freeing a
    /// register twice, or one already released by `exit_scope`, is harmless.
    pub fn free(&mut self, reg: Reg) {
        self.busy[reg.bits() as usize] = false;
        if let Some(scope) = self.scopes.last_mut() {
            scope.regs.retain(|&r| r != reg);
        }
    }
}

impl Default for RegAlloc {
    fn default() -> Self {
        Self::new()
    }
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p tamal-lang --lib regalloc`
Expected: PASS — `4 passed`.

- [ ] **Step 6: Commit**

```bash
git add crates/tamal-lang/src/regalloc.rs crates/tamal-lang/src/lib.rs
git commit -m "feat(tamal-lang): scoped register allocator (x1..x15, no spill)"
```

---

## Task 2: Refactor `emit` to an `Emitter` struct (behavior-preserving)

Moving the flat per-statement match into a struct so later tasks can thread the allocator, gensym counter, and trailers. **No behavior change** — all existing tests stay green.

**Files:**
- Modify: `crates/tamal-lang/src/emit.rs`

- [ ] **Step 1: Replace the `emit` function body with an `Emitter`**

In `crates/tamal-lang/src/emit.rs`, replace the whole `pub fn emit(...) { ... }` function (from `pub fn emit` down to its closing `}`, and the free `fn push(...)` helper below it) with:

```rust
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
    }
    Ok(e.finish())
}

/// The lowering state: the growing asm text + source map.
struct Emitter<'a> {
    consts: &'a Consts,
    asm: String,
    lines: Vec<(Span, Span)>,
}

impl<'a> Emitter<'a> {
    fn new(consts: &'a Consts) -> Self {
        Emitter {
            consts,
            asm: String::new(),
            lines: Vec::new(),
        }
    }

    fn finish(self) -> Lowering {
        Lowering {
            asm: self.asm,
            lines: self.lines,
        }
    }

    /// Append one asm line and record its `(asm byte range, .tam span)` mapping.
    fn push(&mut self, text: &str, span: &Span) {
        let start = self.asm.len();
        self.asm.push_str(text);
        self.lines.push((start..self.asm.len(), span.clone()));
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
}
```

- [ ] **Step 2: Run the existing tests to verify NO behavior change**

Run: `cargo test -p tamal-lang`
Expected: PASS — the same count as before this task (unit + `examples` + `values`), `0 failed`. The `emit` unit tests (`emits_entry_and_pass`, `emits_fail_code_verbatim`, `emits_raw_instructions`, `remap_points_asm_span_at_originating_tam_span`) must all still pass.

- [ ] **Step 3: Commit**

```bash
git add crates/tamal-lang/src/emit.rs
git commit -m "refactor(tamal-lang): move emit into an Emitter struct (no behavior change)"
```

---

## Task 3: `config` statement → `set_config`

**Files:**
- Modify: `crates/tamal-lang/src/parser.rs`
- Modify: `crates/tamal-lang/src/emit.rs`
- Modify: `crates/tamal-lang/src/lib.rs` (M2 `halts` match)

- [ ] **Step 1: Write the failing tests**

Add to the parser test module in `crates/tamal-lang/src/parser.rs` (inside `mod tests`, after `parses_crc_region`):

```rust
    #[test]
    fn parses_config() {
        let m = parse_ok("test t {\n config controller, x1, sck20, alert_pin\n pass\n}\n");
        match &m.tests[0].stmts[0] {
            Stmt::Config {
                role, io, sck, alert, ..
            } => {
                assert_eq!(role, "controller");
                assert_eq!(io, "x1");
                assert_eq!(sck, "sck20");
                assert_eq!(alert, "alert_pin");
            }
            s => panic!("expected Config, got {s:?}"),
        }
    }
```

Add to `crates/tamal-lang/src/lib.rs` test module (after `send_plus_crc8_appends_folded_byte`):

```rust
    #[test]
    fn config_lowers_to_set_config() {
        let asm = lower_to_asm(
            "test t {\n config controller, x1, sck20, alert_pin\n pass\n}\n",
        )
        .unwrap();
        assert!(
            asm.contains("\tset_config controller, x1, sck20, alert_pin\n"),
            "got:\n{asm}"
        );
    }

    #[test]
    fn config_assembles_to_the_v1_config_word() {
        // controller,x1,sck20,alert_pin packs to 0x00 -> SET_CONFIG word 0x5800_0000
        let prog =
            compile("test t {\n config controller, x1, sck20, alert_pin\n pass\n}\n").unwrap();
        let words: Vec<u32> = prog.words().collect();
        assert_eq!(words[0], 0x5800_0000);
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p tamal-lang config`
Expected: FAIL — `no variant Config` / the asserts fail (RED). (The crate may fail to compile until Steps 3–5 are all in; that is expected mid-task.)

- [ ] **Step 3: Add the `Stmt::Config` variant**

In `crates/tamal-lang/src/parser.rs`, add to the `Stmt` enum (after `CrcRegion`):

```rust
    /// `config role, io, sck, alert` → `set_config …` (keywords pass through
    /// verbatim; the assembler validates them and the v1 restriction).
    Config {
        role: String,
        io: String,
        sck: String,
        alert: String,
        span: Span,
    },
```

- [ ] **Step 4: Parse it**

In `parse_stmt`, add a new arm to the `match word.as_str()` (before the final `_ =>` raw arm):

```rust
            "config" => {
                let role = self.expect_ident()?;
                self.expect(Tok::Comma, "`,`")?;
                let io = self.expect_ident()?;
                self.expect(Tok::Comma, "`,`")?;
                let sck = self.expect_ident()?;
                self.expect(Tok::Comma, "`,`")?;
                let alert = self.expect_ident()?;
                let span = head.start..alert.end;
                self.end_stmt()?;
                Ok(Stmt::Config {
                    role: self.lexeme(&role).to_string(),
                    io: self.lexeme(&io).to_string(),
                    sck: self.lexeme(&sck).to_string(),
                    alert: self.lexeme(&alert).to_string(),
                    span,
                })
            }
```

- [ ] **Step 5: Lower it + keep matches exhaustive**

In `crates/tamal-lang/src/emit.rs`, add a `Stmt::Config` arm to `top_stmt` (after the `Stmt::CrcRegion` arm):

```rust
            Stmt::Config { .. } => self.lower_config(stmt)?,
```

and add the method to `impl Emitter` (after `lower_crc_region`):

```rust
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
```

In `crates/tamal-lang/src/lib.rs`, extend the M2 `halts` match (add before the closing `}` of the match, alongside the other non-terminators):

```rust
        parser::Stmt::Config { .. } => false,
```

- [ ] **Step 6: Run to verify they pass**

Run: `cargo test -p tamal-lang config`
Expected: PASS — `3 passed` (`parses_config`, `config_lowers_to_set_config`, `config_assembles_to_the_v1_config_word`).

- [ ] **Step 7: Commit**

```bash
git add crates/tamal-lang/src/parser.rs crates/tamal-lang/src/emit.rs crates/tamal-lang/src/lib.rs
git commit -m "feat(tamal-lang): config statement lowering to set_config"
```

---

## Task 4: `frame { }` — CS scope (RAII deassert)

The plain frame: `cs_assert` · body · `cs_deassert`. The deferred-verdict machinery arrives in Task 7 (with `expect`); the register scope integrates in Task 5 (with `recv`). Here the body may contain `send`/`crc_region`/`raw`.

**Files:**
- Modify: `crates/tamal-lang/src/parser.rs`
- Modify: `crates/tamal-lang/src/emit.rs`
- Modify: `crates/tamal-lang/src/lib.rs` (M2 `halts` match)

- [ ] **Step 1: Write the failing tests**

Add to `crates/tamal-lang/src/parser.rs` `mod tests`:

```rust
    #[test]
    fn parses_frame_with_body() {
        let m = parse_ok("test t {\n frame {\n  send [0x44]\n  tar 2\n }\n pass\n}\n");
        match &m.tests[0].stmts[0] {
            Stmt::Frame { body, .. } => assert_eq!(body.len(), 2),
            s => panic!("expected Frame, got {s:?}"),
        }
    }
```

Add to `crates/tamal-lang/src/lib.rs` `mod tests`:

```rust
    #[test]
    fn frame_wraps_body_in_cs_assert_deassert() {
        let asm = lower_to_asm("test t {\n frame {\n  tar 2\n }\n pass\n}\n").unwrap();
        assert_eq!(
            asm,
            ".globl _start\n_start:\n\tcs_assert\n\ttar 2\n\tcs_deassert\n\thalt 0x00\n"
        );
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p tamal-lang frame`
Expected: FAIL (RED) — `no variant Frame` / asm mismatch.

- [ ] **Step 3: Add the `Stmt::Frame` variant**

In `crates/tamal-lang/src/parser.rs`, add to `Stmt` (after `Config`):

```rust
    /// `frame { … }` — a CS scope: `cs_assert` before the body, `cs_deassert`
    /// on every exit (D9). The body reuses the ordinary statement grammar; the
    /// emitter enforces which statements are legal inside a frame.
    Frame { body: Vec<Stmt>, span: Span },
```

- [ ] **Step 4: Parse it**

Add to the `match word.as_str()` in `parse_stmt` (before the raw `_ =>`):

```rust
            "frame" => {
                self.expect(Tok::LBrace, "`{`")?;
                let mut body = Vec::new();
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
                                "unexpected end of file: missing `}` for `frame`",
                            )]);
                        }
                        _ => body.push(self.parse_stmt()?),
                    }
                };
                self.end_stmt()?;
                Ok(Stmt::Frame {
                    body,
                    span: head.start..end,
                })
            }
```

- [ ] **Step 5: Lower it + keep matches exhaustive**

In `crates/tamal-lang/src/emit.rs`, add to `top_stmt` (after the `Stmt::Config` arm):

```rust
            Stmt::Frame { body, span } => self.lower_frame(body, span)?,
```

Add these methods to `impl Emitter` (after `lower_config`). `frame_body_stmt` is the whitelist of statements legal inside a frame; it will grow in Tasks 5–7:

```rust
    fn lower_frame(&mut self, body: &[Stmt], span: &Span) -> Result<(), Vec<Diagnostic>> {
        self.push("\tcs_assert\n", span);
        for stmt in body {
            self.frame_body_stmt(stmt)?;
        }
        self.push("\tcs_deassert\n", span);
        Ok(())
    }

    fn frame_body_stmt(&mut self, stmt: &Stmt) -> Result<(), Vec<Diagnostic>> {
        match stmt {
            Stmt::Send { .. } => self.lower_send(stmt),
            Stmt::CrcRegion { .. } => self.lower_crc_region(stmt),
            Stmt::Raw { .. } => self.lower_raw(stmt),
            other => Err(vec![Diagnostic::error(
                stmt_span(other),
                "this statement is not allowed inside a `frame`",
            )]),
        }
    }
```

Add a small span helper at the bottom of `emit.rs` (after the `push`-related code, outside `impl Emitter`):

```rust
/// The best source span for a statement, for diagnostics.
fn stmt_span(stmt: &Stmt) -> Span {
    match stmt {
        Stmt::Pass => 0..0,
        Stmt::Fail { span, .. }
        | Stmt::Raw { span, .. }
        | Stmt::Send { span, .. }
        | Stmt::CrcRegion { span, .. }
        | Stmt::Config { span, .. }
        | Stmt::Frame { span, .. } => span.clone(),
    }
}
```

In `crates/tamal-lang/src/lib.rs`, add to the M2 `halts` match:

```rust
        parser::Stmt::Frame { .. } => false,
```

- [ ] **Step 6: Run to verify they pass**

Run: `cargo test -p tamal-lang frame`
Expected: PASS — `2 passed`.

- [ ] **Step 7: Commit**

```bash
git add crates/tamal-lang/src/parser.rs crates/tamal-lang/src/emit.rs crates/tamal-lang/src/lib.rs
git commit -m "feat(tamal-lang): frame { } CS scope (cs_assert/cs_deassert)"
```

---

## Task 5: `recv` — `get_byte` into allocated registers

Introduces the allocator into the `Emitter`, plus the frame register scope.

**Files:**
- Modify: `crates/tamal-lang/src/parser.rs`
- Modify: `crates/tamal-lang/src/emit.rs`
- Modify: `crates/tamal-lang/src/lib.rs` (M2 `halts` match)

- [ ] **Step 1: Write the failing tests**

Add to `crates/tamal-lang/src/parser.rs` `mod tests`:

```rust
    #[test]
    fn parses_recv_names_and_discard() {
        let m = parse_ok("test t {\n recv data, _, status\n pass\n}\n");
        match &m.tests[0].stmts[0] {
            Stmt::Recv { targets, .. } => {
                assert!(matches!(targets[0], RecvTarget::Name(ref n) if n == "data"));
                assert!(matches!(targets[1], RecvTarget::Discard));
                assert!(matches!(targets[2], RecvTarget::Name(ref n) if n == "status"));
            }
            s => panic!("expected Recv, got {s:?}"),
        }
    }

    #[test]
    fn parses_recv_count() {
        let m = parse_ok("test t {\n recv 3\n pass\n}\n");
        match &m.tests[0].stmts[0] {
            Stmt::Recv { targets, .. } => {
                assert_eq!(targets.len(), 3);
                assert!(targets.iter().all(|t| matches!(t, RecvTarget::Discard)));
            }
            s => panic!("expected Recv, got {s:?}"),
        }
    }
```

Add to `crates/tamal-lang/src/lib.rs` `mod tests`:

```rust
    #[test]
    fn recv_names_allocate_ascending_registers() {
        let asm = lower_to_asm("test t {\n recv a, b, c\n pass\n}\n").unwrap();
        assert!(asm.contains("\tget_byte x1\n"), "got:\n{asm}");
        assert!(asm.contains("\tget_byte x2\n"), "got:\n{asm}");
        assert!(asm.contains("\tget_byte x3\n"), "got:\n{asm}");
    }

    #[test]
    fn recv_discards_reuse_the_same_scratch_register() {
        // `_` is freed immediately, so three discards all reuse x1.
        let asm = lower_to_asm("test t {\n recv _, _, _\n pass\n}\n").unwrap();
        assert_eq!(
            asm,
            ".globl _start\n_start:\n\tget_byte x1\n\tget_byte x1\n\tget_byte x1\n\thalt 0x00\n"
        );
    }

    #[test]
    fn frame_scope_frees_recv_registers_on_exit() {
        // recv inside a frame binds x1,x2; after the frame a second recv reuses
        // x1,x2 (the frame scope released them).
        let asm =
            lower_to_asm("test t {\n frame {\n  recv a, b\n }\n recv c, d\n pass\n}\n").unwrap();
        let gets: Vec<&str> = asm.lines().filter(|l| l.contains("get_byte")).collect();
        assert_eq!(
            gets,
            vec!["\tget_byte x1", "\tget_byte x2", "\tget_byte x1", "\tget_byte x2"]
        );
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p tamal-lang recv`
Expected: FAIL (RED) — `no variant Recv` / `RecvTarget` unknown.

- [ ] **Step 3: Add the `Stmt::Recv` variant + `RecvTarget`**

In `crates/tamal-lang/src/parser.rs`, add to `Stmt` (after `Frame`):

```rust
    /// `recv a, b, _` (names / discards) or `recv N` (N discards) → one
    /// `get_byte` per target into an allocated register.
    Recv { targets: Vec<RecvTarget>, span: Span },
```

and add this enum after the `Stmt` enum:

```rust
/// One destination of a `recv`: a named binding or a `_` discard.
#[derive(Debug, Clone)]
pub enum RecvTarget {
    /// Bind the received byte to a named register for the enclosing scope.
    Name(String),
    /// Read and discard (a scratch register, freed immediately).
    Discard,
}
```

- [ ] **Step 4: Parse it**

Add to the `match word.as_str()` in `parse_stmt` (before the raw `_ =>`):

```rust
            "recv" => {
                let mut targets = Vec::new();
                let end;
                if self.peek() == Tok::Number {
                    // `recv N` — N discards.
                    let n = self.expect(Tok::Number, "a byte count")?;
                    let count = parse_number(self.lexeme(&n.span)).filter(|&c| c >= 0).ok_or_else(
                        || vec![Diagnostic::error(n.span.clone(), "invalid recv count")],
                    )?;
                    for _ in 0..count {
                        targets.push(RecvTarget::Discard);
                    }
                    end = n.span.end;
                } else {
                    // `recv a, _, b` — a name/discard list.
                    let mut last;
                    loop {
                        let sp = self.expect_ident()?;
                        last = sp.end;
                        let name = self.lexeme(&sp).to_string();
                        targets.push(if name == "_" {
                            RecvTarget::Discard
                        } else {
                            RecvTarget::Name(name)
                        });
                        if self.peek() == Tok::Comma {
                            self.i += 1;
                            self.skip_newlines();
                            continue;
                        }
                        break;
                    }
                    end = last;
                }
                self.end_stmt()?;
                Ok(Stmt::Recv {
                    targets,
                    span: head.start..end,
                })
            }
```

- [ ] **Step 5: Give the `Emitter` an allocator + lower `recv`**

In `crates/tamal-lang/src/emit.rs`, add the imports at the top (after the existing `use` lines):

```rust
use crate::regalloc::RegAlloc;
use tamal_abi::isa::Reg;
```

Add the field to `struct Emitter`:

```rust
    alloc: RegAlloc,
```

Initialize it in `Emitter::new`:

```rust
            alloc: RegAlloc::new(),
```

Add a register-name helper (free function at the bottom of `emit.rs`):

```rust
/// Render a physical register as its `xN` asm operand.
fn reg_name(reg: Reg) -> String {
    format!("x{}", reg.bits())
}
```

Wrap the frame body in a register scope — in `lower_frame`, change the body loop to bracket it with scope enter/exit:

```rust
    fn lower_frame(&mut self, body: &[Stmt], span: &Span) -> Result<(), Vec<Diagnostic>> {
        self.push("\tcs_assert\n", span);
        self.alloc.enter_scope();
        for stmt in body {
            self.frame_body_stmt(stmt)?;
        }
        self.push("\tcs_deassert\n", span);
        self.alloc.exit_scope();
        Ok(())
    }
```

Add the `Recv` arm to `top_stmt` (after the `Stmt::Frame` arm):

```rust
            Stmt::Recv { targets, span } => self.lower_recv(targets, span)?,
```

Add the `Recv` arm to `frame_body_stmt` (alongside `Send`/`CrcRegion`/`Raw`):

```rust
            Stmt::Recv { targets, span } => self.lower_recv(targets, span),
```

Add the method to `impl Emitter`:

```rust
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
```

Extend `stmt_span` with the `Recv` arm (add `| Stmt::Recv { span, .. }` to the span-bearing group).

In `crates/tamal-lang/src/lib.rs`, add to the M2 `halts` match:

```rust
        parser::Stmt::Recv { .. } => false,
```

- [ ] **Step 6: Run to verify they pass**

Run: `cargo test -p tamal-lang recv`
Expected: PASS — `5 passed` (2 parser + 3 lib).

- [ ] **Step 7: Commit**

```bash
git add crates/tamal-lang/src/parser.rs crates/tamal-lang/src/emit.rs crates/tamal-lang/src/lib.rs
git commit -m "feat(tamal-lang): recv into allocated registers; frame register scope"
```

---

## Task 6: `wait_state [name]` — the WAIT_STATE poll

Introduces the gensym label counter.

**Files:**
- Modify: `crates/tamal-lang/src/parser.rs`
- Modify: `crates/tamal-lang/src/emit.rs`
- Modify: `crates/tamal-lang/src/lib.rs` (M2 `halts` match)

- [ ] **Step 1: Write the failing tests**

Add to `crates/tamal-lang/src/parser.rs` `mod tests`:

```rust
    #[test]
    fn parses_wait_state_bare_and_named() {
        let m = parse_ok("test t {\n wait_state\n wait_state term\n pass\n}\n");
        assert!(matches!(m.tests[0].stmts[0], Stmt::WaitState { bind: None, .. }));
        match &m.tests[0].stmts[1] {
            Stmt::WaitState { bind: Some(n), .. } => assert_eq!(n, "term"),
            s => panic!("expected named WaitState, got {s:?}"),
        }
    }
```

Add to `crates/tamal-lang/src/lib.rs` `mod tests`:

```rust
    #[test]
    fn wait_state_lowers_to_the_poll_idiom() {
        let asm = lower_to_asm("test t {\n wait_state\n pass\n}\n").unwrap();
        assert_eq!(
            asm,
            ".globl _start\n_start:\n\
             __wait0:\n\tcrc_reset\n\tget_byte x1\n\tli x2, 0x0F\n\tbeq x1, x2, __wait0\n\
             \thalt 0x00\n"
        );
    }

    #[test]
    fn named_wait_state_keeps_the_terminal_byte_bound() {
        // `wait_state term` keeps the response register live, so a following
        // recv allocates the NEXT register (x2), not x1.
        let asm = lower_to_asm("test t {\n wait_state term\n recv d\n pass\n}\n").unwrap();
        // poll uses x1 (resp, kept) and x2 (constant, freed); recv d -> x2 reused.
        let gets: Vec<&str> = asm.lines().filter(|l| l.contains("get_byte")).collect();
        assert_eq!(gets, vec!["\tget_byte x1", "\tget_byte x2"]);
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p tamal-lang wait_state`
Expected: FAIL (RED) — `no variant WaitState`.

- [ ] **Step 3: Add the `Stmt::WaitState` variant**

In `crates/tamal-lang/src/parser.rs`, add to `Stmt` (after `Recv`):

```rust
    /// `wait_state [name]` — poll past WAIT_STATE; consumes the response-code
    /// byte (D12). `name` binds the terminal (non-WAIT_STATE) byte.
    WaitState { bind: Option<String>, span: Span },
```

- [ ] **Step 4: Parse it**

Add to the `match word.as_str()` in `parse_stmt` (before the raw `_ =>`):

```rust
            "wait_state" => {
                let mut end = head.end;
                let mut bind = None;
                if self.peek() == Tok::Ident {
                    let sp = self.span();
                    self.i += 1;
                    bind = Some(self.lexeme(&sp).to_string());
                    end = sp.end;
                }
                self.end_stmt()?;
                Ok(Stmt::WaitState {
                    bind,
                    span: head.start..end,
                })
            }
```

- [ ] **Step 5: Give the `Emitter` a gensym counter + lower `wait_state`**

In `crates/tamal-lang/src/emit.rs`, add the field to `struct Emitter`:

```rust
    gensym: u32,
```

Initialize it in `Emitter::new`:

```rust
            gensym: 0,
```

Add a gensym method + the WAIT_STATE constant to `impl Emitter` and above it:

```rust
    /// A fresh, asm-safe label: `__<prefix><n>` (no leading dot — dots are
    /// directives in tamal-asm).
    fn gensym(&mut self, prefix: &str) -> String {
        let label = format!("__{prefix}{}", self.gensym);
        self.gensym += 1;
        label
    }
```

Add the constant near the top of `emit.rs` (after the `use` lines):

```rust
/// The eSPI WAIT_STATE response code the `wait_state` poll spins on.
const WAIT_STATE_CODE: u8 = 0x0F;
```

Add the `WaitState` arm to `top_stmt` (after `Recv`):

```rust
            Stmt::WaitState { bind, span } => self.lower_wait_state(bind, span)?,
```

Add the `WaitState` arm to `frame_body_stmt`:

```rust
            Stmt::WaitState { bind, span } => self.lower_wait_state(bind, span),
```

Add the method to `impl Emitter`:

```rust
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
        self.push(&format!("\tli {}, 0x{WAIT_STATE_CODE:02X}\n", reg_name(k)), span);
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
```

Extend `stmt_span` with the `WaitState` arm (add `| Stmt::WaitState { span, .. }` to the span-bearing group).

In `crates/tamal-lang/src/lib.rs`, add to the M2 `halts` match:

```rust
        parser::Stmt::WaitState { .. } => false,
```

- [ ] **Step 6: Run to verify they pass**

Run: `cargo test -p tamal-lang wait_state`
Expected: PASS — `3 passed`.

- [ ] **Step 7: Commit**

```bash
git add crates/tamal-lang/src/parser.rs crates/tamal-lang/src/emit.rs crates/tamal-lang/src/lib.rs
git commit -m "feat(tamal-lang): wait_state poll idiom (crc_reset/get_byte/li/beq)"
```

---

## Task 7: `expect crc else <byte>` + frame deferred verdicts

The D9 payoff: the residue is latched inside the frame, but the verdict branch runs **after** `cs_deassert`, and the fail-`halt` is hoisted to a per-test trailer.

**Files:**
- Modify: `crates/tamal-lang/src/parser.rs`
- Modify: `crates/tamal-lang/src/emit.rs`
- Modify: `crates/tamal-lang/src/lib.rs` (M2 `halts` match)

- [ ] **Step 1: Write the failing tests**

Add to `crates/tamal-lang/src/parser.rs` `mod tests`:

```rust
    #[test]
    fn parses_expect_crc_else() {
        let m = parse_ok("test t {\n frame {\n  expect crc else 0x11\n }\n pass\n}\n");
        match &m.tests[0].stmts[0] {
            Stmt::Frame { body, .. } => {
                assert!(matches!(body[0], Stmt::Expect { .. }));
            }
            s => panic!("expected Frame, got {s:?}"),
        }
    }
```

Add to `crates/tamal-lang/src/lib.rs` `mod tests`:

```rust
    #[test]
    fn expect_crc_defers_the_verdict_past_cs_deassert() {
        let asm =
            lower_to_asm("test t {\n frame {\n  expect crc else 0x11\n }\n pass\n}\n").unwrap();
        assert_eq!(
            asm,
            ".globl _start\n_start:\n\
             \tcs_assert\n\
             \tget_byte x1\n\trdsr x1, crc\n\
             \tcs_deassert\n\
             \tbnez x1, __fail0\n\
             \thalt 0x00\n\
             __fail0:\n\thalt 0x11\n"
        );
    }

    #[test]
    fn expect_crc_outside_a_frame_is_an_error() {
        let err = lower_to_asm("test t {\n expect crc else 0x11\n pass\n}\n").unwrap_err();
        assert!(
            err[0].message.contains("inside a `frame`"),
            "got: {:?}",
            err[0].message
        );
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p tamal-lang expect`
Expected: FAIL (RED) — `no variant Expect`.

- [ ] **Step 3: Add the `Stmt::Expect` variant**

In `crates/tamal-lang/src/parser.rs`, add to `Stmt` (after `WaitState`):

```rust
    /// `expect crc else <byte>` — consume the trailing CRC byte, latch the RX
    /// residue, and (via the enclosing `frame`) branch to a `fail <byte>` after
    /// CS deasserts (D9/D12). Only legal inside a `frame`.
    Expect { else_code: Expr, span: Span },
```

- [ ] **Step 4: Parse it**

Add to the `match word.as_str()` in `parse_stmt` (before the raw `_ =>`):

```rust
            "expect" => {
                let crc = self.expect_ident()?;
                if self.lexeme(&crc) != "crc" {
                    return Err(vec![Diagnostic::error(
                        crc,
                        "expected `crc` after `expect` (the only check in v1)",
                    )]);
                }
                let els = self.expect_ident()?;
                if self.lexeme(&els) != "else" {
                    return Err(vec![Diagnostic::error(
                        els,
                        "expected `else <byte>` after `expect crc`",
                    )]);
                }
                let else_code = self.parse_expr()?;
                let span = head.start..else_code.span().end;
                self.end_stmt()?;
                Ok(Stmt::Expect { else_code, span })
            }
```

- [ ] **Step 5: Add trailers + deferred verdicts to the `Emitter`**

In `crates/tamal-lang/src/emit.rs`, add a `Deferred` struct (after `struct Emitter`):

```rust
/// A verdict branch an `expect` deferred to its enclosing `frame`: after CS
/// deasserts, emit `bnez <reg>, <label>`, and hoist `<label>: halt <code>` to a
/// per-test trailer.
struct Deferred {
    reg: Reg,
    label: String,
    code: u8,
    span: Span,
}
```

Add the trailer field to `struct Emitter`:

```rust
    trailers: Vec<(Span, String)>,
```

Initialize it in `Emitter::new`:

```rust
            trailers: Vec::new(),
```

Flush trailers at the end of each test — in the free `emit` function, after the `for stmt in &test.stmts { … }` loop and before the test loop closes, add:

```rust
        e.flush_trailers();
```

so the test loop reads:

```rust
    for test in &module.tests {
        e.push(".globl _start\n", &test.name_span);
        e.push("_start:\n", &test.name_span);
        for stmt in &test.stmts {
            e.top_stmt(stmt, &test.name_span)?;
        }
        e.flush_trailers();
    }
```

Add the flush method to `impl Emitter`:

```rust
    fn flush_trailers(&mut self) {
        let trailers = std::mem::take(&mut self.trailers);
        for (span, block) in trailers {
            self.push(&block, &span);
        }
    }
```

Rework `lower_frame` to collect and emit deferred verdicts, and route body statements through a frame-aware helper:

```rust
    fn lower_frame(&mut self, body: &[Stmt], span: &Span) -> Result<(), Vec<Diagnostic>> {
        self.push("\tcs_assert\n", span);
        self.alloc.enter_scope();
        let mut deferred: Vec<Deferred> = Vec::new();
        for stmt in body {
            self.frame_body_stmt(stmt, &mut deferred)?;
        }
        self.push("\tcs_deassert\n", span);
        for d in &deferred {
            self.push(&format!("\tbnez {}, {}\n", reg_name(d.reg), d.label), &d.span);
            self.trailers.push((
                d.span.clone(),
                format!("{}:\n\thalt 0x{:02X}\n", d.label, d.code),
            ));
        }
        self.alloc.exit_scope();
        Ok(())
    }
```

Change `frame_body_stmt` to take the deferred list and handle `Expect`:

```rust
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
            other => Err(vec![Diagnostic::error(
                stmt_span(other),
                "this statement is not allowed inside a `frame`",
            )]),
        }
    }
```

Add the top-level `Expect` arm to `top_stmt` (after `WaitState`) — outside a frame it is an error:

```rust
            Stmt::Expect { span, .. } => {
                return Err(vec![Diagnostic::error(
                    span.clone(),
                    "`expect crc` must appear inside a `frame`",
                )
                .with_help("wrap the response phase in `frame { … }`")]);
            }
```

Extend `stmt_span` with the `Expect` arm (add `| Stmt::Expect { span, .. }` to the span-bearing group).

In `crates/tamal-lang/src/lib.rs`, add to the M2 `halts` match:

```rust
        parser::Stmt::Expect { .. } => false,
```

- [ ] **Step 6: Run to verify they pass**

Run: `cargo test -p tamal-lang expect`
Expected: PASS — `3 passed`.

- [ ] **Step 7: Run the whole crate to catch match/borrow regressions**

Run: `cargo test -p tamal-lang`
Expected: PASS — `0 failed`.

- [ ] **Step 8: Commit**

```bash
git add crates/tamal-lang/src/parser.rs crates/tamal-lang/src/emit.rs crates/tamal-lang/src/lib.rs
git commit -m "feat(tamal-lang): expect crc else <byte> + frame deferred verdicts"
```

---

## Task 8: Capstone — `peripheral_io_read.tam` byte-matches the `.s` (modulo registers)

Proves the whole stack composes: an HLL peripheral I/O read lowers to bytecode byte-identical **modulo register allocation** to the hand-written `examples/peripheral_io_read.s`.

**Files:**
- Create: `crates/tamal-lang/tests/frames.rs`

- [ ] **Step 1: Write the failing tests**

Create `crates/tamal-lang/tests/frames.rs`:

```rust
//! End-to-end: a full eSPI Peripheral channel test written in tamal-lang lowers
//! to bytecode byte-identical *modulo register allocation* to the hand-written
//! `examples/peripheral_io_read.s`, and its `+ crc8` re-derives the 0x16 TX CRC.

use tamal_abi::isa::{Instr, Reg};

/// The HLL peripheral I/O read. Plan 3 has no `import espi` yet (that is Plan 5),
/// so the command opcode is a local `const`; every other line is the domain
/// sugar this plan adds.
const PERIPHERAL_TAM: &str = "\
const PUT_IORD1 = 0x44

test io_read {
    config controller, x1, sck20, alert_pin
    frame {
        send [PUT_IORD1, 0x00, 0x64] + crc8
        tar 2
        wait_state
        recv data, status0, status1
        expect crc else 0x11
    }
    pass
}
";

/// Decode a program and zero every register operand, so two programs compare
/// equal when they differ only in which physical registers the allocator chose.
fn canon(prog: &tamal_asm::Program) -> Vec<Instr> {
    prog.words()
        .map(|w| zero_regs(Instr::decode(w).expect("re-decode assembled word")))
        .collect()
}

fn zero_regs(i: Instr) -> Instr {
    use Instr::*;
    let z = Reg::new(0).unwrap();
    match i {
        PutByteReg(_) => PutByteReg(z),
        GetByte(_) => GetByte(z),
        PutBitsReg(_, n) => PutBitsReg(z, n),
        GetBits(_, n) => GetBits(z, n),
        TarReg(_) => TarReg(z),
        GetAlert(_) => GetAlert(z),
        Beq(_, _, o) => Beq(z, z, o),
        Bne(_, _, o) => Bne(z, z, o),
        Bltu(_, _, o) => Bltu(z, z, o),
        Bgeu(_, _, o) => Bgeu(z, z, o),
        WaitOn(_, c, t) => WaitOn(z, c, t),
        Mark(tag, _) => Mark(tag, z),
        LoadImm(_, v) => LoadImm(z, v),
        Lui(_, v) => Lui(z, v),
        Mov(_, _) => Mov(z, z),
        Add(_, _, _) => Add(z, z, z),
        Addi(_, _, o) => Addi(z, z, o),
        Sub(_, _, _) => Sub(z, z, z),
        And(_, _, _) => And(z, z, z),
        Andi(_, _, o) => Andi(z, z, o),
        Or(_, _, _) => Or(z, z, z),
        Ori(_, _, o) => Ori(z, z, o),
        Xor(_, _, _) => Xor(z, z, z),
        Xori(_, _, o) => Xori(z, z, o),
        Shift(_, _, op, a) => Shift(z, z, op, a),
        Rdsr(_, sr) => Rdsr(z, sr),
        // register-free instructions are unchanged
        other => other,
    }
}

#[test]
fn peripheral_io_read_byte_matches_asm_modulo_registers() {
    let reference = include_str!("../../../examples/peripheral_io_read.s");
    let got = tamal_lang::compile(PERIPHERAL_TAM).expect("compile .tam");
    let want = tamal_asm::assemble(reference).expect("assemble reference .s");
    assert_eq!(
        canon(&got),
        canon(&want),
        "the HLL peripheral read must match the hand-written .s modulo registers"
    );
}

#[test]
fn peripheral_io_read_lowers_to_the_expected_asm() {
    // Pins the allocator's lowest-free-first numbering and the gensym labels.
    // NB: gensym is a single shared counter, so `wait_state` takes `__wait0`
    // (counter -> 1) and the following `expect crc` takes `__fail1` (not
    // `__fail0`).
    let asm = tamal_lang::lower_to_asm(PERIPHERAL_TAM).unwrap();
    let expected = "\
.globl _start
_start:
\tset_config controller, x1, sck20, alert_pin
\tcs_assert
\tput_byte 0x44
\tput_byte 0x00
\tput_byte 0x64
\tput_byte 0x16
\ttar 2
__wait0:
\tcrc_reset
\tget_byte x1
\tli x2, 0x0F
\tbeq x1, x2, __wait0
\tget_byte x1
\tget_byte x2
\tget_byte x3
\tget_byte x4
\trdsr x4, crc
\tcs_deassert
\tbnez x4, __fail1
\thalt 0x00
__fail1:
\thalt 0x11
";
    assert_eq!(asm, expected);
}
```

- [ ] **Step 2: Run to verify they fail (or pass)**

Run: `cargo test -p tamal-lang --test frames`
Expected: with Tasks 1–7 done, these SHOULD PASS immediately. If `peripheral_io_read_lowers_to_the_expected_asm` fails, the diff pinpoints the exact lowering mismatch — fix the responsible task's lowering, not the golden, unless the golden itself is wrong. If `peripheral_io_read_byte_matches_asm_modulo_registers` fails, compare `canon(&got)` vs `canon(&want)` instruction-by-instruction to find the structural divergence.

> This is the one task whose tests may pass on first run — that is expected for a composition/capstone test. The value is the regression guard and the spec-§9 proof.

- [ ] **Step 3: Confirm both tests pass**

Run: `cargo test -p tamal-lang --test frames`
Expected: PASS — `2 passed`. (The `+ crc8` fold produces `put_byte 0x16` in the golden asm above, re-derived from `crc8([0x44,0x00,0x64])`, never hand-typed — so if anyone edits the command bytes, the CRC follows automatically.)

- [ ] **Step 4: Commit**

```bash
git add crates/tamal-lang/tests/frames.rs
git commit -m "test(tamal-lang): peripheral_io_read.tam byte-matches asm modulo registers"
```

---

## Task 9: Docs + fmt/clippy/whole-workspace gate

**Files:**
- Modify: `crates/tamal-lang/README.md`

- [ ] **Step 1: Update the crate README**

In `crates/tamal-lang/README.md`, add a short section documenting the Plan-3 surface. Find the existing statement list (or the "what a `.tam` can express" area) and add:

```markdown
### Frames, verdicts & the register model (Plan 3)

- `config role, io, sck, alert` → `set_config` (keywords pass through; the
  assembler validates them and the v1 restriction).
- `frame { … }` — a CS scope: `cs_assert` before the body, `cs_deassert` on
  every exit, including a failing `expect` (the verdict branch runs *after* the
  deassert).
- `recv a, b, _` / `recv N` — one `get_byte` per target into a
  compiler-allocated register (`_` / count = discard).
- `wait_state [name]` — the WAIT_STATE poll (`crc_reset`/`get_byte`/`li`/`beq`);
  consumes the response-code byte, optionally binding the terminal byte.
- `expect crc else <byte>` — consume the trailing CRC byte, check the RX
  residue, and branch to `fail <byte>` after CS deasserts. Only inside a `frame`.

Named variables (`recv`/`wait_state` bindings) are allocated to `x1`..`x15`
(`x0` is zero, never allocated); there is no spill, so running out of registers
is a compile error. Registers are freed at the end of their `frame` scope.
```

- [ ] **Step 2: Run the formatter and linter**

Run: `cargo fmt -p tamal-lang -p tamal-lang-cli`
Then: `cargo fmt -p tamal-lang -p tamal-lang-cli --check`
Expected: clean (exit 0).

Run: `cargo clippy -p tamal-lang -p tamal-lang-cli --all-targets -- -D warnings`
Expected: clean — no warnings. (If a not-yet-used helper trips `dead_code`, it means a wiring step was missed in an earlier task; wire it rather than `#[allow]`-ing it.)

- [ ] **Step 3: Run the whole workspace**

Run: `cargo test`
Expected: PASS across all crates — `0 failed`. (Nothing outside `tamal-lang` changed, so `tamal-abi`/`tamal-asm`/`tamal-loader` counts are unchanged.)

- [ ] **Step 4: Commit**

```bash
git add crates/tamal-lang/README.md
git commit -m "docs(tamal-lang): document frames, verdicts & the register model"
```

---

## Self-Review (completed by the plan author)

**1. Spec coverage.**

| Spec item | Task |
|---|---|
| §4.2 `config controller, x1, sck20, alert_pin` | Task 3 |
| §4.2 / D9 `frame { }` deassert-on-every-exit | Task 4 (scope) + Task 7 (verdict ordering) |
| §5 / D5 named vars, ≤15 regs, no spill, scope-freed | Task 1 + Tasks 5–7 |
| §4.2 `recv data, status0, status1` (+ `_` / `recv N`) | Task 5 |
| §5 / D12 `wait_state` consumes response byte; optional bind | Task 6 |
| §5 / D12 `expect crc` consumes trailing CRC byte; residue verdict | Task 7 |
| §5 verdict branch = gensym'd labels | Tasks 6–7 |
| §9 peripheral byte-match modulo register allocation | Task 8 |
| §9 TX CRC re-derived (`0x16`), never literal | Task 8 (uses Plan-2 `+ crc8`) |
| §1.1 no `x16`..`x31`, no spill, no invented `call`/`ret` | Task 1 |
| open-Q #1 (hand-rolled poll vs `wait_on`) → hand-rolled | Task 6 |

Out-of-scope-by-design (stated in the header): imports/`espi` (Plan 5), `proc`/`fn`/control flow (Plan 4), `enum`, standalone `let`/`reg`, `--lint`/injection (Plan 6).

**2. Placeholder scan.** No `TBD`/`todo!()`/"implement later". Every code step is complete. The `unreachable!` arms in `lower_*` are real guards (the caller only dispatches the matching variant), not placeholders.

**3. Type consistency.** `RegAlloc::{new, enter_scope, exit_scope, temp, bind, lookup, free}` are defined in Task 1 and used with those exact names/signatures in Tasks 5–7. `Stmt::{Config, Frame, Recv, WaitState, Expect}` and `RecvTarget::{Name, Discard}` are defined in the task that first parses them and matched with identical shapes in `emit.rs` and the `lib.rs` `halts` check. `reg_name`, `gensym`, `flush_trailers`, `stmt_span`, `Deferred`, and `WAIT_STATE_CODE` are each introduced once and reused consistently. `Emitter` fields (`consts`, `asm`, `lines`, `alloc`, `gensym`, `trailers`) are added in the task that first needs them and initialized in `Emitter::new` in the same task.

**Ordering note for the implementer:** `stmt_span` (Task 4) starts covering `Pass..Frame`; Tasks 5/6/7 each extend its span-bearing match group by one arm (`Recv`, `WaitState`, `Expect`). The `lib.rs` `halts` match is likewise extended by one `=> false` arm per statement task. If a task's build fails with a non-exhaustive-match error, the missing arm is the one that task introduces.

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-07-26-tamal-lang-03-frames-and-verdicts.md`. Two execution options:

1. **Subagent-Driven (recommended)** — dispatch a fresh subagent per task, two-stage review (spec-compliance + code-quality) between tasks, fast iteration. Mirrors how Plans 1 & 2 were run.
2. **Inline Execution** — execute tasks in this session with checkpoints for review.

Which approach?
