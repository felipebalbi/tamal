# Tamal BRAM (instruction + trace-ring memories) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build `Tamal.Mem` — two `blockRamPow2`-backed memories, `instrRam` (1024×32) and `ringRam` (4096×32) — that flank the engine, with a hedgehog + HUnit suite against a pure reference model.

**Architecture:** Each memory is a one-line wrapper over Clash's `blockRamPow2 (repeat 0)`, addressed with the engine's native `Unsigned 10`/`Unsigned 12` types. Tests sample the impure `Signal` via `Clash.Prelude.sampleN` (which supplies clock/reset/enable) and compare against a pure assoc-list reference that mirrors `blockRam`'s zero-init, 1-cycle-latency, read-before-write semantics. Ring depth 4096 makes `termAddr = maxBound = D−1` already correct — no engine behavior change.

**Tech Stack:** Clash (`clash-prelude` 1.10), `blockRamPow2`; tasty + tasty-hedgehog + tasty-hunit; `cabal` build from `hdl/`.

**Division of labour (ping-pong TDD — this is a learning tool):** the **assistant writes the failing tests** and mentors on the Clash idioms; the **author writes the synthesizable Clash under `src/`** to make them pass; the two refactor together. Steps are tagged **[assistant]** (test) or **[author]** (`src/`) where it matters; untagged steps (run/commit) are shared.

**Design doc:** `docs/superpowers/specs/2026-07-02-tamal-bram-design.md`.

**Branch:** `feat/hdl-bram` (already created; baseline = 92 tests passing).

---

## File Structure

| File | Responsibility | Action |
|---|---|---|
| `hdl/src/Tamal/Mem.hs` | The two BRAM wrappers `instrRam`, `ringRam`. Dependency-free leaf (no `Tamal.Engine` import). | Create (author) |
| `hdl/tests/Test/Mem.hs` | `tests :: TestTree` — pure `refRam` oracle, `simInstr`/`simRing` samplers, hedgehog + HUnit cases. | Create (assistant) |
| `hdl/tamal.cabal` | Register `Tamal.Mem` (library) and `Test.Mem` (test suite). | Modify |
| `hdl/tests/unittests.hs` | Wire `Test.Mem.tests` into the tasty tree. | Modify |
| `hdl/src/Tamal/Engine.hs` | `termAddr` doc-comment only — record the 4096-word reconciliation. Non-behavioral. | Modify |

All new `hdl/**/*.hs` files carry the REUSE/SPDX header:

```haskell
-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0
```

**Commands (run from `hdl/`):**
- Full suite: `cabal test`
- Focused: `cabal test --test-options '-p "/Mem/"'` (tasty `-p` awk-pattern)
- Formatter: `make format` (fourmolu; run before every commit) / `make format-check`
- Codegen smoke: `cabal run clash -- Tamal --verilog`

---

## Task 1: Bootstrap the harness + wiring; `instrRam` read-back / 1-cycle latency

Establishes `Tamal.Mem` (with `instrRam` real, `ringRam` a deferred stub), the `Test.Mem` reference model + sampler, the cabal/runner wiring, and the first genuine red→green.

**Files:**
- Create: `hdl/src/Tamal/Mem.hs`
- Create: `hdl/tests/Test/Mem.hs`
- Modify: `hdl/tamal.cabal`
- Modify: `hdl/tests/unittests.hs`

- [ ] **Step 1 [author]: create the `Tamal.Mem` skeleton so the tests compile and fail**

`hdl/src/Tamal/Mem.hs` — real signatures; `instrRam` bodied `undefined` for the first red; `ringRam` a deferred stub (Task 3). `undefined` compiles and throws only when sampled, which is exactly the red we want.

```haskell
-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0

{- |
The two block-RAM memories that flank the engine (design doc
2026-07-02-tamal-bram-design.md): the instruction store the engine fetches from,
and the trace ring it writes records into. Both are thin 'blockRamPow2' wrappers
addressed with the engine's own 'Unsigned' widths. This module is a dependency-free
leaf: it does not import 'Tamal.Engine' (the topEntity projects @Maybe Ring@ to the
tuple write port).
-}
module Tamal.Mem
  ( instrRam
  , ringRam
  ) where

import Clash.Prelude

-- | Instruction store: 1024 words (2^10), zero-initialized. Read address is the
-- engine's registered PC (@pcOut :: Unsigned 10@); the write port is the loader's
-- @Maybe (addr, word)@. The output feeds @BusIn.instrWord@. The 1-cycle read
-- latency IS the engine's @Fetch@ bubble. @blockRamPow2@ uses only clock + enable
-- (no content reset), matching the no-reset power-up design.
instrRam ::
  (HiddenClockResetEnable dom) =>
  Signal dom (Unsigned 10) ->
  Signal dom (Maybe (Unsigned 10, BitVector 32)) ->
  Signal dom (BitVector 32)
instrRam = undefined -- Task 1 red; becomes: blockRamPow2 (repeat 0)

-- | Trace ring: 4096 words (2^12), zero-initialized. Implemented in Task 3.
ringRam ::
  (HiddenClockResetEnable dom) =>
  Signal dom (Unsigned 12) ->
  Signal dom (Maybe (Unsigned 12, BitVector 32)) ->
  Signal dom (BitVector 32)
ringRam = undefined -- implemented in the ring slice (plan Task 3)
```

- [ ] **Step 2 [assistant]: write the reference model, the instr sampler, and the first test**

`hdl/tests/Test/Mem.hs`:

```haskell
-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0
{-# LANGUAGE NumericUnderscores #-}

module Test.Mem (tests) where

import Clash.Prelude
import qualified Data.List as L
import Data.Maybe (fromMaybe)
import Test.Tasty
import Test.Tasty.HUnit

import Tamal.Domain (Dom100)
import Tamal.Mem (instrRam)

-- | Pure oracle mirroring 'blockRam' exactly: zero-init, 1-cycle read latency,
-- read-before-write. Produces @[out 1, out 2, ..]@ (the undefined @out 0@ is
-- dropped by the sampler, so the lists align). Assoc list is most-recent-first,
-- so @L.lookup@ returns the value from BEFORE the current cycle's write.
refRam ::
  (KnownNat n) =>
  [Unsigned n] ->
  [Maybe (Unsigned n, BitVector 32)] ->
  [BitVector 32]
refRam addrs writes = go [] (L.zip addrs writes)
 where
  go _ [] = []
  go mem ((a, w) : zs) = fromMaybe 0 (L.lookup a mem) : go (push w mem) zs
  push Nothing m = m
  push (Just (wa, wd)) m = (wa, wd) : m

-- | Sample 'instrRam' over a stimulus, dropping the undefined cycle-0 output.
-- 'sampleN' supplies clock/reset/enable; the inline @:: Signal Dom100 _@ pins the
-- domain so @sampleN@ can solve @KnownDomain@ (the Test.Uart idiom).
simInstr :: [Unsigned 10] -> [Maybe (Unsigned 10, BitVector 32)] -> [BitVector 32]
simInstr addrs writes =
  L.drop 1 $
    sampleN
      (L.length addrs + 1)
      ( instrRam (fromList (addrs <> L.repeat 0)) (fromList (writes <> L.repeat Nothing))
          :: Signal Dom100 (BitVector 32)
      )

tests :: TestTree
tests =
  testGroup
    "Mem"
    [ testCase "instr: write then read-back, exactly 1-cycle latency" $ do
        -- read-before-write: the cycle-0 read of addr 5 still sees 0 (out[1]);
        -- the written value appears from out[2] onward. Pin both the hardware
        -- sampler and the reference oracle to the same concrete expectation.
        let addrs = [0, 5, 5, 5]
            writes = [Just (5, 0xDEAD_BEEF), Nothing, Nothing, Nothing]
            expected = [0, 0xDEAD_BEEF, 0xDEAD_BEEF, 0xDEAD_BEEF]
        simInstr addrs writes @?= expected
        refRam addrs writes @?= expected
    ]
```

> **Imports grow per task** to keep every commit `-Wall`-clean: Task 1 needs only the above; Task 3 adds `ringRam` to the `Tamal.Mem` import; Task 4 adds the Hedgehog imports (`qualified Hedgehog as H`, `Hedgehog.Gen`, `Hedgehog.Range`, `Test.Tasty.Hedgehog (testProperty)`) and `Test.Gen (genWord)`.

- [ ] **Step 3: wire the build (cabal + runner)**

`hdl/tamal.cabal` — add `Tamal.Mem` to the library `exposed-modules` (after `Tamal.Engine`):

```
    Tamal.Engine
    Tamal.Mem
```

and add `Test.Mem` to the test-suite `other-modules` (after `Test.Engine`):

```
    Test.Engine
    Test.Mem
```

`hdl/tests/unittests.hs` — add the import (after `import qualified Test.Engine`):

```haskell
import qualified Test.Mem
```

and add it to the `tests` list (after `Test.Engine.tests`):

```haskell
    , Test.Engine.tests
    , Test.Mem.tests
```

- [ ] **Step 4: run the suite — verify the new test FAILS (red)**

Run: `cabal test --test-options '-p "/Mem/"'`
Expected: builds cleanly; `Mem` group fails — the `instr: write then read-back` case errors on `undefined` (an exception / `errorX`), while the other 92 tests are unaffected.

- [ ] **Step 5 [author]: implement `instrRam` (green)**

In `hdl/src/Tamal/Mem.hs`, replace the `instrRam` body:

```haskell
instrRam = blockRamPow2 (repeat 0)
```

(Leave `ringRam = undefined` — Task 3.) `repeat 0 :: Vec (2^10) (BitVector 32)` is the zero init; Clash lowers it to the BRAM `INIT` strings.

- [ ] **Step 6: run the suite — verify green**

Run: `cabal test --test-options '-p "/Mem/"'`
Expected: `Mem` group passes (1 case). Then `cabal test` → all 93 pass.

- [ ] **Step 7: format and commit**

```bash
make format
git add hdl/src/Tamal/Mem.hs hdl/tests/Test/Mem.hs hdl/tamal.cabal hdl/tests/unittests.hs
git commit -m "feat(hdl): instr BRAM (blockRamPow2) + Test.Mem read-back harness"
```

---

## Task 2: `instrRam` characterization — read-before-write collision + boundaries

Locks the corner behaviors the loader will rely on. `instrRam` already passes these (thin wrapper), so they are characterization tests; an unexpected failure is a real finding.

**Files:**
- Modify: `hdl/tests/Test/Mem.hs`

- [ ] **Step 1 [assistant]: add the collision + boundary cases**

Add these three `testCase`s to the `Mem` `tests` list in `hdl/tests/Test/Mem.hs`:

```haskell
    , testCase "instr: read-before-write collision returns old then new" $
        -- preload addr 3 = 0x1111 at cycle 0; at cycle 1 WRITE 0x2222 to addr 3
        -- while READING addr 3. The colliding read (cycle 1) still yields the old
        -- 0x1111 (out[2]); 0x2222 appears from out[3].
        simInstr
          [0, 3, 3, 3]
          [Just (3, 0x1111), Just (3, 0x2222), Nothing, Nothing]
          @?= [0, 0x1111, 0x2222, 0x2222]
    , testCase "instr: address 0 is a normal slot (no x0 hardwiring here)" $
        simInstr
          [0, 0, 0]
          [Just (0, 0xCAFE_F00D), Nothing, Nothing]
          @?= [0, 0xCAFE_F00D, 0xCAFE_F00D]
    , testCase "instr: top address (maxBound = 1023) reads back" $
        simInstr
          [0, maxBound, maxBound]
          [Just (maxBound, 0x0BAD_C0DE), Nothing, Nothing]
          @?= [0, 0x0BAD_C0DE, 0x0BAD_C0DE]
```

- [ ] **Step 2: run — verify all pass**

Run: `cabal test --test-options '-p "/Mem/"'`
Expected: 4 `Mem` cases pass. If the collision case fails, Clash's `blockRam` is *not* read-before-write in this version — stop and reconcile `refRam`/the design before continuing.

- [ ] **Step 3: format and commit**

```bash
make format
git add hdl/tests/Test/Mem.hs
git commit -m "test(hdl): instr BRAM read-before-write + address-boundary cases"
```

---

## Task 3: `ringRam` slice — read-back, drain sweep, and the `termAddr` reconciliation

The ring's own red→green, its drain-sweep characterization (models the loader), and the non-behavioral engine comment that closes the `termAddr` loop.

**Files:**
- Modify: `hdl/src/Tamal/Mem.hs`
- Modify: `hdl/tests/Test/Mem.hs`
- Modify: `hdl/src/Tamal/Engine.hs`

- [ ] **Step 1 [assistant]: add the ring sampler and the first ring test (red)**

Add `ringRam` to the `Tamal.Mem` import (`import Tamal.Mem (instrRam, ringRam)`), then add the `simRing` sampler (below `simInstr`) in `hdl/tests/Test/Mem.hs`:

```haskell
-- | Sample 'ringRam' over a stimulus, dropping the undefined cycle-0 output.
simRing :: [Unsigned 12] -> [Maybe (Unsigned 12, BitVector 32)] -> [BitVector 32]
simRing addrs writes =
  L.drop 1 $
    sampleN
      (L.length addrs + 1)
      ( ringRam (fromList (addrs <> L.repeat 0)) (fromList (writes <> L.repeat Nothing))
          :: Signal Dom100 (BitVector 32)
      )
```

Add these two `testCase`s to the `Mem` list:

```haskell
    , testCase "ring: write then read-back at a mid address" $
        simRing
          [0, 42, 42, 42]
          [Just (42, 0x1234_5678), Nothing, Nothing, Nothing]
          @?= [0, 0x1234_5678, 0x1234_5678, 0x1234_5678]
    , testCase "ring: drain sweep streams the written block in order" $
        -- write 4 words to addrs 100..103 (one per cycle), then sweep-read
        -- 100..103. sweep = take 4 (drop 4 (simRing ..)): reads issued at cycles
        -- 4..7 surface post-latency, by which point all 4 writes have landed.
        let blk = [0xA0, 0xA1, 0xA2, 0xA3] :: [BitVector 32]
            writes = [Just (100 + fromIntegral i, blk L.!! i) | i <- [0 .. 3]] <> L.replicate 4 Nothing
            addrs = L.replicate 4 0 <> [100 + fromIntegral i | i <- [0 .. 3]]
         in L.take 4 (L.drop 4 (simRing addrs writes)) @?= blk
```

- [ ] **Step 2: run — verify the ring cases FAIL (red)**

Run: `cabal test --test-options '-p "/Mem/"'`
Expected: the two `ring:` cases error on `undefined` (`ringRam` unimplemented); the four instr cases still pass.

- [ ] **Step 3 [author]: implement `ringRam` (green)**

In `hdl/src/Tamal/Mem.hs`, replace the `ringRam` body:

```haskell
ringRam = blockRamPow2 (repeat 0)
```

- [ ] **Step 4: run — verify green**

Run: `cabal test --test-options '-p "/Mem/"'`
Expected: all six `Mem` cases pass.

- [ ] **Step 5 [author]: reconcile the engine's `termAddr` comment (non-behavioral)**

In `hdl/src/Tamal/Engine.hs`, replace the `termAddr` doc-comment (the `{- | Fixed terminator slot … -}` block above `termAddr :: Unsigned 12`) with:

```haskell
{- | Fixed terminator slot: the top of the ring address space. 'Tamal.Mem.ringRam'
pins the ring at 4096 words, so @termAddr = maxBound = D - 1@ — the last usable
record slot sits at @termAddr - 1@ (see 'pushWord').
-}
```

The definition `termAddr = maxBound` is unchanged. Confirm the engine still builds and its tests pass (this only edits a comment): `cabal test --test-options '-p "/Engine/"'` → all pass.

- [ ] **Step 6: format and commit**

```bash
make format
git add hdl/src/Tamal/Mem.hs hdl/tests/Test/Mem.hs hdl/src/Tamal/Engine.hs
git commit -m "feat(hdl): ring BRAM (blockRamPow2); reconcile termAddr to D=4096"
```

---

## Task 4: hedgehog properties — model equivalence + sweep, both widths

The exhaustive coverage: random sequences checked against `refRam`, and an independent last-write-wins sweep oracle.

**Files:**
- Modify: `hdl/tests/Test/Mem.hs`

- [ ] **Step 1 [assistant]: add the Hedgehog imports + the window/command generators**

Extend the imports in `hdl/tests/Test/Mem.hs` (these are the ones deferred from Task 1):

```haskell
import qualified Hedgehog as H
import qualified Hedgehog.Gen as Gen
import qualified Hedgehog.Range as Range
import Test.Tasty.Hedgehog (testProperty)

import Test.Gen (genWord)
```

Add these generators (top level, below `refRam`):

```haskell
-- | A read/write address in the window 0..15 (dense enough for read-after-write
-- hits; safe for both Unsigned 10 and Unsigned 12).
genWin :: (KnownNat n) => H.Gen (Unsigned n)
genWin = fromIntegral <$> Gen.int (Range.linear 0 15)

-- | One cycle of stimulus: a read address and a maybe-write (window addr + data).
genCmd :: (KnownNat n) => H.Gen (Unsigned n, Maybe (Unsigned n, BitVector 32))
genCmd = (,) <$> genWin <*> Gen.maybe ((,) <$> genWin <*> genWord)
```

- [ ] **Step 2 [assistant]: add the property tests**

Add these to the `Mem` `tests` list:

```haskell
    , testProperty "instr: matches the reference model (random sequences)" $ H.property $ do
        cmds <-
          H.forAll
            (Gen.list (Range.linear 0 64) (genCmd :: H.Gen (Unsigned 10, Maybe (Unsigned 10, BitVector 32))))
        let addrs = fmap fst cmds
            writes = fmap snd cmds
        simInstr addrs writes H.=== refRam addrs writes
    , testProperty "ring: matches the reference model (random sequences)" $ H.property $ do
        cmds <-
          H.forAll
            (Gen.list (Range.linear 0 64) (genCmd :: H.Gen (Unsigned 12, Maybe (Unsigned 12, BitVector 32))))
        let addrs = fmap fst cmds
            writes = fmap snd cmds
        simRing addrs writes H.=== refRam addrs writes
    , testProperty "ring: sweep read-back = last write wins (independent oracle)" $ H.property $ do
        ws <-
          H.forAll
            (Gen.list (Range.linear 0 40) ((,) <$> genWin <*> genWord) :: H.Gen [(Unsigned 12, BitVector 32)])
        let win = [0 .. 15] :: [Unsigned 12]
            writes = fmap Just ws <> L.replicate (L.length win) Nothing
            addrs = L.replicate (L.length ws) 0 <> win
            -- reads of win[i] are issued at cycle (len ws + i); after the drop-1
            -- sampler that is index (len ws + i), so:
            sweep = L.take (L.length win) (L.drop (L.length ws) (simRing addrs writes))
            -- independent oracle: most recent write to each address (reverse = latest first)
            oracle = fmap (\a -> fromMaybe 0 (L.lookup a (L.reverse ws))) win
        sweep H.=== oracle
```

- [ ] **Step 3: run — verify all properties pass**

Run: `cabal test --test-options '-p "/Mem/"'`
Expected: 6 unit cases + 3 properties pass (each property `✓ passed 100 tests`). Then `cabal test` → all pass.

- [ ] **Step 4: format and commit**

```bash
make format
git add hdl/tests/Test/Mem.hs
git commit -m "test(hdl): BRAM hedgehog model-equivalence + sweep (both widths)"
```

---

## Task 5: final verification — full suite, format gate, Clash codegen smoke

Confirms the whole branch is green, style-clean, and Clash-clean. No new code expected; commit only if the format gate rewrites anything.

**Files:** none (verification only)

- [ ] **Step 1: full test suite**

Run: `cabal test`
Expected: all tests pass — the original 92 plus the 9 new `Mem` cases/properties (101 total), 0 failures.

- [ ] **Step 2: format gate**

Run: `make format-check`
Expected: no diffs. If it reports changes, run `make format`, then:

```bash
git add -A
git commit -m "style(hdl): fourmolu format Tamal.Mem + Test.Mem"
```

- [ ] **Step 3: Clash codegen smoke**

Run: `cabal run clash -- Tamal --verilog`
Expected: completes without error (the `clash` executable builds the `tamal` library including `Tamal.Mem`, confirming it is Clash-clean; the placeholder top is unchanged, so no `Mem` gateware is emitted yet).

- [ ] **Step 4: report**

Confirm branch `feat/hdl-bram` is green and ready to integrate (or to continue with piece 2, the wire protocol). Summarize: `Tamal.Mem` built + tested, `termAddr` reconciled, baseline preserved.

---

## Self-Review

**1. Spec coverage** (design §2, §4–§7):
- §4 `instrRam`/`ringRam` interface → Task 1 (instr), Task 3 (ring). ✓
- §5 geometry + `termAddr` reconciliation → Task 3 Step 5. ✓
- §6 read-before-write + `out[0]` drop → `refRam`/`simInstr`/`simRing` (Task 1), collision case (Task 2). ✓
- §7.1 reference model → Task 1 Step 2. ✓
- §7.2 samplers → Task 1 (`simInstr`), Task 3 (`simRing`). ✓
- §7.3 P1 model equivalence (both widths) → Task 4. ✓
- §7.3 P2 read-back sweep → Task 4 (independent last-write-wins oracle) + Task 3 drain-sweep case. ✓
- §7.4 C3 write-then-read-back (0 / mid / top) → Task 1 + Task 2. ✓
- §7.4 C4 exact 1-cycle latency → Task 1. ✓
- §7.4 C5 read-before-write collision → Task 2. ✓
- §7.4 C6 ring drain sweep → Task 3. ✓
- §8 files touched → all covered (Mem.hs, Test/Mem.hs, cabal, unittests.hs, Engine.hs). ✓
- §9 verification (cabal test / format / clash smoke) → Task 5. ✓

**2. Placeholder scan:** no TBD/TODO; every code step shows complete code; the deliberate `undefined` bodies are transient red-state skeletons, each replaced within the same task (`instrRam` Task 1 Step 5, `ringRam` Task 3 Step 3). ✓

**3. Type/name consistency:** `refRam`, `simInstr`, `simRing`, `genWin`, `genCmd` used identically across tasks; `instrRam :: … Unsigned 10 …`, `ringRam :: … Unsigned 12 …` match the design and the engine's `pcOut`/`Ring` types; `genWord` imported from `Test.Gen` (existing, `:: Gen (BitVector 32)`); `Dom100` from `Tamal.Domain`. Sampler alignment (`drop 1`, `length + 1`) is consistent everywhere. ✓
