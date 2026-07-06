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

**Gateware (`hdl/`): v1 complete, in simulation.** The full pipeline is built and
tested in Clash — the RISC-V-flavored cycle engine, the instruction + trace-ring
block RAMs, the COBS/CRC-8 wire format, the UART load/drain loader, the tri-state
eSPI pad boundary, and the `topEntity` that wires it all to the Arty A7 pins. A
whole-system cosim streams a program in over UART, runs it, and checks the drained
trace end to end; `cabal run clash -- Tamal.Board.ArtyA7 --verilog` emits a
synthesizable top (four `inout` IO lanes) and `cd hdl && make` builds a
bitstream. v1 is controller role, single (x1) I/O, UART transport; on-hardware
bring-up, target role, dual/quad I/O, and the error-injection + verdict engine
are the next phases. See
[`hdl/README.md`](hdl/README.md).

**Host tooling (`crates/`): v1 implemented.** The Rust ABI, assembler, and loader
are built and tested, mirroring the gateware's wire/bytecode contract byte-for-byte:
`tamal-abi` (ISA encoding + COBS/CRC-8 wire format + typed trace decode),
`tamal-asm`/`tamal-asm-cli` (RISC-V-flavored source → raw bytecode), and
`tamal-loader`/`tamal-loader-cli` (COBS/CRC framing, UART transport, and the
load → trigger → drain session with a decoded-trace + HALT/TRAP verdict). The live
serial path is exercised on hardware rather than in CI; the pass/fail conformance
verdict engine is a later phase. Each crate carries its own README; see also
[`docs/superpowers/specs/`](docs/superpowers/specs/) for the designs and `AGENTS.md`
for orientation.

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
cargo test             # run the host test suite

# assemble a program, then load + run it on a connected rig
cargo run -p tamal-asm-cli    -- assemble examples/peripheral_io_read.s -o /tmp/prog.bin
cargo run -p tamal-loader-cli -- run /tmp/prog.bin --port /dev/tty.usbserial-XXXX
```

### Gateware (Clash -> Vivado)

The gateware is a self-contained project; build it from its own directory:

```sh
cd hdl
make                   # Clash -> Verilog -> Vivado synth/impl/bitstream
make program           # flash the Arty A7 over JTAG
make test              # Haskell unit tests (cabal test)
make clean
```

`make` needs GNU Make, a Unix-ish shell (coreutils), GHC 9.10.3 + `cabal` (e.g.
via `ghcup`), and Vivado. If
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
