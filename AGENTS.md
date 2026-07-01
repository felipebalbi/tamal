# AGENTS.md — tamal

An FPGA-based **eSPI compliance & conformance test rig**: it plays eSPI
**controller or target** on the pins, drives any legal *or* illegal cycle with
deterministic timing, injects errors exactly when asked, observes the bus, and
streams every transaction plus a pass/fail verdict to a host. A work-alike in
spirit to [`mole`](https://github.com/felipebalbi/mole) (I2C/I3C), retargeted to
Intel's Enhanced Serial Peripheral Interface (eSPI, base spec rev 1.0). See
`README.md` for the human-facing intro and `docs/superpowers/specs/` for
designs.

## The one-line pitch

> A programmable eSPI controller/target that turns compliance testing into a
> reproducible, fully-observable, byte-for-byte deterministic exercise.

## Architecture in three planes

- **Control plane** (host → FPGA): load a compiled test program (tamal
  bytecode), set role (controller/target), IO mode (single/dual/quad), CRC,
  error-injection `(seed, ratio)`, and triggers.
- **Bus plane** (FPGA ↔ DUT): the eSPI link itself — `CS#`, `CLK`, `IO[3:0]`,
  `ALERT#`, `RESET#` — driven or sampled against the device under test.
- **Trace / result plane** (FPGA → host): observed transactions, channel decode,
  captured cycles, and verdicts. Never block the bus on trace backpressure —
  drop events with an overflow marker instead.

## The key insight

> This is **not** a throughput problem. It is an **external timing alignment**
> problem. In target role you are not the clock master — the eSPI clock is driven
> externally and you must respond relative to it. The dangerous parts are
> turnaround/tri-state timing, setup/hold against the clock, and IO direction
> control — not fabric speed (eSPI tops out at 66 MHz; that is slow for
> Artix-7).

## The tamal ISA (RISC-V-32-inspired)

> Tamal's on-FPGA engine is programmable. Its instruction set is **inspired by —
> but not 100% compatible with — the RISC-V 32-bit (RV32I) ISA.**

- **Borrowed from RV32I:** 32-bit fixed-width instructions; a 32-entry register
  file `x0`..`x31` with `x0` hardwired to zero; the R/I/S/B/U/J format shapes;
  the ABI register names (`zero`/`ra`/`sp`/`gp`/`tp`/`t0`../`s0`../`a0`..);
  familiar directives (`.text`, `.data`, `.word`, `.globl`, `.equ`, `.align`,
  `.macro`, `.option`); numeric local labels (`1f`/`1b`); and common
  pseudo-instructions (`li`, `la`, `mv`, `nop`, `j`, `call`, `ret`, `beqz`).
- **Diverges from RV32I:** tamal repurposes/extends the opcode space with
  bus-domain instructions for eSPI work — driving/sampling cycles, per-channel
  ops, deterministic timing, compile-time error injection, and capture/verdict.
  Binaries are **not** interchangeable with a stock RISC-V toolchain.

`tamal-asm` assembles this RISC-V-flavored source (conventions follow the
[riscv-asm-manual](https://github.com/riscv-non-isa/riscv-asm-manual)) into tamal
bytecode; `tamal-loader` ships it to the rig; the FPGA engine executes it.

## Repository layout

```
crates/                 Rust host tooling (Cargo workspace, members = crates/*)
  tamal-abi/            transport-agnostic bytecode/ISA encoding + control/result wire types (the ABI)
  tamal-asm/            assembler: RISC-V-flavored tamal asm -> tamal bytecode
  tamal-asm-cli/        `tamal-asm` binary — clap assembler front-end
  tamal-loader/         host-side loader: control + result over a transport
  tamal-loader-cli/     `tamal-loader` binary — clap loader/controller
hdl/                    Clash gateware + Vivado build (self-contained)
docs/superpowers/specs/ design + implementation-plan documents
```

Rust is built with `cargo` from the repo root. The gateware is built
**separately** with `cd hdl && make` — it is not a Cargo workspace member. The
dependency graph is `asm → abi`, `asm-cli → asm + abi`, `loader → abi`,
`loader-cli → loader + abi`.

## Licensing

Split-licensed along the same `crates/` vs `hdl/` boundary:

- **Host tooling** — the Rust crates under `crates/` — is **MIT** (root
  [`LICENSE`](LICENSE); `license = "MIT"` in the workspace `Cargo.toml`).
- **Gateware** — the Clash/HDL under `hdl/` — is **CERN-OHL-P-2.0** (CERN Open
  Hardware Licence v2 – Permissive; [`hdl/LICENSE`](hdl/LICENSE)). A permissive
  open-hardware licence fits hardware description better than a software licence
  like MIT.

Every `hdl/**/*.hs` file carries a REUSE-style header
(`SPDX-FileCopyrightText` + `SPDX-License-Identifier: CERN-OHL-P-2.0`) — keep it
on any new HDL file. `hdl/tamal.cabal` uses `license: LicenseRef-CERN-OHL-P-2.0`
(not the bare `CERN-OHL-P-2.0`) **only** because the pinned Cabal 3.12 predates
the SPDX-list entry added in Cabal-syntax 3.14; switch to the bare id once the
`stack.yaml` snapshot moves to Cabal ≥ 3.14. Note `CERN_OHL_P_2_0` (underscores)
is just the Haskell constructor in `Distribution.SPDX.LicenseId` — the cabal
field and SPDX headers use the hyphenated string.

## Target hardware

- **Digilent Arty A7-100T**, part `xc7a100tcsg324-1`, 100 MHz clock (pin E3).
- Programmed over **JTAG** via the Vivado hardware manager.
- **Transport:** the Arty exposes **USB-UART + JTAG only** (no USB3). v1 uses
  UART. Keep `tamal-abi` transport-agnostic so a future **FX3 GPIF II slave-FIFO
  shield** (cf. `circuitvalley/mipi_csi_receiver_FPGA`) drops in as another
  `tamal-loader` transport backend. A literal FX3 *Pmod* is infeasible — a
  16/32-bit GPIF bus is ~26–42 source-synchronous signals at 100 MHz, which
  wants an FMC/shield, not 8 Pmod pins.

## HDL build flow (Clash → Vivado)

Two stages, mirroring the sibling Clash/Quartus examples but retargeted to
Vivado's Tcl-batch model (no discrete `map/fit/asm` binaries):

1. **Clash → Verilog:** `stack run clash -- Tamal --verilog` →
   `verilog/Tamal.topEntity/`.
2. **Verilog → bitstream:** stage HDL into `_build/Tamal/01-hdl/`, then a
   **single non-project flow** (`vivado/build.tcl`, one Vivado launch) under
   `_build/Tamal/02-vivado/`: synth → opt → place → route → `tamal.bit`.
   It still writes `post_synth.dcp`/`post_route.dcp` + timing/util/DRC reports
   for inspection — one launch instead of three because Vivado's startup cost
   dominates at this size. `program.tcl` flashes over JTAG.

- Tool paths come from `hdl/build.cfg` (copy to `build.cfg.local` to override;
  it is gitignored). `VIVADO` defaults to `vivado` on `PATH`.
- Constraints are a hand-written Vivado **XDC** (`constraints/arty_a7.xdc`) for
  the clock + pins — not Clash's Quartus-oriented `.sdc`.
- `make` expects GNU Make + a Unix-ish shell (coreutils), like the sevenseg
  template it is modelled on.

## Clash notes

- Single `topEntity` in `src/Tamal.hs`; `Dom100` clock domain in
  `src/Tamal/Domain.hs`. The current top is a **placeholder heartbeat**
  (an LED blink) so the Clash→Vivado pipeline has a real entity to build until
  the eSPI cycle engine lands.
- No reset port: the top ties reset permanently de-asserted
  (`unsafeFromActiveHigh (pure False)`), relying on power-up `init`, like the
  sibling examples; Clash then emits no `reset` port.
- The `common-options` ghc-options in `tamal.cabal` are load-bearing for
  Clash — don't trim them. Don't bump Clash off the `stack.yaml` pin without
  updating the `clash-prelude` bound.

## Implementation phases (north star)

1. Link + transaction bring-up: SPI-style framing, command/response/turnaround,
   Get Status / Get Configuration, CRC; controller role over single I/O.
2. The four channels: Peripheral, Virtual Wire, OOB (tunneled SMBus), Runtime
   Flash Access; the tamal ISA + assembler + loader path; result streaming.
3. Target role, alert handling, dual/quad I/O modes.
4. Deterministic compile-time error injection, the verdict engine, and the
   conformance catalog.

eSPI semantics minimums: honour command/response phases and turnaround (TAR);
respect CRC when enabled; track the status register (free/non-free, channel
ready); tolerate WAIT STATE; drive/observe alerts.

## What NOT to do

- Don't block the bus on trace backpressure — drop with an overflow marker.
- Don't give the top a reset port (the no-reset power-up design is deliberate).
- Don't chase the full eSPI channel matrix at once — get link + one channel
  solid first.
- Don't make `tamal-abi` depend on a transport — keep the wire format and the
  bytecode encoding transport-agnostic.
- Don't claim RISC-V compatibility — the ISA is *inspired by* RV32I, not
  binary-compatible with it.
- Don't add runtime randomness to error injection — it must be compile-time and
  reproducible (`ratio = 0` is exactly zero; `(seed, ratio)` replays byte for
  byte).
- Don't relicense `hdl/` under MIT or strip the SPDX headers — the gateware is
  CERN-OHL-P-2.0; only the Rust host tooling under `crates/` is MIT.
