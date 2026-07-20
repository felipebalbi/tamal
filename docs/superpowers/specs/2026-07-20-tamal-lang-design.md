# Tamal — `tamal-lang` High-Level Language Design

Date: 2026-07-20
Status: Approved (design); implementation not started
Scope: A new high-level language (HLL) — working name **tamal-lang**, source
extension `.tam`, compiler `tamalc` — that compiles down to tamal assembly text
and, through the existing `tamal-asm` backend, to tamal bytecode. It exists to
kill the copy-paste boilerplate visible across `examples/*.s` (identical CS
framing, WAIT_STATE polling, CRC-residue verdict, halt codes) and to make the
hand-computed TX CRC byte **correct-by-construction** via a compile-time
`crc8()`. The runtime/compile-time error-injection `(seed, ratio)` machinery is
explicitly deferred (but its determinism invariants are reserved here).

Companion to, and strictly layered above, the assembler design
(`docs/superpowers/specs/2026-07-04-tamal-asm-design.md`) and the ISA/ABI design
(`.../2026-07-03-tamal-abi-isa-design.md`). The authoritative lowering target is
`tamal_asm::assemble` (which owns `li`-tiling, branch resolution, the 1024-word
cap, listings, and disasm); the authoritative CRC is `tamal_abi::crc8` (poly
`0x07`, init `0x00`, MSB-first), itself a byte-exact mirror of the HDL
`Tamal.Crc`. This design was produced through a brainstorming dialogue informed
by architect, reliability, and docs analyses of the `tamal-asm` crate, the
`tamal-abi` ISA/ABI, and all eight `examples/*.s`.

---

## 1. Purpose & the layering

The tamal engine is a **dumb SPI shifter with zero eSPI knowledge** — it shifts
host-built bytes and knows nothing of channels, cycle types, or opcodes. The
guiding principle of this language is: **so does the language.** `tamal-lang`
provides framing, CRC, verdicts, structured control flow, and raw wire bytes.
*eSPI itself* — command opcodes, cycle types, channel semantics, verdict codes —
lives in an importable, **source-form** library (`espi.tam`) that users can read,
fork, and extend. This keeps the wire visible (hardware engineers trust it), the
language core tiny (software engineers learn it fast), and everything
compile-time deterministic (the rig stays byte-for-byte reproducible).

`tamal-lang` is **Layer-2 authoring** above `tamal-asm`'s Layer-1. It is a real
compiler (lexer → parser → resolve → const-eval → expand → emit) whose backend
emits **annotated tamal-asm text** and calls `tamal_asm::assemble`. It never
hand-packs bytecode and never re-implements label math, `li`-tiling, or the size
cap — it reuses the assembler wholesale. `tamalc --emit-asm` exposes the exact
lowering of every construct, which doubles as a teaching tool and as the
"nothing is hidden" trust bridge for hardware engineers.

### 1.1 The ISA constraints that shape everything

The tamal ISA (per the assembler/ISA specs and confirmed in `tamal-asm`)
imposes hard limits the language must design around, not paper over:

- **No `call`/`ret`, no `JALR`, no stack, no data memory.** Therefore "procedures"
  **cannot** be runtime calls — every `proc` is **inlined** at each call site.
  This is stated explicitly so the register-hygiene story is not left implicit.
- **16 physical registers (`x0`..`x15`, `x0` = zero), no spill target.** The
  compiler allocates named variables into the ≤15 usable registers; exhaustion is
  a **compile error**, never a silent spill.
- **No constant-expression arithmetic, no macros, no includes in the asm.** The
  HLL owns all of it: expression evaluation, `proc`/`fn` inlining, compile-time
  CRC, module linking, and packet layout.
- **Branch reach ±1024 words; program ≤1024 words.** Surfaced by the assembler's
  existing checks, re-pointed to `.tam` spans via a source map.

---

## 2. Scope & non-goals

**In scope (v1)**

- The language core: `const`, `enum`, `let`/`reg` named variables, `proc`
  (emitting, inlined) and `fn` (pure, value-returning) callables with
  positional + **named** arguments and default values.
- Compile-time value model: `byte`, `int`, `bytes`, `bool`, `reg`, `enum`; a pure
  constant evaluator (arithmetic, comparisons, `bytes` concat/index/`len`).
- Built-in **`crc8(bytes) -> byte`** (compile-time fold over `tamal_abi::crc8`),
  plus `len`, `lo`, `hi`.
- Three CRC surfaces: `crc8(x)` value primitive; `send pkt + crc8` append sugar;
  `crc_region { … }` structural block. All fold the CRC over *exactly the bytes
  emitted*.
- **Structured control flow only:** `if`/`else`, `while`, `do`/`while`,
  `repeat N` (compile-time unroll). No user-visible labels or branch mnemonics;
  the compiler owns all branching (gensym'd labels), inspectable via `--emit-asm`.
- Domain sugar: `config`, `frame { … }` (RAII CS scope with deassert-on-every-exit),
  `send`, `recv`, `wait_state`, `expect crc else <byte>`, `pass`, `fail <byte>`,
  `mark`.
- Namespaced `import` with deterministic resolution (canonical-path dedupe,
  cycle detection, duplicate-different-body = error).
- A bundled, source-form **`espi`** standard library (channel opcodes, cycle
  types, verdict codes, a `controller` config helper, a `command`/frame-and-verify
  `proc`), discovered on a search path.
- CLI `tamalc`: `compile` (`--emit bin|asm|listing`), `-L`/`-I` search paths,
  `--lint`, span diagnostics rendered with ariadne.
- Compiles HLL equivalents of the current `examples/*.s` to byte-identical
  bytecode.

**Out of scope (deferred; the invariants are reserved, the machinery is not built)**

- Compile-time **error injection** `(seed, ratio)` — AGENTS.md phase 4. The
  determinism invariants (integer/rational ratio, `ratio = 0` structurally zero,
  pinned+versioned PRNG, deterministic site enumeration, bus-stimulus-only) are
  fixed here (§7) so the later feature slots in without breaking replay.
- Data-backed payloads / arrays in RAM (the ISA has no data memory).
- Target-role and dual/quad-IO helpers (the assembler currently rejects them);
  the language exposes the config surface but defers role-specific procedures.
- Emitting `Instr`/bytecode directly (bypassing asm text) — a later optimization;
  v1 keeps the inspectable text seam.

---

## 3. Design decisions

| # | Decision | Rationale |
|---|----------|-----------|
| D1 | **Structured mini-language, not a preprocessor or host-language eDSL.** Own lexer/parser/AST/const-eval/expander, emitting annotated tamal-asm text. | Only this serves *both* audiences (req 4) with real types/imports/diagnostics; a preprocessor abandons software engineers and has poor diagnostics, an eDSL forces a toolchain on hardware engineers or breaks determinism (a host `random`/`time` escape hatch violates AGENTS.md). |
| D2 | **Two callables: `proc` (emits, inlined) + `fn` (pure, returns `byte`/`int`/`bytes`).** Both inline; no runtime call exists. | `proc` captures the emit boilerplate; `fn` lets library authors write compile-time packet/header builders composed with `crc8()` (req 5, 6). The inlining is explicit so register hygiene is designed in, not assumed. |
| D3 | **`crc8()` reuses `tamal_abi::crc8`; three surfaces all fold over exactly-emitted bytes.** | Req 7. One CRC implementation shared by wire, HDL, and compiler → the compile-time byte can never drift. `+ crc8` and `crc_region` bind covered-bytes to emitted-bytes *structurally*, killing the stale-hand-CRC hazard (the headline reliability finding). |
| D4 | **Structured control flow only; no user labels/branches.** Compiler lowers `if`/`while`/`do-while`/`repeat` to gensym'd branches. | Cleanest, most software-familiar surface; eliminates the label-collision-on-expansion hazard entirely. Instruction statements stay 1:1 with asm mnemonics; only branch mnemonics are withheld. `--emit-asm` exposes the lowering. |
| D5 | **Named variables, compiler-allocated, with per-`proc` hygiene; ≤15 live is a hard error.** | Removes register juggling (ergonomics), and per-`proc` fresh registers mean a called procedure can never clobber the caller's live values — the register-clobber hazard is solved by construction. No spill target exists, so exhaustion must error, never spill. |
| D6 | **Namespaced `import` by default; deterministic resolution.** | Avoids the flat-dump collision hazard; canonical-path dedupe + cycle detection + duplicate-different-body = error defend against include-order and diamond-skew nondeterminism in source-form libraries. |
| D7 | **Bundle a source-form `espi` stdlib on a search path.** | Reqs 1, 8: `import espi` must work out of the box for the 5-minute test; keeping it plain source preserves the "wire visible / forkable" principle; a single vendored stdlib versioned with the toolchain sidesteps version-skew concerns. |
| D8 | **Emit annotated tamal-asm text through `tamal_asm::assemble`; reuse its `Diagnostic`/`Severity`/`Span`, extended with a `FileId`.** | Reuses `li`-tiling, branch resolution, the 1024-word cap, listing, and disasm for free; `--emit-asm` is a free trust bridge; a generated-asm→`.tam` source map re-points the rare backend diagnostic. |
| D9 | **`frame { }` guarantees `cs_deassert` on *every* exit, including a failing `expect`.** | Encodes the load-bearing "CS deasserts before the verdict, verdict-independent" invariant that is easy to botch by hand in the `.s`. |
| D10 | **Deliberate-wrong is first-class and loud, never a bare literal.** `send pkt ++ [crc8(pkt) ^ 0xFF]`; an opt-in `--lint` flags literal CRC bytes disagreeing with `crc8()`. | A compliance rig must send bad CRCs/illegal TARs on purpose; ergonomic abstractions must never silently legalize an intended violation, and intent must be greppable and distinguishable from an accidental stale byte. |
| D11 | **New crates `tamal-lang` + `tamal-lang-cli` under `crates/`, MIT.** DAG: `tamal-lang → tamal-asm → tamal-abi` (and `tamal-lang → tamal-abi`); never reversed. | Extends the existing clean dependency graph; matches the host-tooling license boundary; the HLL is a strict superstratum. |

---

## 4. Surface syntax

### 4.1 Conventions

- **Comments:** `//` line, `/* … */` block.
- **Numbers:** decimal, `0x…`, `0b…`, `_` digit separators, negatives (`0xDE_AD`).
- **Byte/packet literal:** `[0x00, 0x64, 0xAB]`; concat `a ++ b`.
- **Imports:** `import espi` (stdlib module, namespaced as `espi`);
  `import "lib/foo.tam" as foo` (path import). References are qualified:
  `espi.PUT_OOB`, `foo.helper(...)`.
- **Test entry:** `test NAME { … }` — one entry, must reach `pass`/`fail`.
- **Constants / enums:** `const NAME = 0x44`; `enum Verdict: byte { Ok = 0x00, Crc = 0x11 }`.

### 4.2 A complete simple test (`peripheral_io_read.tam`)

Lowers to byte-identical bytecode as the existing `peripheral_io_read.s`:

```rust
// peripheral_io_read.tam — eSPI Peripheral channel: short I/O read
import espi

test io_read {
    config controller, x1, sck20, alert_pin

    frame {                                         // CS# low … CS# high (RAII)
        send [espi.PUT_IORD1, 0x00, 0x64] + crc8    // command phase — the only bytes that vary
        tar 2                                       // legal turnaround
        wait_state                                  // poll past WAIT_STATE (0x0F)
        recv resp, data, status0, status1           // response phase
        expect crc else 0x11                        // RX CRC-8 residue == 0, else verdict 0x11
    }
    pass                                            // halt 0x00
}
```

What disappeared vs the 50-line `.s`, and why it is safe:

| `.s` construct | Fate in `.tam` | Why safe to fold |
|---|---|---|
| `.equ RSP_WAIT_STATE/VERDICT_*` | gone (owned by `wait_state`/`pass`/`expect`) | poll/verdict constants are helper internals |
| `.text`/`.globl _start`/`_start:` | gone (implied by `test { }`) | a `test` *is* the entry + text section |
| `cs_assert … cs_deassert` | `frame { }` | RAII scope; guarantees CS deasserts before the verdict (D9) |
| `put_byte 0x16  # TX CRC-8` | `+ crc8` | the #1 error-prone line; now auto-recomputed on any byte edit |
| `poll:`/`crc_reset`/`li`/`beq` | `wait_state` | byte-identical in all four channel examples |
| `rdsr`/`bnez`/`bad_crc:` | `expect crc else 0x11` | residue verdict identical everywhere; only the code varies |
| `t0`/`t1`/`t2` | named vars | neither audience should hand-juggle 3 of 16 registers |
| `[PUT_IORD1, 0x00, 0x64]`, `tar 2`, `config`, `recv` | **stays visible** | this is the wire; it never hides |

### 4.3 A reusable library + a test that imports it

Bundled `espi.tam` (excerpt) captures the copy-pasted block once:

```rust
// espi.tam — bundled eSPI library (excerpt; source-form, forkable)
pub const PUT_IORD1 = 0x44
pub const PUT_OOB   = 0x06

pub enum Verdict: byte { Ok = 0x00, Crc = 0x11 }

pub proc controller() { config controller, x1, sck20, alert_pin }

// Send a host-built packet, append its CRC-8, turn the bus around, poll past
// WAIT_STATE, read `ndata` payload + 2 status bytes, consume the trailing CRC
// byte and verify the residue. The block copy-pasted into every example .s.
pub proc command(pkt: bytes, ndata: int, err: byte = Verdict.Crc) {
    frame {
        send pkt + crc8                 // TX CRC-8 auto-appended over pkt
        tar 2
        wait_state
        repeat ndata { recv _ }         // payload bytes (compile-time unroll)
        recv status0, status1
        expect crc else err             // consumes trailing CRC byte, checks residue
    }
}

// Pure value-returning helper: build a short-I/O header from an address.
pub fn iowr_hdr(op: byte, addr: int) -> bytes { [op, hi(addr), lo(addr)] }
```

```rust
// oob_smbus_msg.tam — OOB channel, built from the shared library
import espi

test oob_msg {
    espi.controller()
    espi.command(
        pkt   = [espi.PUT_OOB, 0x21, 0x00, 0x04,   // eSPI OOB header
                 0x10, 0x00, 0x01, 0xAB],          // tunneled SMBus {dest,cmd,count,data}
        ndata = 1,
    )
    pass
}
```

The 62-line `oob_smbus_msg.s` collapses to ~10 meaningful lines; its hand-written
`0xB1` CRC becomes `+ crc8`; the poll/residue/verdict skeleton lives in one
reviewed library `proc`. Named arguments (`ndata = 1`) make the call
self-documenting.

### 4.4 Deliberate-wrong (compliance negative tests)

```rust
// deliberately corrupt the CRC to confirm the DUT rejects the packet
send pkt ++ [crc8(pkt) ^ 0xFF]     // loud + greppable: the ^0xFF signals intent
// illegal turnaround is always an explicit width, never hidden by a helper
tar 3                              // deliberate TAR violation
```

---

## 5. Semantic model

**Types.** `byte` (`0..=255`, one wire byte); `int` (compile-time integer, must
narrow to fit its target field); `bytes` (compile-time byte-string, the packet
type); `bool` (conditions); `reg` (a named runtime variable — a distinct type so
operand positions type-check); `enum` (named byte set). `label` is implicit and
compiler-owned.

**Compile-time evaluation.** A pure, total evaluator over `int`/`byte`/`bytes`/
`enum`: arithmetic `+ - * / % << >> & | ^ ~`, comparisons, `bytes` concat/index/
`len`, and builtins `crc8(bytes) -> byte`, `len(bytes) -> int`, `lo(int)`/
`hi(int) -> byte`. It reads no clock, environment, filesystem order, or RNG, so
identical source yields byte-identical bytecode. `crc8` delegates to
`tamal_abi::crc8` — the compile-time and on-wire CRCs are literally the same
code. Every `bytes` element and every field is range-checked (`byte ≤ 255`,
`tar ≤ 15`, `mark` tag ≤ 2047, branch reach, etc.); out-of-range is an error,
never a silent wrap. `crc8()` of a non-constant (a register / `get_byte` result)
is a type error ("`crc8` is compile-time only").

**Scoping & namespacing.** One file = one module; items are private unless `pub`.
`import m [as n]` binds a module namespace; references are qualified. Resolution
order: local → module → qualified import. Imports resolve source-form from the
search path (importer dir, `-I`/`-L`, bundled stdlib); paths are canonicalized
and deduped by identity; cycles are a hard error; a duplicate definition with a
*different* body is a hard error (identical bodies may dedupe).

**Register mapping.** The compiler allocates each `let`/`reg` named variable to a
physical register from `x1`..`x15` (`x0` is zero, never allocated), releasing at
end of lexical scope. Each `proc` expansion gets fresh registers (hygiene). The
allocator never exceeds 16 live registers, never aliases a live variable, and
never emits `x16`..`x31`; exhaustion is a diagnostic naming the offending scope.

**Bus-opcode mapping (sugar → mnemonics).**

| HLL construct | Lowers to |
|---|---|
| instruction statement (`cs_assert`, `put_byte b`, `tar 2`, `get_byte r`, `mark`, `rdsr`…) | the identical mnemonic, 1:1 |
| `const`/`enum`/`let` used as operand | folded immediate (const-eval) |
| `send [a,b,c]` | `put_byte a` · `put_byte b` · `put_byte c` |
| `send x + crc8` / `crc_region { … }` | the emitted bytes, then `put_byte <folded crc8>` |
| `crc8(pkt)` in operand position | folded literal byte |
| `recv a, b, c` / `recv N` | `get_byte`s into allocated regs (`_` = discard) |
| `wait_state` | the documented `crc_reset`/`get_byte`/`li`/`beq` poll idiom |
| `do { … } while r == K` / `while` / `if`/`else` | `beq/bne/bltu/bgeu` + `j` with gensym'd labels |
| `repeat N { … }` | compile-time unroll ×N |
| `expect crc else X` | residue read + branch; on failure `frame` deasserts CS, then `halt X` |
| `pass` / `fail X` | `halt 0x00` / `halt X` |
| `frame { … }` | `cs_assert` · body · `cs_deassert` on every exit (D9) |
| `config …` | `set_config` with the same role/io/sck/alert keywords |
| `proc` call | hygienic inline expansion |
| `fn` call | replaced by its returned const value |
| `test NAME { … }` | `.globl _start` · `_start:` · body (must reach `halt`) |

---

## 6. Compiler architecture

Two new crates under `crates/` (MIT):

- `crates/tamal-lang/` — the compiler library. Public seam:
  `compile(entry, opts) -> Result<Program, Vec<Diagnostic>>`, mirroring
  `tamal_asm::assemble`.
- `crates/tamal-lang-cli/` — the `tamalc` binary (clap front-end, ariadne
  rendering, `-L`/`-I`, `--emit asm|bin|listing`, `--lint`), mirroring
  `tamal-asm-cli`.

Dependency direction (never reversed): `tamal-lang → tamal-asm → tamal-abi`, and
`tamal-lang → tamal-abi` (for `crc8`, config enums, field bounds). `tamal-asm`
and `tamal-abi` must not depend on `tamal-lang`.

Internal modules of `tamal-lang`, each with one job:

| Module | Responsibility |
|---|---|
| `lexer` | `.tam` source → tokens + spans (`{}`, operators, `//`/`/* */`) |
| `parser` | tokens → HLL AST (`Module`, `Item::{Const,Enum,Fn,Proc,Import,Test}`, `Stmt`, `Expr`) |
| `resolve` | module loading, import-cycle detection, name binding, symbol tables |
| `consteval` | pure evaluator: arithmetic, `bytes`, `crc8`/`len`/`lo`/`hi` |
| `expand` | hygienic `proc` inlining (label gensym), register allocation, structured-control → branch lowering |
| `emit` | render the asm-line stream to annotated text (+ generated-asm→`.tam` source map) |
| `diagnostics` | `FileId` + `Span` + `Diagnostic` (re-exports tamal-asm's, file-aware) |
| `driver` (`lib.rs`) | `compile()` — orchestrates the pipeline and calls `tamal_asm::assemble` |

**Lowering pipeline:** `.tam` → AST → const-eval + hygienic `proc`-inlining +
structured-control lowering → flat asm-line stream → annotated tamal-asm text →
`tamal_asm::assemble` → `Program` (`to_le_bytes()`, `listing()`).

---

## 7. Reliability invariants & guardrails

**Invariants that must always hold.**

- **Determinism:** identical source (+ identical injection seeds, once that
  lands) → byte-identical bytecode. Const-eval touches no clock, env, filesystem
  order, or RNG; all output-bearing collections are ordered (no `HashMap`
  iteration reaching bytes, labels, or listings); artifacts contain no timestamps
  or absolute paths.
- **CRC single source of truth:** `crc8()` ≡ `tamal_abi::crc8` ≡ HDL
  `Tamal.Crc`; CI-pinned to the residue law `crc8(msg ++ [crc8 msg]) == 0`. No
  second transcription is permitted.
- **Structural CRC:** `+ crc8` and `crc_region` fold over exactly the bytes
  emitted, in emission order — covered-bytes and emitted-bytes cannot drift.
- **Valid-emit:** HLL semantic/range checking is a superset of the assembler's,
  so `tamal_asm::assemble` on emitted text never fails on user error; any failure
  is an internal compiler bug.
- **Register safety:** the allocator never emits `x16`..`x31`, never aliases a
  live variable, never clobbers `x0`; no silent spill.
- **Namespace isolation:** imported names never leak or silently collide; import
  cycles and duplicate-different-body definitions are rejected.
- **No invented linkage:** all procedures inline; the compiler never fabricates a
  `call`/`ret` the ISA lacks.
- **Visible danger:** ergonomic abstractions never silently legalize an intended
  illegal cycle; `tar` width is always explicit; deliberate-wrong CRCs are loud
  and greppable (D10).

**Deferred error-injection invariants (reserved now, built later).** Injection is
a pure function of `(seed, ratio, program)`; **ratio is integer/rational** so
`ratio = 0` is *exactly* zero and is a structural no-op that never draws from the
PRNG; the **PRNG is named, versioned, and pinned** (a change bumps a recorded
injection-engine version); site enumeration is deterministic (a pure function of
the AST in source order); injection may mutate **only** the eSPI-bus stimulus,
never the transport frame, its wire-CRC, or the verdict path.

**Diagnostics / verification hooks.**

- `--emit-asm` / `--emit-listing`: expose every lowering (trust bridge + teaching).
- `--lint`: recompute `crc8()` over each covered payload and flag any literal CRC
  byte that disagrees, unless marked deliberate-wrong; migrate golden fixtures off
  hand-literals so a future payload edit fails loudly.
- CI: double-build byte-compare (reproducibility); `crc8()` ↔ HDL residue
  property; a determinism check that perturbs hash seed / include order / CWD and
  asserts identical output.
- Toolchain provenance (HLL version, CRC-spec id, and — later — injection-engine
  version) recorded in the listing header.

---

## 8. Open questions (non-blocking; settle during planning)

1. `wait_state` lowering: reproduce the examples' hand-rolled `beq` poll, or use
   the ISA's native `wait_on rd, cond, timeout` (tighter, bounded)? The examples
   hand-roll; a bounded timeout may be safer for the rig.
2. `expect crc` residue read placement vs `frame` deassert: confirm the residue
   is latched inside `frame` (after the trailing `get_byte`) and the branch runs
   on the deassert-then-halt path (D9), byte-matching the `.s`.
3. Source-map fidelity: is a generated-asm→`.tam` map sufficient for backend
   diagnostics (size cap, branch reach), or should `tamal-asm` expose a structured
   lowering API (`encode_line`) so the front-end tracks provenance directly?
4. Exact stdlib surface + discovery: which procs ship in `espi` v1, and the
   search-path precedence (`$TAMAL_LIB` vs `-L` vs vendored)?
5. `config` gating: expose the full role/IO/SCK enum and let the assembler reject
   not-yet-supported combos (forward-compatible), or mirror the v1 restriction in
   the front-end?

---

## 9. Success criteria

- HLL equivalents of the four channel `examples/*.s` (`peripheral_io_read`,
  `oob_smbus_msg`, `virtual_wire_pltrst`, `flash_completion`) plus `smoke_halt`
  and `mark_trace` compile to **byte-identical** bytecode as the hand-written
  `.s`, verified against `tamal-asm`.
- Every hand-computed TX CRC in those examples is replaced by `+ crc8` /
  `crc8()` and re-derives the same byte (`0x16`, `0xB1`, `0xE8`, `0x89`).
- `import espi` works out of the box; a new user writes `smoke_halt` and a
  peripheral I/O read from a one-page guide.
- `tamalc --emit-asm` renders an annotated lowering that a hardware engineer can
  read line-for-line against the wire.
- Determinism CI gate (double-build byte-compare) is green; `crc8()` matches the
  HDL residue property.
