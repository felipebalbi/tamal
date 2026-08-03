# tamal-lang-cli

The command-line front-end for [`tamal-lang`](../tamal-lang) — compile `.tam`
source to bytecode, the generated tamal assembly, or a listing. Ships the
**`tamalc`** binary. Diagnostics render with
[ariadne](https://crates.io/crates/ariadne).

Part of the [tamal](../../README.md) eSPI compliance rig. **MIT-licensed.**

## Status

**Early walking skeleton** (Plan 1). `tamalc compile <input> --emit bin|asm|listing`
compiles the Plan-1 language subset — a single `test NAME { … }` with `pass`,
`fail N`, and raw-instruction pass-through — by lowering to tamal-asm text and
assembling it through `tamal_asm::assemble`. `bin` (default) writes `<input>.bin`;
`asm`/`listing` print to stdout unless `-o` is given. On a source error it exits
non-zero with an ariadne diagnostic pointing at the `.tam` source.

## See also

- Library: [`tamal-lang`](../tamal-lang)
- Language design: [`docs/superpowers/specs/2026-07-20-tamal-lang-design.md`](../../docs/superpowers/specs/2026-07-20-tamal-lang-design.md)
- Walking-skeleton plan: [`docs/superpowers/plans/2026-07-20-tamal-lang-01-walking-skeleton.md`](../../docs/superpowers/plans/2026-07-20-tamal-lang-01-walking-skeleton.md)
