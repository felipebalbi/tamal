# Tamal

Tamal is an FPGA-based eSPI compliance & conformance test rig — a work-alike in
spirit to [`mole`](https://github.com/felipebalbi/mole) (I2C/I3C), retargeted to
Intel's Enhanced Serial Peripheral Interface (eSPI, base spec rev 1.0).

It allows developers to:

- Play eSPI **controller or target** on the pins from a single bitstream
- Drive any legal *or* illegal cycle with deterministic timing
- Inject errors exactly when — and only when — the test author asked for them
- Observe the bus and capture every transaction
- Stream every transaction plus a pass/fail verdict to a host for analysis

Named after the wrapped, layered dish, Tamal reflects what eSPI does: it wraps
many logical channels — Peripheral, Virtual Wire, tunneled-SMBus (OOB), and
Runtime Flash Access — into one serial packet stream. Tamal unwraps every layer
and checks it against the spec.

## Key Features

- Programmable engine driven by a compact, RISC-V-32-inspired ISA
- Controller **and** target roles (runtime-selectable)
- Single / dual / quad I/O modes
- Deterministic, compile-time error injection
- Full transaction capture with pass/fail verdicts
- Host interface for control + result streaming

## The tamal ISA (RISC-V-32-inspired)

Tamal's on-FPGA engine is programmable, and its instruction set is **inspired by
— but not 100% compatible with — the RISC-V 32-bit (RV32I) ISA.** It borrows the
32-bit fixed-width instruction shape, the `x0`..`x31` register model (`x0`
hardwired to zero), the R/I/S/B/U/J formats, the familiar ABI register names, and
common directives / pseudo-instructions. It **diverges** by repurposing and
extending the opcode space with eSPI bus operations (drive/sample cycles,
per-channel ops, timing control, error injection, capture/verdict). Tamal
bytecode is therefore **not** interchangeable with a stock RISC-V toolchain.

`tamal-asm` assembles this RISC-V-flavored source into tamal bytecode; the FPGA
engine executes it.

## Philosophy

Compliance should be reproducible.

Tamal makes eSPI behavior — legal and illegal — deterministic and observable.

## Status

Early scaffold. The repository structure builds, but the eSPI engine, the
assembler, and the loader logic are not implemented yet — only a placeholder
heartbeat top entity and stub crates exist. See
[`docs/superpowers/specs/`](docs/superpowers/specs/) for the design and
`AGENTS.md` for orientation.

## Hardware

Target board: **Digilent Arty A7-100T** (`xc7a100tcsg324-1`), 100 MHz system
clock, JTAG programming.

> Note on the transport: the Arty's FT2232 exposes **USB-UART + JTAG**, not a
> USB3 SuperSpeed FIFO. v1 uses UART for the control/result transport. The wire
> format is transport-agnostic, so a future EZ-USB **FX3 (GPIF II slave-FIFO)
> shield** can be added as another backend without ABI changes.

## Layout

```
crates/
  tamal-abi/          transport-agnostic bytecode/ISA encoding + control/result wire types
  tamal-asm/          assembler — RISC-V-flavored tamal asm -> tamal bytecode
  tamal-asm-cli/      `tamal-asm` — assembler CLI
  tamal-loader/       host-side loader: control + result streaming over a transport (UART first)
  tamal-loader-cli/   `tamal-loader` — loader/controller CLI
hdl/                  Clash gateware + Vivado build (self-contained)
docs/                 design notes and plans
```

## Building

### Host tooling (Rust)

```sh
cargo build            # build all crates
cargo run -p tamal-asm-cli -- --help
cargo run -p tamal-loader-cli -- --help
```

### Gateware (Clash -> Vivado)

The gateware is a self-contained project; build it from its own directory:

```sh
cd hdl
make                   # Clash -> Verilog -> Vivado synth/impl/bitstream
make program           # flash the Arty A7 over JTAG
make test              # Haskell unit tests (stack test)
make clean
```

`make` needs GNU Make, a Unix-ish shell (coreutils), `stack`, and Vivado. If
`vivado` isn't on your `PATH`, copy `hdl/build.cfg` to `hdl/build.cfg.local`
and set `VIVADO` there.

## License

Tamal is split-licensed by concern:

- **Host tooling** — the Rust crates under `crates/` — is **MIT**; see
  [`LICENSE`](LICENSE).
- **Gateware** — the Clash/HDL under `hdl/` — is **CERN-OHL-P-2.0** (CERN Open
  Hardware Licence Version 2 – Permissive); see [`hdl/LICENSE`](hdl/LICENSE). A
  permissive open-hardware licence fits hardware description better than a
  software licence like MIT.
