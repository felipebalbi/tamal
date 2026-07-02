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
the **UART transport** included — and now the keystone **`Engine.step`** Mealy is
built and hedgehog-tested too. What remains is the impure `topEntity` shell that
wires everything to the pins.

| Module | Purpose | Status |
|---|---|---|
| `Tamal.Isa` | `Instr` ADT, total `encode`/`decode`, reserved-field trap | done, tested |
| `Tamal.Crc` | `crc8Update` (poly 0x07, init 0x00, MSB-first) | done, tested |
| `Tamal.Bus.Serdes` | x1 serialize/deserialize + `tarBeat` | done, tested |
| `Tamal.Config` | `SET_CONFIG` decode | done, tested |
| `Tamal.Trace` | ring record encode (+ trap flag/reason) + `ringPush` overflow | done, tested |
| `Tamal.Alu` | `alu` core + `dataResult` wrapper (DATA group) | done, tested |
| `Tamal.Branch` | `branchTaken` comparator (CTRL group, unsigned) | done, tested |
| `Tamal.RegFile` | 16×32 register file, `x0` hardwired 0 | done, tested |
| `Tamal.Uart.*` | 8N1 UART (NCO tick, RX, TX, umbrella) — host transport | done, tested |
| `Tamal.Engine` | `step` Mealy: fetch/decode/datapath + SCK bus FSM + trace + HALT/TRAP | done, tested |
| `Tamal.Domain` | `Dom100` clock domain | done |
| `Tamal` (top) | synthesis entry point | **placeholder heartbeat** (LED blink) |

Confirmed absent: only the real `topEntity` shell (the Engine wired to the pins).

## What remains: the impure `topEntity` shell

`Engine.step` is done and hedgehog-tested (92 engine+leaf properties). Every
*pure* core exists. What's left is the **impure shell** that lifts `step` onto
the board: memories, the host load/drain path, tri-state IO, and the top that
wires it all to the pins.

This shell decomposes into **five independently-startable pieces**. Each is meant
to be picked up in its own fresh session and taken through the repo's standard
cycle — **brainstorming → writing-plans → TDD** (see "Per-piece workflow" below).
Build order is **BRAM → wire protocol → loader → IOBUF → topEntity**; `topEntity`
integrates everything and is deliberately last.

```
   host ──UART──►┌─────────┐  start   ┌───────────┐  pcOut  ┌────────────┐
                 │ loader  │─────────►│  Engine    │────────►│ instr BRAM │
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
- **Memory geometry + `termAddr` reconciliation.** The engine currently hardcodes
  `termAddr = maxBound :: Unsigned 12` (a 4096-word ring) with a "top-shell sizes
  it" note. The **BRAM** piece pins the real depths — instruction store = 1024
  words (`AW = 10`, ≈ 1 BRAM36); ring depth = TBD (4096×32 ≈ 4 BRAM36) — and
  reconciles `termAddr` to the true ring depth − 1 (parameterize the engine or set
  the constant). This value feeds the loader's drain length too.
- **Wire-format gap.** `crates/tamal-abi` (`control`, `trace` modules) is still
  placeholder — there is **no** control/result framing yet. The loader cannot be
  built until a minimal framing exists, so it gets its own mini-spec (piece 2).
- **Board / XDC.** Arty A7-100T; USB-UART on the FTDI pins; eSPI pins on Pmods.
  `constraints/arty_a7.xdc` currently constrains only `clk` + `led`; every piece
  that adds pins extends it (pin numbers from Digilent's Arty-A7-100 master XDC).

---

### 1. BRAM — instruction + ring memories  *(do first; unblocked)*

- **What:** two Clash `blockRam`s (read-address + `Maybe (waddr, wdata)` = simple
  dual-port).
  - *instr BRAM:* read addr = engine `pcOut :: Unsigned 10`; write = loader
    `Maybe (Unsigned 10, BitVector 32)`; output feeds `BusIn.instrWord`. Its
    **1-cycle read latency is exactly the engine's `Fetch` bubble.**
  - *ring BRAM:* write = engine `Maybe Ring` (as `Maybe (Unsigned 12, BitVector
    32)`); read addr = loader drain counter; output streams to the loader.
- **Decisions to make:** final depths; `blockRamU`/zero-init vs seeded; the
  `termAddr` reconciliation above.
- **Depends on:** nothing (engine + Clash primitives).
- **Testing:** `Signal`-level `simulate`/`sampleN` — write-then-read-back, verify
  1-cycle latency, ring write + drain sweep. (Impure but simulatable.)

### 2. Wire protocol — fill the `tamal-abi` placeholder  *(loader prerequisite)*

- **What:** the minimal, transport-agnostic control (host→FPGA) and result
  (FPGA→host) byte framing, defined in `crates/tamal-abi` (`control`, `trace`),
  consumed by *both* the gateware loader and the host `tamal-loader`.
- **Scope (v1):** control = `LOAD_PROGRAM(len, words…)` + `TRIGGER`; result =
  the drained ring stream (`REVISION … records … HALT terminator`) with framing.
  Little-endian on the wire (ISA §4). Keep it lean.
- **Decisions:** framing (length-prefix / sync bytes), error handling.
- **Depends on:** nothing; informs the loader and host tooling.

### 3. Loader — UART load/drain FSM

- **What:** the impure FSM bridging `Uart` ↔ BRAMs ↔ engine control. **Load:**
  parse control bytes → write instr BRAM → pulse `startIn`. **Drain:** on
  `haltedOut`, sweep the ring BRAM → UART TX per the result framing.
- **Interface:** consumes `uart` `rxByte`/`txReady`, drives `txByte`; writes instr
  BRAM; reads ring BRAM; drives engine `startIn`; observes `haltedOut`.
- **Decisions:** `mealyS` (State-monad) candidate; load-then-run sequencing (drain
  is post-HALT, so the bus is idle — no bus backpressure concern).
- **Depends on:** BRAM (ports), wire protocol (framing), `Uart` (done), engine
  (`startIn`/`haltedOut`).
- **Testing:** `Signal`-level sim — feed a UART byte stream, check instr-BRAM
  contents + `start` pulse; drive a halted engine with known ring contents, check
  the UART drain stream.

### 4. IOBUF — tri-state IO + sideband pins

- **What:** the bidirectional buffers for `IO[3:0]` (`oe → T`, `o → I`, pad
  `O → sampler`), plus `CS#`/`SCK`/`RESET#` outputs and the `ALERT#` synchronizer.
- **Decisions:** Clash `BiSignalIn`/`BiSignalOut` (`Clash.Signal.BiSignal`) vs
  instantiating the Vivado `IOBUF` primitive — decide in its spec.
- **Depends on:** engine `lanesOut` `(o, oe)` per lane; XDC pins.
- **Testing:** minimal (a vendor primitive / bidir signal) — validated in
  whole-top sim and on hardware.

### 5. topEntity — integration  *(LAST)*

- **What:** the synthesis entry point. Instantiate BRAMs + loader + `engine`
  (`mealy stepM initState`) + IOBUFs; wire the 100 MHz clock and all pins; extend
  the XDC; retire the heartbeat placeholder.
- **Depends on:** all of the above.
- **Testing:** `stack run clash -- Tamal --verilog` (codegen), then the full Vivado
  flow (`cd hdl && make`) to a bitstream, then on-hardware bring-up.

---

### Per-piece workflow (for each fresh session)

1. **brainstorming** skill → design doc at
   `docs/superpowers/specs/YYYY-MM-DD-tamal-<piece>-design.md`, committed.
2. **writing-plans** skill → TDD plan at `docs/superpowers/plans/`.
3. Execute the plan test-first. Everything runs from `hdl/`: `stack test`,
   `make format` before each commit, `stack run clash -- Tamal --verilog` as a
   codegen smoke. Keep the split-license headers on new `hdl/**/*.hs` files
   (CERN-OHL-P-2.0).

## Ordering

1. **ALU + branch comparator** — done
2. **Register file** (16×32, `x0` = 0) — done
3. **UART transport** (8N1 RX/TX, NCO 16× oversample) — done
4. **`Engine.hs` — `step` Mealy** — done, hedgehog-tested
5. **BRAM** (instr + ring `blockRam`; pin depths; reconcile `termAddr`) ← next
6. **Wire protocol** (`tamal-abi` control/result framing) — loader prerequisite
7. **Loader** (UART load/drain FSM)
8. **IOBUF** (tri-state IO + sideband pins)
9. **`topEntity`** (integration; retire the heartbeat) — last

Short version: the pure leaves and the Engine keystone are all in place and
tested. The impure shell remains, decomposed into BRAM → wire protocol → loader →
IOBUF → topEntity — each its own spec → plan → TDD session, with `topEntity` last.

