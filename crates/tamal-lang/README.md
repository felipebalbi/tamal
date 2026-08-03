# tamal-lang

The **compiler** for tamal-lang: a high-level language that lowers to tamal
assembly text and, through [`tamal_asm::assemble`](../tamal-asm), to tamal
bytecode.

Part of the [tamal](../../README.md) eSPI compliance rig. **MIT-licensed.** The
command-line front-end is [`tamal-lang-cli`](../tamal-lang-cli) (the `tamalc`
binary).

## Status

**Link + two channels, compiling end-to-end** (through Plan 4a). The front-end
(lexer → parser → const-eval → emit) lowers `.tam` source to tamal-asm text and
hands it to the `tamal_asm::assemble` backend. The language now covers:

- **Plan 1** — a single `test NAME { … }` entry, the `pass` / `fail N` verdicts,
  and verbatim **raw-instruction pass-through** (any tamal-asm mnemonic as-is).
- **Plan 2** — `const` items and the compile-time value/expression layer
  (`bytes` literals, `++`/`^`, the `crc8`/`len`/`lo`/`hi` builtins) with `send` /
  `send … + crc8` / `crc_region`, folding the TX CRC at compile time.
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

## Frames, verdicts & the register model (Plan 3)

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
is a compile error. Registers bound inside a `frame` are freed at the end of that scope; top-level bindings live for the whole test.

## Callables & unroll (Plan 4a)

The tamal ISA has **no `call`/`ret`, no stack and no data memory**, so neither
callable is a runtime call — both disappear before assembly.

- `fn NAME(params) -> type { expr }` — pure and compile-time; a call is replaced
  by the value the body evaluates to.
- `proc NAME(params) { stmts }` — emits bus activity; **inlined** at every call
  site. Each expansion runs in a fresh register scope, so it can never clobber a
  value that is live in the caller, and its labels come from the shared gensym
  counter, so two expansions never collide.
- Arguments are positional and/or **named** (`command(pkt = …, ndata = 0)`,
  positional first), and a parameter may declare a default (`err: byte = 0x11`).
  Parameter types are `byte` / `int` / `bytes`, and every bound value is checked
  against its type.
- `repeat N { … }` — a compile-time unroll: the body is emitted `N` times, with
  no loop counter and no branch. `N` may be any compile-time expression,
  including a `proc` parameter.
- **Recursion is a compile error** in both callables, and `repeat`/`recv` counts
  are bounded by the 1024-word program cap.
- A per-construct cap does not bound *composition* (nested `repeat`s, or a
  `proc` that fans out), so two whole-program budgets also apply: at most 4096
  emitted asm lines, and at most 65536 expansions — every `repeat` iteration and
  every `proc` call counts as one, even when its body emits nothing. Exceeding
  either is a diagnostic anchored on the outermost expansion in progress, with
  the whole chain labelled. They bound *expansion*; folding a `fn` call graph is
  not yet bounded.

A `proc` may be called at the top level of a test or inside a `frame`; an
`expect` inside an expansion still defers its verdict to the enclosing frame, so
CS# always deasserts before the verdict (D9).

## Public API

- `lower_to_asm(source) -> Result<String, Vec<Diagnostic>>` — `.tam` → tamal-asm text.
- `compile(source) -> Result<Program, Vec<Diagnostic>>` — `.tam` → bytecode.

Diagnostics reuse `tamal_asm::{Diagnostic, Severity, Span}` carrying `.tam` byte spans.

## See also

- Language design: [`docs/superpowers/specs/2026-07-20-tamal-lang-design.md`](../../docs/superpowers/specs/2026-07-20-tamal-lang-design.md)
- Implementation plans: [`docs/superpowers/plans/`](../../docs/superpowers/plans/) — walking skeleton (01), values & CRC (02), frames & verdicts (03), callables & unroll (04a).
