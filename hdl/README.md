# Tamal gateware (`hdl/`)

The Clash FPGA gateware for the tamal eSPI compliance rig, targeting the
**Digilent Arty A7-100T** (`xc7a100tcsg324-1`, 100 MHz). Self-contained: Clash →
Verilog → Vivado bitstream, driven by `make`. Not a Cargo workspace member.

Licensed **CERN-OHL-P-2.0** (see [`LICENSE`](LICENSE)); every `hdl/**/*.hs` carries
a REUSE-style SPDX header. (The Rust host tooling under `crates/` is MIT.)

## What it does

A host loads a compiled tamal program over UART; the on-FPGA engine executes it,
driving/sampling the eSPI bus with deterministic timing and recording every
transaction into a trace ring; on `HALT` the trace is drained back to the host.
The whole path is one clock domain (`Dom100`, 100 MHz) — no PLL, no CDC, no FIFOs
(the trace ring BRAM is the buffer; UART is ~500× slower than the fabric).

```
 host ──UART──► loader ──► instr BRAM ──► engine (mealy step) ──► eSPI pads (IO[3:0], CS#, SCK, RESET#, ALERT#)
      ◄─UART──── drain ◄──── ring BRAM ◄──────┘  trace records
```

- **`Tamal.Top` `system`** wires the block RAMs, loader, UART, and engine
  (`mealy stepM initState`) over plain `Signal`s — no `BiSignal`, so the whole
  integration is simulated end-to-end.
- **`Tamal` (`topEntity`)** is the thin pin shell: the 100 MHz clock, `espiPads`
  (tri-state IO), and the named ports the XDC binds. No reset port — registers rely
  on power-up `init` (like the sibling Clash examples).

The module-by-module map and roadmap live in [`PLAN.md`](PLAN.md).

## Build

```sh
make                 # Clash -> Verilog -> Vivado synth/place/route -> tamal.bit
make program         # flash the Arty A7 over JTAG
make test            # Haskell unit + cosim tests (cabal test)
make format          # fourmolu (in place)
make format-check    # fourmolu style gate
make clean
```

Two stages (both under `_build/Tamal/`): (1) `cabal run clash -- Tamal --verilog`
emits `verilog/Tamal.topEntity/`; (2) a single non-project Vivado flow
(`vivado/build.tcl`) synthesizes to `tamal.bit`, writing timing/util/DRC reports
for inspection. Tool paths come from `build.cfg` (copy to `build.cfg.local` to
override; `VIVADO` defaults to `vivado` on `PATH`). Needs GNU Make, a Unix-ish
shell, GHC 9.10.3 + `cabal` (e.g. via `ghcup`), and Vivado.

Interactive Clash REPL: `cabal run clashi` (e.g. `sampleN`, `:verilog <expr>`).

## Testing

`cabal test` runs the tasty suite (hedgehog + HUnit). Each pure leaf and the
engine keystone are property-tested against reference models. The integration is
covered by:

- **Pure helpers** — `stepM`, `ringWrite`, `rigState`, `ledPattern` (hedgehog/HUnit).
- **Whole-system cosim** (`Test.Top`) — serialize a `Tamal.Wire` `LOAD_PROGRAM` +
  `TRIGGER` onto the UART line, run load → run → drain, decode the UART output, and
  assert the drained trace (`REVISION` + records + `HALT` terminator) plus eSPI pin
  activity. This exercises UART → loader → engine → eSPI → ring → drain in one shot.
- **Codegen gate** — `cabal run clash -- Tamal --verilog` must emit the top with
  the four `inout` IO lanes; then `make` (Vivado) is the ultimate gate.

## Pin map (Arty A7-100T)

The eSPI bus is on two neighbouring Pmods (JA data / JB control) so an adapter is
easy to build and each lane has a nearby ground return. Confirm the physical Pmod
pair against your board; only the `PACKAGE_PIN`s change. Constraints live in
[`constraints/arty_a7.xdc`](constraints/arty_a7.xdc).

| Signal              | Port         | Dir   | Pin                | Note                 |
|---------------------|--------------|-------|--------------------|----------------------|
| clock (100 MHz)     | `clk`        | in    | E3                 |                      |
| UART RX (from host) | `uart_rx`    | in    | D10                | FTDI                 |
| UART TX (to host)   | `uart_tx`    | out   | A9                 | FTDI                 |
| `IO[0..3]`          | `io0`..`io3` | inout | G13, B11, A11, D12 | JA, `PULLUP`         |
| `SCK`               | `sck`        | out   | E15                | JB                   |
| `CS#`               | `cs_n`       | out   | E16                | JB                   |
| `RESET#`            | `reset_n`    | out   | D15                | JB                   |
| `ALERT#`            | `alert_n`    | in    | C15                | JB, `PULLUP`         |
| status LED (LD4)    | `led`        | out   | H5                 | Waiting/Running/Done |

The LED encodes the rig lifecycle: slow heartbeat = waiting for a program, fast
blink = running, solid = halted (trace ready). It's the first on-hardware sanity
check.

## Clash notes

- Single `topEntity` in `src/Tamal.hs`; `Dom100` domain in `src/Tamal/Domain.hs`.
- **Four scalar `inout` lanes.** Clash fuses a per-lane `BiSignalIn` argument with
  its matching `BiSignalOut` result into one `inout` port; a `Vec` of BiSignals
  does *not* (it lowers to a plain input), so the IO lanes are `io0`..`io3`, not a
  bus.
- **No reset port** — the top ties reset permanently de-asserted; Clash emits no
  `reset` port.
- **Test idioms.** `sampleN` asserts the Dom100 async reset on cycle 0, so
  register-based harnesses lead with one idle cycle. The cosim's UART serializer
  leaves one idle bit-time between bytes — the RX drops truly back-to-back bytes on
  the falling-edge resync (a realistic transmitter always leaves inter-byte idle).
- The `common-options` ghc-options in `tamal.cabal` are load-bearing for Clash —
  don't trim them.

See `docs/superpowers/specs/` for the per-piece design docs.
