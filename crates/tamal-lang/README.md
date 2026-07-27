# tamal-lang

The **compiler** for tamal-lang: a high-level language that lowers to tamal
assembly text and, through [`tamal_asm::assemble`](../tamal-asm), to tamal
bytecode.

Part of the [tamal](../../README.md) eSPI compliance rig. **MIT-licensed.** The
command-line front-end is [`tamal-lang-cli`](../tamal-lang-cli) (the `tamalc`
binary).

## Status

**Link + one channel, compiling end-to-end** (through Plan 3). The front-end
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

Still to come: `proc`/`fn` + structured control flow (Plan 4), `import` + the
bundled `espi` stdlib (Plan 5), and `--lint` + compile-time error injection
(Plan 6).

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
is a compile error. Registers are freed at the end of their `frame` scope.

## Public API

- `lower_to_asm(source) -> Result<String, Vec<Diagnostic>>` — `.tam` → tamal-asm text.
- `compile(source) -> Result<Program, Vec<Diagnostic>>` — `.tam` → bytecode.

Diagnostics reuse `tamal_asm::{Diagnostic, Severity, Span}` carrying `.tam` byte spans.

## See also

- Language design: [`docs/superpowers/specs/2026-07-20-tamal-lang-design.md`](../../docs/superpowers/specs/2026-07-20-tamal-lang-design.md)
- Implementation plans: [`docs/superpowers/plans/`](../../docs/superpowers/plans/) — walking skeleton (01), values & CRC (02), frames & verdicts (03).
