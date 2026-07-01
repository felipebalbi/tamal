# Tamal Register File Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build `Tamal.RegFile` — the 16×32 register file as an opaque, hedgehog-tested pure leaf (`Regs`, `initRegs`, `readReg`, `writeReg`) with `x0` hardwired to zero.

**Architecture:** A new pure module under `hdl/src/Tamal/`, mirroring the existing leaf layout. `Regs` is an opaque `newtype` over `Vec 16 (BitVector 32)`; `readReg`/`writeReg` are pure combinational functions taking the 5-bit `Reg` selector from `Tamal.Isa` (truncated to the low 4 bits), with `x0` reading 0 and writes to `x0` discarded. The Engine will later hold a `Regs` in its Mealy `State`; nothing here is stateful.

**Tech Stack:** Clash 1.10 (`clash-prelude`), GHC, Stack; tasty + tasty-hunit + tasty-hedgehog + hedgehog; fourmolu (2-space, leading commas). Source of truth: `docs/superpowers/specs/2026-07-01-tamal-register-file-design.md`.

---

## File Structure

**New files**

| File | Responsibility |
|------|----------------|
| `hdl/src/Tamal/RegFile.hs` | opaque `newtype Regs` + `initRegs` + `readReg` + `writeReg` |
| `hdl/tests/Test/RegFile.hs` | `tests :: TestTree` — hedgehog + hunit for the register file |

**Modified files**

| File | Change |
|------|--------|
| `hdl/tamal.cabal` | `library exposed-modules += Tamal.RegFile`; `test-suite other-modules += Test.RegFile` |
| `hdl/tests/unittests.hs` | import + append `Test.RegFile.tests` to the tree |

**No change**

- `hdl/tests/Test/Gen.hs` — reuse `genReg`, `genWord`; the `genNonZeroReg`/`genRegs`/`idx4` helpers are local to `Test.RegFile`.

**Notes**

- `Regs` is exported as an **opaque type** (the `Regs` data constructor is *not* exported). Tests build values only through `initRegs` + `writeReg`, inspect via `readReg`, and compare via the derived `Eq`.
- `readReg`/`writeReg` take `Reg` (`= BitVector 5`, imported from `Tamal.Isa`), not `Index 16`.

---

## Task 1: `Tamal.RegFile` scaffold + `Test.RegFile` (RED)

Stand up the module (with real `initRegs` and `errorX` stubs for `readReg`/`writeReg`) and the full test module. Do **not** commit — the whole feature is committed in Task 2.

**Files:**
- Create: `hdl/src/Tamal/RegFile.hs`
- Modify: `hdl/tamal.cabal`
- Create: `hdl/tests/Test/RegFile.hs`
- Modify: `hdl/tests/unittests.hs`

- [ ] **Step 1: Create `hdl/src/Tamal/RegFile.hs` (real `initRegs`, stubbed read/write)**

```haskell
-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0

{- |
16×32 register file (register-file design). A pure leaf: 'Regs' is an opaque
'NFDataX' value the Engine will hold in its Mealy 'State'; 'readReg'/'writeReg'
are pure and combinational, and 'x0' is hardwired to 0. See
docs/superpowers/specs/2026-07-01-tamal-register-file-design.md.
-}
module Tamal.RegFile
  ( Regs
  , initRegs
  , readReg
  , writeReg
  ) where

import Clash.Prelude
import Tamal.Isa (Reg)

newtype Regs = Regs (Vec 16 (BitVector 32))
  deriving stock (Generic, Show, Eq)
  deriving anyclass NFDataX

-- | Power-up contents: all 16 registers zeroed.
initRegs :: Regs
initRegs = Regs (repeat 0)

-- Implemented in Task 2.
readReg :: Regs -> Reg -> BitVector 32
readReg = errorX "Tamal.RegFile.readReg: unimplemented (Task 2)"

-- Implemented in Task 2.
writeReg :: Regs -> Reg -> BitVector 32 -> Regs
writeReg = errorX "Tamal.RegFile.writeReg: unimplemented (Task 2)"
```

- [ ] **Step 2: Wire the library module in `hdl/tamal.cabal`**

Add `Tamal.RegFile` to the `library` `exposed-modules` list (after `Tamal.Alu`):

```
    Tamal.Trace
    Tamal.Branch
    Tamal.Alu
    Tamal.RegFile
```

- [ ] **Step 3: Confirm the library builds with the stubs**

Run: `stack build`
Expected: PASS (real `initRegs` uses the `Regs` constructor, so no unused-constructor warning; the `errorX` stubs type-check; `-Wall` is on but not `-Werror`).

- [ ] **Step 4: Create `hdl/tests/Test/RegFile.hs`**

Literals use **no** numeric underscores (`NumericUnderscores` is not enabled). `genRegs` uses `Data.List.foldl'` — Clash.Prelude's `foldl` is the `Vec` one, and `ws` is a list.

```haskell
-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0

module Test.RegFile (tests) where

import Clash.Prelude
import Data.List (foldl')
import Test.Tasty
import Test.Tasty.HUnit
import Test.Tasty.Hedgehog (testProperty)
import Hedgehog (Gen, property, forAll, (===))
import qualified Hedgehog.Gen as Gen
import qualified Hedgehog.Range as Range

import Tamal.Isa (Reg)
import Tamal.RegFile
import Test.Gen (genReg, genWord)

-- A register whose low-4 index is non-zero (x1..x15).
genNonZeroReg :: Gen Reg
genNonZeroReg = fromIntegral <$> (Gen.integral (Range.linear 1 15) :: Gen Int)

-- An arbitrary Regs built only through the public API (no constructor access).
genRegs :: Gen Regs
genRegs = do
  ws <- Gen.list (Range.linear 0 20) ((,) <$> genReg <*> genWord)
  pure (foldl' (\rs (r, v) -> writeReg rs r v) initRegs ws)

-- Low-4 physical index of a selector (regIndex is not exported).
idx4 :: Reg -> BitVector 4
idx4 = truncateB

tests :: TestTree
tests =
  testGroup "RegFile"
    [ testProperty "read-after-write (r /= x0)" $ property $ do
        rs <- forAll genRegs
        r <- forAll genNonZeroReg
        v <- forAll genWord
        readReg (writeReg rs r v) r === v
    , testProperty "x0 always reads 0" $ property $ do
        rs <- forAll genRegs
        v <- forAll genWord
        readReg rs 0 === 0
        readReg (writeReg rs 0 v) 0 === 0
    , testProperty "write to x0 is a no-op" $ property $ do
        rs <- forAll genRegs
        v <- forAll genWord
        writeReg rs 0 v === rs
    , testProperty "register independence (distinct indices)" $ property $ do
        rs <- forAll genRegs
        (r1, r2) <-
          forAll $
            Gen.filter
              (\(a, b) -> idx4 a /= idx4 b)
              ((,) <$> genNonZeroReg <*> genNonZeroReg)
        v <- forAll genWord
        readReg (writeReg rs r1 v) r2 === readReg rs r2
    , testProperty "untouched registers read 0 from init" $ property $ do
        r <- forAll genReg
        readReg initRegs r === 0
    , testProperty "x16..x31 alias x0..x15" $ property $ do
        r <- forAll genNonZeroReg -- x1..x15
        v <- forAll genWord
        readReg (writeReg initRegs (r + 16) v) r === v
    , testCase "x16 aliases x0 (write discarded)" $
        writeReg initRegs 16 42 @?= initRegs
    ]
```

- [ ] **Step 5: Wire the test module in `hdl/tamal.cabal` and `hdl/tests/unittests.hs`**

In `hdl/tamal.cabal`, add `Test.RegFile` to the `test-suite test-library` `other-modules` list (after `Test.Alu`):

```
    Test.Trace
    Test.Branch
    Test.Alu
    Test.RegFile
```

In `hdl/tests/unittests.hs`, add the import (after `import qualified Test.Alu`):

```haskell
import qualified Test.RegFile
```

and append `Test.RegFile.tests` to the `testGroup "tamal"` list (after `Test.Alu.tests`):

```haskell
    , Test.Alu.tests
    , Test.RegFile.tests
    ]
```

- [ ] **Step 6: Run the suite to confirm RegFile is RED**

Run: `stack test --ta '-p "RegFile"'`
Expected: FAIL — every RegFile case errors with `Tamal.RegFile.readReg/writeReg: unimplemented (Task 2)` (generation forces the stubs through `genRegs`/`readReg`/`writeReg`). This also confirms the module is exposed and `Test.RegFile` compiles. **Do not commit.** Proceed to Task 2.

---

## Task 2: Implement `readReg` / `writeReg` (GREEN) + commit

**Files:**
- Modify: `hdl/src/Tamal/RegFile.hs`

- [ ] **Step 1: Replace the two stubs with the real implementation**

In `hdl/src/Tamal/RegFile.hs`, delete the `readReg`/`writeReg` stub lines (and their `-- Implemented in Task 2.` comments) and add the `regIndex` helper plus the real functions:

```haskell
-- | Map a 5-bit selector to a physical slot: the low 4 bits. Out-of-window
-- selectors (x16..x31) alias their low-4 twin — the leaf stays total.
regIndex :: Reg -> Index 16
regIndex r = unpack (truncateB r)

-- | Read a register value; x0 (index 0) reads 0 regardless of slot contents.
readReg :: Regs -> Reg -> BitVector 32
readReg (Regs v) r
  | idx == 0 = 0
  | otherwise = v !! idx
  where
    idx = regIndex r

-- | Write a register value; writes to x0 (index 0) are discarded.
writeReg :: Regs -> Reg -> BitVector 32 -> Regs
writeReg regs@(Regs v) r x
  | idx == 0 = regs
  | otherwise = Regs (replace idx x v)
  where
    idx = regIndex r
```

- [ ] **Step 2: Run the RegFile suite to verify GREEN**

Run: `stack test --ta '-p "RegFile"'`
Expected: PASS — all 7 RegFile cases green.

- [ ] **Step 3: Normalize formatting and confirm the full suite is green**

Run (from `hdl/`): `make format` (rewrites the two new files to the fourmolu style; the rest of the repo is already formatted so only these change), then `stack test`.
Expected: `make format` succeeds; `stack test` reports all tests passing (smoke, Crc, Isa, Config, Serdes, Trace, Branch, Alu, RegFile).

- [ ] **Step 4: Commit the feature (scaffold + tests + impl together)**

```bash
git add hdl/src/Tamal/RegFile.hs hdl/tests/Test/RegFile.hs hdl/tamal.cabal hdl/tests/unittests.hs
git commit -m "feat(hdl): 16x32 register file (x0 hardwired) with hedgehog tests"
```

---

## Task 3: Codegen + format verification

**Files:** none (verification only)

- [ ] **Step 1: Clash codegen smoke**

Run: `stack run clash -- Tamal --verilog`
Expected: PASS — compiles `Tamal.topEntity`, confirming `Tamal.RegFile` is Clash-clean even though `topEntity` does not reference it yet (library compilation under Clash).

- [ ] **Step 2: fourmolu style gate**

Run (from `hdl/`): `make format-check`
Expected: PASS — nothing unformatted (the new files were normalized by `make format` in Task 2). This is the same check CI runs.

---

## Done criteria

- `hdl/src/Tamal/RegFile.hs` exists with the SPDX header, an opaque `newtype Regs`, and `initRegs` / `readReg` / `writeReg`; `x0` reads 0 and writes to `x0` are discarded; selectors truncate to the low 4 bits (x16..x31 alias).
- `Test.RegFile` passes under `stack test`; the full suite is green.
- `stack run clash -- Tamal --verilog` succeeds; `make format-check` passes.
- One commit: the register-file feature.
- Out of scope, unchanged: `Engine.step`, the `State` aggregate, `RDSR`, x16..x31 trapping, BRAM/UART shell (spec §2, §9).
```
