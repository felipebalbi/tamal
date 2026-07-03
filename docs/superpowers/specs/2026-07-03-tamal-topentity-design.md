# Tamal — topEntity (integration) Design

Date: 2026-07-03
Status: Approved (design); implementation not started
Scope: The synthesis entry point that lifts the whole tamal design onto the
Arty A7-100T. Split into a new pure, cosim-testable **`Tamal.Top` `system`** (all
the wiring: `instrRam`/`ringRam` + `loader` + `uart` + engine `mealy stepM
initState` + the pure projections + the 3-state status LED) and a thin **`Tamal`
`topEntity` shell** (100 MHz clock + `espiPads` BiSignal↔pad binding + named
ports). Extends `constraints/arty_a7.xdc` with the eSPI/UART/LED pins. Retires the
heartbeat placeholder. This is the **last** shell piece (`hdl/PLAN.md` piece 5).

Companion to every prior piece: the engine (`Engine.step`/`initState`/`BusIn`/
`BusOut`/`Ring`), the memories (`Tamal.Mem`), the loader (`Tamal.Loader`,
`LoaderIn`/`LoaderOut`), the UART (`Tamal.Uart`), the wire format (`Tamal.Wire`),
and the pad boundary (`Tamal.Io`, `espiPads`/`alertSync`). Anchored to `hdl/PLAN.md`
piece 5 and the IOBUF design (`docs/superpowers/specs/2026-07-02-tamal-io-design.md`,
esp. §9 the codegen/inout risk). Written to be implementable from a fresh session.

---

## 1. Purpose & context

Every leaf and the impure shell pieces (BRAM, wire, loader, IOBUF) are built and
tested. What remains is the **integration**: instantiate the built blocks, close
the fetch loop, wire the host load/drain path, bind the eSPI/UART pins, and emit a
bitstream. Confirmed by an earlier port-map audit — there is **no missing glue
module**, only wiring plus two tiny projections; every `LoaderIn`/`LoaderOut`,
`BusIn`/`BusOut`, `uart`, `espiPads`, and BRAM port has a counterpart.

The committed interfaces this piece consumes:

- Engine: `step :: State -> BusIn -> (State, BusOut, Maybe Ring)`, `initState`,
  `BusIn{instrWord, ioIn, alertIn, startIn}`, `BusOut{pcOut, csOut, sckOut,
  rstOut, lanesOut, haltedOut, ringPtrOut}`, `Ring{rAddr, rData}` (Engine.hs).
- Memories: `instrRam :: Signal (Unsigned 10) -> Signal (Maybe (Unsigned 10,
  BitVector 32)) -> Signal (BitVector 32)`; `ringRam` at `Unsigned 12` (Mem.hs).
- Loader: `loader :: Signal LoaderIn -> Signal LoaderOut`, `LoaderIn{rxByte,
  txReady, halted, ringPtrIn, ringData}`, `LoaderOut{txByte, instrWr, ringAddr,
  startOut}` (Loader.hs).
- UART: `uart :: SNat baud -> Signal Bit -> Signal (Maybe (BitVector 8)) ->
  (Signal (Maybe (BitVector 8)), Signal Bool, Signal Bit, Signal Bool)` =
  `(rxByte, rxErr, txLine, txReady)` (Uart.hs).
- Pads: `espiPads :: Signal Lanes -> Signal Bit ->..-> Vec 4 (BiSignalIn 'PullUp
  dom 1) -> (Vec 4 (BiSignalOut 'PullUp dom 1), Signal Bit, Signal Bit, Signal
  Bit, Signal (Vec 4 Bit), Signal Bit)` (Io.hs).

The **key architectural move** is where `espiPads` (the only `BiSignal` code) lives:
placing it in the thin shell keeps the rest — `system` — operating on plain
`Signal`s (`lanesOut`/`ioIn`), so the whole integration is **cosim-testable without
`BiSignal`**. On hardware `espiPads` lowers to real Xilinx `IOBUF`s; the
simulation-loopback fragility documented in the IOBUF work is a *simulation*
artifact that never touches synthesis or the `system` cosim.

---

## 2. Scope & non-goals

**In scope**

- `Tamal.Top`: `system` (BiSignal-free whole-design wiring) + pure helpers
  `stepM`, `ringWrite`, `rigState`/`RigState`, `ledPattern`.
- `Tamal` (`topEntity`): the shell — 100 MHz clock, `espiPads`, named ports,
  `noReset`; retire `heartbeat`.
- `constraints/arty_a7.xdc`: add eSPI (`IO[3:0]`, `CS#`, `SCK`, `RESET#`,
  `ALERT#`), UART (`uart_rx`/`uart_tx`), and status `led` pins (+ `PULLUP` on the
  IO lanes and `ALERT#`).
- Tests: `Test.Top` — hedgehog/HUnit on the pure helpers + a Signal-level whole-
  `system` cosim; the `--verilog` codegen gate.
- Cabal wiring for `Tamal.Top` + `Test.Top`.

**Out of scope (deferred)**

- **Target role** (external eSPI clock; CDC/setup-hold against a foreign clock) —
  Phase 3. v1 is controller role, `SCK` fabric-derived (`Dom100/5`), single domain.
- **Trap-specific LED / richer observability** (engine `phase`/trap flag are not in
  `BusOut`) — would need an engine change; `ledPattern` uses only `halted`+`startOut`.
- **`rxErr` surfacing** — the UART framing-error strobe is dropped in v1 (not in
  `LoaderIn`); could feed the trace later.
- **The Vivado flow internals** (`vivado/build.tcl`) — unchanged; only the XDC grows.
- **FIFOs / second clock domain** — none needed (single `Dom100`; UART ≪ fabric;
  ring BRAM + overflow-marker is the trace buffer). Confirmed by prior audit.
- **Multiple status LEDs / RGB** — v1 uses one status LED.

---

## 3. Design decisions

| # | Decision | Rationale |
|---|----------|-----------|
| 1 | **Split `system` (pure, in `Tamal.Top`) vs `topEntity` shell (in `Tamal.hs`)**, with `espiPads` in the shell. | Keeps all `BiSignal` in the shell so `system` is plain-`Signal` and **cosim-testable** end-to-end. The shell is the only untested-by-sim part (validated by codegen + hardware); `espiPads` is already unit-tested. Mirrors the repo's pure-core/thin-impure-shell discipline. |
| 2 | **Engine lift = `mealy stepM initState`** with a thin `stepM :: State -> BusIn -> (State, (BusOut, Maybe Ring))`. | The PLAN-locked lift: `step` stays pure and untouched; `stepM` only re-associates the tuple. No `mealyS`/`Bundle` for the engine (evaluated and rejected earlier). |
| 3 | **Cosim-test `system` at the UART line level** (`rxLine`/`txLine` are `Signal Bit`). | `system`'s honest boundary is the physical UART line, so the whole pipeline (UART framing → load → run → eSPI → trace → drain) is exercised. Test helpers serialize/deserialize 8N1 frames (reusing `Test.Uart` patterns) + build/parse control/result frames with `Tamal.Wire`. |
| 4 | **3-state status LED via a pure `ledPattern`** (Waiting/Running/Done) + a 1-bit `running` latch. | A headless-rig bring-up indicator that is pure and hedgehog-testable, using only the cleanly-available `halted`+`startOut`. Distinguishes "idle" / "test executing" / "done, trace ready" without a host or scope. |
| 5 | **`'PullUp` IO lanes + `ALERT#` set `PULLUP TRUE` in XDC**; driven outputs get no pull. | eSPI mandates IO pull-ups; the XDC pull is the *hardware* realization of the `espiPads` `'PullUp` sim default, giving a defined idle-high. `SCK`/`CS#`/`RESET#` are always driven. |
| 6 | **Two neighbouring Pmods, data/control split** — JA (`IO[0..3]`) + JB (`SCK`/`CS#`/`RESET#`/`ALERT#`), both bank 15. | User choice: doubles GND-per-signal for SI headroom toward 66 MHz and yields a clean adapter. Bank-15 adjacency ≈ physically neighbouring; the physical pair is confirm-on-board (only `PACKAGE_PIN`s change). |
| 7 | **2 Mbaud UART** (`uart (SNat @2_000_000)`). | Matches the UART design's own NCO sizing; fast program load + trace drain. `115200` is a one-line fallback. |
| 8 | **No-reset power-up retained** (`unsafeFromActiveHigh (pure False)`); named ports via `makeTopEntity`. | AGENTS.md deliberate no-reset design; named ports fix the Verilog port names the XDC binds (as the current placeholder already does). |
| 9 | **BiSignal inout as `Vec 4 (BiSignalIn/Out 'PullUp Dom100 1)` top ports** (fallback: 4 scalar inouts). | Per-lane independent OE (IOBUF spec). The `Vec`→`inout` lowering is the one **codegen risk** — validated from the emitted Verilog before the XDC is written. |

---

## 4. Module layout & public interface

### 4.1 `Tamal.Top` (new) — the pure, testable core

Carries the SPDX header. Imports the built blocks (`Tamal.Mem`, `Tamal.Loader`,
`Tamal.Uart`, `Tamal.Engine`, `Tamal.Bus.Serdes` for `Lanes`); it does **not**
import `Tamal.Io` (no `BiSignal` here).

```haskell
module Tamal.Top (system, stepM, ringWrite, RigState (..), rigState, ledPattern) where

-- | The mealy adapter: re-associates 'Engine.step' so it lifts with 'mealy',
-- leaving 'step' pure and untouched.
stepM :: State -> BusIn -> (State, (BusOut, Maybe Ring))
stepM s i = let (s', bo, mr) = step s i in (s', (bo, mr))

-- | Project the engine's ring write to the BRAM write-port tuple.
ringWrite :: Maybe Ring -> Maybe (Unsigned 12, BitVector 32)
ringWrite = fmap (\(Ring a d) -> (a, d))

-- | Lifecycle state shown on the status LED.
data RigState = Waiting | Running | Done
  deriving (Generic, Show, Eq, NFDataX)

-- | Pure status → LED level over a free-running counter: Waiting = slow
-- heartbeat (MSB), Running = faster blink, Done = solid on.
ledPattern :: RigState -> Unsigned 26 -> Bit
ledPattern Waiting c = msb c              -- ~0.75 Hz
ledPattern Running c = c ! (22 :: Index 26) -- ~6 Hz
ledPattern Done    _ = 1                  -- solid

-- | The whole design minus pin binding. All feedback is through the engine's
-- registered state (no combinational loops). @ioIn@/@alertIn@ come from
-- 'espiPads' in the shell (DUT stimulus in the cosim).
system ::
  (HiddenClockResetEnable dom) =>
  Signal dom Bit ->          -- ^ uart RX line (from host)
  Signal dom (Vec 4 Bit) ->  -- ^ ioIn (sampled IO pads)
  Signal dom Bit ->          -- ^ alertIn (synchronized ALERT#)
  ( Signal dom Bit           -- uart TX line (to host)
  , Signal dom Lanes         -- lanesOut (to espiPads)
  , Signal dom Bit           -- csOut
  , Signal dom Bit           -- sckOut
  , Signal dom Bit           -- rstOut
  , Signal dom Bit )         -- led (ledPattern output)
```

`rigState` derives the LED state from a `running` latch:

```haskell
-- running: set on the loader's startOut pulse, cleared when the engine halts.
-- Waiting = never/again idle; Running = started & not halted; Done = halted.
rigState :: Bool {-running-} -> Bool {-halted-} -> RigState
rigState _       True  = Done
rigState running False = if running then Running else Waiting
```

### 4.2 `system` internal wiring

Registered feedback only — the `pcOut → instrRam → instrWord → step → pcOut` loop
has two registers (engine `pc` + BRAM output); the `lanesOut/ioIn` loop closes
through the engine's registered `lanesOut`.

```haskell
system rxLine ioIn alertIn = (txLine, lanesOut, csO, sckO, rstO, ledOut)
 where
  -- UART (2 Mbaud)
  (rxByte, _rxErr, txLine, txReady) = uart (SNat @2_000_000) rxLine txByte

  -- Loader FSM (fields projected from the LoaderOut signal, cf. busOut below)
  lOut     = loader (LoaderIn <$> rxByte <*> txReady <*> halted <*> ringPtrO <*> ringData)
  txByte   = txByte   <$> lOut
  instrWr  = instrWr  <$> lOut
  ringAddr = ringAddr <$> lOut
  startO   = startOut <$> lOut

  -- Memories
  instrWord = instrRam pcO instrWr
  ringData  = ringRam ringAddr (ringWrite <$> maybeRing)

  -- Engine
  (busOut, maybeRing) = unbundle (engine busIn)
  engine  = mealy stepM initState
  busIn   = BusIn <$> instrWord <*> ioIn <*> alertIn <*> startO
  pcO      = pcOut     <$> busOut
  lanesOut = lanesOut  <$> busOut
  csO      = csOut     <$> busOut
  sckO     = sckOut    <$> busOut
  rstO     = rstOut    <$> busOut
  halted   = haltedOut <$> busOut
  ringPtrO = ringPtrOut<$> busOut

  -- Status LED
  running = register False (mux startO (pure True) (mux halted (pure False) running))
  ledOut  = ledPattern <$> (rigState <$> running <*> halted) <*> ledCounter
  ledCounter = register 0 (ledCounter + 1) :: Signal dom (Unsigned 26)
```

(Record field-accessor `fmap`s over `busOut`/`lOut`; the exact `unbundle`/record
construction is the author's to write. Field-name shadowing like `lanesOut =
lanesOut <$> busOut` is illustrative — the implementation may alias to avoid it.)

### 4.3 `Tamal` (`topEntity`) — the shell

```haskell
topEntity ::
  "clk"     ::: Clock Dom100 ->
  "uart_rx" ::: Signal Dom100 Bit ->                    -- pin D10
  "io"      ::: Vec 4 (BiSignalIn 'PullUp Dom100 1) ->  -- IO[3:0] pads
  "alert_n" ::: Signal Dom100 Bit ->                    -- ALERT# (raw)
  ( "io"      ::: Vec 4 (BiSignalOut 'PullUp Dom100 1)  -- IO[3:0] drive (same inout)
  , "uart_tx" ::: Signal Dom100 Bit                     -- pin A9
  , "cs_n"    ::: Signal Dom100 Bit
  , "sck"     ::: Signal Dom100 Bit
  , "reset_n" ::: Signal Dom100 Bit
  , "led"     ::: Signal Dom100 Bit )
topEntity clk uartRx ioPads alertN =
  withClockResetEnable clk noReset enableGen $
    let (txLine, lanesOut, csO, sckO, rstO, ledOut) = system uartRx ioIn alertIn
        (ioDrive, csPin, sckPin, rstPin, ioIn, alertIn) =
          espiPads lanesOut csO sckO rstO alertN ioPads
     in (ioDrive, txLine, csPin, sckPin, rstPin, ledOut)
 where
  noReset = unsafeFromActiveHigh (pure False)

makeTopEntity 'topEntity
```

The `system`↔`espiPads` recursive-let is legal Clash feedback (closed through the
engine's registered `lanesOut`), not a combinational loop. All pin drives route
*through* `espiPads` (it passes `cs`/`sck`/`rst` through), giving it sole ownership
of future IOB packing.

**Codegen risk (decision 9).** Naming the `BiSignalIn` arg and `BiSignalOut` result
both `"io"` is how Clash fuses them into one `inout`. Whether `Vec 4` emits
`inout [3:0] io` vs `io_0..io_3` is confirmed from the generated Verilog **before**
the XDC is written; fallback is four scalar inout pairs.

---

## 5. Pin map & XDC

Recommended pair **JA (data) + JB (control)** — both bank 15 (adjacent); confirm the
physical pair on the board (only `PACKAGE_PIN`s change). Pins from the Arty A7-100
master XDC.

| Signal     | Dir   | Port      | Pmod | `PACKAGE_PIN` | XDC extra                       |
|------------|-------|-----------|------|---------------|---------------------------------|
| `IO[0]`    | inout | `io[0]`   | JA-1 | G13           | `PULLUP TRUE`                   |
| `IO[1]`    | inout | `io[1]`   | JA-2 | B11           | `PULLUP TRUE`                   |
| `IO[2]`    | inout | `io[2]`   | JA-3 | A11           | `PULLUP TRUE`                   |
| `IO[3]`    | inout | `io[3]`   | JA-4 | D12           | `PULLUP TRUE`                   |
| `SCK`      | out   | `sck`     | JB-1 | E15           |                                 |
| `CS#`      | out   | `cs_n`    | JB-2 | E16           |                                 |
| `RESET#`   | out   | `reset_n` | JB-3 | D15           |                                 |
| `ALERT#`   | in    | `alert_n` | JB-4 | C15           | `PULLUP TRUE`                   |
| UART RX    | in    | `uart_rx` | FTDI | D10           |                                 |
| UART TX    | out   | `uart_tx` | FTDI | A9            |                                 |
| status LED | out   | `led`     | LD4  | H5            | (existing pin)                  |
| clock      | in    | `clk`     | osc  | E3            | (existing) `create_clock` 10 ns |

All `IOSTANDARD LVCMOS33`. `SCK`↔data cross one connector; skew is ~tens of ps,
negligible even at 66 MHz. The physical `PULLUP` realizes the `espiPads` `'PullUp`
default; port names must match the codegen output (§4.3).

---

## 6. Test plan — the assistant's deliverable

`hdl/tests/Test/Top.hs` (`tests :: TestTree`, SPDX, added to `unittests.hs`), tasty
+ hedgehog + HUnit. Ping-pong TDD: assistant writes the failing test and mentors;
author writes the synthesizable Clash under `src/`.

### 6.1 Pure helpers (fast, first)

- **`ledPattern`** — per state, sweep the `Unsigned 26` counter: `Done` is always 1;
  `Waiting`/`Running` toggle at their bit, and `Running` is strictly faster than
  `Waiting` (property: `Waiting` uses `msb`, `Running` a lower bit). HUnit vectors at
  representative counter values.
- **`rigState`** — truth table over `(running, halted)`: `(_,True)→Done`,
  `(True,False)→Running`, `(False,False)→Waiting`.
- **`ringWrite`** — `Nothing→Nothing`; `Just (Ring a d)→Just (a,d)` (property over
  random `Ring`).
- **`stepM`** — equals `step` re-associated: `stepM s i == (s', (bo, mr))` where
  `(s',bo,mr) = step s i` (property over `Test.Gen` states/inputs, reusing engine
  generators).

### 6.2 `system` cosim (centerpiece, Signal-level)

One end-to-end harness proving UART-load → run → eSPI → trace → UART-drain:

1. **Build a control frame** with `Tamal.Wire`: `LOAD_PROGRAM(words…)` for a tiny
   program (e.g. a `PUT_BYTE`/`GET_BYTE` + `HALT`, or a `MARK` + `HALT`), then
   `TRIGGER`. Serialize each frame byte onto `rxLine` as 8N1 at the test baud with a
   UART-TX serializer helper (mirrors `Tamal.Uart.Tx`; reuse `Test.Uart` patterns).
2. **Model a trivial eSPI DUT** on `ioIn` (e.g. drive a known byte on IO[1] during
   the GET window) and hold `alertIn` idle-high.
3. **Run** enough cycles for load + run + drain (bounded; ~10–50 k cycles).
4. **Deserialize `txLine`** back to bytes (UART-RX deserializer helper) and
   **decode the result frame** with `Tamal.Wire`; assert it is a well-formed
   `TRACE_DRAIN` (REVISION word + the expected record(s) + HALT terminator, CRC ok).
5. **Assert eSPI activity**: `sck` toggles and `cs_n` asserts during the program;
   `lanesOut` drives IO[0] during a `PUT`.

Scope: one or two representative programs — enough to exercise the full loop, not a
re-test of each leaf (they are already covered). Lead the harness with one idle
cycle (the Dom100 `sampleN` cycle-0 reset idiom, per `hdl/PLAN.md`).

### 6.3 Codegen + build gates

- `stack run clash -- Tamal --verilog` succeeds; **inspect the port list** for the
  intended `inout` `io` (decision 9) + `uart_rx`/`uart_tx`/`cs_n`/`sck`/`reset_n`/
  `alert_n`/`led`/`clk`. Resolve the port shape here **before** finalizing the XDC.
- `cd hdl && make` → `tamal.bit` (full Vivado non-project flow) is the ultimate gate;
  may be slow / environment-dependent — run when a toolchain is available. On-
  hardware: the LED lifecycle (Waiting→Running→Done) is the first sanity check.

---

## 7. Files touched

```
new:      hdl/src/Tamal/Top.hs        -- system + stepM/ringWrite/rigState/ledPattern (author; +SPDX)
          hdl/tests/Test/Top.hs       -- tests :: TestTree (assistant; +SPDX)

modified: hdl/src/Tamal.hs            -- topEntity shell; retire heartbeat (author; +SPDX kept)
          hdl/constraints/arty_a7.xdc -- eSPI + UART + LED pins (author; after codegen confirms ports)
          hdl/tamal.cabal             -- exposed-modules += Tamal.Top; test other-modules += Test.Top
          hdl/tests/unittests.hs      -- import + Test.Top.tests
          hdl/PLAN.md                 -- mark topEntity done (close-out)
```

No engine/leaf changes.

---

## 8. Verification

From `hdl/`:

```
stack test                           # hedgehog + HUnit: Test.Top (+ existing)
stack run clash -- Tamal --verilog   # codegen gate — confirm inout/ports
make format-check                    # fourmolu (make format to fix)
make                                 # full Vivado -> tamal.bit (toolchain-dependent)
```

The Verilog gate now exercises **real gateware** (unlike the earlier leaf pieces):
the whole `topEntity` cone elaborates, including the `BiSignal` inout lowering.

---

## 9. Implementation approach (ping-pong TDD)

Suggested slices, each red → green → refactor:

1. **Pure helpers.** Author writes `stepM`/`ringWrite`/`RigState`/`rigState`/
   `ledPattern` in `Tamal.Top`; assistant's §6.1 tests go green. (No wiring yet.)
2. **`system` skeleton + cosim harness.** Author bodies `system`; assistant builds
   the §6.2 UART-serialize/deserialize + `Tamal.Wire` frame harness and the first
   end-to-end assertion; iterate to green.
3. **`topEntity` shell.** Author rewrites `Tamal.hs` (shell + `espiPads` + named
   ports, retire heartbeat); assistant runs the codegen gate and resolves the
   `inout` port shape (decision 9).
4. **XDC + close-out.** Author extends `arty_a7.xdc` with the confirmed port names;
   `make format`; codegen gate; `make` if a toolchain is available; update
   `hdl/PLAN.md`; commit.

The exact task list is the job of the follow-up implementation plan.

---

## 10. Out of scope / follow-ups (roadmap)

`topEntity` is the last shell piece; after it, tamal has a working v1 bitstream
(controller role, x1 I/O, UART transport). Future work (all previously scoped):

1. **Target role** — external eSPI clock; setup/hold/CDC (Phase 3). The one place a
   CDC/async-FIFO could appear.
2. **Dual/quad I/O**, alert-driven flows, the deterministic error-injection +
   verdict engine, and the conformance catalog (Phases 3–4).
3. **FX3/USB3 transport** — a second `tamal-loader` backend (GPIF II slave-FIFO).
4. **Observability** — surface the engine trap flag / `rxErr` into the trace/LED.

---

## 11. Prior art

- The current **placeholder `topEntity`** (`Tamal.hs`) — the `makeTopEntity` +
  named-port + `noReset` pattern this piece keeps and extends.
- **`Tamal.Io`** — the pad boundary whose `espiPads` this shell instantiates; its
  §9 flagged exactly this integration's `inout`/codegen risk.
- The sibling Clash starters (sevenseg, etc.) — same Clash→Vivado non-project flow
  and no-reset power-up.
- **[mole](https://github.com/felipebalbi/mole)** — the sibling I2C/I3C rig whose
  host/gateware split tamal mirrors.
```
