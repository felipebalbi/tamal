# tamal-lang Plan 4a — Callables (`proc`/`fn`) & Compile-Time Unroll Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the two tamal-lang callables — **`fn`** (pure, compile-time, its call replaced by the value it returns) and **`proc`** (emits bus activity, **inlined** at every call site with fresh registers) — with positional + **named** arguments and defaults, plus **`repeat N { … }`** compile-time unroll, so that an HLL `oob_smbus_msg.tam` built from a local `command` proc lowers to bytecode **byte-identical modulo register allocation** to the hand-written `examples/oob_smbus_msg.s`.

**Architecture:** The tamal ISA has **no `call`/`ret`, no stack, no data memory** (spec §1.1), so neither callable is a runtime call and the compiler must never fabricate linkage the ISA lacks. `fn` is evaluated by the existing constant evaluator: a call binds arguments, evaluates the body expression in a **child environment** (module `const`s + that call's parameters — never the caller's locals), and yields a `Value`. `proc` is expanded by the emitter: a call binds arguments, swaps in the same kind of child environment, opens a **`RegAlloc` scope** (so the callee cannot clobber a live caller value — D5), and lowers the body statements in place; gensym'd labels come from the existing shared counter, so two expansions never collide. Recursion is rejected in both (there is no stack to recurse on). `repeat N` is a pure compile-time unroll — no counter, no branch — with a per-iteration register scope.

**Tech Stack:** Rust (edition 2024), building on Plans 1–3 (`crates/tamal-lang`, `crates/tamal-lang-cli`); `tamal-asm` (`assemble`, `Program`, `Diagnostic`, `Span`) and `tamal-abi` (`isa::{Instr, Reg}`, `crc8`).

**Spec:** `docs/superpowers/specs/2026-07-20-tamal-lang-design.md` — §1.1 (the ISA constraints that shape everything), §2 (scope), §4.3 (the library `proc`/`fn` this plan must be able to express), §5 (semantic model + the lowering table rows for `proc` call / `fn` call / `repeat N`), §7 (register safety, no invented linkage, determinism), and decisions **D2** (two callables, both inlined), **D5** (per-expansion register hygiene, ≤15 live, no spill).

**Scope note:** the handoff's "Plan 4" bundled two independent subsystems. This is **Plan 4a — callables + unroll**, which is exactly what Plan 5's bundled `espi` stdlib needs (`controller`, `command`, `iowr_hdr`, `repeat ndata { recv _ }`). **Plan 4b** will add the branching half of D4: comparison operators, `bool`, `if`/`else`, `while`, `do`/`while`, and standalone `let`/`reg` declarations.

**Deferred — NOT in scope here:**
- `if`/`else`/`while`/`do-while`, comparison operators, `bool`, `let`/`reg` declaration statements → **Plan 4b**.
- `pub`, `import`, module namespacing, qualified names (`espi.command`), the bundled `espi.tam` → **Plan 5**. This plan's capstone therefore defines its `command` proc **locally**, which is precisely the body Plan 5 will move into the stdlib.
- `enum` (the capstone uses a plain `const`/literal verdict byte), general arithmetic operators, `bytes` indexing, typed `const` → later; add when a test needs them.
- `--lint`, determinism CI gates, error injection → **Plan 6**.

---

## Architecture & key design decisions

Read this before starting; every task assumes it.

1. **`Env` replaces the `Consts` type alias (Task 1).** Today `pub type Consts = HashMap<String, Value>`. Evaluation now needs three more things: the `fn` table, the parameters bound by the innermost call, and the chain of calls in progress (to reject recursion). `Env` holds them, and — critically — keeps the module `const`s **separate** from the innermost `locals`, so `child_for_call` can build a callee scope from the *module* base rather than the caller's locals. That is what makes scoping **lexical**: a callee sees its own parameters and the module's constants, never the caller's variables.

2. **`Env` is owned and cloned, not borrowed (Task 1).** `Emitter` must be able to *swap* its environment around a `proc` expansion, which needs ownership; so `emit()` takes `Env` by value and `Emitter` drops its `'a` lifetime. `child_for_call` clones the `const` and `fn` maps. This is deliberate: a tamal program is at most 1024 words, so the number of call sites is small and bounded, and clarity beats micro-optimisation here. (If it ever matters, wrap the two tables in `Rc` — the shape of the code does not change.)

3. **Two callables, two homes.** `fn`s live in the `Env` because **consteval** resolves them (a `fn` call is an expression). `proc`s live on the **`Emitter`** because only emit expands them (a `proc` call is a statement). Neither is a runtime call — spec §1.1.

4. **Argument binding is written once (`consteval::bind_args`)** and used by both callables: positional arguments in declaration order, then named (`name = value`) arguments, then each unbound parameter's default. Positional-after-named is an error. Every bound value is type-checked against its declared type (`byte` also range-checks `0..=255`). Diagnostics are emitted in **declaration order** (iterating the `params` `Vec`, never a `HashMap`), so the same source always produces the same first error — a determinism requirement, not a nicety.

5. **Arguments and defaults are evaluated in *different* scopes, and the asymmetry is load-bearing.** An **argument** is written at the *call site*, so it is evaluated in the **caller's** scope with the callee **not** yet on the in-progress chain — otherwise ordinary nesting like `f(f(1))` would be misreported as recursion. A **default** is written at the *definition site*, so it is evaluated in the callee's **module scope** with the callee **already on** the chain — otherwise a self-referential default (`fn f(n: int = f())`) recurses forever. Module scope also stops a default from capturing a caller local that happens to share its name.

   > **Correction (found by review during execution).** The first draft of this plan had `bind_args` evaluate defaults via a plain `env.module_scope()`, which preserves the chain *unchanged* — and `eval_fn_call` only pushes the callee **after** `bind_args` returns. A self-referential default therefore never saw itself on the chain and overflowed the stack (exit 134, no diagnostic) on `fn f(n: int = f())`, on a mutual default cycle, and on a default cycle reached from another `fn`'s body. The fix belongs **inside `bind_args`** — a `module_scope_for(callee)` used only by the defaults loop, leaving the argument loop on the caller's `env` — so `proc` inherits it in Task 6 for free. Note the *obvious* fix (pushing the callee before `bind_args`) is wrong: it silently breaks `f(f(1))` while leaving the suite green.

6. **Register hygiene comes free from `RegAlloc` (D5).** A `proc` expansion brackets its body with `enter_scope()`/`exit_scope()`. Registers live in the caller stay marked busy, so the callee's lowest-free-first allocations cannot alias them (this is exactly the invariant pinned by `regalloc.rs::nested_scope_does_not_clobber_a_live_outer_register`), and everything the callee bound is released on exit. **Label** hygiene comes free from the shared `Emitter::gensym` counter.

7. **The `Emitter` gets a frame *stack* (Task 5).** Plan 3 threaded `&mut Vec<Deferred>` through a second, near-duplicate statement matcher (`frame_body_stmt`). Once a `proc` can be called *inside* a `frame`, its body statements must reach that frame's deferred-verdict list — threading the parameter through expansion is unpleasant, so the list moves onto the `Emitter` as `frames: Vec<Vec<Deferred>>`. The two matchers then collapse into **one** `stmt()` with an `in_frame()` guard. This is a **behavior-preserving refactor** (Plan 3's goldens pin it) and it drops the "add a `Stmt` variant → update FOUR exhaustive matches" gotcha to three.

8. **Recursion is a hard error, in both callables.** Every call is inlined, so a self-call would expand forever. `fn` recursion is caught by `Env::is_active`; `proc` recursion by `Emitter::active_procs`. The message names the ISA reason rather than the compiler's limitation.

9. **`repeat N` is a compile-time unroll, capped (Tasks 7–8).** No loop counter and no branch: the body is emitted `N` times, each iteration in its own register scope so a binding made inside is released before the next iteration reuses the register. `N` is bounded by `MAX_UNROLL = 1024` — a tamal program is at most 1024 words, so a larger unroll can never assemble, and rejecting it up front turns an out-of-memory into a diagnostic. Task 8 applies the same cap to `recv N` (a tracked Plan-3 follow-up: `recv 99999999` currently OOMs before the assembler's cap can reject it).

10. **The M2 "test never halts" scan stays conservative.** Plan 3 already treats `frame { … }` as non-halting (`Frame => false`); `Call` and `Repeat` follow the same rule rather than growing a reachability analysis. A verdict belongs at test level. Task 6 pays for the conservatism with a clearer help line naming the limitation.

11. **Verification: "byte-identical modulo register allocation" (spec §9).** The capstone decodes both programs to `tamal_abi::isa::Instr`, zeroes every `Reg` operand, and compares — tolerating the allocator's register choices while proving the opcodes, immediates, folded CRC bytes and branch offsets all match the hand-written `.s`. Per-construct tasks additionally pin exact generated-asm text.

**Verified reference facts** (checked against the tree while writing this plan — trust them):
- `crc8([0x06,0x21,0x00,0x04,0x10,0x00,0x01,0xAB]) == 0xB1` (the OOB packet) and `crc8([0x44,0x00,0x64]) == 0x16` (the peripheral command).
- `examples/oob_smbus_msg.s` assembles to **24 words**, with `beq` offset `-3` and `bnez` offset `+2`.
- Baseline test counts: **90 in `tamal-lang`** (82 lib + 2 `examples` + 2 `frames` + 4 `values`), **196 workspace-wide**.
- `li x2, 0x0F` tiles to **one** word (`load_imm`), which is why the poll idiom is 4 words.

---

## File Structure

- `crates/tamal-lang/src/consteval.rs` — **modify**: `Consts` → an `Env` struct (module consts + call locals + `fn` table + in-progress chain); multi-argument `Expr::Call` evaluation; `fn` call evaluation; the shared `bind_args` + `check_type`; `BUILTINS`.
- `crates/tamal-lang/src/lexer.rs` — **modify**: two new tokens, `:` (`Tok::Colon`) and `->` (`Tok::Arrow`).
- `crates/tamal-lang/src/parser.rs` — **modify**: `Arg`, `Param`, `Type`, `FnDef`, `ProcDef`; `Module.fns`/`Module.procs`; `Stmt::{Call, Repeat}`; multi-arg call expressions; a shared `parse_block`; the `recv N` cap.
- `crates/tamal-lang/src/emit.rs` — **modify**: `Emitter` owns its `Env`, gains a frame stack (collapsing the two statement matchers into one), a `proc` table, an active-`proc` chain, `lower_call` (hygienic inline expansion) and `lower_repeat` (unroll).
- `crates/tamal-lang/src/lib.rs` — **modify**: `MAX_UNROLL`; build and validate the `fn`/`proc` tables (duplicates, builtin shadowing, `fn`/`proc` name collisions); extend the M2 `halts` match and its help text.
- `crates/tamal-lang/tests/common/mod.rs` — **create**: the `canon`/`zero_regs` "modulo register allocation" comparator, moved out of `tests/frames.rs` so both integration tests share one copy.
- `crates/tamal-lang/tests/frames.rs` — **modify**: use the shared comparator.
- `crates/tamal-lang/tests/callables.rs` — **create**: the capstones — an OOB channel test built from a local `command` proc, and a peripheral read built from a `fn` + the same proc, each byte-matching its `.s` modulo registers.
- `crates/tamal-lang/README.md` — **modify**: document the Plan-4a surface.

---

## Task 1: `Env` — the compile-time environment (behavior-preserving)

Replaces the `Consts` type alias with a struct that owns the module `const`s and (from Task 3) the `fn` table, and hands ownership to the `Emitter`. **No behavior change** — every existing test stays green.

**Files:**
- Modify: `crates/tamal-lang/src/consteval.rs`
- Modify: `crates/tamal-lang/src/emit.rs`
- Modify: `crates/tamal-lang/src/lib.rs`

- [ ] **Step 1: Replace the `Consts` alias with `Env`**

In `crates/tamal-lang/src/consteval.rs`, replace these three lines:

```rust
/// The resolved `const` environment, keyed by name.
pub type Consts = HashMap<String, Value>;
```

with:

```rust
/// The compile-time environment threaded through evaluation: the module's
/// `const`s (the base scope) plus the parameters bound by the innermost
/// `fn`/`proc` expansion.
///
/// Lookup is **locals first, then consts**, and a call body is evaluated in a
/// child built from the module base — never from the caller's locals — so
/// scoping is lexical: a callee sees its own parameters and the module's
/// constants, and nothing else.
#[derive(Debug, Clone, Default)]
pub struct Env {
    /// Module-level `const`s: the base scope every call body starts from.
    consts: HashMap<String, Value>,
    /// Parameters bound by the innermost call. Empty at module level; filled by
    /// `fn`/`proc` expansion.
    locals: HashMap<String, Value>,
}

impl Env {
    /// An empty environment.
    pub fn new() -> Self {
        Self::default()
    }

    /// Define a module-level `const`.
    pub fn insert_const(&mut self, name: String, value: Value) {
        self.consts.insert(name, value);
    }

    /// Is `name` already defined as a module-level `const`?
    pub fn has_const(&self, name: &str) -> bool {
        self.consts.contains_key(name)
    }

    /// The value bound to `name`: the innermost parameter binding if there is
    /// one, else a module `const`.
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.locals.get(name).or_else(|| self.consts.get(name))
    }
}
```

- [ ] **Step 2: Re-point every `consts: &Consts` parameter at `&Env`**

Still in `consteval.rs`, rename the parameter and type in all five evaluation functions. `eval`:

```rust
/// Fold an expression to a [`Value`] under the given environment.
pub fn eval(e: &Expr, env: &Env) -> Result<Value, Diagnostic> {
    match e {
        Expr::Int { value, .. } => Ok(Value::Int(*value)),
        Expr::Name { name, span } => env
            .get(name)
            .cloned()
            .ok_or_else(|| Diagnostic::error(span.clone(), format!("unknown name `{name}`"))),
        Expr::Bytes { elems, .. } => {
            let mut out = Vec::with_capacity(elems.len());
            for el in elems {
                out.push(eval_byte(el, env)?);
            }
            Ok(Value::Bytes(out))
        }
        Expr::Binary {
            op: BinOp::Concat,
            lhs,
            rhs,
            ..
        } => {
            let mut a = eval_bytes(lhs, env)?;
            a.extend(eval_bytes(rhs, env)?);
            Ok(Value::Bytes(a))
        }
        Expr::Binary {
            op: BinOp::Xor,
            lhs,
            rhs,
            ..
        } => Ok(Value::Int(eval_int(lhs, env)? ^ eval_int(rhs, env)?)),
        Expr::Call { func, arg, span } => eval_call(func, arg, span, env),
    }
}
```

and the four helpers below it — replace each `consts: &Consts` with `env: &Env` and each use of `consts` with `env`:

```rust
fn eval_call(func: &str, arg: &Expr, span: &Span, env: &Env) -> Result<Value, Diagnostic> {
    match func {
        "crc8" => Ok(Value::Int(
            tamal_abi::crc8::crc8(&eval_bytes(arg, env)?) as i64
        )),
        "len" => Ok(Value::Int(eval_bytes(arg, env)?.len() as i64)),
        "lo" => Ok(Value::Int(eval_int(arg, env)? & 0xff)),
        "hi" => Ok(Value::Int((eval_int(arg, env)? >> 8) & 0xff)),
        _ => Err(
            Diagnostic::error(span.clone(), format!("unknown builtin `{func}`"))
                .with_help("the builtins are crc8, len, lo, hi"),
        ),
    }
}

/// Evaluate `e` to an integer.
pub fn eval_int(e: &Expr, env: &Env) -> Result<i64, Diagnostic> {
    match eval(e, env)? {
        Value::Int(n) => Ok(n),
        Value::Bytes(_) => Err(Diagnostic::error(
            e.span(),
            "expected an integer, found bytes",
        )),
    }
}

/// Evaluate `e` to a byte string.
pub fn eval_bytes(e: &Expr, env: &Env) -> Result<Vec<u8>, Diagnostic> {
    match eval(e, env)? {
        Value::Bytes(b) => Ok(b),
        Value::Int(_) => Err(Diagnostic::error(
            e.span(),
            "expected bytes, found an integer",
        )),
    }
}

/// Evaluate `e` to a single byte (an integer in `0..=255`).
pub fn eval_byte(e: &Expr, env: &Env) -> Result<u8, Diagnostic> {
    let n = eval_int(e, env)?;
    u8::try_from(n)
        .map_err(|_| Diagnostic::error(e.span(), format!("byte value {n} is out of range 0..=255")))
}
```

- [ ] **Step 3: Update the `consteval` unit tests**

In the `#[cfg(test)] mod tests` block of `consteval.rs`, replace every `&Consts::new()` with `&Env::new()` (six occurrences: in `folds_int_and_xor`, `folds_bytes_and_concat`, `byte_out_of_range_errors`, `crc8_folds_peripheral_command_bytes`, `len_lo_hi_builtins`, and twice in `type_mismatch_and_unknown_builtin_error`), and rewrite `resolves_and_rejects_names` to use the new API:

```rust
    #[test]
    fn resolves_and_rejects_names() {
        let mut c = Env::new();
        c.insert_const("X".into(), Value::Int(0x44));
        assert_eq!(
            eval(
                &Expr::Name {
                    name: "X".into(),
                    span: 0..0
                },
                &c
            )
            .unwrap(),
            Value::Int(0x44)
        );
        assert!(
            eval(
                &Expr::Name {
                    name: "NOPE".into(),
                    span: 0..0
                },
                &c
            )
            .is_err()
        );
    }
```

- [ ] **Step 4: Give the `Emitter` an owned `Env`**

In `crates/tamal-lang/src/emit.rs`, change the import on line 9:

```rust
use crate::consteval::{self, Env};
```

Change `emit` to take the environment **by value** and drop the `Emitter` lifetime:

```rust
pub fn emit(module: &Module, env: Env) -> Result<Lowering, Vec<Diagnostic>> {
    let mut e = Emitter::new(env);
```

(the rest of the function body is unchanged), then the struct and constructor:

```rust
/// The lowering state: the growing asm text + source map.
struct Emitter {
    env: Env,
    asm: String,
    lines: Vec<(Span, Span)>,
    alloc: RegAlloc,
    gensym: u32,
    trailers: Vec<(Span, String)>,
}
```

```rust
impl Emitter {
    fn new(env: Env) -> Self {
        Emitter {
            env,
            asm: String::new(),
            lines: Vec::new(),
            alloc: RegAlloc::new(),
            gensym: 0,
            trailers: Vec::new(),
        }
    }
```

Note `impl<'a> Emitter<'a>` becomes `impl Emitter`.

- [ ] **Step 5: Re-point the three evaluation call sites in `emit.rs`**

Replace `self.consts` with `&self.env` in `lower_send`, `lower_crc_region`, and the `Stmt::Expect` arm of `frame_body_stmt`:

```rust
        let mut bs = consteval::eval_bytes(bytes, &self.env).map_err(|d| vec![d])?;
```

```rust
            total.extend(consteval::eval_bytes(e, &self.env).map_err(|d| vec![d])?);
```

```rust
                let code = consteval::eval_byte(else_code, &self.env).map_err(|d| vec![d])?;
```

- [ ] **Step 6: Update the `emit` unit tests**

In `emit.rs`'s `mod tests`, the four `emit(...)` calls now pass an owned `Env`. Replace `&Consts::new()` with `Env::new()` in all four (`emits_entry_and_pass`, `emits_fail_code_verbatim`, `emits_raw_instructions`, `remap_points_asm_span_at_originating_tam_span`), e.g.:

```rust
        let asm = emit(&one(vec![Stmt::Pass]), Env::new()).unwrap().asm;
```

```rust
        let low = emit(&m, Env::new()).unwrap();
```

- [ ] **Step 7: Update the driver**

In `crates/tamal-lang/src/lib.rs`, replace the const-resolution block and the final `emit` call:

```rust
    let mut env = consteval::Env::new();
    for c in &module.consts {
        if env.has_const(&c.name) {
            return Err(vec![Diagnostic::error(
                c.name_span.clone(),
                format!("duplicate const `{}`", c.name),
            )]);
        }
        let v = consteval::eval(&c.value, &env).map_err(|d| vec![d])?;
        env.insert_const(c.name.clone(), v);
    }
```

and at the end of `lower`:

```rust
    emit::emit(&module, env)
```

- [ ] **Step 8: Run the whole crate to verify NO behavior change**

Run: `cargo test -p tamal-lang`
Expected: PASS — the same counts as before this task: `82 passed` (lib), `2 passed` (examples), `2 passed` (frames), `4 passed` (values), `0 failed` everywhere.

- [ ] **Step 9: Commit**

```bash
cargo fmt -p tamal-lang
git add crates/tamal-lang/src/consteval.rs crates/tamal-lang/src/emit.rs crates/tamal-lang/src/lib.rs
git commit -m "refactor(tamal-lang): Env replaces the Consts alias (no behavior change)"
```

---

## Task 2: Multi-argument call expressions (`Arg`)

Generalises `Expr::Call`'s single `arg` into an argument **list** that can carry names, so Task 3's `fn` calls and Task 6's `proc` calls share one grammar. The builtins (`crc8`/`len`/`lo`/`hi`) keep their exact arity-1 positional contract, now enforced explicitly.

**Files:**
- Modify: `crates/tamal-lang/src/parser.rs`
- Modify: `crates/tamal-lang/src/consteval.rs`

- [ ] **Step 1: Write the failing tests**

Add to the `mod tests` block in `crates/tamal-lang/src/parser.rs` (after `parses_call`):

```rust
    #[test]
    fn parses_call_with_several_positional_args() {
        match parse_expr_ok("iord_hdr(0x44, 0x0064)") {
            Expr::Call { func, args, .. } => {
                assert_eq!(func, "iord_hdr");
                assert_eq!(args.len(), 2);
                assert!(args.iter().all(|a| a.name.is_none()));
            }
            e => panic!("expected Call, got {e:?}"),
        }
    }

    #[test]
    fn parses_named_arguments() {
        match parse_expr_ok("command(pkt = [0x44], ndata = 0)") {
            Expr::Call { args, .. } => {
                assert_eq!(args[0].name.as_deref(), Some("pkt"));
                assert_eq!(args[1].name.as_deref(), Some("ndata"));
            }
            e => panic!("expected Call, got {e:?}"),
        }
    }

    #[test]
    fn parses_multiline_call_with_trailing_comma() {
        match parse_expr_ok("command(\n  pkt = [0x44],\n  ndata = 0,\n)") {
            Expr::Call { args, .. } => assert_eq!(args.len(), 2),
            e => panic!("expected Call, got {e:?}"),
        }
    }

    #[test]
    fn parses_zero_argument_call() {
        match parse_expr_ok("controller()") {
            Expr::Call { func, args, .. } => {
                assert_eq!(func, "controller");
                assert!(args.is_empty());
            }
            e => panic!("expected Call, got {e:?}"),
        }
    }
```

Add to the `mod tests` block in `crates/tamal-lang/src/consteval.rs` (after `type_mismatch_and_unknown_builtin_error`):

```rust
    #[test]
    fn builtins_require_exactly_one_positional_argument() {
        // zero args
        let none = Expr::Call {
            func: "crc8".into(),
            args: vec![],
            span: 0..0,
        };
        assert!(
            eval(&none, &Env::new())
                .unwrap_err()
                .message
                .contains("takes 1 argument")
        );
        // two args
        let two = Expr::Call {
            func: "crc8".into(),
            args: vec![
                Arg {
                    name: None,
                    value: bytes(&[1]),
                    span: 0..0,
                },
                Arg {
                    name: None,
                    value: bytes(&[2]),
                    span: 0..0,
                },
            ],
            span: 0..0,
        };
        assert!(
            eval(&two, &Env::new())
                .unwrap_err()
                .message
                .contains("takes 1 argument")
        );
        // a named argument
        let named = Expr::Call {
            func: "crc8".into(),
            args: vec![Arg {
                name: Some("b".into()),
                value: bytes(&[1]),
                span: 0..0,
            }],
            span: 0..0,
        };
        assert!(
            eval(&named, &Env::new())
                .unwrap_err()
                .message
                .contains("does not take named arguments")
        );
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p tamal-lang --lib`
Expected: **compile error** — `struct Arg` not found / `Expr::Call` has no field `args`. This is the RED state.

- [ ] **Step 3: Add `Arg` and widen `Expr::Call`**

In `crates/tamal-lang/src/parser.rs`, add this type immediately after the `RecvTarget` enum:

```rust
/// One argument at a call site: positional, or `name = value` (named).
#[derive(Debug, Clone)]
pub struct Arg {
    /// `Some(name)` for a named argument (`command(ndata = 0)`), `None` when
    /// the argument is positional.
    pub name: Option<String>,
    /// The argument's value expression.
    pub value: Expr,
    /// The span covering the whole argument (the name too, when named).
    pub span: Span,
}
```

and replace the `Call` variant of `Expr`:

```rust
    /// A call: a builtin (`crc8(e)`, `len`, `lo`, `hi`) or a user `fn`, with
    /// positional and/or named arguments.
    Call {
        func: String,
        args: Vec<Arg>,
        span: Span,
    },
```

- [ ] **Step 4: Parse the argument list**

In `crates/tamal-lang/src/parser.rs`, add a two-token lookahead helper next to `peek` in `impl<'a> P<'a>`:

```rust
    /// The token `n` positions ahead (`peek_at(0) == peek()`).
    fn peek_at(&self, n: usize) -> Tok {
        self.toks
            .get(self.i + n)
            .map(|t| t.kind)
            .unwrap_or(Tok::Eof)
    }
```

Add the two argument parsers next to `parse_expr` (before `parse_concat`):

```rust
    /// Parse a call's argument list, up to but not including the closing `)`
    /// (the `(` is already consumed). Newlines inside the parens are not
    /// statement terminators and a trailing comma before `)` is allowed.
    fn parse_args(&mut self) -> Result<Vec<Arg>, Vec<Diagnostic>> {
        let mut args = Vec::new();
        self.skip_newlines();
        while self.peek() != Tok::RParen {
            args.push(self.parse_arg()?);
            self.skip_newlines();
            if self.peek() == Tok::Comma {
                self.i += 1;
                self.skip_newlines();
            } else {
                break;
            }
        }
        Ok(args)
    }

    /// One argument: `name = expr` when an identifier is directly followed by
    /// `=`, else a positional expression.
    fn parse_arg(&mut self) -> Result<Arg, Vec<Diagnostic>> {
        if self.peek() == Tok::Ident && self.peek_at(1) == Tok::Eq {
            let sp = self.span();
            self.i += 2; // the name and the `=`
            let name = self.lexeme(&sp).to_string();
            self.skip_newlines();
            let value = self.parse_expr()?;
            let span = sp.start..value.span().end;
            Ok(Arg {
                name: Some(name),
                value,
                span,
            })
        } else {
            let value = self.parse_expr()?;
            let span = value.span();
            Ok(Arg {
                name: None,
                value,
                span,
            })
        }
    }
```

Then rewrite the call branch of `parse_primary` — replace this block inside the `Tok::Ident` arm:

```rust
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
```

with:

```rust
                if self.peek() == Tok::LParen {
                    self.i += 1;
                    let args = self.parse_args()?;
                    let close = self.expect(Tok::RParen, "`)`")?;
                    Ok(Expr::Call {
                        func: name,
                        args,
                        span: sp.start..close.span.end,
                    })
                } else {
```

- [ ] **Step 5: Evaluate multi-argument calls**

In `crates/tamal-lang/src/consteval.rs`, change the import to bring `Arg` in:

```rust
use crate::parser::{Arg, BinOp, Expr};
```

Change the `Expr::Call` arm of `eval` to pass the list:

```rust
        Expr::Call { func, args, span } => eval_call(func, args, span, env),
```

and replace `eval_call` with the arity-checked version plus its helper:

```rust
fn eval_call(func: &str, args: &[Arg], span: &Span, env: &Env) -> Result<Value, Diagnostic> {
    match func {
        "crc8" => Ok(Value::Int(
            tamal_abi::crc8::crc8(&eval_bytes(builtin_arg(func, args, span)?, env)?) as i64,
        )),
        "len" => Ok(Value::Int(
            eval_bytes(builtin_arg(func, args, span)?, env)?.len() as i64,
        )),
        "lo" => Ok(Value::Int(
            eval_int(builtin_arg(func, args, span)?, env)? & 0xff,
        )),
        "hi" => Ok(Value::Int(
            (eval_int(builtin_arg(func, args, span)?, env)? >> 8) & 0xff,
        )),
        _ => Err(
            Diagnostic::error(span.clone(), format!("unknown builtin `{func}`"))
                .with_help("the builtins are crc8, len, lo, hi"),
        ),
    }
}

/// The single positional argument of a builtin call. The builtins take exactly
/// one positional argument; anything else is a diagnostic (the arity is part of
/// their contract, not something a caller may vary).
fn builtin_arg<'a>(func: &str, args: &'a [Arg], span: &Span) -> Result<&'a Expr, Diagnostic> {
    if args.len() != 1 {
        return Err(Diagnostic::error(
            span.clone(),
            format!("`{func}` takes 1 argument, got {}", args.len()),
        ));
    }
    if let Some(name) = &args[0].name {
        return Err(Diagnostic::error(
            args[0].span.clone(),
            format!("`{func}` does not take named arguments (found `{name} = …`)"),
        ));
    }
    Ok(&args[0].value)
}
```

- [ ] **Step 6: Update the `consteval` test helper**

Still in `consteval.rs`'s `mod tests`, replace the `call` helper so it builds a one-element argument list:

```rust
    fn call(func: &str, arg: Expr) -> Expr {
        Expr::Call {
            func: func.into(),
            args: vec![Arg {
                name: None,
                value: arg,
                span: 0..0,
            }],
            span: 0..0,
        }
    }
```

- [ ] **Step 7: Run to verify they pass**

Run: `cargo test -p tamal-lang`
Expected: PASS — `0 failed`, with the five new tests present (`parses_call_with_several_positional_args`, `parses_named_arguments`, `parses_multiline_call_with_trailing_comma`, `parses_zero_argument_call`, `builtins_require_exactly_one_positional_argument`). Lib count rises 82 → 87.

- [ ] **Step 8: Commit**

```bash
cargo fmt -p tamal-lang
git add crates/tamal-lang/src/parser.rs crates/tamal-lang/src/consteval.rs
git commit -m "feat(tamal-lang): multi-argument call expressions with named arguments"
```

---

## Task 3: `bind_args` — the shared argument binder

The semantics both callables share, written **once** and unit-tested in isolation: positional arguments in declaration order, then named arguments, then defaults, with every bound value type-checked. No grammar yet — this task adds the `Param`/`Type` AST nodes and the binder; Task 4 gives them a syntax.

**Files:**
- Modify: `crates/tamal-lang/src/parser.rs`
- Modify: `crates/tamal-lang/src/consteval.rs`

- [ ] **Step 1: Write the failing tests**

Add to the `mod tests` block in `crates/tamal-lang/src/consteval.rs` (after `builtins_require_exactly_one_positional_argument`):

```rust
    use crate::parser::{Param, Type};

    fn param(name: &str, ty: Type, default: Option<Expr>) -> Param {
        Param {
            name: name.into(),
            ty,
            default,
            span: 0..0,
        }
    }

    fn pos(value: Expr) -> Arg {
        Arg {
            name: None,
            value,
            span: 0..0,
        }
    }

    fn named(name: &str, value: Expr) -> Arg {
        Arg {
            name: Some(name.into()),
            value,
            span: 0..0,
        }
    }

    /// `(pkt: bytes, ndata: int, err: byte = 0x11)` — the shape of the library
    /// `command` proc, which is what this binder exists to serve.
    fn command_params() -> Vec<Param> {
        vec![
            param("pkt", Type::Bytes, None),
            param("ndata", Type::Int, None),
            param("err", Type::Byte, Some(int(0x11))),
        ]
    }

    fn bind_ok(args: Vec<Arg>) -> std::collections::HashMap<String, Value> {
        bind_args("command", &command_params(), &args, &Env::new(), &(0..0)).unwrap()
    }

    fn bind_err(args: Vec<Arg>) -> String {
        bind_args("command", &command_params(), &args, &Env::new(), &(0..0))
            .unwrap_err()
            .message
    }

    #[test]
    fn binds_positional_arguments_in_order() {
        let b = bind_ok(vec![pos(bytes(&[0x44])), pos(int(1))]);
        assert_eq!(b["pkt"], Value::Bytes(vec![0x44]));
        assert_eq!(b["ndata"], Value::Int(1));
    }

    #[test]
    fn binds_named_arguments_by_name() {
        let b = bind_ok(vec![named("ndata", int(2)), named("pkt", bytes(&[0x06]))]);
        assert_eq!(b["pkt"], Value::Bytes(vec![0x06]));
        assert_eq!(b["ndata"], Value::Int(2));
    }

    #[test]
    fn unbound_parameters_take_their_default() {
        let b = bind_ok(vec![pos(bytes(&[0x44])), pos(int(0))]);
        assert_eq!(b["err"], Value::Int(0x11));
    }

    #[test]
    fn an_explicit_argument_overrides_a_default() {
        let b = bind_ok(vec![pos(bytes(&[0x44])), pos(int(0)), named("err", int(0x22))]);
        assert_eq!(b["err"], Value::Int(0x22));
    }

    #[test]
    fn a_positional_argument_may_not_follow_a_named_one() {
        let msg = bind_err(vec![named("pkt", bytes(&[0x44])), pos(int(0))]);
        assert!(msg.contains("cannot follow a named argument"), "got: {msg}");
    }

    #[test]
    fn rejects_unknown_duplicate_and_surplus_arguments() {
        let unknown = bind_err(vec![named("nope", int(0))]);
        assert!(unknown.contains("no parameter `nope`"), "got: {unknown}");

        let dup = bind_err(vec![pos(bytes(&[0x44])), named("pkt", bytes(&[0x06]))]);
        assert!(dup.contains("bound twice"), "got: {dup}");

        let surplus = bind_err(vec![
            pos(bytes(&[0x44])),
            pos(int(0)),
            pos(int(0x11)),
            pos(int(9)),
        ]);
        assert!(surplus.contains("takes 3 argument"), "got: {surplus}");
    }

    #[test]
    fn a_missing_required_parameter_is_reported_in_declaration_order() {
        // Neither `pkt` nor `ndata` is bound; the FIRST declared one is named,
        // deterministically (params are a Vec, never a HashMap).
        let msg = bind_err(vec![]);
        assert!(msg.contains("`pkt`"), "got: {msg}");
    }

    #[test]
    fn arguments_are_checked_against_their_declared_type() {
        let wrong = bind_err(vec![pos(int(5)), pos(int(0))]); // pkt: bytes
        assert!(wrong.contains("expects `bytes`"), "got: {wrong}");

        let too_big = bind_err(vec![pos(bytes(&[0x44])), pos(int(0)), named("err", int(256))]);
        assert!(too_big.contains("expects `byte`"), "got: {too_big}");
    }

    #[test]
    fn a_default_is_evaluated_in_module_scope_not_the_callers() {
        // The caller has a local `K`; the callee's default refers to `K` too.
        // Lexical scoping means the default must NOT see the caller's binding —
        // here there is no module `K`, so it is an "unknown name" error rather
        // than silently picking up 0x99.
        let mut caller = Env::new();
        caller.insert_const("BASE".into(), Value::Int(1));
        let caller = caller.child_for_call_values(
            [("K".to_string(), Value::Int(0x99))].into_iter().collect(),
        );
        let params = vec![param(
            "err",
            Type::Byte,
            Some(Expr::Name {
                name: "K".into(),
                span: 0..0,
            }),
        )];
        let err = bind_args("p", &params, &[], &caller, &(0..0)).unwrap_err();
        assert!(err.message.contains("unknown name `K`"), "got: {}", err.message);
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p tamal-lang --lib consteval`
Expected: **compile error** — `Param`/`Type` not found in `crate::parser`, `bind_args` not found. This is the RED state.

- [ ] **Step 3: Add the `Type` and `Param` AST nodes**

In `crates/tamal-lang/src/parser.rs`, add both types immediately after the `Arg` struct:

```rust
/// A compile-time parameter or return type. (`bool` and `reg` arrive with the
/// conditionals in Plan 4b.)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Type {
    /// One wire byte, `0..=255`.
    Byte,
    /// A compile-time integer.
    Int,
    /// A compile-time byte string (the packet type).
    Bytes,
}

impl Type {
    /// The spelling used in source, and in diagnostics.
    pub fn name(self) -> &'static str {
        match self {
            Type::Byte => "byte",
            Type::Int => "int",
            Type::Bytes => "bytes",
        }
    }
}

/// One declared parameter of a `fn`/`proc`: `name: type`, with an optional
/// default (`err: byte = 0x11`).
#[derive(Debug, Clone)]
pub struct Param {
    /// The parameter name, bound in the callee's scope.
    pub name: String,
    /// Its declared type; every bound value is checked against it.
    pub ty: Type,
    /// The default used when a call does not bind this parameter.
    pub default: Option<Expr>,
    /// The span of the declaration, for diagnostics.
    pub span: Span,
}
```

- [ ] **Step 4: Add the environment helpers the binder needs**

In `crates/tamal-lang/src/consteval.rs`, add these two methods to `impl Env` (after `get`):

```rust
    /// This environment with no parameter bindings — the module scope.
    ///
    /// A parameter default is written at the callee's *definition* site, so it
    /// is evaluated here rather than in the caller's scope: a default must
    /// never be able to capture a caller local that happens to share its name.
    ///
    /// **Correction during execution:** the defaults loop must additionally push
    /// the callee onto the in-progress chain, or a self-referential default
    /// (`fn f(n: int = f())`) recurses until the stack overflows. Implement this
    /// as `module_scope_for(callee)` and use it *only* for defaults — the
    /// argument loop must stay on the caller's `env`, or legal nesting like
    /// `f(f(1))` is misreported as recursion. See architecture note 5.
    pub fn module_scope(&self) -> Env {
        let mut e = self.clone();
        e.locals.clear();
        e
    }

    /// This environment with `bindings` as its parameter scope, replacing any
    /// locals. The module `const`s survive; the caller's locals do not.
    pub fn child_for_call_values(&self, bindings: HashMap<String, Value>) -> Env {
        let mut e = self.clone();
        e.locals = bindings;
        e
    }
```

- [ ] **Step 5: Write the binder**

Still in `crates/tamal-lang/src/consteval.rs`, extend the import to bring the new AST nodes in:

```rust
use crate::parser::{Arg, BinOp, Expr, Param, Type};
```

and add the binder plus its type check at the end of the file, before the `#[cfg(test)]` block:

```rust
/// Bind a call's arguments to a callable's parameters.
///
/// Positional arguments bind parameters in declaration order; named
/// (`name = value`) arguments bind by name and must follow every positional
/// one; each parameter still unbound takes its default, or is an error if it
/// has none. Every bound value is checked against its declared type.
///
/// Shared by `fn` (evaluated here) and `proc` (expanded by the emitter) so the
/// two callables can never drift apart. Errors are produced by walking `params`
/// and `args` **in order**, so the same source always yields the same first
/// diagnostic — the determinism rule applies to error output too.
pub fn bind_args(
    callee: &str,
    params: &[Param],
    args: &[Arg],
    env: &Env,
    call_span: &Span,
) -> Result<HashMap<String, Value>, Diagnostic> {
    let mut bound: HashMap<String, Value> = HashMap::new();
    let mut seen_named = false;
    for (i, arg) in args.iter().enumerate() {
        let p = match &arg.name {
            None => {
                if seen_named {
                    return Err(Diagnostic::error(
                        arg.span.clone(),
                        "a positional argument cannot follow a named argument",
                    )
                    .with_help("pass the positional arguments first, or name them all"));
                }
                params.get(i).ok_or_else(|| {
                    Diagnostic::error(
                        arg.span.clone(),
                        format!(
                            "`{callee}` takes {} argument(s), got {}",
                            params.len(),
                            args.len()
                        ),
                    )
                })?
            }
            Some(name) => {
                seen_named = true;
                params.iter().find(|p| &p.name == name).ok_or_else(|| {
                    Diagnostic::error(
                        arg.span.clone(),
                        format!("`{callee}` has no parameter `{name}`"),
                    )
                })?
            }
        };
        if bound.contains_key(&p.name) {
            return Err(Diagnostic::error(
                arg.span.clone(),
                format!("argument `{}` of `{callee}` is bound twice", p.name),
            ));
        }
        let v = eval(&arg.value, env)?;
        check_type(
            &v,
            p.ty,
            &arg.span,
            &format!("argument `{}` of `{callee}`", p.name),
        )?;
        bound.insert(p.name.clone(), v);
    }
    // Fill the rest from defaults, in declaration order so that the first
    // missing parameter reported is stable across runs.
    for p in params {
        if bound.contains_key(&p.name) {
            continue;
        }
        let Some(default) = &p.default else {
            return Err(Diagnostic::error(
                call_span.clone(),
                format!("`{callee}` is missing a value for parameter `{}`", p.name),
            ));
        };
        let v = eval(default, &env.module_scope())?;
        check_type(
            &v,
            p.ty,
            &p.span,
            &format!("the default for `{}` of `{callee}`", p.name),
        )?;
        bound.insert(p.name.clone(), v);
    }
    Ok(bound)
}

/// Check a value against a declared type; `what` names the position for the
/// diagnostic. A `byte` additionally range-checks `0..=255` — out of range is
/// an error, never a silent wrap.
fn check_type(v: &Value, ty: Type, span: &Span, what: &str) -> Result<(), Diagnostic> {
    let ok = match (ty, v) {
        (Type::Bytes, Value::Bytes(_)) => true,
        (Type::Int, Value::Int(_)) => true,
        (Type::Byte, Value::Int(n)) => (0..=255).contains(n),
        _ => false,
    };
    if ok {
        return Ok(());
    }
    let found = match v {
        Value::Int(n) => format!("the integer {n}"),
        Value::Bytes(b) => format!("{} byte(s)", b.len()),
    };
    Err(Diagnostic::error(
        span.clone(),
        format!("{what} expects `{}`, found {found}", ty.name()),
    ))
}
```

- [ ] **Step 6: Run to verify they pass**

Run: `cargo test -p tamal-lang --lib consteval`
Expected: PASS — the nine new binder tests plus the existing `consteval` tests, `0 failed`.

- [ ] **Step 7: Commit**

```bash
cargo fmt -p tamal-lang
git add crates/tamal-lang/src/parser.rs crates/tamal-lang/src/consteval.rs
git commit -m "feat(tamal-lang): shared argument binder (positional, named, defaults)"
```

---

## Task 4: `fn` items — pure, value-returning callables

`fn NAME(params) -> type { expr }`, with its call replaced by the value it returns. Adds the two tokens the parameter syntax needs (`:` and `->`), the grammar, the `fn` table on the `Env`, and the driver wiring.

**Files:**
- Modify: `crates/tamal-lang/src/lexer.rs`
- Modify: `crates/tamal-lang/src/parser.rs`
- Modify: `crates/tamal-lang/src/consteval.rs`
- Modify: `crates/tamal-lang/src/emit.rs` (two test `Module` literals gain a field)
- Modify: `crates/tamal-lang/src/lib.rs`

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `crates/tamal-lang/src/lexer.rs`:

```rust
    #[test]
    fn lexes_colon_and_arrow() {
        assert_eq!(
            kinds("fn f(n: int) -> byte\n"),
            vec![
                Tok::Ident, // fn
                Tok::Ident, // f
                Tok::LParen,
                Tok::Ident, // n
                Tok::Colon,
                Tok::Ident, // int
                Tok::RParen,
                Tok::Arrow,
                Tok::Ident, // byte
                Tok::Newline,
                Tok::Eof,
            ]
        );
    }
```

Add to `mod tests` in `crates/tamal-lang/src/parser.rs`:

```rust
    #[test]
    fn parses_fn_item() {
        let m = parse_ok(
            "fn iord_hdr(op: byte, addr: int) -> bytes { [op, hi(addr), lo(addr)] }\ntest t {\n pass\n}\n",
        );
        assert_eq!(m.fns.len(), 1);
        let f = &m.fns[0];
        assert_eq!(f.name, "iord_hdr");
        assert_eq!(f.ret, Type::Bytes);
        assert_eq!(f.params.len(), 2);
        assert_eq!(f.params[0].name, "op");
        assert_eq!(f.params[0].ty, Type::Byte);
        assert_eq!(f.params[1].ty, Type::Int);
        assert!(f.params.iter().all(|p| p.default.is_none()));
    }

    #[test]
    fn parses_fn_param_default() {
        let m = parse_ok("fn f(err: byte = 0x11) -> byte { err }\ntest t {\n pass\n}\n");
        assert!(m.fns[0].params[0].default.is_some());
    }

    #[test]
    fn rejects_an_unknown_param_type() {
        let src = "fn f(x: word) -> byte { x }\ntest t {\n pass\n}\n";
        let toks = lex(src).unwrap();
        let err = parse(src, &toks).unwrap_err();
        assert!(err[0].message.contains("unknown type `word`"));
    }
```

Add to `mod tests` in `crates/tamal-lang/src/lib.rs`:

```rust
    #[test]
    fn fn_call_folds_to_its_returned_value() {
        // A `fn` disappears entirely: the call is replaced by its bytes, and
        // `+ crc8` folds the CRC over exactly those bytes (0x16).
        let asm = lower_to_asm(
            "fn iord_hdr(op: byte, addr: int) -> bytes { [op, hi(addr), lo(addr)] }\n\
             test t {\n send iord_hdr(0x44, 0x0064) + crc8\n pass\n}\n",
        )
        .unwrap();
        assert_eq!(
            asm,
            ".globl _start\n_start:\n\
             \tput_byte 0x44\n\tput_byte 0x00\n\tput_byte 0x64\n\tput_byte 0x16\n\
             \thalt 0x00\n"
        );
    }

    #[test]
    fn a_const_can_call_a_fn() {
        // The `fn` table is installed before `const`s resolve, so a `const` may
        // be built by a compile-time helper.
        let asm = lower_to_asm(
            "fn iord_hdr(op: byte, addr: int) -> bytes { [op, hi(addr), lo(addr)] }\n\
             const HDR = iord_hdr(0x44, 0x0064)\n\
             test t {\n send HDR + crc8\n pass\n}\n",
        )
        .unwrap();
        assert!(asm.contains("\tput_byte 0x16\n"), "got:\n{asm}");
    }

    #[test]
    fn a_recursive_fn_is_rejected() {
        let err = lower_to_asm(
            "fn f(n: int) -> int { f(n) }\ntest t {\n send [f(1)]\n pass\n}\n",
        )
        .unwrap_err();
        assert!(
            err[0].message.contains("already being expanded"),
            "got: {:?}",
            err[0]
        );
    }

    #[test]
    fn a_fn_body_must_produce_its_declared_type() {
        let err = lower_to_asm(
            "fn bad(n: int) -> bytes { n }\ntest t {\n send bad(5)\n pass\n}\n",
        )
        .unwrap_err();
        assert!(
            err[0].message.contains("expects `bytes`"),
            "got: {:?}",
            err[0]
        );
    }

    #[test]
    fn a_fn_may_not_shadow_a_builtin_or_repeat_a_name() {
        let shadow =
            lower_to_asm("fn crc8(b: bytes) -> byte { 0 }\ntest t {\n pass\n}\n").unwrap_err();
        assert!(shadow[0].message.contains("builtin"), "got: {:?}", shadow[0]);

        let dup = lower_to_asm(
            "fn f(n: int) -> int { n }\nfn f(n: int) -> int { n }\ntest t {\n pass\n}\n",
        )
        .unwrap_err();
        assert!(dup[0].message.contains("duplicate fn"), "got: {:?}", dup[0]);
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p tamal-lang --lib`
Expected: **compile error** — no `Tok::Colon` / no `Module.fns` / `Type` not in scope. This is the RED state.

- [ ] **Step 3: Lex `:` and `->`**

In `crates/tamal-lang/src/lexer.rs`, add two variants to `Tok` (after `Eq`):

```rust
    /// `:` (a parameter's type annotation)
    Colon,
    /// `->` (a `fn`'s return type)
    Arrow,
```

and two arms to the `match c` in `lex`, immediately **before** the `b'^' => {` arm:

```rust
            b':' => {
                toks.push(Token {
                    kind: Tok::Colon,
                    span: i..i + 1,
                });
                i += 1;
            }
            b'-' if i + 1 < b.len() && b[i + 1] == b'>' => {
                toks.push(Token {
                    kind: Tok::Arrow,
                    span: i..i + 2,
                });
                i += 2;
            }
```

(A bare `-` still falls through to the "unexpected character" arm — negative literals are not part of this plan.)

- [ ] **Step 4: Add the `FnDef` AST node and the `fns` module field**

In `crates/tamal-lang/src/parser.rs`, add after the `Param` struct:

```rust
/// A `fn NAME(params) -> type { expr }` item: a pure, compile-time function.
///
/// A call is replaced by the value the body evaluates to — the tamal ISA has no
/// `call`/`ret` and no stack, so there is no runtime linkage to invent (D2).
#[derive(Debug, Clone)]
pub struct FnDef {
    pub name: String,
    pub name_span: Span,
    pub params: Vec<Param>,
    /// The declared return type; the body's value is checked against it.
    pub ret: Type,
    /// The body — a single expression.
    pub body: Expr,
}
```

Add the field to `Module`:

```rust
/// A parsed `.tam` module: `const` items, `fn` items, and tests.
#[derive(Debug, Clone)]
pub struct Module {
    pub consts: Vec<Const>,
    pub fns: Vec<FnDef>,
    pub tests: Vec<Test>,
}
```

- [ ] **Step 5: Parse `fn` items**

In `crates/tamal-lang/src/parser.rs`, update `parse` to collect them:

```rust
pub fn parse(src: &str, toks: &[Token]) -> Result<Module, Vec<Diagnostic>> {
    let mut p = P { src, toks, i: 0 };
    let mut consts = Vec::new();
    let mut fns = Vec::new();
    let mut tests = Vec::new();
    p.skip_newlines();
    while p.peek() != Tok::Eof {
        let kw = p.expect_ident()?;
        match p.lexeme(&kw) {
            "const" => consts.push(p.parse_const()?),
            "fn" => fns.push(p.parse_fn()?),
            "test" => tests.push(p.parse_test()?),
            other => {
                return Err(vec![Diagnostic::error(
                    kw,
                    format!("expected `const`, `fn`, or `test`, found `{other}`"),
                )]);
            }
        }
        p.skip_newlines();
    }
    Ok(Module { consts, fns, tests })
}
```

Add the three parsers to `impl<'a> P<'a>`, right after `parse_const`:

```rust
    fn parse_fn(&mut self) -> Result<FnDef, Vec<Diagnostic>> {
        let name_span = self.expect_ident()?;
        let name = self.lexeme(&name_span).to_string();
        self.expect(Tok::LParen, "`(`")?;
        let params = self.parse_params()?;
        self.expect(Tok::RParen, "`)`")?;
        self.expect(Tok::Arrow, "`->` and a return type")?;
        let ret = self.parse_type()?;
        self.expect(Tok::LBrace, "`{`")?;
        self.skip_newlines();
        let body = self.parse_expr()?;
        self.skip_newlines();
        self.expect(Tok::RBrace, "`}`")?;
        self.end_stmt()?;
        Ok(FnDef {
            name,
            name_span,
            params,
            ret,
            body,
        })
    }

    /// Parse a parameter list up to but not including the closing `)` (the `(`
    /// is already consumed). Newlines inside the parens are not statement
    /// terminators and a trailing comma is allowed.
    fn parse_params(&mut self) -> Result<Vec<Param>, Vec<Diagnostic>> {
        let mut params = Vec::new();
        self.skip_newlines();
        while self.peek() != Tok::RParen {
            let name_span = self.expect_ident()?;
            let name = self.lexeme(&name_span).to_string();
            self.expect(Tok::Colon, "`:` and a type")?;
            let ty = self.parse_type()?;
            let mut end = name_span.end;
            let mut default = None;
            if self.peek() == Tok::Eq {
                self.i += 1;
                let e = self.parse_expr()?;
                end = e.span().end;
                default = Some(e);
            }
            params.push(Param {
                name,
                ty,
                default,
                span: name_span.start..end,
            });
            self.skip_newlines();
            if self.peek() == Tok::Comma {
                self.i += 1;
                self.skip_newlines();
            } else {
                break;
            }
        }
        Ok(params)
    }

    fn parse_type(&mut self) -> Result<Type, Vec<Diagnostic>> {
        let sp = self.expect_ident()?;
        match self.lexeme(&sp) {
            "byte" => Ok(Type::Byte),
            "int" => Ok(Type::Int),
            "bytes" => Ok(Type::Bytes),
            other => Err(vec![
                Diagnostic::error(sp, format!("unknown type `{other}`"))
                    .with_help("the types are byte, int, bytes"),
            ]),
        }
    }
```

- [ ] **Step 6: Fix the existing parser test whose message changed**

Still in `parser.rs`'s `mod tests`, relax `missing_test_keyword_is_an_error` so it survives this task *and* Task 6 (which adds `proc` to the same message):

```rust
    #[test]
    fn missing_test_keyword_is_an_error() {
        let toks = lex("smoke {\n  pass\n}\n").unwrap();
        let err = parse("smoke {\n  pass\n}\n", &toks).unwrap_err();
        assert!(err[0].message.contains("expected `const`"));
    }
```

- [ ] **Step 7: Add the `fn` table to `Env` and evaluate `fn` calls**

In `crates/tamal-lang/src/consteval.rs`, extend the import:

```rust
use crate::parser::{Arg, BinOp, Expr, FnDef, Param, Type};
```

Add the builtin list just above the `Env` struct:

```rust
/// The compile-time builtins. A user `fn` may not take one of these names — a
/// builtin must always mean the same thing.
pub const BUILTINS: [&str; 4] = ["crc8", "len", "lo", "hi"];
```

Add two fields to `Env`:

```rust
    /// The module's `fn` table; calls resolve against it.
    fns: HashMap<String, FnDef>,
    /// The callables whose expansion is in progress, innermost last. Every call
    /// is inlined, so a name that appears twice is recursion — rejected,
    /// because the ISA has no stack to recurse on.
    active: Vec<String>,
```

and these methods to `impl Env` (after `child_for_call_values`):

```rust
    /// Define a `fn`. Returns `false` if one of that name already exists.
    pub fn define_fn(&mut self, f: FnDef) -> bool {
        if self.fns.contains_key(&f.name) {
            return false;
        }
        self.fns.insert(f.name.clone(), f);
        true
    }

    /// The `fn` bound to `name`, if any.
    pub fn get_fn(&self, name: &str) -> Option<&FnDef> {
        self.fns.get(name)
    }

    /// Is a call to `name` already in progress (i.e. would this recurse)?
    pub fn is_active(&self, name: &str) -> bool {
        self.active.iter().any(|n| n == name)
    }

    /// The environment a call body is evaluated in: the module scope plus
    /// `bindings`, with `callee` pushed onto the in-progress chain.
    pub fn child_for_call(&self, callee: &str, bindings: HashMap<String, Value>) -> Env {
        let mut e = self.child_for_call_values(bindings);
        e.active.push(callee.to_string());
        e
    }
```

Replace the `_ =>` arm of `eval_call` so an unknown builtin falls through to the `fn` table:

```rust
        _ => match env.get_fn(func) {
            Some(f) => eval_fn_call(f, args, span, env),
            None => Err(
                Diagnostic::error(span.clone(), format!("unknown function `{func}`")).with_help(
                    "the builtins are crc8, len, lo, hi; a `proc` emits instructions and is called as a statement, not inside an expression",
                ),
            ),
        },
```

and add the evaluator right below `eval_call`:

```rust
/// Evaluate a `fn` call: bind the arguments, evaluate the body in the callee's
/// own scope, and check the result against the declared return type. There is
/// no runtime call — the value simply replaces the call site.
fn eval_fn_call(f: &FnDef, args: &[Arg], span: &Span, env: &Env) -> Result<Value, Diagnostic> {
    if env.is_active(&f.name) {
        return Err(Diagnostic::error(
            span.clone(),
            format!("`{}` is already being expanded: recursion is not possible", f.name),
        )
        .with_help("every call is inlined — the tamal ISA has no call/ret and no stack"));
    }
    let bindings = bind_args(&f.name, &f.params, args, env, span)?;
    let child = env.child_for_call(&f.name, bindings);
    let v = eval(&f.body, &child)?;
    check_type(
        &v,
        f.ret,
        &f.body.span(),
        &format!("the body of `{}`", f.name),
    )?;
    Ok(v)
}
```

- [ ] **Step 8: Install the table in the driver**

In `crates/tamal-lang/src/lib.rs`, insert this block **before** the `const` resolution loop (a `const` may call a `fn`, so the table must exist first):

```rust
    let mut env = consteval::Env::new();
    // Install the `fn` table first: a `const` may be built by a compile-time
    // helper, and a `fn` body is only evaluated when it is called (by which
    // time every `const` it names is resolved).
    for f in &module.fns {
        if consteval::BUILTINS.contains(&f.name.as_str()) {
            return Err(vec![
                Diagnostic::error(
                    f.name_span.clone(),
                    format!("`{}` is a builtin and cannot be redefined", f.name),
                )
                .with_help("the builtins are crc8, len, lo, hi"),
            ]);
        }
        if !env.define_fn(f.clone()) {
            return Err(vec![Diagnostic::error(
                f.name_span.clone(),
                format!("duplicate fn `{}`", f.name),
            )]);
        }
    }
```

and delete the now-duplicated `let mut env = consteval::Env::new();` line that Task 1 left at the head of the `const` loop.

- [ ] **Step 9: Add the new field to the two test `Module` literals**

In `crates/tamal-lang/src/emit.rs`'s `mod tests`, both `Module { … }` literals (in the `one` helper and in `remap_points_asm_span_at_originating_tam_span`) need the new field:

```rust
            consts: vec![],
            fns: vec![],
```

- [ ] **Step 10: Run to verify they pass**

Run: `cargo test -p tamal-lang`
Expected: PASS — `0 failed`, including the ten new tests (`lexes_colon_and_arrow`, `parses_fn_item`, `parses_fn_param_default`, `rejects_an_unknown_param_type`, `fn_call_folds_to_its_returned_value`, `a_const_can_call_a_fn`, `a_recursive_fn_is_rejected`, `a_fn_body_must_produce_its_declared_type`, `a_fn_may_not_shadow_a_builtin_or_repeat_a_name`).

- [ ] **Step 11: Commit**

```bash
cargo fmt -p tamal-lang
git add crates/tamal-lang/src/lexer.rs crates/tamal-lang/src/parser.rs crates/tamal-lang/src/consteval.rs crates/tamal-lang/src/emit.rs crates/tamal-lang/src/lib.rs
git commit -m "feat(tamal-lang): fn items — pure compile-time callables"
```

---

## Task 5: A frame *stack* on the `Emitter` (behavior-preserving)

Plan 3 threaded the deferred-verdict list as a `&mut Vec<Deferred>` parameter through a second, near-duplicate statement matcher. A `proc` called *inside* a `frame` has to reach that same list, and threading the parameter through an inline expansion is unpleasant — so the list moves onto the `Emitter` as a stack and the two matchers collapse into one. **No behavior change:** Plan 3's goldens pin the output exactly.

**Files:**
- Modify: `crates/tamal-lang/src/emit.rs`

- [ ] **Step 1: Add the frame stack field**

In `crates/tamal-lang/src/emit.rs`, add to `struct Emitter` (after `trailers`):

```rust
    /// One entry per `frame` currently being lowered, innermost last; each
    /// holds the verdicts its `expect`s deferred to the frame exit. A stack
    /// (rather than a parameter) so an inlined `proc` body reaches the
    /// enclosing frame's list without threading it through the expansion.
    frames: Vec<Vec<Deferred>>,
```

and initialise it in `Emitter::new`:

```rust
            frames: Vec::new(),
```

- [ ] **Step 2: Replace both statement matchers with one**

Still in `emit.rs`, replace the whole `top_stmt` method **and** the whole `frame_body_stmt` method with this single dispatcher plus its context predicate:

```rust
    /// Is a `frame` currently open?
    fn in_frame(&self) -> bool {
        !self.frames.is_empty()
    }

    /// Lower one statement.
    ///
    /// `entry` is the test's name span, used by statements (like `pass`) that
    /// have no more specific span of their own. Legality is context-dependent:
    /// `pass`/`fail`/`config`/`frame` are rejected inside a `frame`, and
    /// `expect` is rejected outside one. The match is wildcard-free by design,
    /// so a new `Stmt` variant forces a decision here.
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
        }
        Ok(())
    }
```

- [ ] **Step 3: Rework `lower_frame` onto the stack**

Replace the whole `lower_frame` method with:

```rust
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
```

- [ ] **Step 4: Give `expect` its own method**

Add this method to `impl Emitter` (after `lower_wait_state`). Its body is the `Stmt::Expect` arm that used to live inside `frame_body_stmt`, plus the out-of-frame check that used to live in `top_stmt`:

```rust
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
        self.frames
            .last_mut()
            .expect("in_frame() was just checked")
            .push(Deferred {
                reg: res,
                label,
                code,
                span: span.clone(),
            });
        Ok(())
    }
```

- [ ] **Step 5: Update the import, the entry loop, and `stmt_span`**

Change the parser import at the top of `emit.rs` to bring `Expr` in:

```rust
use crate::parser::{Expr, Module, Stmt};
```

In the free `emit` function, call the renamed dispatcher:

```rust
        for stmt in &test.stmts {
            e.stmt(stmt, &test.name_span)?;
        }
```

Replace the free `stmt_span` helper so `pass` gets a real span:

```rust
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
        | Stmt::Expect { span, .. } => span.clone(),
    }
}
```

- [ ] **Step 6: Run the whole crate to verify NO behavior change**

Run: `cargo test -p tamal-lang`
Expected: PASS — `0 failed`, same counts as after Task 4. The Plan-3 goldens are what prove this refactor: `frame_wraps_body_in_cs_assert_deassert`, `expect_crc_defers_the_verdict_past_cs_deassert`, `expect_crc_outside_a_frame_is_an_error`, `config_inside_a_frame_is_rejected`, and both `frames.rs` capstones must all still pass **unchanged**.

- [ ] **Step 7: Commit**

```bash
cargo fmt -p tamal-lang
git add crates/tamal-lang/src/emit.rs
git commit -m "refactor(tamal-lang): frame stack on the Emitter (no behavior change)"
```

---

## Task 6: `proc` items — hygienic inline expansion

`proc NAME(params) { stmts }`, expanded **inline** at every call site with fresh registers and gensym'd labels. Legal at the top level of a test *and* inside a `frame` (the library `command` proc contains its own `frame`; a smaller helper may be called from inside one).

**A hazard this task must close.** An `expect` inside an expansion latches the RX residue into a register allocated in the *expansion's* scope, but the branch on it runs at the enclosing **frame's** exit. If the expansion's `exit_scope` simply released that register, a later statement in the frame could reuse it and clobber the residue before the verdict reads it. `RegAlloc::reserve` + `Emitter::reserve_deferred` close that hole; Step 1's `a_verdict_latched_inside_a_proc_survives_later_allocations` is the regression test.

**Files:**
- Modify: `crates/tamal-lang/src/regalloc.rs`
- Modify: `crates/tamal-lang/src/parser.rs`
- Modify: `crates/tamal-lang/src/emit.rs`
- Modify: `crates/tamal-lang/src/lib.rs`

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `crates/tamal-lang/src/regalloc.rs`:

```rust
    #[test]
    fn reserve_retakes_a_register_whose_scope_has_ended() {
        // The `expect`-inside-a-`proc` case: a value is latched in a nested
        // scope but read after that scope ends, so the register must be re-taken
        // or a later allocation would clobber it.
        let mut a = RegAlloc::new();
        a.enter_scope();
        let latched = a.temp(&(0..0)).unwrap(); // x1, owned by the inner scope
        a.exit_scope(); // x1 would now be free …
        a.reserve(latched); // … but the value is still live
        assert_eq!(a.temp(&(0..0)).unwrap(), r(2), "must not hand out x1 again");
    }
```

Add to `mod tests` in `crates/tamal-lang/src/parser.rs`:

```rust
    #[test]
    fn parses_proc_item() {
        let m = parse_ok(
            "proc command(pkt: bytes, ndata: int, err: byte = 0x11) {\n send pkt + crc8\n tar 2\n}\ntest t {\n pass\n}\n",
        );
        assert_eq!(m.procs.len(), 1);
        let p = &m.procs[0];
        assert_eq!(p.name, "command");
        assert_eq!(p.params.len(), 3);
        assert!(p.params[2].default.is_some());
        assert_eq!(p.body.len(), 2);
    }

    #[test]
    fn parses_a_call_statement() {
        let m = parse_ok("test t {\n command(pkt = [0x44], 0)\n pass\n}\n");
        match &m.tests[0].stmts[0] {
            Stmt::Call { name, args, .. } => {
                assert_eq!(name, "command");
                assert_eq!(args.len(), 2);
                assert_eq!(args[0].name.as_deref(), Some("pkt"));
                assert!(args[1].name.is_none());
            }
            s => panic!("expected Call, got {s:?}"),
        }
    }
```

Add to `mod tests` in `crates/tamal-lang/src/lib.rs`:

```rust
    #[test]
    fn a_proc_call_inlines_its_body() {
        let asm = lower_to_asm(
            "proc cfg() { config controller, x1, sck20, alert_pin }\n\
             test t {\n cfg()\n pass\n}\n",
        )
        .unwrap();
        assert_eq!(
            asm,
            ".globl _start\n_start:\n\tset_config controller, x1, sck20, alert_pin\n\thalt 0x00\n"
        );
    }

    #[test]
    fn proc_parameters_are_visible_in_its_body() {
        let asm = lower_to_asm(
            "proc emit_pkt(pkt: bytes) { send pkt + crc8 }\n\
             test t {\n emit_pkt([0x44, 0x00, 0x64])\n pass\n}\n",
        )
        .unwrap();
        assert_eq!(
            asm,
            ".globl _start\n_start:\n\
             \tput_byte 0x44\n\tput_byte 0x00\n\tput_byte 0x64\n\tput_byte 0x16\n\
             \thalt 0x00\n"
        );
    }

    #[test]
    fn two_expansions_get_fresh_registers_and_labels() {
        // Hygiene (D2/D5): each expansion re-uses the same registers (the
        // previous one released them) but never the same label.
        let asm = lower_to_asm("proc poll() { wait_state }\ntest t {\n poll()\n poll()\n pass\n}\n")
            .unwrap();
        assert_eq!(
            asm,
            ".globl _start\n_start:\n\
             __wait0:\n\tcrc_reset\n\tget_byte x1\n\tli x2, 0x0F\n\tbeq x1, x2, __wait0\n\
             __wait1:\n\tcrc_reset\n\tget_byte x1\n\tli x2, 0x0F\n\tbeq x1, x2, __wait1\n\
             \thalt 0x00\n"
        );
    }

    #[test]
    fn a_proc_expansion_cannot_clobber_a_live_caller_register() {
        let asm = lower_to_asm(
            "proc read() { recv inner }\n\
             test t {\n recv keep\n read()\n recv after\n pass\n}\n",
        )
        .unwrap();
        let gets: Vec<&str> = asm.lines().filter(|l| l.contains("get_byte")).collect();
        // `keep` holds x1 across the call, so the callee is handed x2 — and
        // releases it on exit, so the next binding reuses x2.
        assert_eq!(gets, vec!["\tget_byte x1", "\tget_byte x2", "\tget_byte x2"]);
    }

    #[test]
    fn a_proc_call_inside_a_frame_defers_its_expect_to_that_frame() {
        let asm = lower_to_asm(
            "proc verify() { expect crc else 0x11 }\n\
             test t {\n frame {\n  verify()\n }\n pass\n}\n",
        )
        .unwrap();
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
    fn a_verdict_latched_inside_a_proc_survives_later_allocations() {
        // The residue lives in the proc's scope but is branched on at the
        // frame's exit: the `recv` after the call must NOT be handed x1.
        let asm = lower_to_asm(
            "proc verify() { expect crc else 0x11 }\n\
             test t {\n frame {\n  verify()\n  recv extra\n }\n pass\n}\n",
        )
        .unwrap();
        assert_eq!(
            asm,
            ".globl _start\n_start:\n\
             \tcs_assert\n\
             \tget_byte x1\n\trdsr x1, crc\n\
             \tget_byte x2\n\
             \tcs_deassert\n\
             \tbnez x1, __fail0\n\
             \thalt 0x00\n\
             __fail0:\n\thalt 0x11\n"
        );
    }

    #[test]
    fn a_recursive_proc_is_rejected() {
        let err = lower_to_asm("proc p() { p() }\ntest t {\n p()\n pass\n}\n").unwrap_err();
        assert!(
            err[0].message.contains("already being expanded"),
            "got: {:?}",
            err[0]
        );
    }

    #[test]
    fn a_fn_cannot_be_called_as_a_statement() {
        let err = lower_to_asm(
            "fn f(n: int) -> int { n }\ntest t {\n f(1)\n pass\n}\n",
        )
        .unwrap_err();
        assert!(err[0].message.contains("is a `fn`"), "got: {:?}", err[0]);
    }

    #[test]
    fn an_unknown_call_is_rejected() {
        let err = lower_to_asm("test t {\n nope()\n pass\n}\n").unwrap_err();
        assert!(err[0].message.contains("unknown `proc`"), "got: {:?}", err[0]);
    }

    #[test]
    fn a_proc_may_not_collide_with_a_fn() {
        let err = lower_to_asm(
            "fn dup(n: int) -> int { n }\nproc dup() { cs_assert }\ntest t {\n pass\n}\n",
        )
        .unwrap_err();
        assert!(
            err[0].message.contains("already defined as a `fn`"),
            "got: {:?}",
            err[0]
        );
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p tamal-lang --lib`
Expected: **compile error** — no `RegAlloc::reserve`, no `Module.procs`, no `Stmt::Call`. This is the RED state.

- [ ] **Step 3: Add `RegAlloc::reserve`**

In `crates/tamal-lang/src/regalloc.rs`, add this method to `impl RegAlloc` (after `free`):

```rust
    /// Re-take `reg` in the **current** scope, for a value whose allocating
    /// scope has ended but which is still live.
    ///
    /// The case this exists for: an `expect` inside an inlined `proc` latches
    /// the RX residue into a register owned by the expansion's scope, but the
    /// verdict branches on it at the enclosing `frame`'s exit. Without this the
    /// register would look free and a later statement could clobber the value
    /// before it is read. Idempotent.
    pub fn reserve(&mut self, reg: Reg) {
        if let Some(slot) = self.busy.get_mut(reg.bits() as usize) {
            *slot = true;
        }
        if let Some(scope) = self.scopes.last_mut() {
            if !scope.regs.contains(&reg) {
                scope.regs.push(reg);
            }
        }
    }
```

- [ ] **Step 4: Add the `ProcDef` / `Stmt::Call` AST**

In `crates/tamal-lang/src/parser.rs`, add after the `FnDef` struct:

```rust
/// A `proc NAME(params) { stmts }` item: a procedure that emits bus activity.
///
/// It is **inlined** at every call site — the tamal ISA has no `call`/`ret`, no
/// stack and no data memory, so there is no runtime linkage to invent (D2).
#[derive(Debug, Clone)]
pub struct ProcDef {
    pub name: String,
    pub name_span: Span,
    pub params: Vec<Param>,
    pub body: Vec<Stmt>,
}
```

Add the `procs` field to `Module`:

```rust
pub struct Module {
    pub consts: Vec<Const>,
    pub fns: Vec<FnDef>,
    pub procs: Vec<ProcDef>,
    pub tests: Vec<Test>,
}
```

and add the statement variant to `Stmt` (after `Expect`):

```rust
    /// `name(args)` — a `proc` call, expanded inline at this point.
    Call {
        name: String,
        name_span: Span,
        args: Vec<Arg>,
        span: Span,
    },
```

- [ ] **Step 5: Share one block parser, and parse `proc`**

In `crates/tamal-lang/src/parser.rs`, add this method to `impl<'a> P<'a>` (right before `parse_test`):

```rust
    /// Parse statements up to and including the matching `}` (the `{` is
    /// already consumed); returns the body and the closing brace's end offset.
    /// `what` names the construct in the unterminated-block diagnostic.
    fn parse_block(&mut self, what: &str) -> Result<(Vec<Stmt>, usize), Vec<Diagnostic>> {
        let mut body = Vec::new();
        loop {
            self.skip_newlines();
            match self.peek() {
                Tok::RBrace => {
                    let end = self.span().end;
                    self.i += 1;
                    return Ok((body, end));
                }
                Tok::Eof => {
                    return Err(vec![Diagnostic::error(
                        self.span(),
                        format!("unexpected end of file: missing `}}` for `{what}`"),
                    )]);
                }
                _ => body.push(self.parse_stmt()?),
            }
        }
    }
```

Rewrite `parse_test` to use it:

```rust
    fn parse_test(&mut self) -> Result<Test, Vec<Diagnostic>> {
        let name_span = self.expect_ident()?;
        let name = self.lexeme(&name_span).to_string();
        self.expect(Tok::LBrace, "`{`")?;
        let (stmts, _) = self.parse_block("test")?;
        Ok(Test {
            name,
            name_span,
            stmts,
        })
    }
```

Replace the body of the `"frame"` arm in `parse_stmt` with the shared version:

```rust
            "frame" => {
                self.expect(Tok::LBrace, "`{`")?;
                let (body, end) = self.parse_block("frame")?;
                self.end_stmt()?;
                Ok(Stmt::Frame {
                    body,
                    span: head.start..end,
                })
            }
```

Add `parse_proc` next to `parse_fn`:

```rust
    fn parse_proc(&mut self) -> Result<ProcDef, Vec<Diagnostic>> {
        let name_span = self.expect_ident()?;
        let name = self.lexeme(&name_span).to_string();
        self.expect(Tok::LParen, "`(`")?;
        let params = self.parse_params()?;
        self.expect(Tok::RParen, "`)`")?;
        self.expect(Tok::LBrace, "`{`")?;
        let (body, _) = self.parse_block("proc")?;
        self.end_stmt()?;
        Ok(ProcDef {
            name,
            name_span,
            params,
            body,
        })
    }
```

and wire it into `parse`:

```rust
    let mut consts = Vec::new();
    let mut fns = Vec::new();
    let mut procs = Vec::new();
    let mut tests = Vec::new();
```

```rust
            "const" => consts.push(p.parse_const()?),
            "fn" => fns.push(p.parse_fn()?),
            "proc" => procs.push(p.parse_proc()?),
            "test" => tests.push(p.parse_test()?),
            other => {
                return Err(vec![Diagnostic::error(
                    kw,
                    format!("expected `const`, `fn`, `proc`, or `test`, found `{other}`"),
                )]);
            }
```

```rust
    Ok(Module {
        consts,
        fns,
        procs,
        tests,
    })
```

- [ ] **Step 6: Parse the call statement**

> ⚠ **EDIT HAZARD.** `parse_stmt` contains *two* 12-space `_ => {` arms — the raw-instruction fallback and a deeper one inside `crc_region`. Anchor this edit on the **two-line** needle below (the second line is unique to the raw arm), then re-read `crc_region` to confirm it is untouched.

In `crates/tamal-lang/src/parser.rs`, find:

```rust
            _ => {
                let mut operands = Vec::new();
```

and insert the call check between those two lines, so the arm begins:

```rust
            _ => {
                // `name(...)` is a `proc` call, not a raw instruction: an asm
                // operand never starts with `(`.
                if self.peek() == Tok::LParen {
                    self.i += 1;
                    let args = self.parse_args()?;
                    let close = self.expect(Tok::RParen, "`)`")?;
                    let span = head.start..close.span.end;
                    self.end_stmt()?;
                    return Ok(Stmt::Call {
                        name: word,
                        name_span: head,
                        args,
                        span,
                    });
                }
                let mut operands = Vec::new();
```

- [ ] **Step 7: Expand `proc` calls in the emitter**

In `crates/tamal-lang/src/emit.rs`, extend the imports:

```rust
use crate::parser::{Arg, Expr, Module, ProcDef, Stmt};
```

and add, next to the other `use` lines:

```rust
use std::collections::HashMap;
```

Add two fields to `struct Emitter` (after `frames`):

```rust
    /// The module's `proc` table. `proc`s live here rather than in the `Env`
    /// because only emit expands them (a `proc` call is a statement); `fn`s
    /// live in the `Env` because consteval resolves them (a `fn` call is an
    /// expression). Duplicates were already rejected by the driver.
    procs: HashMap<String, ProcDef>,
    /// The `proc`s whose expansion is in progress, innermost last — a name that
    /// appears twice is recursion, which cannot be inlined.
    active_procs: Vec<String>,
```

Give `Emitter::new` the module so it can build the table, and update the one caller in `emit`:

```rust
pub fn emit(module: &Module, env: Env) -> Result<Lowering, Vec<Diagnostic>> {
    let mut e = Emitter::new(env, module);
```

```rust
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
```

Add the expansion and its register-safety helper to `impl Emitter` (after `lower_expect`):

```rust
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
```

Add the dispatch arm to `stmt` (after the `Stmt::Expect` arm) — note a call is legal both at the top level and inside a `frame`, so it is **not** in the in-frame rejection list:

```rust
            Stmt::Call {
                name,
                name_span,
                args,
                span,
            } => self.lower_call(name, name_span, args, span, entry)?,
```

and the span arm to `stmt_span`, in the span-bearing group:

```rust
        | Stmt::Call { span, .. } => span.clone(),
```

- [ ] **Step 8: Validate `proc` names and extend the halt scan**

In `crates/tamal-lang/src/lib.rs`, add this loop immediately after the `fn`-table loop from Task 4:

```rust
    // `proc`s are expanded by the emitter, but their names are validated here,
    // beside the `fn`s, so every callable-name collision is caught in one place.
    for (i, p) in module.procs.iter().enumerate() {
        if consteval::BUILTINS.contains(&p.name.as_str()) {
            return Err(vec![Diagnostic::error(
                p.name_span.clone(),
                format!("`{}` is a builtin and cannot be redefined", p.name),
            )]);
        }
        if env.get_fn(&p.name).is_some() {
            return Err(vec![Diagnostic::error(
                p.name_span.clone(),
                format!("`{}` is already defined as a `fn`", p.name),
            )]);
        }
        if module.procs[..i].iter().any(|q| q.name == p.name) {
            return Err(vec![Diagnostic::error(
                p.name_span.clone(),
                format!("duplicate proc `{}`", p.name),
            )]);
        }
    }
```

Add the new variant to the M2 `halts` match:

```rust
        parser::Stmt::Call { .. } => false,
```

and replace that check's help line so the conservatism is visible to the user:

```rust
            .with_help(
                "a test must reach `pass`, `fail`, or a `halt` at the top level of the test — a verdict inside a `frame` or `proc` body is not counted",
            ),
```

- [ ] **Step 9: Run to verify they pass**

Run: `cargo test -p tamal-lang`
Expected: PASS — `0 failed`, including all thirteen new tests. If `a_verdict_latched_inside_a_proc_survives_later_allocations` fails with `get_byte x1` where `x2` is expected, `reserve_deferred` is not being called after `exit_scope` in `lower_call`.

- [ ] **Step 10: Commit**

```bash
cargo fmt -p tamal-lang
git add crates/tamal-lang/src/regalloc.rs crates/tamal-lang/src/parser.rs crates/tamal-lang/src/emit.rs crates/tamal-lang/src/lib.rs
git commit -m "feat(tamal-lang): proc items with hygienic inline expansion"
```

---

## Task 7: `repeat N { … }` — compile-time unroll

No loop counter, no branch: the body is emitted `N` times (spec §5). `N` is any compile-time integer expression — including a `proc` parameter, which is exactly how the library `command` proc reads a variable payload (`repeat ndata { recv _ }`).

**Files:**
- Modify: `crates/tamal-lang/src/lib.rs`
- Modify: `crates/tamal-lang/src/parser.rs`
- Modify: `crates/tamal-lang/src/emit.rs`

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `crates/tamal-lang/src/parser.rs`:

```rust
    #[test]
    fn parses_repeat() {
        let m = parse_ok("test t {\n repeat 3 {\n  recv _\n }\n pass\n}\n");
        match &m.tests[0].stmts[0] {
            Stmt::Repeat { body, .. } => assert_eq!(body.len(), 1),
            s => panic!("expected Repeat, got {s:?}"),
        }
    }
```

Add to `mod tests` in `crates/tamal-lang/src/lib.rs`:

```rust
    #[test]
    fn repeat_unrolls_at_compile_time() {
        let asm = lower_to_asm("test t {\n repeat 3 {\n  recv _\n }\n pass\n}\n").unwrap();
        assert_eq!(
            asm,
            ".globl _start\n_start:\n\tget_byte x1\n\tget_byte x1\n\tget_byte x1\n\thalt 0x00\n"
        );
    }

    #[test]
    fn repeat_zero_emits_nothing() {
        // The OOB write-completion case: `ndata = 0` contributes no reads.
        let asm = lower_to_asm("test t {\n repeat 0 {\n  recv _\n }\n pass\n}\n").unwrap();
        assert_eq!(asm, ".globl _start\n_start:\n\thalt 0x00\n");
    }

    #[test]
    fn a_repeat_count_may_come_from_a_proc_parameter() {
        let asm = lower_to_asm(
            "proc payload(n: int) { repeat n { recv _ } }\n\
             test t {\n payload(2)\n pass\n}\n",
        )
        .unwrap();
        assert_eq!(
            asm,
            ".globl _start\n_start:\n\tget_byte x1\n\tget_byte x1\n\thalt 0x00\n"
        );
    }

    #[test]
    fn each_repeat_iteration_gets_its_own_register_scope() {
        // `a` is released at the end of each iteration, so the next one reuses
        // the same register instead of leaking a fresh one.
        let asm = lower_to_asm("test t {\n repeat 2 {\n  recv a\n }\n pass\n}\n").unwrap();
        let gets: Vec<&str> = asm.lines().filter(|l| l.contains("get_byte")).collect();
        assert_eq!(gets, vec!["\tget_byte x1", "\tget_byte x1"]);
    }

    #[test]
    fn an_oversized_repeat_is_rejected() {
        let err = lower_to_asm("test t {\n repeat 99999999 {\n  cs_assert\n }\n pass\n}\n")
            .unwrap_err();
        assert!(err[0].message.contains("not in 0..=1024"), "got: {:?}", err[0]);
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p tamal-lang --lib repeat`
Expected: **compile error** — no `Stmt::Repeat`. This is the RED state.

- [ ] **Step 3: Add the unroll cap**

In `crates/tamal-lang/src/lib.rs`, add just below the `pub use` lines near the top:

```rust
/// The largest compile-time unroll (`repeat N`, `recv N`) the compiler accepts.
///
/// A tamal program is capped at 1024 words, so a larger unroll can never
/// assemble; rejecting it up front turns what would be an out-of-memory into a
/// diagnostic.
pub const MAX_UNROLL: i64 = 1024;
```

- [ ] **Step 4: Add the `Stmt::Repeat` variant and parse it**

In `crates/tamal-lang/src/parser.rs`, add to `Stmt` (after `Call`):

```rust
    /// `repeat N { … }` — a compile-time unroll: the body is emitted `N` times.
    /// There is no loop counter and no branch.
    Repeat {
        count: Expr,
        body: Vec<Stmt>,
        span: Span,
    },
```

Add the keyword arm to `parse_stmt`. Anchor it immediately **after** the closing brace of the `"expect" => { … }` arm (a unique, named sibling — do not anchor on a bare `_ => {`):

```rust
            "repeat" => {
                let count = self.parse_expr()?;
                self.expect(Tok::LBrace, "`{`")?;
                let (body, end) = self.parse_block("repeat")?;
                self.end_stmt()?;
                Ok(Stmt::Repeat {
                    count,
                    body,
                    span: head.start..end,
                })
            }
```

- [ ] **Step 5: Lower it**

In `crates/tamal-lang/src/emit.rs`, add the dispatch arm to `stmt` (after the `Stmt::Call` arm) — `repeat` is legal at the top level, inside a `frame`, and inside a `proc`, so it is not in the in-frame rejection list:

```rust
            Stmt::Repeat { count, body, span } => self.lower_repeat(count, body, span, entry)?,
```

Add the span arm to `stmt_span`'s span-bearing group:

```rust
        | Stmt::Repeat { span, .. } => span.clone(),
```

and add the method to `impl Emitter` (after `reserve_deferred`):

```rust
    /// `repeat N { … }` — a compile-time unroll (spec §5): emit the body `N`
    /// times. There is no loop counter and no branch.
    ///
    /// Each iteration gets its own register scope, so a binding made in the
    /// body is released before the next iteration reuses the register — and
    /// `reserve_deferred` keeps any verdict latched inside the body alive for
    /// the enclosing frame.
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
                    format!(
                        "`repeat` count {n} is not in 0..={}",
                        crate::MAX_UNROLL
                    ),
                )
                .with_help("a tamal program is at most 1024 words, so a larger unroll can never assemble"),
            ]);
        }
        for _ in 0..n {
            self.alloc.enter_scope();
            let mut result = Ok(());
            for s in body {
                if let Err(e) = self.stmt(s, entry) {
                    result = Err(e);
                    break;
                }
            }
            // Task 6's helper: closes the scope *and* re-takes any register a
            // pending verdict still depends on. Never call `exit_scope` here
            // directly — that reintroduces the residue-liveness hazard.
            self.exit_expansion_scope();
            result?;
        }
        Ok(())
    }
```

**And a global emission budget (added during execution — the per-construct cap is not enough).** Review of Task 6 showed a per-construct `MAX_UNROLL` does **not** bound composition: `repeat 1024 { p() }` where `p` itself contains `repeat 1024 { … }` is 10⁶ statements, and `proc pN() { pN+1() pN+1() }` nested 22 deep is a 95-line file that emits 2²² statements and hangs the compiler with no diagnostic. So the cap alone does not achieve its own stated rationale ("turn an out-of-memory into a diagnostic").

Add a **central** budget to `Emitter::push`, which every emitted line already funnels through:

```rust
/// The largest number of asm lines a single program may emit.
///
/// A tamal program is capped at 1024 words, so anything beyond this can never
/// assemble. The per-construct `MAX_UNROLL` gives a *better message* for the
/// obvious case (`repeat 99999999`), but only a central budget bounds
/// *composition* — nested `repeat`s, or a `proc` that fans out to two calls per
/// level. Without it those hang the compiler with no diagnostic.
const MAX_EMITTED_LINES: usize = 4096;
```

and have `push` refuse to grow past it, returning a diagnostic rather than emitting. Note `push` is currently infallible and is called from many places, so making it fallible ripples; the cheaper shape is a `budget_exceeded: Option<Diagnostic>` latch on the `Emitter` that `push` sets on first overflow and that `emit` checks before returning `Ok`. Either is acceptable — pick the one that keeps the code clearest, and pin it with a test that a fan-out `proc` and a nested `repeat` each produce a diagnostic instead of hanging.

The budget is deliberately looser than 1024 words: `li` can tile to two words and labels/directives emit lines that are not words, so a tight bound would reject legal programs. Its job is to stop unbounded growth, not to replace the assembler's exact cap.

- [ ] **Step 6: Extend the halt scan**

In `crates/tamal-lang/src/lib.rs`, add the variant to the M2 `halts` match:

```rust
        // Conservative, like `Frame` and `Call`: a `repeat` body may execute
        // zero times, so a verdict inside one is never counted as reaching a
        // halt. A verdict belongs at the top level of the test.
        parser::Stmt::Repeat { .. } => false,
```

and extend that check's help line to name the third construct:

```rust
            .with_help(
                "a test must reach `pass`, `fail`, or a `halt` at the top level of the test — a verdict inside a `frame`, `proc`, or `repeat` body is not counted",
            ),
```

- [ ] **Step 7: Run to verify they pass**

Run: `cargo test -p tamal-lang`
Expected: PASS — `0 failed`, including the six new tests.

- [ ] **Step 8: Commit**

```bash
cargo fmt -p tamal-lang
git add crates/tamal-lang/src/lib.rs crates/tamal-lang/src/parser.rs crates/tamal-lang/src/emit.rs
git commit -m "feat(tamal-lang): repeat N compile-time unroll"
```

---

## Task 8: Bound `recv N` with the same cap

Closes a tracked Plan-3 follow-up: `recv 99999999` currently materialises 99,999,999 AST nodes (an out-of-memory) *before* the assembler's 1024-word cap can reject it. `repeat` is now bounded; `recv N` is the same hazard and takes the same bound, at parse time where the count is a literal.

**Files:**
- Modify: `crates/tamal-lang/src/parser.rs`

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `crates/tamal-lang/src/parser.rs`:

```rust
    #[test]
    fn an_oversized_recv_count_is_rejected() {
        // Must be a diagnostic, not 99,999,999 AST nodes.
        let src = "test t {\n recv 99999999\n pass\n}\n";
        let toks = lex(src).unwrap();
        let err = parse(src, &toks).unwrap_err();
        assert!(
            err[0].message.contains("not in 0..=1024"),
            "got: {}",
            err[0].message
        );
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p tamal-lang --lib an_oversized_recv_count_is_rejected`
Expected: FAIL (RED) — the test hangs or is killed while allocating, or fails the assertion. **If it hangs, that is the bug**; interrupt it and continue to Step 3.

- [ ] **Step 3: Apply the cap**

In `crates/tamal-lang/src/parser.rs`, in the `"recv"` arm of `parse_stmt`, insert the bound check between resolving `count` and the loop that pushes the targets:

```rust
                    let count = parse_number(self.lexeme(&n.span))
                        .filter(|&c| c >= 0)
                        .ok_or_else(|| {
                            vec![Diagnostic::error(n.span.clone(), "invalid recv count")]
                        })?;
                    if count > crate::MAX_UNROLL {
                        return Err(vec![
                            Diagnostic::error(
                                n.span.clone(),
                                format!("`recv` count {count} is not in 0..={}", crate::MAX_UNROLL),
                            )
                            .with_help(
                                "a tamal program is at most 1024 words, so a larger read can never assemble",
                            ),
                        ]);
                    }
                    for _ in 0..count {
                        targets.push(RecvTarget::Discard);
                    }
```

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p tamal-lang --lib recv`
Expected: PASS — `an_oversized_recv_count_is_rejected` plus the existing `parses_recv_names_and_discard` / `parses_recv_count`, returning promptly.

- [ ] **Step 5: Commit**

```bash
cargo fmt -p tamal-lang
git add crates/tamal-lang/src/parser.rs
git commit -m "fix(tamal-lang): bound recv N so a huge count is a diagnostic, not an OOM"
```

---

## Task 9: Capstones — a local `command` proc byte-matches two channel examples

The proof that the whole increment composes. One `command` proc — the exact body Plan 5 will move into the bundled `espi` stdlib — is called two ways: with named arguments and `ndata = 0` for the **OOB** channel, and with a `fn`-built header and `ndata = 1` for the **Peripheral** channel. Each must lower to bytecode byte-identical *modulo register allocation* to its hand-written `.s`, with both TX CRCs (`0xB1`, `0x16`) re-derived by `+ crc8` and never typed.

**Files:**
- Create: `crates/tamal-lang/tests/common/mod.rs`
- Modify: `crates/tamal-lang/tests/frames.rs`
- Create: `crates/tamal-lang/tests/callables.rs`

- [ ] **Step 1: Extract the shared comparator**

Create `crates/tamal-lang/tests/common/mod.rs` — this is `canon`/`zero_regs` moved verbatim out of `tests/frames.rs` so both integration tests share one copy. (It must be `common/mod.rs`, not `common.rs`: cargo would compile the latter as its own test binary.)

```rust
//! Shared test helper: compare two programs **modulo register allocation**.
//!
//! Decode each assembled word back to a `tamal_abi::isa::Instr` and zero every
//! register operand, so two programs compare equal when they differ only in
//! which physical registers the allocator happened to choose — while the
//! opcodes, immediates, folded CRC bytes and branch offsets still have to match
//! exactly (spec §9).

use tamal_abi::isa::{Instr, Reg};

/// Decode a program and zero every register operand.
pub fn canon(prog: &tamal_asm::Program) -> Vec<Instr> {
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
```

Then in `crates/tamal-lang/tests/frames.rs`, delete the `use tamal_abi::isa::{Instr, Reg};` line and the whole `canon` + `zero_regs` definitions, and put this in their place (directly under the file's `//!` doc comment):

```rust
mod common;

use common::canon;
```

- [ ] **Step 2: Write the failing capstone tests**

Create `crates/tamal-lang/tests/callables.rs`:

```rust
//! End-to-end: one `proc` — the body Plan 5 will move into the bundled `espi`
//! stdlib — drives two different eSPI channels, and each lowers to bytecode
//! byte-identical *modulo register allocation* to its hand-written `.s`.

mod common;

use common::canon;

/// The shared framing/poll/verify skeleton, written once. Plan 4a has no
/// `import` yet (that is Plan 5), so it lives in the test source; Plan 5 will
/// move exactly this into `espi.tam` and the call sites will not change.
const COMMAND_PROC: &str = "\
proc command(pkt: bytes, ndata: int, err: byte = 0x11) {
    frame {
        send pkt + crc8
        tar 2
        wait_state
        repeat ndata { recv _ }
        recv status0, status1
        expect crc else err
    }
}
";

/// OOB channel: a tunnelled SMBus message. A write completion returns status
/// only, so `ndata = 0`; the arguments are named, as in the design's §4.3.
fn oob_tam() -> String {
    format!(
        "const PUT_OOB = 0x06\n\n{COMMAND_PROC}\n\
test oob_msg {{
    config controller, x1, sck20, alert_pin
    command(
        pkt = [PUT_OOB, 0x21, 0x00, 0x04,
               0x10, 0x00, 0x01, 0xAB],
        ndata = 0,
    )
    pass
}}
"
    )
}

/// Peripheral channel: a short I/O read. The header is built by a `fn`, and the
/// completion carries one payload byte, so `ndata = 1` — passed positionally.
fn peripheral_tam() -> String {
    format!(
        "const PUT_IORD1 = 0x44\n\
fn iord_hdr(op: byte, addr: int) -> bytes {{ [op, hi(addr), lo(addr)] }}\n\n{COMMAND_PROC}\n\
test io_read {{
    config controller, x1, sck20, alert_pin
    command(iord_hdr(PUT_IORD1, 0x0064), 1)
    pass
}}
"
    )
}

#[test]
fn oob_smbus_msg_byte_matches_asm_modulo_registers() {
    let reference = include_str!("../../../examples/oob_smbus_msg.s");
    let got = tamal_lang::compile(&oob_tam()).expect("compile .tam");
    let want = tamal_asm::assemble(reference).expect("assemble reference .s");
    assert_eq!(
        canon(&got),
        canon(&want),
        "the HLL OOB message must match the hand-written .s modulo registers"
    );
}

#[test]
fn oob_smbus_msg_lowers_to_the_expected_asm() {
    // Pins the inline expansion, the folded 0xB1 TX CRC (never typed), the
    // allocator's lowest-free-first numbering, and the gensym labels. The
    // counter is shared, so `wait_state` takes `__wait0` and the `expect` that
    // follows takes `__fail1`.
    let asm = tamal_lang::lower_to_asm(&oob_tam()).expect("lower .tam");
    let expected = "\
.globl _start
_start:
\tset_config controller, x1, sck20, alert_pin
\tcs_assert
\tput_byte 0x06
\tput_byte 0x21
\tput_byte 0x00
\tput_byte 0x04
\tput_byte 0x10
\tput_byte 0x00
\tput_byte 0x01
\tput_byte 0xAB
\tput_byte 0xB1
\ttar 2
__wait0:
\tcrc_reset
\tget_byte x1
\tli x2, 0x0F
\tbeq x1, x2, __wait0
\tget_byte x1
\tget_byte x2
\tget_byte x3
\trdsr x3, crc
\tcs_deassert
\tbnez x3, __fail1
\thalt 0x00
__fail1:
\thalt 0x11
";
    assert_eq!(asm, expected);
}

#[test]
fn peripheral_io_read_via_fn_and_proc_byte_matches_asm_modulo_registers() {
    let reference = include_str!("../../../examples/peripheral_io_read.s");
    let got = tamal_lang::compile(&peripheral_tam()).expect("compile .tam");
    let want = tamal_asm::assemble(reference).expect("assemble reference .s");
    assert_eq!(
        canon(&got),
        canon(&want),
        "the same `command` proc must serve the peripheral read too"
    );
}

#[test]
fn peripheral_io_read_via_fn_and_proc_lowers_to_the_expected_asm() {
    // `iord_hdr(0x44, 0x0064)` folds to [0x44, 0x00, 0x64] and `+ crc8` to
    // 0x16; `repeat 1 { recv _ }` contributes the single payload read.
    let asm = tamal_lang::lower_to_asm(&peripheral_tam()).expect("lower .tam");
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
\tget_byte x1
\tget_byte x2
\tget_byte x3
\trdsr x3, crc
\tcs_deassert
\tbnez x3, __fail1
\thalt 0x00
__fail1:
\thalt 0x11
";
    assert_eq!(asm, expected);
}
```

- [ ] **Step 3: Run them**

Run: `cargo test -p tamal-lang --test callables`
Expected: PASS — `4 passed`. With Tasks 1–8 done these should pass on the first run; that is expected for a composition capstone, whose value is the regression guard and the spec-§9 proof.

If `*_lowers_to_the_expected_asm` fails, the diff pinpoints the lowering mismatch — fix the responsible task's lowering, not the golden, unless the golden itself is wrong. If only `*_byte_matches_*` fails, compare `canon(&got)` against `canon(&want)` instruction by instruction to find the structural divergence.

- [ ] **Step 4: Confirm the moved helper did not break Plan 3's capstone**

Run: `cargo test -p tamal-lang --test frames`
Expected: PASS — `2 passed`, unchanged.

- [ ] **Step 5: Commit**

```bash
cargo fmt -p tamal-lang
git add crates/tamal-lang/tests/common/mod.rs crates/tamal-lang/tests/frames.rs crates/tamal-lang/tests/callables.rs
git commit -m "test(tamal-lang): one command proc byte-matches the OOB and peripheral examples"
```

---

## Task 10: Docs + fmt/clippy/whole-workspace gate

**Files:**
- Modify: `crates/tamal-lang/README.md`

- [ ] **Step 1: Update the crate README**

In `crates/tamal-lang/README.md`, replace the last bullet and the paragraph that follows it in the **Status** section — that is, replace:

```markdown
- **Plan 3** — `config`, `frame { … }`, `recv`, `wait_state`, and
  `expect crc else <byte>`, backed by a compiler-managed register model (see the
  section below). A full peripheral I/O read now compiles byte-identically
  (modulo register allocation) to the hand-written
  `examples/peripheral_io_read.s`.

Still to come: `proc`/`fn` + structured control flow (Plan 4), `import` + the
bundled `espi` stdlib (Plan 5), and `--lint` + compile-time error injection
(Plan 6).
```

with:

```markdown
- **Plan 3** — `config`, `frame { … }`, `recv`, `wait_state`, and
  `expect crc else <byte>`, backed by a compiler-managed register model (see the
  section below). A full peripheral I/O read now compiles byte-identically
  (modulo register allocation) to the hand-written
  `examples/peripheral_io_read.s`.
- **Plan 4a** — the two callables `fn` and `proc` (with positional and named
  arguments and defaults) plus `repeat N { … }` (see the section below). One
  `command` proc now drives both the OOB and the peripheral channel examples,
  each still byte-identical modulo register allocation to its `.s`.

Still to come: structured control flow — `if`/`else`, `while`, `do`/`while` —
(Plan 4b), `import` + the bundled `espi` stdlib (Plan 5), and `--lint` +
compile-time error injection (Plan 6).
```

Then add this section immediately after the "Frames, verdicts & the register model (Plan 3)" section:

```markdown
## Callables & unroll (Plan 4a)

The tamal ISA has **no `call`/`ret`, no stack and no data memory**, so neither
callable is a runtime call — both disappear before assembly.

- `fn NAME(params) -> type { expr }` — pure and compile-time; a call is replaced
  by the value the body evaluates to.
- `proc NAME(params) { stmts }` — emits bus activity; **inlined** at every call
  site. Each expansion runs in a fresh register scope, so it can never clobber a
  value that is live in the caller, and its labels come from the shared gensym
  counter, so two expansions never collide.
- Arguments are positional and/or **named** (`command(pkt = …, ndata = 0)`), and
  a parameter may declare a default (`err: byte = 0x11`). Parameter types are
  `byte` / `int` / `bytes`, and every bound value is checked against its type.
- `repeat N { … }` — a compile-time unroll: the body is emitted `N` times, with
  no loop counter and no branch. `N` may be any compile-time expression,
  including a `proc` parameter.
- **Recursion is a compile error** in both callables, and `repeat`/`recv` counts
  are bounded by the 1024-word program cap.

A `proc` may be called at the top level of a test or inside a `frame`; an
`expect` inside an expansion still defers its verdict to the enclosing frame, so
CS# always deasserts before the verdict (D9).
```

- [ ] **Step 2: Run the formatter and linter**

Run: `cargo fmt -p tamal-lang -p tamal-lang-cli`
Then: `cargo fmt --all --check`
Expected: clean (exit 0).

Run: `cargo clippy -p tamal-lang -p tamal-lang-cli --all-targets -- -D warnings`
Expected: clean — no warnings. If an unused helper trips `dead_code`, a wiring step was missed in an earlier task; wire it rather than `#[allow]`-ing it. (`RegAlloc::lookup` is a `pub` method and is *expected* to remain unused until Plan 4b wires named-variable references in operand position — `pub` items do not trip `dead_code`.)

- [ ] **Step 3: Run the whole workspace**

Run: `cargo test`
Expected: PASS across all crates — `0 failed`. Nothing outside `tamal-lang` changed, so `tamal-abi` (50), `tamal-asm` (41), `tamal-loader` (12 + 3) are unchanged; `tamal-lang` grows from its 90 baseline to roughly 143 (≈129 lib + 2 examples + 2 frames + 4 values + 4 callables).

- [ ] **Step 4: Commit**

```bash
git add crates/tamal-lang/README.md
git commit -m "docs(tamal-lang): document the callables and the compile-time unroll"
```

---

## Known limitations & tracked follow-ups

Found by review during execution. None blocks this increment; all are recorded here because "the compiler aborts or hangs with no diagnostic" is exactly the class the Task-4 blocker taught us to take seriously.

1. **`fn` consteval is unbounded: compile time is exponential in call-graph depth, with no diagnostic.** Each `fn` call re-evaluates the callee's body once per path, so a fan-2 chain (`fn f0() -> int { f1() ^ f1() }`, …) costs 2^depth evaluations. Measured (release): depth 22 → 11.3 s, depth 24 → 47.3 s, depth 26 → ~180 s, depth 28 → >600 s — a clean ×4 per two levels. Depth 40 is a **45-line source file** extrapolating to ≈35 days, exit 0, no diagnostic, no output. Task 4 removed the dominant *constant* factor by wrapping `Env`'s tables in `Rc` (measured ~8× on the same input); the complexity class is unchanged.

   **The remedy is a consteval work budget, not memoization.** An earlier version of this entry said memoization — that is wrong. Memoization collapses the *no-argument* chain, but a chain whose arguments vary has 2^depth distinct `(fn, args)` cache keys and gets no asymptotic saving at all. Measured on `fn g20(a: int) -> int { g21(a) ^ g21(a ^ 0x100000) }` chained: depth 22 → 13.5 s, depth 24 → 59.2 s — *slower* than the no-argument chain, not faster, and on exactly the same 2^depth curve. A future implementer following the old note would build the cache, watch the no-arg reproducer go green, and ship with the hole still open.

   The shape of the fix: a counter charged per `eval` step and checked against a cap, mirroring `MAX_EXPANSIONS` in `emit.rs`. It is deferred because it needs shared mutable state (`Rc<Cell<usize>>`) threaded through an `Env` that is cloned per call, and it touches every `eval*` signature — `consteval`-shaped work, not `emit`-shaped. Memoization remains worth doing for its constant factor (`fn` calls are pure by construction, so it is sound), but it is an optimisation, not the bound.

   **Task 7 added a reachability path: `repeat` multiplies it.** `lower_repeat` evaluates its count expression through `consteval::eval_int` once per invocation, and `lower_repeat` itself runs once per enclosing iteration — so the fan-out is paid per iteration rather than once. Measured:

   ```
   fn f13() -> int { 0 }   …   fn f0() -> int { f1() ^ f1() }
   test t { repeat 1024 { repeat f0() { } } pass }
   ```

   21 source lines → 13.1 s, **3 emitted asm lines**, 1024 expansions (1.5 % of `MAX_EXPANSIONS`), exit 0. Neither budget fires: the emission budget sees three lines, the expansion budget sees 1024 expansions, and all the work is in consteval where neither is looking. Task 7's budgets bound *expansion* work only — `emit.rs`'s `MAX_EXPANSIONS` doc says so explicitly under "What this does *not* bound" — and this entry is the other half of that disclosure.


2. **Deep `fn` expansion still aborts without a diagnostic.** An *acyclic* chain of roughly a thousand `fn`s overflows the stack (exit 134/-6, no diagnostic) — the same failure mode as the Task-4 blocker, reached by depth rather than by a cycle. Note this class is **pre-existing and not confined to callables**: the recursive-descent parser overflows on ~2000-deep expression nesting (`lo(lo(lo(…)))`) and predates this plan. A `fn`-only expansion cap would therefore give false confidence while the parser path remains open; the right fix caps recursion depth across the front end and reports a diagnostic. Deferred as its own piece of work.

3. **`--emit asm` diagnostics for an unclosed `(` can be misleading.** Because newlines are transparent *between* arguments, a missing `)` lets an argument list swallow following statements, so `send crc8([0x44],` / `cs_assert)` reports an arity error rather than a missing paren. This is the standard consequence of newline-transparent argument lists (Rust, C and Python behave the same way) and the obvious mitigations do not address it — the close-paren check *succeeds* in that example. Belongs with the diagnostics polish planned alongside `--lint` in Plan 6.

4. **`bind_args` returns a `HashMap`; iterating it would be a determinism violation.** Both consumers move it straight into `Env::locals` and read it by key. The doc comment says so; if a future feature (a listing of per-expansion bindings, an unused-parameter lint, a debug dump) needs an order, walk `params` instead.

5. **A recursion-guard regression aborts the whole test binary.** The name-based chain guard is correct and pinned, but if it is ever broken the resulting stack overflow takes down the entire `--lib` test binary rather than failing one test. Accepted property of recursion guards; noted so a future `SIGABRT` in CI is recognised for what it is.

6. **`RegAlloc::bind` does not shadow — a callee binding a caller's name destroys the caller's binding.** `bind` overwrites `bindings[name]` and records the name in the *current* scope, so `exit_scope` then removes it outright. A `proc` body doing `recv status`, inlined into a test that also bound `status`, leaves the caller's `status` unresolvable afterwards even though its register is still busy. Unreachable today because `RegAlloc::lookup` has no non-test caller — but **Plan 4b wires named references into operand position**, at which point this becomes live, and `proc` inlining makes the collision likely in practice (it is exactly what a shared library `proc` looks like). Fix when 4b lands: make it a per-scope shadow stack (`bindings: HashMap<String, Vec<Reg>>`, popped on `exit_scope`).

   **Sharper statement of the same hole: `recv <name>` names are write-only decoration today, and the binding itself is untested.** Replacing `lower_recv`'s `RecvTarget::Name` arm (`emit.rs:513-514`) with `alloc.temp()` — keeping the register, dropping only the *name* — **survives every suite in the workspace**: 311 passed, 0 failed, verified. It is the *whole* `bind` call that is unpinned, not merely the shadowing edge case. The distinction that matters when reading this: the mutant must **not** also add `alloc.free(reg)`. That variant reuses the register (`status0` and `status1` both take x1) and *is* killed by the two `*_lowers_to_the_expected_asm` goldens — so a survey that frees the register measures register lifetime and concludes, wrongly, that the binding is covered.

   Nothing pins `bind` because nothing reads it: `RegAlloc::lookup` (`regalloc.rs:113`) has no caller outside `regalloc.rs`'s own `#[cfg(test)]` module, so the binding path acquires teeth only in Plan 4b when an expression can read a `recv` target. Until then a test asserting the shadow fix would have to reach through `lookup` directly. Both halves — the shadowing bug and the binding — close together in 4b.

7. **`const` `++` doubling blows up memory with no *useful* diagnostic.** `const c1 = c0 ++ c0` repeated turns a handful of source lines into an arbitrarily large `bytes` value. Predates this plan entirely (Plan 2's `++` operator) and is a sibling of item 1 — the same missing consteval bound, reached through value *size* rather than call count, so the same work budget should cover both (charge per emitted byte as well as per eval step). Recorded here because item 1 is where a future implementer will look, and item 1 is currently written up purely as a *time* blowup; this is the **memory** half of the same class.

   Measured (release), `const A0 = [0;8]` then `const A{i} = A{i-1} ++ A{i-1}`:

   | depth | source | value | wall |
   |---|---|---|---|
   | 20 | 25 lines | 8 MB | 39 ms |
   | 22 | 27 lines | 32 MB | 127 ms |
   | 24 | 29 lines | 128 MB | 389 ms |

   A clean ×4 per two levels, so **depth 30 is a 35-line source file demanding 8 GiB**.

   `MAX_EMITTED_LINES` *does* fire here — every row above ends in `the program emits more than 4096 asm lines`. That is precisely the problem: `eval_bytes` (`consteval.rs:184-193`) materialises the whole `Vec<u8>` **before any budget sees a line**, so the memory is already spent when the diagnostic arrives, and the diagnostic names *asm lines* when the fault is one `const`. The same shape Task 8 fixed for `recv N`: a global budget that fires late and accuses the wrong construct is not a substitute for bounding the construct that allocates.

8. **`recv <name>` silently shadows a `const` instead of reading that many bytes.** Given `const N = 8`, `recv N` lowers to **one** `get_byte` bound to a register named `N` — the name/discard arm of the grammar — while `recv 8` lowers to eight. Verified. Pre-existing and a direct consequence of the grammar: the count arm is entered only on `Tok::Number` (`parser.rs:625`), so a symbolic count is not "unsupported", it is *a different statement that happens to parse*. Silent, plausible-looking, and exactly the kind of thing `--lint` should refuse — a natural Plan 6 diagnostic ("`recv N` where `N` names a const reads one byte; write the count as a literal").

   **This makes Task 8's placement comment load-bearing.** The cap lives in the parser because the count is a literal there. A future symbolic count would arrive through `consteval`, at which point the cap must move or be duplicated, or it silently stops applying and `recv BIG_CONST` is an OOM again.

9. **The cap's help text hard-codes "1024 words".** Both `recv`'s and `repeat`'s help say "a tamal program is at most 1024 words", but that figure comes from `tamal-asm`'s **private** `const MAX_WORDS: usize = 1024` (`tamal-asm/src/lib.rs:59`) and is retyped in `tamal-lang`, not derived. Retuning `MAX_UNROLL` alone leaves the pair reading *"count 600 is not in 0..=512 — a tamal program is at most 1024 words"*: still true, no longer obviously so. Fixing it means making `MAX_WORDS` public, which is a `tamal-asm` public-API decision affecting both constructs equally — deliberately **not** folded into Task 8. Note the two constants are independent on purpose: `MAX_UNROLL` is a *safety* bound one notch looser than the real cap, so a near-miss (`recv 1024` → 1025 words) gets the assembler's precise word-count message rather than a blunt refusal.

10. **`zero_regs`' catch-all arm defeats exhaustiveness checking on `Instr`.** `tests/common/mod.rs`'s `other => other` is correct *today* — verified against `tamal-abi`: `Instr` has 36 variants, `zero_regs` lists 26 explicitly, and the 10 the wildcard swallows (`CsAssert`, `CsDeassert`, `PutByteImm`, `PutBitsImm`, `TarImm`, `RstAssert`, `RstDeassert`, `Halt`, `SetConfig`, `CrcReset`) genuinely mention no `Reg`. But add a *register-bearing* variant to `tamal-abi` and the wildcard swallows it unzeroed, with no compile-time prompt.

    It **fails safe**, which is why this is not urgent: the two sides of a byte-match use different registers by construction (the hand-written `.s` uses `t0`/`t1`/`t2` = x5/x6/x7, the compiler allocates x1/x2/x3), so an unzeroed register yields a false *failure*, never a false pass. But it is the opposite convention to `emit.rs:347-348`, which is deliberately wildcard-free "so a new `Stmt` variant forces a decision" — worth knowing the two files disagree on purpose-by-default rather than by design.

    Not introduced by Task 9: it is inherited verbatim from Plan 3's `frames.rs` and reproduced literally by this plan's own Task-9 code block, so Task 9 moved it rather than wrote it. The right moment to fix it — replace the wildcard with the 10 variants spelled out — is `tamal-abi`'s next ISA change, when the compiler would otherwise be silent about the new one.


---

## Notes for the implementer

- **Adding a `parser::Stmt` variant now touches THREE exhaustive matches** (Task 5 merged two of the old four): `emit.rs::stmt`, `emit.rs::stmt_span`, and the `lib.rs` M2 `halts` scan. All three are wildcard-free on purpose — lean on the compiler to flag an omission. A new emitting statement is not a terminator, so it is `=> false` in `halts`.
- **Adding an `Expr` variant** still requires updating the `eval` match in `consteval.rs` (exhaustive, no wildcard, intentional).
- **The asm label grammar is `[A-Za-z_][A-Za-z0-9_]*` — no leading dot** (a `.`-prefixed token lexes as a directive). Gensym labels use the `__` prefix.
- **`gensym` is a single shared counter**, so numbers do not restart per prefix: a `wait_state` followed by an `expect` yields `__wait0` then `__fail1`. Both capstone goldens depend on this.
- **`li` tiles small values to one word** (anything fitting signed-21), so `li x2, 0x0F` is a single `load_imm` — which is why the poll idiom is four words and the byte-match works.
- **Run `cargo fmt -p tamal-lang` before every commit**; CI enforces `cargo fmt --all --check` per commit.
- **The human works in this repo concurrently.** Re-check `git status` / `git log` before committing; do not assume SHAs.

---

## Self-Review (completed by the plan author)

**1. Spec coverage.**

| Spec item | Task |
|---|---|
| §1.1 no `call`/`ret`/stack → every callable inlines | Tasks 4, 6 (and the recursion rejections that follow from it) |
| §2 / D2 `fn` — pure, returns `byte`/`int`/`bytes`, call replaced by its value | Task 4 |
| §2 / D2 `proc` — emits, inlined at every call site | Task 6 |
| §2 positional **and named** arguments with defaults | Task 3 (binder) + Tasks 4, 6 (both callers) |
| §2 / §5 `repeat N { … }` compile-time unroll | Task 7 |
| §5 / D5 per-expansion register hygiene (fresh registers, no clobber) | Task 6 (`enter_scope`/`exit_scope` + the clobber test) |
| §5 label hygiene across expansions (gensym) | Task 6 (`two_expansions_get_fresh_registers_and_labels`) |
| §5 lexical scoping: a callee sees its params + module consts only | Tasks 1, 3 (`child_for_call` / `module_scope`, with a test) |
| §5 every field/type range-checked, never silently wrapped | Task 3 (`check_type`) |
| §7 register safety: never alias a live value; no spill | Task 6 (`reserve` closes the deferred-verdict hole) |
| §7 no invented linkage | Tasks 4, 6 (recursion is an error, not a stack) |
| §7 determinism: no `HashMap` iteration reaches output or error order | Task 3 (errors walk `params`/`args` in order) |
| §9 channel examples byte-match modulo register allocation | Task 9 (OOB **and** peripheral) |
| §9 every TX CRC re-derived, never typed (`0xB1`, `0x16`) | Task 9 |
| §4.3 the library `command`/`iowr_hdr` shapes are expressible | Task 9 (`COMMAND_PROC`, `iord_hdr`) |
| Plan-3 follow-up: unbounded `recv N` OOM | Task 8 |

Out of scope by design and stated in the header: `if`/`while`/`do-while`/comparisons/`bool`/`let`/`reg` (Plan 4b); `pub`/`import`/namespacing/`espi.tam` (Plan 5); `enum`, general arithmetic, `bytes` indexing; `--lint` and error injection (Plan 6).

**2. Placeholder scan.** No `TBD`, `todo!()`, "implement later", or "similar to Task N". Every step that changes code shows the code. The `unreachable!` arms inherited from Plan 3's `lower_*` helpers are real guards (the caller only dispatches the matching variant), not placeholders. Both capstone goldens and both CRC bytes were computed against the actual tree, not guessed.

**3. Type consistency.**
- `Env::{new, insert_const, has_const, get, module_scope, child_for_call_values, define_fn, get_fn, is_active, child_for_call}` — each introduced once (Tasks 1, 3, 4) and used with those exact names afterwards.
- `bind_args(callee, params, args, env, call_span) -> Result<HashMap<String, Value>, Diagnostic>` (Task 3) is called with that exact signature by `eval_fn_call` (Task 4) and `lower_call` (Task 6). `check_type(v, ty, span, what)` likewise.
- `Arg { name, value, span }`, `Param { name, ty, default, span }`, `Type::{Byte, Int, Bytes}` + `Type::name()`, `FnDef { name, name_span, params, ret, body }`, `ProcDef { name, name_span, params, body }` — defined once, matched with identical shapes everywhere.
- `Stmt::Call { name, name_span, args, span }` and `Stmt::Repeat { count, body, span }` are destructured identically in `emit.rs::stmt`, `emit.rs::stmt_span`, and the `lib.rs` `halts` scan.
- `Emitter` fields (`env`, `asm`, `lines`, `alloc`, `gensym`, `trailers`, `frames`, `procs`, `active_procs`) are each added in the task that first needs them and initialised in `Emitter::new` in that same task. `Emitter::new(env, module)` gains its second parameter in Task 6, along with its only caller.
- `MAX_UNROLL` is defined once (Task 7, `lib.rs`) and referenced as `crate::MAX_UNROLL` by both `emit.rs` (Task 7) and `parser.rs` (Task 8).
- `RegAlloc::reserve` (Task 6) is called only through `Emitter::reserve_deferred`, which Tasks 6 and 7 both invoke after closing a nested scope.

**4. Ordering note.** `Module` grows a field twice — `fns` in Task 4, `procs` in Task 6 — and each time the two test `Module` literals in `emit.rs` must gain it or the crate will not compile. Both tasks call this out explicitly. Similarly, `parse`'s "expected `const`…" message changes in Task 4 and again in Task 6; Task 4 relaxes the assertion in `missing_test_keyword_is_an_error` once, so Task 6 needs no further test edit.

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-07-29-tamal-lang-04a-callables-and-unroll.md`. Two execution options:

1. **Subagent-Driven (recommended)** — a fresh implementer subagent per task, then two independent reviews per task (spec-compliance + code-quality) that verify by reading the code and running the tests themselves rather than trusting the implementer's report; a final whole-increment review at the end. Exactly how Plans 1–3 were run.
2. **Inline Execution** — execute the tasks in this session with checkpoints for review.

Which approach?








