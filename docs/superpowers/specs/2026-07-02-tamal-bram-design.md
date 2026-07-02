# Tamal — BRAM (instruction + trace-ring memories) Design

Date: 2026-07-02
Status: Approved (design); implementation not started
Scope: The two block-RAM memories that flank the engine — a new leaf module
`Tamal.Mem` exposing `instrRam` (1024×32) and `ringRam` (4096×32), both thin
`blockRamPow2` wrappers. Signal-level, hedgehog + HUnit tested against a pure
reference model. The `Maybe Ring` projection, the loader drain FSM, the topEntity
wiring, UART, and IOBUF are **out of scope** (§2, §11).

Companion to the ISA & HDL Engine design
(`docs/superpowers/specs/2026-07-01-tamal-isa-design.md`, esp. §10) and the Engine
design (`docs/superpowers/specs/2026-07-02-tamal-engine-design.md`, esp. §3.1–3.3
memory ports, §7.1 ring layout, §12 out-of-scope shell). Anchored to
`hdl/PLAN.md` piece 1 ("BRAM — instruction + ring memories; do first; unblocked").
Written to be implementable from a fresh session.

---

## 1. Purpose & context

`Engine.step` is built and hedgehog-tested; every *pure* leaf exists. What remains
is the **impure `topEntity` shell**, decomposed by `hdl/PLAN.md` into
BRAM → wire protocol → loader → IOBUF → topEntity. This spec is the first piece:
the two memories the engine reads/writes.

The engine's committed types pin the interface (Engine.hs):

- Instruction store: read address `pcOut :: Unsigned 10` (Engine.hs:34-35,
  `type AW = 10`); output feeds `BusIn.instrWord :: BitVector 32` (Engine.hs:90).
  Its **1-cycle read latency is exactly the engine's `Fetch` bubble**
  (Engine.hs:216-218).
- Trace ring: written by the engine's `Maybe Ring` (Engine.hs:114-122,
  `Ring { rAddr :: Unsigned 12, rData :: BitVector 32 }`); addressed for drain by
  the loader; `termAddr = maxBound :: Unsigned 12` (Engine.hs:342-343).

Because these two BRAMs are the same primitive at two widths, we build them as one
small leaf now so that when the loader and topEntity land, the memory is *wiring*,
not *invention* — the same discipline that produced `Tamal.RegFile` ahead of
`Engine.step`.

---

## 2. Scope & non-goals

**In scope**

- `Tamal.Mem`: `instrRam` (1024×32) and `ringRam` (4096×32), both
  `blockRamPow2 (repeat 0)`.
- Signal-level hedgehog properties + HUnit unit cases against a pure reference
  model (`Test.Mem`).
- Cabal / test-runner wiring.
- A **non-behavioral** doc-comment update to `Engine.hs`'s `termAddr` recording the
  4096-word reconciliation.

**Out of scope (deferred to later pieces)**

- The `Maybe Ring → Maybe (rAddr, rData)` projection and **all** port wiring — that
  is the topEntity (piece 5). `Tamal.Mem` does **not** import `Tamal.Engine`.
- The loader's drain counter / load path / FSM (piece 3), the UART transport, the
  IOBUF tri-states (piece 4), and any XDC change (these memories are internal, no
  pins).
- Widening the ring address type, non-power-of-two depths, true-dual-port, byte
  write-enables, ECC — none are needed for v1.

---

## 3. Design decisions

| # | Decision | Rationale |
|---|----------|-----------|
| 1 | **Single `Tamal.Mem` module** with `instrRam` + `ringRam` (vs two modules, vs folding into the topEntity). | Both are the same primitive at different widths; one focused leaf keeps them discoverable and testable together, matching the repo's one-concern-per-module layout, and keeps the topEntity thin. |
| 2 | **`blockRamPow2`** (vs `blockRam` + `Index n`, vs `blockRamU`/`blockRam1`). | The engine addresses with `Unsigned 10`/`Unsigned 12` and the depths are exact powers of two, so `blockRamPow2 :: Vec (2^n) a -> Signal dom (Unsigned n) -> …` maps **1:1** with no `Unsigned → Index` conversion and no out-of-range guard. Maps to a Xilinx block RAM. |
| 3 | **Zero-init (`repeat 0`)** (vs `blockRamU` undefined, vs `blockRam1` single value). | Deterministic simulation for hedgehog — every read at cycle ≥ 1 is defined. Becomes the BRAM `INIT` on hardware. The loader overwrites instruction slots before the engine runs, and the ring's records/terminator are written before drain, so zero is a safe, testable power-up value. |
| 4 | **Generic tuple write port** `Maybe (Unsigned n, BitVector 32)` (vs `Maybe Ring`, which would import `Tamal.Engine`). | Keeps `Tamal.Mem` a dependency-free leaf, exactly like `RegFile` (which never imports `Engine`). The `Maybe Ring → (rAddr, rData)` unpack is a one-liner that belongs in the topEntity. |
| 5 | **Ring depth D = 4096** → `termAddr = maxBound = D − 1`; **no engine behavior change**. | Full `Unsigned 12` range maximizes trace capacity within the existing address width; the engine's constant is already exactly `D − 1`; 4 BRAM36 is negligible against the xc7a100t's 135 BRAM36. |
| 6 | **`HiddenClockResetEnable dom`** on the wrappers (vs the precise `HiddenClock dom, HiddenEnable dom`). | Uniform with the rest of the codebase (`uart`, the heartbeat top) and with what the topEntity supplies via `withClockResetEnable`. A Clash-note records that `blockRam` uses only clock + enable and never resets contents — consistent with the deliberate no-reset power-up design (AGENTS.md). |

---

## 4. Module layout & public interface

One new module `hdl/src/Tamal/Mem.hs`, carrying the REUSE/SPDX header required of
every `hdl/**/*.hs` file:

```haskell
-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0
```

```haskell
module Tamal.Mem
  ( instrRam
  , ringRam
  ) where

import Clash.Prelude

-- | Instruction store: 1024 words (2^10), zero-initialized. The read address is
-- the engine's registered PC (@pcOut :: Unsigned 10@); the write port is the
-- loader's @Maybe (addr, word)@. The output feeds @BusIn.instrWord@. The 1-cycle
-- read latency IS the engine's @Fetch@ bubble. @AW = 10@ matches the engine's
-- program-address width and the ±1024-instruction branch range.
instrRam ::
  (HiddenClockResetEnable dom) =>
  Signal dom (Unsigned 10) ->
  Signal dom (Maybe (Unsigned 10, BitVector 32)) ->
  Signal dom (BitVector 32)
instrRam = blockRamPow2 (repeat 0)

-- | Trace ring: 4096 words (2^12), zero-initialized. The write port is the
-- engine's ring write (the topEntity projects @Maybe Ring@ to
-- @Maybe (rAddr, rData)@); the read address is the loader's drain counter.
-- Depth D = 4096, so @termAddr = D - 1 = 4095 = maxBound :: Unsigned 12@.
ringRam ::
  (HiddenClockResetEnable dom) =>
  Signal dom (Unsigned 12) ->
  Signal dom (Maybe (Unsigned 12, BitVector 32)) ->
  Signal dom (BitVector 32)
ringRam = blockRamPow2 (repeat 0)
```

**Interface rationale**

- Both wrappers only fix the width and the zero-init on `blockRamPow2`; the whole
  synthesizable body of each is `blockRamPow2 (repeat 0)`.
- Address types are the engine's own `Unsigned 10` / `Unsigned 12`, so the
  topEntity connects `pcOut`/drain-counter straight through with no conversion.
- The write port is a plain `Maybe (Unsigned n, BitVector 32)` (no `Ring` import),
  keeping the leaf dependency-free.

**Clash notes (learning-tool detail)**

- `blockRamPow2 :: (KnownDomain dom, HiddenClock dom, HiddenEnable dom, NFDataX a,
  KnownNat n) => Vec (2^n) a -> Signal dom (Unsigned n) -> Signal dom (Maybe
  (Unsigned n, a)) -> Signal dom a`. The wrappers' `HiddenClockResetEnable`
  constraint is a superset — GHC discharges the clock + enable it needs and
  ignores reset (block RAM has no content reset).
- `repeat 0 :: Vec (2^10) (BitVector 32)` (resp. `Vec (2^12)`) is the initial
  content; Clash lowers it to the block-RAM `INIT` strings, so Vivado infers BRAM
  (not distributed/LUT RAM).
- **Latency:** the output at cycle *t* is the value read at cycle *t − 1*
  (1-cycle). The **first** output (cycle 0) is `deepErrorX` (undefined) — see §6.
- **Read-before-write:** a same-cycle read + write to one address surfaces the
  **old** value on the next cycle. Pinned by a unit test (§6) so the loader can
  rely on it.

---

## 5. Memory geometry & `termAddr` reconciliation

| Memory | Depth | Address | Data | Primitive | ≈ BRAM36 |
|---|---|---|---|---|---|
| instruction | 1024 = 2^10 | `Unsigned 10` | `BitVector 32` | `blockRamPow2 (repeat 0)` | 1 |
| trace ring | 4096 = 2^12 | `Unsigned 12` | `BitVector 32` | `blockRamPow2 (repeat 0)` | 4 |

With D = 4096, `termAddr = maxBound :: Unsigned 12 = 4095 = D − 1`, which is the
engine's **existing** constant (Engine.hs:342-343) — so **no behavioral edit** is
required. The only `Engine.hs` change is replacing the `termAddr` doc-comment
("Fixed terminator slot … the top-shell sizes it", Engine.hs:339-341) with a note
that `Tamal.Mem` pins the ring at 4096 words, hence `maxBound = D − 1`. The ring
layout (Engine design §7.1) is unchanged:

```
word[0]                 REVISION            (written in Preamble)
word[1 .. termAddr-1]   record stream       (ringPtr starts at 1)
word[termAddr]          HALT terminator     (fixed, overflow-proof)
```

The instruction store's `AW = 10` matches Engine.hs:35 and the ISA's
±1024-instruction branch offset range.

---

## 6. Latency & integration contract (informative — not built here)

Recorded so the interface is the right shape for the loader (piece 3) and
topEntity (piece 5).

- **Instruction-fetch loop:** `pcOut` is registered inside the engine's `State`
  (projected by `busOut`), so the loop `pcOut → instrRam → instrWord → step → next
  pc` contains **two** registers (the `pc` register and the BRAM output register) —
  there is no combinational loop, and the BRAM's 1-cycle latency is absorbed by the
  `Fetch` phase.
- **Ring write → drain:** the engine emits `Maybe Ring` only during RUN; the loader
  reads only during DRAIN (post-HALT). RUN never reads and DRAIN never writes, so
  there is no read/write port collision in normal operation; the read-before-write
  rule (below) defines the corner anyway.
- **Read-before-write (Clash `blockRam` default):** if a read and a write target
  the same address in the same cycle, the read returns the value from *before* the
  write. Documented and pinned by a unit test.
- **The `out[0]` gotcha:** `blockRamPow2`'s cycle-0 output is `deepErrorX`
  (undefined). Every test **must** `L.drop 1` the sampled stream before forcing it
  (the standard Clash `blockRam` doctest `L.tail` pattern) and must never compare
  sample 0.

---

## 7. Testing (hedgehog + HUnit baseline) — the assistant's deliverable

One new module `hdl/tests/Test/Mem.hs` (`tests :: TestTree`, SPDX header, added to
`unittests.hs`), in the existing tasty + tasty-hedgehog style.

### 7.1 Reference model

A pure, dependency-free assoc-list memory that mirrors `blockRam` exactly
(zero-init, 1-cycle latency, read-before-write). Polymorphic in the address width
`n`, so one model serves both BRAMs. It produces `[out[1], out[2], …]` (the
undefined `out[0]` is dropped by the harness, §7.2):

```haskell
-- out[t+1] = (memory state before cycle t's write) read at addr[t]
refRam ::
  (KnownNat n) =>
  [Unsigned n] ->
  [Maybe (Unsigned n, BitVector 32)] ->
  [BitVector 32]
refRam addrs writes = go [] (L.zip addrs writes)
 where
  go _   []            = []
  go mem ((a, w) : zs) = fromMaybe 0 (L.lookup a mem) : go (push w mem) zs
  push Nothing         m = m
  push (Just (wa, wd)) m = (wa, wd) : m   -- most-recent-first; L.lookup reads OLD
```

`L.lookup` on a most-recent-first assoc list returns the latest write to an
address (read-before-write is the read happening *before* the current cycle's push).

### 7.2 Harness

The proven `Test.Uart` pattern — `Clash.Prelude.sampleN` supplies
`clockGen`/`resetGen`/`enableGen` to the `HiddenClockResetEnable` signal — with the
mandatory `L.drop 1`:

```haskell
simRam ::
  (KnownNat n) =>
  (Signal Dom100 (Unsigned n) ->
   Signal Dom100 (Maybe (Unsigned n, BitVector 32)) ->
   Signal Dom100 (BitVector 32)) ->
  [Unsigned n] ->
  [Maybe (Unsigned n, BitVector 32)] ->
  [BitVector 32]
simRam ramFn addrs writes =
  L.drop 1 $
    sampleN
      (L.length addrs + 1)
      ( ramFn
          (fromList (addrs <> L.repeat 0))
          (fromList (writes <> L.repeat Nothing))
          :: Signal Dom100 (BitVector 32)
      )
```

**Alignment:** for input length `n`, `refRam addrs writes` has length `n`
(`out[1..n]`), and `simRam` returns `L.drop 1 (sampleN (n+1) …) = [out[1..n]]` —
equal length, index-aligned. Dropping sample 0 guarantees no `deepErrorX` is ever
forced (zero-init makes every remaining element defined).

### 7.3 Properties (hedgehog) — run for `instrRam` (n=10) **and** `ringRam` (n=12)

1. **Model equivalence** — random interleaved read-address / write sequences (a
   `Gen.list` of `(addr, Maybe (waddr, wdata))`), with addresses drawn from a small
   window (e.g. 0..15) so reads frequently hit prior writes:
   `simRam ramFn addrs writes === refRam addrs writes`. Exercises latency,
   read-before-write ordering, and per-address independence in one property.
2. **Full read-back sweep** — apply random `(waddr, wdata)` writes (window addresses
   for collisions), then a deterministic address sweep over the window: each slot
   reads back its last-written value, else 0. Confirms writes land at the intended
   slot with no aliasing.

### 7.4 Unit cases (HUnit)

3. **Write-then-read-back** — write a value, then read it a cycle later; check at
   address 0, a mid address, and the top address (`maxBound` = 1023 for instr, 4095
   for ring) for boundary coverage.
4. **Exact 1-cycle latency** — write address `a` at cycle 0; issue read of `a` at
   cycle 1; the cycle-1 output is still the *old* (zero) value and the value appears
   at cycle 2 — asserting the delay is exactly one cycle, not zero or two.
5. **Read-before-write collision** — read and write the same address in the same
   cycle: the next output is the old value, and the value after is the new one.
6. **Ring drain sweep** — write a known contiguous block `word[b..b+k]`, then
   sweep-read `b..b+k`; the streamed outputs equal the written block (models the
   loader's post-HALT drain).

### 7.5 Generator strategy

Addresses for the equivalence/sweep properties are drawn from a **small window**
(0..15) so random sequences produce read-after-write hits; boundary/max addresses
are covered by the unit cases. `wdata` is `genDefinedBitVector` (reuse
`Test.Gen`-style helpers). Both BRAMs share the same polymorphic `refRam`/`simRam`;
the property bodies instantiate at `Unsigned 10` and `Unsigned 12`.

---

## 8. Files touched

```
new:      hdl/src/Tamal/Mem.hs        -- instrRam, ringRam (author writes; +SPDX)
          hdl/tests/Test/Mem.hs       -- tests :: TestTree (assistant writes; +SPDX)

modified: hdl/tamal.cabal             -- library exposed-modules += Tamal.Mem
                                      --   test other-modules      += Test.Mem
          hdl/tests/unittests.hs      -- import qualified Test.Mem; add Test.Mem.tests
          hdl/src/Tamal/Engine.hs     -- termAddr doc-comment only (non-behavioral)
```

---

## 9. Verification

From `hdl/` (cold Clash/GHC builds are slow — expected):

```
stack build
stack test                           # hedgehog + HUnit: Test.Mem (+ existing)
stack run clash -- Tamal --verilog   # codegen smoke — confirms Tamal.Mem is Clash-clean
make format-check                    # fourmolu style gate (make format to fix)
```

`Tamal.Mem` is not yet referenced by `topEntity`, so the Verilog smoke exercises
library compilation under Clash (that `blockRamPow2` elaborates), not new gateware.

---

## 10. Implementation approach (ping-pong TDD — the repo idiom)

This is a **learning tool**: the division of labour follows the Engine design §10.

- The **assistant writes the failing test** for the next slice (red) and mentors on
  the Clash idioms; the **author writes the synthesizable Clash under `src/`** to
  pass it (green); the two **refactor together**.

Suggested slice order (each a red → green → refactor loop):

1. **Skeleton + first red.** Author creates `Tamal.Mem` with the two signatures
   bodied `undefined` so the assistant's `Test.Mem` compiles and goes red; assistant
   explains `blockRamPow2`, pow2 addressing, and the `out[0]` gotcha.
2. **`instrRam`.** Author writes `blockRamPow2 (repeat 0)`; model-equivalence
   (n=10), latency, and collision tests go green.
3. **`ringRam`.** Same primitive at width 12; the sweep and drain tests go green.
4. **Wire-up + close-out.** Cabal + `unittests.hs`; the `termAddr` doc-comment;
   `make format`; the Verilog codegen smoke; commit.

The exact task list is the job of the follow-up implementation plan
(`writing-plans`).

---

## 11. Out of scope / follow-ups (roadmap)

Ordered as in `hdl/PLAN.md`:

1. **This spec** — the two BRAMs (Signal-level, hedgehog + HUnit). ← implement next
2. **Wire protocol** — fill the `tamal-abi` control/result framing (loader
   prerequisite).
3. **Loader** — the UART load/drain FSM: parses control bytes → writes `instrRam` →
   pulses `startIn`; on `haltedOut`, sweeps `ringRam` → UART TX. Owns the
   `Maybe Ring → (rAddr, rData)` projection's *consumer* side and the drain counter.
4. **IOBUF** — tri-state `IO[3:0]` + sideband pins.
5. **topEntity** — instantiate `instrRam`/`ringRam` + loader + `mealy stepM
   initState` + IOBUFs; wire pins; extend the XDC; retire the heartbeat. Performs
   the `Maybe Ring` projection and closes the fetch loop.

Explicitly deferred here: the `Maybe Ring` projection, all port wiring, the drain
FSM, address-type widening, and non-power-of-two depths.

---

## 12. Prior art

- **Clash `blockRam`/`blockRamPow2`** — the standard synchronous-BRAM primitives;
  their 1-cycle-latency, read-before-write, undefined-`out[0]` semantics (and the
  `L.tail`/`L.drop 1` test idiom) are exactly what §6–§7 mirror.
- **[Lion](https://github.com/standardsemiconductor/lion)** — RV32I in Clash; its
  instruction/data memories are likewise `blockRam`-backed with a registered-PC
  read and a 1-cycle fetch bubble.
- **[mole](https://github.com/felipebalbi/mole)** — the sibling I2C/I3C rig whose
  layered split keeps memory protocol-ignorant.
