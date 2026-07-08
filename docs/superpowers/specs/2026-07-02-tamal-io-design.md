# Tamal — IOBUF (tri-state IO + sideband pins) Design

Date: 2026-07-02
Status: Approved (design); implementation not started
Scope: The eSPI pad boundary — a new leaf module `Tamal.Io` exposing `espiPads`
(four per-lane tri-state `IO[3:0]` buffers + `CS#`/`SCK`/`RESET#` output buffers +
the `ALERT#` synchronizer) and `alertSync` (the reusable 2-flop synchronizer).
Realized with Clash `BiSignal` (`Clash.Signal.BiSignal`), `'PullUp` default,
per-lane width-1. Signal-level tested (hedgehog + HUnit) with a
`veryUnsafeToBiSignalIn` loopback harness. The topEntity wiring, the XDC pins, and
any `Engine.hs` change are **out of scope** (§2, §11).

Companion to the ISA & HDL Engine design
(`docs/superpowers/specs/2026-07-01-tamal-isa-design.md`, esp. §6.5 alerts) and the
Engine design (`docs/superpowers/specs/2026-07-02-tamal-engine-design.md`, esp. the
`BusIn`/`BusOut` pin projection). Anchored to `hdl/PLAN.md` piece 4
("IOBUF — tri-state IO + sideband pins ← next"). Written to be implementable from a
fresh session.

---

## 1. Purpose & context

`Engine.step` is built and hedgehog-tested; every *pure* leaf exists, and the
first three impure-shell pieces — BRAM (`Tamal.Mem`), the wire protocol
(`Tamal.Wire`), and the loader (`Tamal.Loader`) — are done. `hdl/PLAN.md`
decomposes the remaining impure shell into **IOBUF → topEntity**. This spec is the
IOBUF piece: the bidirectional pad boundary between the engine's registered pin
drives and the physical `IO[3:0]` / `CS#` / `SCK` / `RESET#` / `ALERT#` pins.

The engine's committed types pin the interface (Engine.hs):

- **Drives (`BusOut`, Engine.hs:103-113):** `lanesOut :: Lanes` (= `Vec 4 (Bit, Bit)`,
  per-lane `(output value, output enable)`; `oe = 0` is tri-state), plus
  `csOut`/`sckOut`/`rstOut :: Bit`.
- **Samples (`BusIn`, Engine.hs:89-96):** `ioIn :: Vec 4 Bit` (the sampled IO
  lanes) and `alertIn :: Bit` (the synchronized `ALERT#` level). The engine reads
  `ioIn` **combinationally** at the mid-beat sample point (`stepBusBeat`'s
  `sampleGet`, Engine.hs:242-244, reads `ioIn inp !! 1` at `busPhase 3`) and reads
  `alertIn` as a waited-on **level** (`stepWaitAlert`/`GetAlert`,
  Engine.hs:262-281, 402-408). `Test.Engine` drives `BusIn` with a fresh
  `Vec 4 Bit` each cycle (Test/Engine.hs:201), confirming the combinational
  same-cycle contract.

`Lanes`, `hiZ`, and `serializeX1` already live in the pure leaf
`Tamal.Bus.Serdes` (Serdes.hs:28-46). `IO[3:0]` per-beat drive states like
`serializeX1`'s `(bit', 1) :> (0,0) :> (0,0) :> (0,0)` (IO[0] driven, IO[1..3]
tri-stated) are exactly what the pad buffers must realize — so the four lanes need
**independent** output-enables.

Because this is a thin bidirectional buffer, we build it as one focused leaf now so
that when the topEntity lands, the pad boundary is *wiring*, not *invention* — the
same discipline that produced `Tamal.Mem` ahead of the loader.

---

## 2. Scope & non-goals

**In scope**

- `Tamal.Io`: `espiPads` (four per-lane `BiSignal` IO buffers + three sideband
  output buffers + the `ALERT#` synchronizer) and `alertSync` (the exported 2-flop
  synchronizer helper).
- Signal-level hedgehog properties + HUnit cases (`Test.Io`), including a
  `veryUnsafeToBiSignalIn` loopback harness for the tri-state drive/sample paths.
- Cabal / test-runner wiring.

**Out of scope (deferred to later pieces / roles)**

- **All topEntity wiring** (piece 5): projecting `BusOut`→drives and pads→`BusIn`,
  the `inout` **port shape** (`Vec 4 (BiSignalIn ...)` vs four scalar `inout`s vs a
  bus), instantiating `espiPads`, and closing the loop to the engine + BRAMs.
  `Tamal.Io` does **not** import `Tamal.Engine`.
- **XDC pins** for `IO[3:0]`/`CS#`/`SCK`/`RESET#`/`ALERT#` — added with the
  topEntity (piece 5). This piece adds no pins (its ports are internal until
  instantiated).
- **Target-role driving of `ALERT#`** (Phase 3): v1 is controller role, so
  `ALERT#` is input-only. Making it bidirectional is a later role change.
- **Dual/quad IO maps** (Phase 3): the buffers are mode-agnostic — they drive/read
  whatever `lanesOut` says — but the engine only produces x1 drive patterns in v1.
- **IOB output-register packing, DDR, `IDELAY`/`ODELAY`, `IOSTANDARD`/`DRIVE`/
  `SLEW`, physical pull resistors** — timing-closure / XDC knobs for piece 5 and
  hardware bring-up, not logic here.

---

## 3. Design decisions

| # | Decision | Rationale |
|---|----------|-----------|
| 1 | **Clash `BiSignal`** (`BiSignalIn`/`BiSignalOut`) vs instantiating the Xilinx `IOBUF` primitive vs a pure T/I/sample core. | `BiSignal` is portable, Clash-idiomatic, and **simulatable** — `writeToBiSignal`/`readFromBiSignal` have real Haskell sim semantics, so the tri-state drive/sample is unit-testable via a loopback (§7.3), not just whole-top. Clash lowers a paired `BiSignalIn`+`BiSignalOut` to a Verilog `inout` port + tri-state assign; Vivado infers the `IOBUF` cell. The vendor primitive would need a hand-written blackbox + a sim model and is Xilinx-locked; a pure core would push the actual bidirectional binding out of this piece. |
| 2 | **Per-lane, width-1** `Vec 4 (BiSignalIn/Out 'PullUp dom 1)` vs one width-4 `BiSignal`. | x1 drives IO[0] while IO[1..3] tri-state (`serializeX1`), and GET tri-states all four while sampling IO[1] — the four lanes have **independent** output-enables. `writeToBiSignal` on a width-4 `BiSignal` takes `Maybe (BitVector 4)` = one **shared** OE (all-drive or all-hi-Z), which cannot express the mixed pattern. Four width-1 buffers map 1:1 to four Xilinx `IOBUF` cells (one `T` each). |
| 3 | **`'PullUp` default** vs `'Floating` vs `'PullDown`. | The `BiSignalDefault` sets what a net reads when **nobody** drives: `'Floating` → `errorX` (undefined), `'PullUp` → `maxBound` (idle-high), `'PullDown` → `minBound`. eSPI mandates weak pull-ups on the IO lines; `'PullUp` makes a DUT that fails to drive during a GET sample as a deterministic idle-high `1` (reproducible verdicts) rather than undefined, and keeps the loopback harness's both-tri-stated case defined. The *physical* pull is still set in the XDC (piece 5); the `ds` fixes **simulation** semantics and documents intent. |
| 4 | **IO sample is combinational** (`readFromBiSignal`), **no input synchronizer**. | The engine's tested contract is `BusIn.ioIn` = the current-cycle pad value, sampled at `busPhase 3` relative to the SCK it drives (Engine.hs:242-244). Adding a 2-flop synchronizer would insert 2 cycles of latency and desync the SCK-relative sampling. In v1 controller role the DUT drives IO in response to *our* SCK, so setup/hold is a static-timing (XDC) concern, not a metastability one. |
| 5 | **Only `ALERT#` is synchronized** (2-flop, init high) via `alertSync`. | `ALERT#` is a truly-async sideband with no clock relationship, active-low, consumed as a *level* (`WAIT_ON`/`GET_ALERT`). A 2-flop synchronizer (the `Tamal.Uart.Rx` `sync1`/`synced` pattern, Rx.hs:55-56, init high because deasserted = high) removes metastability; 2 cycles of latency on a level is harmless. |
| 6 | **Sideband outputs are combinational pass-through** — no added register. | `csN`/`sck`/`rstN`/`lanes` are already registered inside the engine `State` and projected glitch-free by `busOut`; re-registering only adds latency and desync risk. Optional IOB output-register **packing** for `Tco` is a piece-5 XDC/attribute knob, not logic here. |
| 7 | **`ALERT#` stays a plain input** `Signal dom Bit`, not a `BiSignal`. | v1 controller role only *receives* `ALERT#`. Keeping it a plain input avoids a needless tri-state; target-role driving is Phase 3 (§2). |
| 8 | **`alertSync` is exported** (not inlined in `espiPads`). | It is the one part of this piece testable **without** `BiSignal` (plain signal-level, like `Test.Uart`), so exporting it gives a clean first red→green slice and a reusable synchronizer for future async inputs. `Tamal.Io` imports only `Clash.Prelude` + `Tamal.Bus.Serdes` (for `Lanes`), staying a dependency-free leaf (no `Tamal.Engine`). |

---

## 4. Module layout & public interface

One new module `hdl/src/Tamal/Io.hs`, carrying the REUSE/SPDX header required of
every `hdl/**/*.hs` file:

```haskell
-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-W-2.0
```

```haskell
module Tamal.Io
  ( espiPads
  , alertSync
  ) where

import Clash.Prelude
import Tamal.Bus.Serdes (Lanes)   -- Vec 4 (Bit, Bit) = per-lane (value, output-enable)

-- | 2-flop synchronizer for the asynchronous, active-low @ALERT#@ sideband.
-- Both flops init high (deasserted), matching the @Tamal.Uart.Rx@ line
-- synchronizer. Output lags the raw input by exactly two cycles.
alertSync ::
  (HiddenClockResetEnable dom) =>
  Signal dom Bit ->
  Signal dom Bit
--   a1 = register 1 raw ; a2 = register 1 a1 ; alertSync raw = a2   (author writes)

-- | The eSPI pad boundary. Four per-lane tri-state @IO[3:0]@ buffers ('PullUp),
-- three sideband output buffers (@CS#@/@SCK@/@RESET#@, combinational
-- pass-through), and the @ALERT#@ synchronizer.
--
-- Per lane i: drive @Just o@ when @oe == 1@ else @Nothing@ (hi-Z); read the pad
-- combinationally into @ioIn !! i@. The drives arrive already registered from the
-- engine 'State', so no output register is added here.
espiPads ::
  (HiddenClockResetEnable dom) =>
  Signal dom Lanes ->                     -- ^ engine @lanesOut@ (per-lane (o, oe))
  Signal dom Bit ->                       -- ^ @csOut@
  Signal dom Bit ->                       -- ^ @sckOut@
  Signal dom Bit ->                       -- ^ @rstOut@
  Signal dom Bit ->                       -- ^ @ALERT#@ (raw, async, active-low)
  Vec 4 (BiSignalIn 'PullUp dom 1) ->     -- ^ the four IO pads (read side)
  ( Vec 4 (BiSignalOut 'PullUp dom 1)     -- the four IO pads (drive side)
  , Signal dom Bit                        -- @CS#@   pin out
  , Signal dom Bit                        -- @SCK@   pin out
  , Signal dom Bit                        -- @RESET#@ pin out
  , Signal dom (Vec 4 Bit)                -- @ioIn@    -> @BusIn.ioIn@   (combinational)
  , Signal dom Bit                        -- @alertIn@ -> @BusIn.alertIn@ (synchronized)
  )
```

**Body shape** (author writes the synthesizable Clash):

```haskell
espiPads lanesS csS sckS rstS alertRaw pads =
  (outs, csS, sckS, rstS, ioInS, alertSync alertRaw)
 where
  laneSigs = unbundle lanesS                      -- Vec 4 (Signal dom (Bit, Bit))
  outs     = zipWith drive pads laneSigs          -- Vec 4 (BiSignalOut 'PullUp dom 1)
  ioInS    = bundle (map readFromBiSignal pads)   -- Signal dom (Vec 4 Bit)
  drive padIn laneSig = writeToBiSignal padIn (toDrive <$> laneSig)
  toDrive (o, oe) = if oe == (1 :: Bit) then Just o else Nothing
```

**Interface rationale**

- `Signal dom Lanes` + three scalar `Signal dom Bit` drives (not a `Signal dom
  BusOut`): the topEntity projects the pad-relevant `BusOut` fields, keeping
  `Tamal.Io` decoupled from `Tamal.Engine` (mirrors `Tamal.Mem`'s
  `Engine`-free leaf discipline).
- `Vec 4 (BiSignalIn 'PullUp dom 1)` in / `Vec 4 (BiSignalOut 'PullUp dom 1)` out:
  the paired in/out `BiSignal`s the topEntity fuses into `inout` ports. `readFrom`
  gives `Signal dom Bit` per lane (`BitSize Bit = 1`); `bundle` assembles the
  `Vec 4 Bit` the engine's `ioIn` wants.
- `alertSync` returns `Signal dom Bit`, wired straight to `BusIn.alertIn`.

**Clash notes (learning-tool detail)** — verified against `clash-prelude-1.10.0`
`Clash/Signal/BiSignal.hs`:

- `readFromBiSignal :: (HasCallStack, BitPack a) => BiSignalIn ds d (BitSize a) ->
  Signal d a`. For `'PullUp`, an undriven net reads `maxBound` (idle-high); for
  `'Floating`, `errorX`; for `'PullDown`, `minBound`.
- `writeToBiSignal :: (HasCallStack, BitPack a, NFDataX a) => BiSignalIn ds d
  (BitSize a) -> Signal d (Maybe a) -> BiSignalOut ds d (BitSize a)`. `Just a`
  drives; `Nothing` is hi-Z. **The written value depends only on the drive
  command, not on reading the pad** (`writeToBiSignal#` forces `bIn` to WHNF but
  returns `maybeSignal`), so an `espiPads` loopback has **no combinational loop**.
- `mergeBiSignalOuts :: (HasCallStack, KnownNat n) => Vec n (BiSignalOut ds dom m)
  -> BiSignalOut ds dom m` — combine multiple drivers of one net.
- `veryUnsafeToBiSignalIn :: (HasCallStack, KnownNat n, Given (SBiSignalDefault
  ds)) => BiSignalOut ds d n -> BiSignalIn ds d n` — close a simulated net; it is
  **time-aligned (zero extra latency)** and, in simulation, **errors (`errorX`) if
  more than one component drives the same cycle** (contention detection). Call it
  **once** per net. `'PullUp` has the required `Given (SBiSignalDefault 'PullUp)`
  instance.
- These primitives are `hasBlackBox`/`OPAQUE`: the Haskell bodies drive
  **simulation** (our tests); the blackboxes drive **synthesis** (piece 5).

---

## 5. Timing & latency contract

| Path | Latency | Note |
|---|---|---|
| `lanesOut` → IO drive | combinational | drives already registered in engine `State` |
| IO pad → `ioIn` | combinational | matches the engine's same-cycle sample contract |
| `csOut`/`sckOut`/`rstOut` → pins | combinational | pass-through, already registered upstream |
| raw `ALERT#` → `alertIn` | **2 cycles** | `alertSync` 2-flop; harmless on a waited-on level |

No path in `espiPads` besides `alertSync` contains a register, so the IO and
sideband tests need no cycle-0 lead-in; the `alertSync` tests do (§7.1, the
`Test.Uart`/Dom100 async-reset gotcha).

---

## 6. eSPI / hardware alignment (informative)

- **Active-low sidebands.** `CS#`, `RESET#`, and `ALERT#` are active-low; the
  engine already carries the correct polarity in `State` (`csN`/`rstN` init 1 =
  deasserted, Engine.hs:157-159), so the buffers pass levels through verbatim.
- **`ALERT#` init high.** Deasserted `ALERT#` is high, so `alertSync` inits both
  flops high — a startup glitch cannot look like a spurious assertion.
- **PullUp vs the DUT.** On real silicon eSPI mandates pull-ups; the `'PullUp`
  `ds` documents that and gives deterministic idle-high sampling. The physical
  resistor is asserted via the XDC in piece 5 (e.g. `PULLTYPE`), independent of the
  Clash `ds`.
- **Mode-agnostic.** The buffers drive/read exactly what `lanesOut`/the pads say;
  dual/quad simply change the `lanesOut` pattern the engine produces (Phase 3), not
  these buffers.

---

## 7. Testing (hedgehog + HUnit) — the assistant's deliverable

One new module `hdl/tests/Test/Io.hs` (`tests :: TestTree`, SPDX header, added to
`unittests.hs`), in the existing tasty + tasty-hedgehog style. Division of labor
(the repo's ping-pong TDD idiom, Engine design §10; BRAM design §10): the
**assistant writes the failing test** and mentors the Clash idioms; the **author
writes the synthesizable Clash under `src/`** to pass it; the two **refactor
together**.

Three test groups, authored in TDD slice order (simplest → richest).

### 7.1 `alertSync` — 2-flop synchronizer (signal-level, no BiSignal)

The first red→green slice. Reference model: output lags the raw input by exactly
two cycles, led by the two init-high samples. The exact index alignment against
`sampleN` (Dom100 asserts reset at cycle 0 — the `Test.Uart` gotcha, also in
`hdl/PLAN.md` piece 3) is **calibrated during authoring** and then pinned:

```haskell
-- output[t] = 1 for t < 2, else raw[t-2]  (calibrated to sampleN)
refAlert :: [Bit] -> [Bit]
refAlert xs = L.take (L.length xs) (1 : 1 : xs)
```

- **HUnit vector** — a concrete stream (e.g. `[1,1,0,0,0,1,1] -> [1,1,1,1,0,0,0]`)
  pins the 2-cycle lag and idle-high lead exactly.
- **HUnit edge** — `ALERT#` asserted (`0`) for a single cycle still propagates as a
  single-cycle `0` (no edge swallow / no extra stretch).
- **Hedgehog property** — for a random `[Bit]`, `simAlert xs === refAlert xs`.

`simAlert` samples `alertSync (fromList (xs <> repeat 1))` under `sampleN`, mirroring
`Test.Uart`'s `runRx` harness (the RAM/`register` function applied **directly
inside** `sampleN` so its `HiddenClockResetEnable` constraint discharges).

### 7.2 Sideband output pass-through (signal-level)

Drive random `cs`/`sck`/`rst` `[Bit]` streams through `espiPads`, with the lanes
tri-stated and the pads undriven — under `'PullUp` an undriven net reads `1`
(defined), so the IO side stays defined and irrelevant to this group.

- **Hedgehog property** — the three scalar outputs each `===` their input stream,
  cycle-for-cycle (combinational, zero lag).

Building the `espiPads` call needs the loopback plumbing from §7.3 (to supply the
`Vec 4 (BiSignalIn ...)`), so §7.2 reuses the §7.3 harness with a
never-driving DUT.

### 7.3 Tri-state drive + sample — `BiSignal` loopback (centerpiece)

The learning core: simulate a bidirectional net. Inside `sampleN`, build a per-lane
closed net from `espiPads`'s drive side and a test "DUT" driver:

```haskell
-- inside the HiddenClockResetEnable region:
(outs, csO, sckO, rstO, ioInS, _) =
  espiPads lanesS csS sckS rstS alertS padIns
dutOuts = zipWith writeToBiSignal padIns dutDrive            -- Vec 4 (BiSignalOut 'PullUp dom 1)
padIns  = map veryUnsafeToBiSignalIn                         -- Vec 4 (BiSignalIn  'PullUp dom 1)
            (zipWith (\o d -> mergeBiSignalOuts (o :> d :> Nil)) outs dutOuts)
-- sample ioInS   (Signal dom (Vec 4 Bit)) -> the engine's view
```

`padIns` feeds `espiPads` **and** is built from `espiPads`'s output — the circular
definition Clash resolves via `veryUnsafeToBiSignalIn`'s lazy `prepend#`. There is
no combinational loop (drive values depend only on `lanesS`/`dutDrive`, not on the
reads, §4), and `veryUnsafeToBiSignalIn` is time-aligned, so `ioInS[t]` is a pure
function of `lanesS[t]` and `dutDrive[t]`. No `L.drop 1` is needed (combinational),
but the harness leads with one idle cycle for uniformity with `alertSync` and to
sidestep the Dom100 cycle-0 reset when convenient.

**Per-lane truth table** (`dutDrive !! i`, `lanes !! i = (o, oe)`):

| our `oe` | DUT drive | expected `ioIn !! i` | exercises |
|:---:|:---:|:---:|---|
| `1` (`o=b`) | `Nothing` | `b` | drive-out: we read our own drive |
| `0` (hi-Z) | `Just d` | `d` | sample-in: we read the DUT |
| `0` (hi-Z) | `Nothing` | `1` | `'PullUp` idle-high |
| `1` (`o=b`) | `Just d` | `errorX` | contention (optional negative case) |

- **HUnit** — one case per defined row (last row optional, checked with
  `Clash.XException.hasX` on the sample rather than an equality).
- **x1 integration case (the payoff).** `lanes = serializeX1 byte !! 0` (lane0
  `oe=1` driving `bit0`; lanes 1..3 hi-Z) with the DUT driving IO[1]=`d`:
  `ioIn === bit0 :> d :> 1 :> 1 :> Nil`. This proves the **four independent
  output-enables** — the entire reason for per-lane `BiSignal` (decision 2).
- **Hedgehog property** — randomize `byte`, the DUT value `d`, and *which* lanes
  the DUT drives (constrained so no lane is driven by both sides in the same cycle,
  avoiding contention); assert `ioIn` equals the per-lane truth-table oracle.

### 7.4 Harness & generator notes

- `Test.Uart`/`Test.Mem` idiom: samplers apply `espiPads`/`alertSync` **directly
  inside** `sampleN` (the `HiddenClockResetEnable` constraint only discharges
  there); inputs are `fromList (xs <> repeat <pad>)`.
- Generators: `genBit` (0/1), `genLanes` (`Vec 4 (Bit, Bit)`), `genDutDrive`
  (`Vec 4 (Maybe Bit)` with a per-lane no-contention constraint against the
  matching `oe`). Reuse `Test.Gen` helpers where they fit.
- Contention and both-tri-stated-under-`'Floating` are **not** asserted by
  equality (they are `errorX`); the optional contention case uses `hasX`.

---

## 8. Files touched

```
new:      hdl/src/Tamal/Io.hs      -- espiPads, alertSync (author writes; +SPDX)
          hdl/tests/Test/Io.hs     -- tests :: TestTree (assistant writes; +SPDX)

modified: hdl/tamal.cabal          -- library exposed-modules += Tamal.Io
                                   --   test other-modules      += Test.Io
          hdl/tests/unittests.hs   -- import qualified Test.Io; add Test.Io.tests
```

No XDC change and no `Engine.hs` change in this piece.

---

## 9. Verification

From `hdl/` (cold Clash/GHC builds are slow — expected):

```
cabal build
cabal test                           # hedgehog + HUnit: Test.Io (+ existing)
cabal run clash -- Tamal --verilog   # codegen smoke — library compiles under Clash
make format-check                    # fourmolu style gate (make format to fix)
```

**Codegen-smoke caveat (risk 1).** `espiPads` is only fully elaborated by Clash
when instantiated in a top with paired `inout` ports; the `--verilog` smoke
confirms `Tamal.Io` **compiles** under Clash (that `BiSignal`, `writeToBiSignal`,
etc. type-check and elaborate as library code), **not** that it lowers to an
`inout`. Real synthesis/`inout` validation happens in piece 5 (topEntity). A
throwaway synthesizable wrapper could force fuller elaboration if desired — flagged,
not built here.

---

## 10. Implementation approach (ping-pong TDD — the repo idiom)

This is a **learning tool**. Suggested slices, each a red → green → refactor loop:

1. **Skeleton + first red.** Author creates `Tamal.Io` with `espiPads`/`alertSync`
   signatures bodied `undefined` so `Test.Io` compiles and goes red; assistant
   explains the `BiSignal` API, per-lane OE, and `'PullUp` semantics.
2. **`alertSync`.** Author writes the two `register 1`s; the §7.1 vector, edge, and
   property go green. (No `BiSignal` yet — cleanest first green.)
3. **`espiPads` drive/sample + sidebands.** Author writes the `writeToBiSignal`/
   `readFromBiSignal`/pass-through body; the §7.2 sideband property and the §7.3
   truth-table cases go green.
4. **x1 integration + property.** The `serializeX1`-driven payoff case and the
   randomized §7.3 property go green — proving independent per-lane tri-state.
5. **Wire-up + close-out.** Cabal + `unittests.hs`; `make format`; the Verilog
   codegen smoke; commit.

The exact task list is the job of the follow-up implementation plan
(`writing-plans`).

---

## 11. Out of scope / follow-ups (roadmap)

Ordered as in `hdl/PLAN.md`:

1. **This spec** — `Tamal.Io` (`espiPads` + `alertSync`), signal-level tested. ←
   implement next
2. **topEntity** (piece 5) — instantiate `instrRam`/`ringRam` + loader + `mealy
   stepM initState` + `espiPads`; project `BusOut`→drives and pads→`BusIn`; decide
   the `inout` **port shape** (`Vec 4 (BiSignalIn ...)` vs four scalar `inout`s vs a
   bus — **risk 2**); extend the XDC (`IO[3:0]`/`CS#`/`SCK`/`RESET#`/`ALERT#`, pull
   resistors, `IOSTANDARD`); optional IOB output-register packing; retire the
   heartbeat. Closes the fetch loop and the drain path.

Explicitly deferred here: all port wiring, the XDC, target-role `ALERT#` driving,
dual/quad IO maps, and IOB timing attributes.

---

## 12. Prior art

- **Clash `Clash.Signal.BiSignal`** — the standard bidirectional-port primitives;
  the two-counters-sharing-a-bus haddock example is exactly the
  `writeToBiSignal`/`mergeBiSignalOuts`/`veryUnsafeToBiSignalIn` loopback §7.3
  mirrors.
- **`Tamal.Uart.Rx`** (Rx.hs:55-56) — the sibling 2-flop line synchronizer
  (`register high` twice) `alertSync` follows.
- **`Tamal.Mem`** — the sibling `Engine`-free impure leaf whose signal-level
  hedgehog+HUnit shape (§7) this spec matches.
- **[mole](https://github.com/felipebalbi/mole)** — the sibling I2C/I3C rig whose
  layered split keeps the pad boundary protocol-ignorant.
