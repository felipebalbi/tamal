# tamal-lang

The **compiler** for tamal-lang: a high-level language that lowers to tamal
assembly text and, through [`tamal_asm::assemble`](../tamal-asm), to tamal
bytecode.

Part of the [tamal](../../README.md) eSPI compliance rig. **MIT-licensed.** The
command-line front-end is [`tamal-lang-cli`](../tamal-lang-cli) (the `tamalc`
binary).

## Status

**Early walking skeleton** (Plan 1). It supports only the smallest useful
subset: a single `test NAME { … }` block, the `pass` and `fail N` verdicts, and
verbatim **raw-instruction pass-through** (any tamal-asm mnemonic written
as-is). The front-end (lexer → parser → emit) lowers `.tam` source to tamal-asm
text and hands it to the existing `tamal_asm::assemble` backend for encoding.
Types, const-eval, named variables, `send`/`recv`, control flow, and modules
land in later plans — do not expect them yet.

## Public API

- `lower_to_asm(source) -> Result<String, Vec<Diagnostic>>` — `.tam` → tamal-asm text.
- `compile(source) -> Result<Program, Vec<Diagnostic>>` — `.tam` → bytecode.

Diagnostics reuse `tamal_asm::{Diagnostic, Severity, Span}` carrying `.tam` byte spans.

## See also

- Language design: [`docs/superpowers/specs/2026-07-20-tamal-lang-design.md`](../../docs/superpowers/specs/2026-07-20-tamal-lang-design.md)
- Walking-skeleton plan: [`docs/superpowers/plans/2026-07-20-tamal-lang-01-walking-skeleton.md`](../../docs/superpowers/plans/2026-07-20-tamal-lang-01-walking-skeleton.md)
