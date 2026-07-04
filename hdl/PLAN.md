# Tamal HDL — build plan / next steps

Living roadmap for the Clash gateware. Tracks what pure cores exist, what's
missing, and the order to build them. Anchored to the ISA & HDL Engine design
(`docs/superpowers/specs/2026-07-01-tamal-isa-design.md`, esp. §6, §10, §11), the
ALU/branch design (`docs/superpowers/specs/2026-07-01-tamal-alu-branch-design.md`),
the register-file design (`.../2026-07-01-tamal-register-file-design.md`), and the
UART design (`.../2026-07-01-tamal-uart-design.md`).

## Where things stand

Every **pure leaf core** from the §10 module decomposition is built and
hedgehog-tested — the compute layer (ALU + branch), the **register file**, and
the **UART transport** included — and the keystone **`Engine.step`** Mealy is
built and hedgehog-tested too. The impure shell is complete as well — the
**instruction + ring BRAMs** (`Tamal.Mem`), the **wire-format core**
(`Tamal.Wire`), the **UART load/drain loader** (`Tamal.Loader`), the **eSPI pad
boundary** (`Tamal.Io`), and the **integration** (`Tamal.Top`'s `system` + the
`topEntity` shell) — all built and tested, with a whole-system UART/eSPI cosim.
tamal now builds to a v1 bitstream (controller role, x1 I/O, UART transport).

| Module              | Purpose                                                                                                    | Status                                |
|---------------------|------------------------------------------------------------------------------------------------------------|---------------------------------------|
| `Tamal.Isa`         | `Instr` ADT, total `encode`/`decode`, reserved-field trap                                                  | done, tested                          |
| `Tamal.Crc`         | `crc8Update` (poly 0x07, init 0x00, MSB-first)                                                             | done, tested                          |
| `Tamal.Bus.Serdes`  | x1 serialize/deserialize + `tarBeat`                                                                       | done, tested                          |
| `Tamal.Config`      | `SET_CONFIG` decode                                                                                        | done, tested                          |
| `Tamal.Trace`       | ring record encode (+ trap flag/reason) + `ringPush` overflow                                              | done, tested                          |
| `Tamal.Alu`         | `alu` core + `dataResult` wrapper (DATA group)                                                             | done, tested                          |
| `Tamal.Branch`      | `branchTaken` comparator (CTRL group, unsigned)                                                            | done, tested                          |
| `Tamal.RegFile`     | 16×32 register file, `x0` hardwired 0                                                                      | done, tested                          |
| `Tamal.Uart.*`      | 8N1 UART (NCO tick, RX, TX, umbrella) — host transport                                                     | done, tested                          |
| `Tamal.Engine`      | `step` Mealy: fetch/decode/datapath + SCK bus FSM + trace + HALT/TRAP; `ringPtrOut` drain-depth projection | done, tested                          |
| `Tamal.Mem`         | instr + ring memories (`blockRamPow2`; 1024×32, 4096×32)                                                   | done, tested                          |
| `Tamal.Wire.Cobs`   | COBS encode/decode (`0x00`-delimiter framing)                                                              | done, tested                          |
| `Tamal.Wire`        | LE word↔bytes, CRC-8 fold, control/result frame + message layer                                            | done, tested                          |
| `Tamal.Loader.Cobs` | streaming COBS decode/encode step functions (embedded in the loader mealy)                                 | done, tested                          |
| `Tamal.Loader`      | UART load/drain lifecycle FSM (`RxControl→Run→Drain`)                                                      | done, tested                          |
| `Tamal.Io`          | eSPI pad boundary: per-lane `BiSignal` `IO[3:0]` tri-state + `CS#`/`SCK`/`RESET#` buffers + `ALERT#` sync   | done, tested                          |
| `Tamal.Top`         | `system` (BRAMs + loader + UART + engine `mealy stepM`) + pure helpers (`stepM`/`ringWrite`/`rigState`/`ledPattern`) | done, tested (cosim)        |
| `Tamal.Domain`      | `Dom100` clock domain                                                                                      | done                                  |
| `Tamal` (top)       | synthesis entry point: clock + `espiPads` + named pins (4 scalar `inout` IO lanes)                          | done                                  |

Nothing absent: the full pipeline (host UART → loader → engine → eSPI pads →
trace → drain) is wired and cosim-tested; `cabal run clash -- Tamal --verilog`
emits the top with four `inout` IO lanes (`io0`..`io3`).

## What remains: the impure `topEntity` shell

`Engine.step` is done and hedgehog-tested (92 engine+leaf properties). Every
*pure* core exists. What's left is the **impure shell** that lifts `step` onto
the board: memories, the host load/drain path, tri-state IO, and the top that
wires it all to the pins.

This shell decomposes into **five independently-startable pieces** (the first
four — **BRAM**, the **wire protocol**, the **loader**, and **IOBUF** — are now
complete). Each remaining one is meant
to be picked up in its own fresh session and taken through the repo's standard
cycle — **brainstorming → writing-plans → TDD** (see "Per-piece workflow" below).
Build order is **BRAM (done) → wire protocol (done) → loader (done) → IOBUF (done) → topEntity**;
`topEntity` integrates everything and is deliberately last.

```
   host ──UART──►┌─────────┐  start   ┌────────────┐  pcOut   ┌────────────┐
                 │ loader  │─────────►│  Engine    │─────────►│ instr BRAM │
                 │  FSM    │  halted  │  (mealy    │◄instrWord└────────────┘
                 │(load/   │◄─────────│   step)    │
                 │ drain)  │  ring rd │            │ Maybe   ┌────────────┐
                 │         │◄─────────│            │  Ring   │  ring BRAM │
                 └─────────┘          └─────┬──────┘────────►└────────────┘
                                            │ lanesOut (o,oe), cs/sck/rst
                                      ┌─────▼──────┐
                                      │  IOBUF     │◄──► IO[3:0] pads, ALERT#
                                      └────────────┘
```

### Decisions already locked (apply across the shell)

- **Engine lift = plain `mealy`.** Wrap `step` with a thin adapter and lift it;
  `step` stays a pure, untouched, tested function:
  ```haskell
  stepM :: State -> BusIn -> (State, (BusOut, Maybe Ring))
  stepM s i = let (s', bo, mr) = step s i in (s', (bo, mr))
  engine = mealy stepM initState   -- Signal dom BusIn -> Signal dom (BusOut, Maybe Ring)
  ```
  No `mealyS`/`mealySB` and no `Bundle` instances for the engine (evaluated and
  rejected — the boilerplate is already factored into helpers, and the pure form
  keeps the cosim tests trivial). **Revisit `mealyS` for the loader FSM only** — a
  long sequential state machine is where the State-monad idiom actually pays off.
- **Memory geometry + `termAddr` reconciliation — RESOLVED (piece 1).** `Tamal.Mem`
  pins the instruction store at 1024 words (`AW = 10`, ≈ 1 BRAM36) and the ring at
  4096 words (`Unsigned 12`, ≈ 4 BRAM36), so `termAddr = maxBound :: Unsigned 12 =
  D − 1` is already correct — the engine needed **no** behavioral change (only a
  `termAddr` doc-comment). This 4096 value feeds the loader's drain length too.
 - **Wire-format gap — RESOLVED (piece 2).** The control/result framing now exists
   as a pure Clash core (`Tamal.Wire.Cobs` + `Tamal.Wire`, hedgehog-tested), so the
   loader has a contract to speak. The Rust `crates/tamal-abi` (`control`, `trace`)
   mirror stays a placeholder — deferred to post-silicon, to implement the same
   contract once the gateware is validated.
- **Board / XDC.** Arty A7-100T; USB-UART on the FTDI pins; eSPI pins on Pmods.
  `constraints/arty_a7.xdc` currently constrains only `clk` + `led`; every piece
  that adds pins extends it (pin numbers from Digilent's Arty-A7-100 master XDC).

---

### 1. BRAM — instruction + ring memories  *(done — `Tamal.Mem`)*

- **What:** two Clash `blockRamPow2`s (read-address + `Maybe (waddr, wdata)` =
  simple dual-port), in `hdl/src/Tamal/Mem.hs`.
  - *instr BRAM* (`instrRam`, 1024×32): read addr = engine `pcOut :: Unsigned 10`;
    write = loader `Maybe (Unsigned 10, BitVector 32)`; output feeds
    `BusIn.instrWord`. Its **1-cycle read latency is exactly the engine's `Fetch`
    bubble.**
  - *ring BRAM* (`ringRam`, 4096×32): write = engine `Maybe Ring` (as `Maybe
    (Unsigned 12, BitVector 32)`); read addr = loader drain counter; output streams
    to the loader.
- **Decisions made:** ring = 4096 (`Unsigned 12`) so `termAddr = maxBound = D − 1`
  needs no engine change; instr store = 1024 (`AW = 10`); zero-init (`repeat 0` →
  BRAM `INIT`); generic tuple write ports (no `Tamal.Engine` import — the topEntity
  projects `Maybe Ring`).
- **Depends on:** nothing (engine + Clash primitives).
- **Tested:** `Test.Mem` — a pure `refRam` oracle (+ `simInstr`/`simRing` samplers);
  6 HUnit cases (write-then-read-back, 1-cycle latency, read-before-write collision,
  address boundaries, ring drain sweep) + 3 hedgehog properties (model-equivalence
  at both widths + last-write-wins sweep). Spec + plan under `docs/superpowers/`.

### 2. Wire protocol — `Tamal.Wire` pure core  *(done — `Tamal.Wire.Cobs` + `Tamal.Wire`)*

- **What:** the transport-agnostic control (host→FPGA) and result (FPGA→host)
  byte framing, implemented as a pure Clash core: `Tamal.Wire.Cobs` (COBS
  encode/decode) + `Tamal.Wire` (LE word↔bytes, CRC-8 fold, frame + message
  layer). Spec: `docs/superpowers/specs/2026-07-02-tamal-wire-format-design.md`.
- **Delivered (v1):** control = `LOAD_PROGRAM(words…)` + `TRIGGER`; result = the
  `TRACE_DRAIN` frame (`REVISION … records … HALT terminator`). Frame =
  `COBS(opcode ++ payload ++ CRC-8) ++ 0x00`, little-endian (ISA §4).
  Fire-and-forget control plane.
- **Decisions made:** COBS `0x00`-delimiter framing (self-delimiting on both
  planes; dissolves the result-drain length/zero-gap problem); CRC-8 reused from
  `Tamal.Crc` (poly 0x07); no length field (COBS-implied); `Cobs` is a
  dependency-free leaf (`Maybe`), the frame layer owns `WireError`.
- **Deferred:** the Rust `crates/tamal-abi` mirror and `tamal-loader` (post-silicon;
  they implement the same contract). The gateware loader (piece 3) consumes this core.
- **Tested:** `Test.Wire` — COBS round-trip + 254/255 boundary vectors, LE
  round-trip, frame round-trips (control + result), CRC/corruption detection, and
  the error taxonomy (25 cases).

### 3. Loader — UART load/drain FSM  *(done — `Tamal.Loader` + `Tamal.Loader.Cobs`)*

- **What:** the impure FSM bridging `Uart` ↔ BRAMs ↔ engine control. **Load:**
  parse control bytes → write instr BRAM → pulse `startIn`. **Drain:** on
  `haltedOut`, sweep the ring BRAM → UART TX per the result framing.
- **Delivered:** `Tamal.Loader` — a plain `mealy` over a pure `loaderStep`
  (matching the engine lift) with an `RxControl → Run → Drain` lifecycle;
  streaming COBS decode/encode isolated as pure step functions in
  `Tamal.Loader.Cobs` and embedded in the mealy. RX = streaming decode + one-byte
  holdback + CRC-8 + LE word-assembly (write-through, 1024-word saturating cap);
  TX = ring sweep (`word[0..ringPtr−1]` + terminator) + streaming COBS encode +
  `0x00` delimiter, paced by `txReady`. The one engine change was the pure
  `BusOut.ringPtrOut` drain-depth projection (§4 of the loader spec).
- **Decisions made:** plain `mealy` over a pure step (not `mealyS`); the codec is
  pure step functions (no Signal-feedback plumbing); no UART flow control (Arty
  FTDI has no RTS/CTS) — tolerated by fire-and-forget + whole-frame CRC + COBS
  resync (spec D5).
- **Depends on:** BRAM (ports), wire protocol (`Tamal.Wire`, done), `Uart` (done), engine
  (`startIn`/`haltedOut`/`ringPtrOut`).
- **Tested:** `Test.Loader` — Signal-level harnesses (RX load, drain, robustness +
  re-run lifecycle) + pure codec properties against the `Tamal.Wire.Cobs` oracle
  (150 total suite). Both streaming paths were cycle-validated out-of-band before
  implementation. **Gotcha found + documented:** `sampleN` asserts `resetGen` on
  cycle 0 (Dom100 async reset), so a byte fed at cycle 0 is lost — `mealy`+`sampleN`
  harnesses must lead with one idle cycle (the `Test.Uart` pattern).

### 4. IOBUF — tri-state IO + sideband pins  *(done — `Tamal.Io`)*

- **What:** the per-lane bidirectional buffers for `IO[3:0]` (`oe → T`, `o → I`,
  pad `O → sampler`), plus `CS#`/`SCK`/`RESET#` outputs and the `ALERT#` synchronizer.
- **Decisions made:** Clash `BiSignal` (`BiSignalIn`/`BiSignalOut`), `'PullUp`,
  per-lane width-1 (independent OE — x1 drives IO[0] while IO[1..3] tri-state);
  combinational IO sample (matches the engine's `ioIn` same-cycle contract); only
  `ALERT#` synchronized (2-flop, init high); sideband outputs pass through
  (already registered upstream); a dependency-free leaf (no `Tamal.Engine` import).
- **Depends on:** engine `lanesOut` `(o, oe)` per lane; XDC pins (deferred to topEntity).
- **Tested:** `Test.Io` — `alertSync` (2-cycle-delay model, calibrated to the
  `sampleN` cycle-0 reset), sideband pass-through, and tri-state drive/sample via
  `veryUnsafeToBiSignalIn` loopback harnesses (`simDrive`/`simSample`/`simSide`):
  drive-out, sample-in, pull-up idle, per-lane independent OE (x1). **Gotcha found +
  documented:** feeding espiPads's own drive back into the net it also reads
  diverges in Clash `BiSignal` simulation, so the harnesses use single-driver nets
  bound to a throwaway idle pad (the engine drives XOR samples a lane anyway).

### 5. topEntity — integration  *(done — `Tamal.Top` + `Tamal` shell)*

- **What:** the synthesis entry point, split into a cosim-testable `Tamal.Top`
  `system` (BRAMs + loader + UART + `engine` `mealy stepM initState` + status LED)
  and a thin `Tamal` shell (100 MHz clock + `espiPads` + named pins). `system` is
  BiSignal-free so the whole integration is cosim-testable; the shell owns the
  tri-state binding.
- **Decisions made:** `espiPads` lives in the shell (keeps `system` plain-`Signal`);
  the four IO lanes are **four scalar `inout` ports** (`io0`..`io3`) — Clash fuses a
  per-lane `BiSignalIn` arg + `BiSignalOut` result into one `inout`, but a `Vec` of
  BiSignals does not; 2 Mbaud UART; 3-state status LED (`ledPattern`); no-reset
  power-up retained.
- **Depends on:** all of the above.
- **Tested:** `Test.Top` — pure helpers (`stepM`/`ringWrite`/`rigState`/`ledPattern`)
  + a whole-system UART/eSPI **cosim** (serialize a `Tamal.Wire` `LOAD_PROGRAM`+
  `TRIGGER` onto `rxLine`, run load→run→drain, decode `txLine`, assert the drained
  trace + CS#/SCK activity). **Gotcha found + documented:** the UART RX drops truly
  back-to-back bytes on the falling-edge resync, so the cosim's `serialize` leaves
  one idle bit-time between bytes (a realistic transmitter). Codegen gate confirms
  the four `inout` lanes; `cd hdl && make` → `tamal.bit` is the ultimate gate.

---

### Per-piece workflow (for each fresh session)

1. **brainstorming** skill → design doc at
   `docs/superpowers/specs/YYYY-MM-DD-tamal-<piece>-design.md`, committed.
2. **writing-plans** skill → TDD plan at `docs/superpowers/plans/`.
3. Execute the plan test-first. Everything runs from `hdl/`: `cabal test`,
   `make format` before each commit, `cabal run clash -- Tamal --verilog` as a
   codegen smoke. Keep the split-license headers on new `hdl/**/*.hs` files
   (CERN-OHL-P-2.0).

## Ordering

1. **ALU + branch comparator** — done
2. **Register file** (16×32, `x0` = 0) — done
3. **UART transport** (8N1 RX/TX, NCO 16× oversample) — done
4. **`Engine.hs` — `step` Mealy** — done, hedgehog-tested
5. **BRAM** (instr + ring `blockRamPow2`; ring = 4096, `termAddr = maxBound`) — done, hedgehog-tested
6. **Wire protocol** (`Tamal.Wire` COBS + CRC-8 framing core) — done, hedgehog-tested
7. **Loader** (UART load/drain FSM; `Tamal.Loader` + `Tamal.Loader.Cobs`) — done, tested
8. **IOBUF** (tri-state IO + sideband pins; `Tamal.Io`) — done, tested
9. **`topEntity`** (integration; `Tamal.Top` `system` + shell; retire the heartbeat) — done, cosim-tested

Short version: every piece is in place and tested — the pure leaves, the Engine
keystone, the **BRAM memories** (`Tamal.Mem`), the **wire-format core**
(`Tamal.Wire.Cobs` + `Tamal.Wire`), the **UART load/drain loader**
(`Tamal.Loader` + `Tamal.Loader.Cobs`), the **eSPI pad boundary** (`Tamal.Io`),
and the **integration** (`Tamal.Top` + the `topEntity` shell), which a
whole-system UART/eSPI cosim exercises end to end. tamal v1 builds to a bitstream.

