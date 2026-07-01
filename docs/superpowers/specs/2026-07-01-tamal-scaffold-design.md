# Tamal — Initial Repository Scaffold (Design)

Date: 2026-07-01
Status: Approved (scaffold only — no eSPI-engine logic yet)

## 1. Purpose

Stand up the tamal repository skeleton: a Cargo workspace for the Rust host
tooling (a mole-style assembler + loader, each with a CLI), a self-contained
Clash + Vivado gateware project, docs, and licensing. The scaffold must *build*
(Rust stubs compile; the HDL has one real heartbeat top entity), but contains
**no eSPI-engine logic**. Real subsystems land in later, separately-specified
plans.

Tamal is an FPGA-based **eSPI compliance & conformance test rig** — a sibling in
spirit to [`mole`](https://github.com/felipebalbi/mole) (I2C/I3C), retargeted to
Intel's Enhanced Serial Peripheral Interface (eSPI, base spec rev 1.0). It plays
eSPI **controller or target** on the pins, drives any legal *or* illegal cycle
with deterministic timing, injects errors exactly when asked, observes the bus,
and streams every transaction plus a pass/fail verdict to a host for live
analysis.

One-liner: *like a tamal wraps its filling in a husk, eSPI wraps many channels —
Peripheral, Virtual Wire, tunneled-SMBus (OOB), and Runtime Flash Access — in one
serial packet stream. Tamal unwraps every layer and checks it against the spec.*

The architecture has three planes:

- **Control plane** (host → FPGA): load a compiled test program (tamal
  bytecode), set role (controller/target), IO mode (single/dual/quad), CRC
  on/off, error-injection `(seed, ratio)`, and triggers.
- **Bus plane** (FPGA ↔ DUT): the eSPI link itself — `CS#`, `CLK`, `IO[3:0]`,
  `ALERT#`, `RESET#` — driven or sampled against the device under test.
- **Trace / result plane** (FPGA → host): observed transactions, channel decode,
  captured cycles, and verdicts. The stream must tolerate dropped events (an
  overflow marker) so the bus is never blocked by trace backpressure.

## 2. The tamal ISA (RISC-V-32-inspired)

Tamal's on-FPGA engine is programmable. Its instruction set is **inspired by —
but not 100% compatible with — the RISC-V 32-bit (RV32I) ISA**:

- **Borrowed from RV32I:** 32-bit fixed-width instructions; a 32-entry general
  register file `x0`..`x31` with `x0` hardwired to zero; the R / I / S / B / U /
  J instruction-format shapes; the ABI register names (`zero`, `ra`, `sp`, `gp`,
  `tp`, `t0`..`t6`, `s0`..`s11`, `a0`..`a7`); familiar assembler directives
  (`.text`, `.data`, `.word`, `.globl`, `.equ`, `.align`, `.macro`, `.option`);
  numeric local labels (`1f` / `1b`); and common pseudo-instructions (`li`,
  `la`, `mv`, `nop`, `j`, `call`, `ret`, `beqz`).
- **Diverges from RV32I:** tamal repurposes/extends the opcode space with
  bus-domain instructions for eSPI work — driving and sampling eSPI cycles,
  per-channel operations, deterministic timing control, compile-time error
  injection, and capture/verdict — analogous to how mole's engine carries
  distinct WIRE / CTRL / DATA opcode groups. Binaries are therefore **not**
  interchangeable with a stock RISC-V toolchain.

`tamal-asm` assembles this RISC-V-flavored source into tamal bytecode;
`tamal-loader` ships that bytecode to the rig and drains the result stream; the
FPGA engine (future work) executes it.

The RISC-V asm conventions referenced above follow the
[riscv-asm-manual](https://github.com/riscv-non-isa/riscv-asm-manual).

## 3. Target hardware

- **Board:** Digilent Arty A7-100T — part `xc7a100tcsg324-1`.
- **Clock:** 100 MHz system clock (`CLK100MHZ`, pin E3).
- **Programming:** JTAG via Vivado hardware manager (volatile bitstream).
- **Timing headroom:** eSPI tops out at 66 MHz SCK; 100 MHz Artix-7 fabric is
  comfortably fast for the engine — the hard parts are external timing alignment
  (turnaround/tri-state, setup/hold against an externally-driven clock when in
  target role), not fabric speed.
- **Transport reality:** the Arty's FT2232 provides **USB-UART + JTAG only**. v1
  uses UART for the control/trace transport. The wire format stays
  transport-agnostic so a future **FX3 (GPIF II slave-FIFO) shield** can be added
  as another `tamal-loader` backend with no ABI changes.

## 4. Repository layout

```
tamal/
  Cargo.toml              workspace, members = ["crates/*"], resolver 3
  Cargo.lock              committed (we ship CLI binaries)
  README.md               human-facing intro + quickstart (+ ISA note)
  AGENTS.md               agent/contributor orientation + project spirit (+ ISA note)
  LICENSE                 MIT
  .gitignore              Rust + Haskell/Clash + Vivado
  crates/
    tamal-abi/            lib: tamal bytecode/ISA encoding + control/result wire types
    tamal-asm/            lib: assembler — RISC-V-flavored tamal asm -> tamal bytecode
    tamal-asm-cli/        bin `tamal-asm`: clap loader/driver over tamal-asm
    tamal-loader/         lib: host-side loader (UART backend to start)
    tamal-loader-cli/     bin `tamal-loader`: clap loader/controller over tamal-loader
  hdl/                    Clash gateware (self-contained; `cd hdl && make`)
    Makefile build.cfg    Clash -> staged Vivado flow -> JTAG program
    vivado/               build.tcl program.tcl
    constraints/          arty_a7.xdc (part clock + pins)
    bin/                  Clash.hs Clashi.hs (Clash.Main wrappers)
    src/                  Tamal.hs (heartbeat top) + Tamal/Domain.hs
    tests/                unittests.hs (trivial tasty placeholder)
    tamal.cabal stack.yaml hie.yaml
  docs/superpowers/specs/ design + plan docs (this file)
```

## 5. Rust crates (stubs only)

All share `[workspace.package]` metadata (edition 2024, MIT) and pull deps from
`[workspace.dependencies]`. Dependency graph: `asm → abi`, `asm-cli → asm + abi`,
`loader → abi`, `loader-cli → loader + abi`.

- **tamal-abi** — `#![forbid(unsafe_code)]` lib; the project ABI. Doc-only
  modules sketching (a) the tamal bytecode/ISA **encoding** (32-bit fixed-width
  instruction words, register/format model) and (b) the **control** and
  **result/trace** wire types (`LOAD_PROGRAM`, `SET_ROLE`, `SET_IO_MODE`,
  `SET_CRC`, `SET_INJECT(seed, ratio)`, `ARM`/`TRIGGER`; `BusEvent { timestamp,
  channel, cycle_type, tag, length, verdict }` plus an overflow marker). No
  external deps.
- **tamal-asm** — lib depending on `tamal-abi`; placeholder `assemble()` entry
  and doc modules for the lexer/parser/encoder pipeline. Deps: `tamal-abi`,
  `thiserror`.
- **tamal-asm-cli** — bin `tamal-asm`; depends on `tamal-asm` + `tamal-abi`.
  Deps: `clap`, `color-eyre`. Prints a banner for now.
- **tamal-loader** — lib depending on `tamal-abi`; placeholder `Device` +
  `transport` module (UART backend stub). Deps: `tamal-abi`, `thiserror`,
  `serialport`.
- **tamal-loader-cli** — bin `tamal-loader`; depends on `tamal-loader` +
  `tamal-abi`. Deps: `clap`, `color-eyre`. Prints a banner for now.

No TUI crate (matching mole's crate set). Live visualisation, if wanted later,
is a separate plan.

## 6. HDL build flow (Clash -> Vivado)

Identical in shape to molcajete's gateware, retargeted in name only. Vivado has
no discrete `map/fit/asm` CLIs; the idiom is Tcl batch. The Makefile runs the
whole implementation in a **single non-project flow** (`vivado/build.tcl`, one
Vivado launch) — Vivado's tool-startup cost dominates a design this size — while
still writing intermediate checkpoints/reports so each stage stays inspectable:

1. **Clash -> Verilog** (`stack run clash -- Tamal --verilog`) ->
   `verilog/Tamal.topEntity/`.
2. Stage the HDL into `_build/Tamal/01-hdl/`.
3. **build** (`vivado -mode batch -source vivado/build.tcl`): synth -> opt ->
   place -> route -> `write_bitstream`, emitting `post_synth.dcp`,
   `post_route.dcp`, timing/util/DRC reports, and `tamal.bit`.
4. **`make program`** (`vivado -mode batch -source vivado/program.tcl`) -> JTAG.

Tool paths live in `build.cfg` (`VIVADO = vivado`, `CLASH = stack run clash --`,
`TEST = stack test`), overridable via gitignored `build.cfg.local`. Constraints
are a hand-written Vivado **XDC** (clock + pins). The Windows/nushell Makefile
notes and the Clash 1.10 `stack.yaml` pin carry over from molcajete verbatim.

The current top entity is a **placeholder heartbeat** (`Tamal.hs`): a
free-running counter whose MSB blinks the board LED, so the Clash -> Vivado
pipeline has a real synthesizable entity to build until the eSPI cycle engine
lands. There is intentionally no reset port (registers rely on power-up `init`).
The XDC constrains only `clk` (E3) and `led` (H5) for now; the real eSPI pins
(`CS#`, `CLK`, `IO[3:0]`, `ALERT#`, `RESET#`) land with the engine.

## 7. Decisions

- License: **MIT** only (single `LICENSE` file).
- `Cargo.lock` is committed.
- No top-level Makefile — the HDL is built with `cd hdl && make`; Rust with
  `cargo`.
- New standalone git repository (`main` branch), sibling to molcajete.
- No plain `tamal` binary; the CLIs are `tamal-asm` and `tamal-loader` (matching
  mole's `mole-asm` / `mole-loader`).
- Both `README.md` and `AGENTS.md` carry the note that the tamal ISA is inspired
  by, but not 100% compatible with, the RISC-V 32-bit ISA.

## 8. Explicitly out of scope (later plans)

The eSPI link layer (single/dual/quad IO, TAR turnaround, CRC), the transaction
layer (cycle types, tags, the four channels), controller/target role logic,
deterministic compile-time error injection, the result-ring / trace stream, the
verdict engine, the real `tamal-asm` lexer/parser/encoder, the real
`tamal-loader` transport, the FPGA cycle engine itself, and any test-vector DSL.
The scaffold only guarantees the structure builds.
