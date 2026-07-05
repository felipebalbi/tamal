# tamal-asm

The **assembler** for the tamal ISA: it parses RISC-V-flavored tamal assembly and
emits tamal bytecode in the [`tamal-abi`](../tamal-abi) `isa` encoding.

Part of the [tamal](../../README.md) eSPI compliance rig. **MIT-licensed.** The
command-line front-end is [`tamal-asm-cli`](../tamal-asm-cli).

## The language

Conventions follow the
[riscv-asm-manual](https://github.com/riscv-non-isa/riscv-asm-manual), but the ISA
is **inspired by — not compatible with — RV32I**. Borrowed: 32-bit fixed-width
instructions; `x0`..`x31` with `x0` hardwired to zero; the ABI register names;
directives (`.text`, `.data`, `.word`, `.globl`, `.equ`, `.align`, `.macro`,
`.option`); numeric local labels (`1f`/`1b`); and pseudo-instructions (`li`, `la`,
`mv`, `nop`, `j`, `call`, `ret`, `beqz`). Diverged: bus-domain opcodes for eSPI
work (drive/sample cycles, per-channel ops, deterministic timing, capture/verdict).
The bytecode is **not** interchangeable with a stock RISC-V toolchain.

## Public API

```rust
use tamal_asm::{assemble, Diagnostic};

let src = "start:\n  cs_assert\n  put_byte_imm 0x0F\n  halt 0\n";
match assemble(src) {
    Ok(prog) => {
        let _bytes: Vec<u8> = prog.to_le_bytes();       // loader-ready LE bytecode
        let _words = prog.words().collect::<Vec<u32>>(); // 32-bit instruction words
        let _listing = prog.listing(src);               // addr / word / mnemonic ; source
    }
    Err(diags) => {
        // Rich, span-carrying diagnostics; the CLI renders them with ariadne.
        for d in &diags { eprintln!("{}", d.message); }
    }
}
```

- [`assemble`]`(source) -> Result<Program, Vec<Diagnostic>>` — the one entry point.
- [`Program`] exposes `words()`, `to_le_bytes()`, and `listing(source)`.
- [`disasm::disassemble`]`(bytes) -> String` — the inverse: bytecode → a listing.
- The pipeline modules ([`lexer`], [`parser`], [`encoder`], [`symbol`],
  [`mnemonics`], [`diagnostics`]) are public for tooling that needs the stages.

Diagnostics carry byte [`Span`]s and a [`Severity`], so a front-end can point at
the exact offending source.

## Where it sits

```
tamal-asm ──► tamal-abi   (isa: the instruction encoding it emits)
```

Its only dependency is `tamal-abi`.

## Status

Complete and tested (lexer/parser/encoder unit tests, `assemble`/`disasm`
round-trips). Emits raw little-endian bytecode; framing for the wire (COBS + CRC)
is [`tamal-loader`](../tamal-loader)'s job, not the assembler's.

## See also

- Assembler design: [`docs/superpowers/specs/2026-07-04-tamal-asm-design.md`](../../docs/superpowers/specs/2026-07-04-tamal-asm-design.md)
- Example programs: [`examples/`](../../examples)

[`assemble`]: https://docs.rs/tamal-asm/latest/tamal_asm/fn.assemble.html
[`Program`]: https://docs.rs/tamal-asm/latest/tamal_asm/struct.Program.html
[`disasm::disassemble`]: https://docs.rs/tamal-asm/latest/tamal_asm/disasm/fn.disassemble.html
[`lexer`]: https://docs.rs/tamal-asm/latest/tamal_asm/lexer/
[`parser`]: https://docs.rs/tamal-asm/latest/tamal_asm/parser/
[`encoder`]: https://docs.rs/tamal-asm/latest/tamal_asm/encoder/
[`symbol`]: https://docs.rs/tamal-asm/latest/tamal_asm/symbol/
[`mnemonics`]: https://docs.rs/tamal-asm/latest/tamal_asm/mnemonics/
[`diagnostics`]: https://docs.rs/tamal-asm/latest/tamal_asm/diagnostics/
[`Span`]: https://docs.rs/tamal-asm/latest/tamal_asm/diagnostics/struct.Span.html
[`Severity`]: https://docs.rs/tamal-asm/latest/tamal_asm/diagnostics/enum.Severity.html
