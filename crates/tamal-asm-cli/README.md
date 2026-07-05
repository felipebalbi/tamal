# tamal-asm-cli

The command-line front-end for [`tamal-asm`](../tamal-asm) — assemble tamal source
to bytecode, or disassemble bytecode to a listing. Ships the **`tamal-asm`**
binary. Diagnostics render with [ariadne](https://crates.io/crates/ariadne).

Part of the [tamal](../../README.md) eSPI compliance rig. **MIT-licensed.**

## Usage

```sh
# Assemble to loader-ready little-endian bytecode (default: <input>.bin)
tamal-asm assemble prog.s
tamal-asm assemble prog.s -o prog.bin

# Other emit formats
tamal-asm assemble prog.s --emit hex        # one 8-digit hex word per line (stdout)
tamal-asm assemble prog.s --emit listing    # addr  word  mnemonic ; source (stdout)

# Disassemble bytecode back to a listing
tamal-asm disasm prog.bin
```

Run `tamal-asm --help` / `tamal-asm assemble --help` for the full surface.

| Subcommand | What it does |
|------------|--------------|
| `assemble <input> [-o <out>] [--emit bin\|hex\|listing]` | Assemble `.s` source. `bin` (default) writes raw LE words to `<input>.bin`; `hex`/`listing` print to stdout unless `-o` is given. |
| `disasm <input> [-o <out>]` | Decode a `.bin` into a textual listing. |

On an assembly error the process exits non-zero and prints span-anchored ariadne
diagnostics pointing at the offending source.

## Where it sits

```
tamal-asm-cli ──► tamal-asm ──► tamal-abi
              └──► tamal-abi
```

The `.bin` it emits is **raw** little-endian bytecode — no COBS, no CRC, no
framing. Shipping it to a rig (COBS + CRC framing over UART) is
[`tamal-loader`](../tamal-loader-cli)'s job.

## Build / run

```sh
cargo run -p tamal-asm-cli -- assemble examples/peripheral_io_read.s -o /tmp/prog.bin
cargo install --path crates/tamal-asm-cli   # installs the `tamal-asm` binary
```

## See also

- Library: [`tamal-asm`](../tamal-asm)
- Assembler design: [`docs/superpowers/specs/2026-07-04-tamal-asm-design.md`](../../docs/superpowers/specs/2026-07-04-tamal-asm-design.md)
