# Tamal ALU & Branch Comparator — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: use `superpowers:subagent-driven-development` or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
>
> **COLLABORATION NOTE:** This is a *paired* plan. **Tasks 3 and 5 are written by the human (Felipe)** at the keyboard, with a guided walkthrough. Do **not** auto-implement the `alu` core or `branchTaken` — present the walkthrough, let the human write the body, then run the tests together. All other tasks (scaffolding, `dataResult` wrapper, decode tightening, tests, wiring) are agent/assistant work.

**Goal:** Build the pure, combinational compute layer of the tamal engine — the DATA-group ALU (`Tamal.Alu`) and the CTRL-group branch comparator (`Tamal.Branch`) — as two hedgehog-tested Clash modules, plus one decode tightening in `Tamal.Isa`.

**Architecture:** Two new pure leaf modules under `hdl/src/Tamal/`, mirroring the existing `Isa`/`Crc`/`Config`/`Serdes`/`Trace` layout. `Tamal.Alu` layers a thin `Op`-dispatched `alu` core under a `dataResult` wrapper that owns immediate/`LUI`/`MOV` glue and takes register *values* (no register file). `Tamal.Branch` is a 4-way comparator returning only "taken?". Both are property-tested in isolation before the Engine assembles them. Neither is referenced by `topEntity` yet, so this ships as library + tests only.

**Tech Stack:** Clash 1.10 (`clash-prelude`), GHC, Stack; tasty + tasty-hunit + tasty-hedgehog + hedgehog + clash-prelude-hedgehog. Source of truth: `docs/superpowers/specs/2026-07-01-tamal-alu-branch-design.md`.

---

## File Structure

**New files**

| File | Responsibility |
|------|----------------|
| `hdl/src/Tamal/Branch.hs` | `BranchOp` enum + `branchTaken` comparator (CTRL group). |
| `hdl/src/Tamal/Alu.hs` | `AluOp` enum + `alu` core + `dataResult` wrapper (DATA group). |
| `hdl/tests/Test/Branch.hs` | `tests :: TestTree` — hedgehog + hunit for `branchTaken`. |
| `hdl/tests/Test/Alu.hs` | `tests :: TestTree` — hedgehog + hunit for `alu` and `dataResult`. |

**Modified files**

| File | Change |
|------|--------|
| `hdl/src/Tamal/Isa.hs` | `decodeData` sub `0xc`: add `&& shOp /= 0b11` (line 227). |
| `hdl/tests/Test/Isa.hs` | Add `testCase`: reserved `SHIFT` op `0b11` → `Left ReservedFieldNonZero`. |
| `hdl/tamal.cabal` | `library exposed-modules += Tamal.Alu, Tamal.Branch`; `test-suite other-modules += Test.Alu, Test.Branch`. |
| `hdl/tests/unittests.hs` | Import + append `Test.Alu.tests`, `Test.Branch.tests` to the tree. |

**Verify-only (already updated when the ISA design was authored — Task 6 confirms, does not edit unless drift is found)**

- `docs/superpowers/specs/2026-07-01-tamal-isa-design.md` — line 86 already says `li` = `LUI` + `ADDI`; line 186 already says `LOAD_IMM: rd ← sext(imm[10:0])`; line 187 `LUI` "pairs with `ADDI`".

**No change**

- `hdl/tests/Test/Gen.hs` — `genWord :: Gen (BitVector 32)` (line 38) is reused directly as a 32-bit operand generator; `genShOp` (line 93) already excludes `0b11`. No new generator is needed.

### The one real gotcha: constructor name clash (read before Tasks 4–5)

`AluOp` has constructors `Add` and `Sub`. `Tamal.Isa.Instr` **also** has constructors `Add` and `Sub` (see `Isa.hs:57,59`). `dataResult` consumes an `Instr`, so `Tamal.Alu` must import `Tamal.Isa` — bringing both `Add`s into scope and making the bare name ambiguous.

**Resolution:** in `Tamal.Alu` and `Test.Alu`, import the ISA **qualified** as `Isa` and refer to instruction constructors as `Isa.Add`, `Isa.Addi`, `Isa.Mov`, … while `AluOp` constructors stay bare (`Add`, `Sub`, `And`, …). `Tamal.Branch` has the *same-named* constructors `Beq/Bne/Bltu/Bgeu` as the ISA, but it does **not** import `Tamal.Isa`, so there is no clash there.

---

## Task 1: Trap the reserved SHIFT op (`0b11`) at decode

**Files:**
- Test: `hdl/tests/Test/Isa.hs` (add one `testCase`)
- Modify: `hdl/src/Tamal/Isa.hs:227`

Spec §9. The `SHIFT` op field `0b11` is reserved and must trap; today `decodeData` accepts it. TDD: assert the trap (RED), then tighten the guard (GREEN). The trap word is built by re-using `encode` — `encode` never validates, so `encode (Shift 0 0 0b11 0)` produces exactly the malformed word.

- [ ] **Step 1: Write the failing test**

In `hdl/tests/Test/Isa.hs`, add a new `testCase` to the `testGroup "Isa"` list (put it right after the existing `reserved non-zero field traps` case at line 38–40):

```haskell
    , testCase "reserved SHIFT op (0b11) traps" $
        -- encode never validates, so this builds a SHIFT word whose op field
        -- is the reserved 0b11; the tightened decoder must reject it (spec §9).
        decode (encode (Shift 0 0 0b11 0)) @?= Left ReservedFieldNonZero
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `stack test --ta '-p "reserved SHIFT op"'`
Expected: FAIL — the assertion reports `Right (Shift 0 0 0b11 0)` where `Left ReservedFieldNonZero` was expected (the current decoder at `Isa.hs:227` only checks `z rs2 && shMid == 0`).

- [ ] **Step 3: Tighten the decode guard**

In `hdl/src/Tamal/Isa.hs`, edit the `0xc` arm of `decodeData` (line 227). `shOp` is already bound in the `where` clause (line 234), so only the guard changes:

```haskell
    0xc -> only (z rs2 && shMid == 0 && shOp /= 0b11) (Shift rd rs1 shOp shAmt)  -- imm=op[10:9]++rsv[8:5]++amt[4:0]; op 0b11 reserved
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `stack test --ta '-p "reserved SHIFT op"'`
Expected: PASS. Then run the whole ISA group to confirm no regression: `stack test --ta '-p "Isa"'` — all green (the "any 32-bit word: decode is canonical or traps" property still holds because such words now take the trap branch; `genDataInstr` never emits `0b11`, so the round-trip properties are unaffected).

- [ ] **Step 5: Commit**

```bash
git add hdl/src/Tamal/Isa.hs hdl/tests/Test/Isa.hs
git commit -m "fix(hdl): trap reserved SHIFT op (0b11) at decode"
```

---

## Task 2: `Tamal.Branch` scaffold + `Test.Branch` (RED)

**Files:**
- Create: `hdl/src/Tamal/Branch.hs` (stubbed core)
- Create: `hdl/tests/Test/Branch.hs`
- Modify: `hdl/tamal.cabal`, `hdl/tests/unittests.hs`

Assistant work. Stand up the module (with an `errorX` stub so the library compiles) and the full test module. Do **not** implement `branchTaken` — the human does that in Task 3. Do **not** commit at the end of this task; the Branch feature is committed as one unit in Task 3.

- [ ] **Step 1: Create `hdl/src/Tamal/Branch.hs` (stubbed)**

```haskell
-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0

{- |
CTRL-group branch comparator (ALU/branch design §8). Pure, combinational,
single-cycle. 'branchTaken' returns only "taken?"; the PC / branch-offset math
is the Engine's job. Unsigned compares (@Bltu@/@Bgeu@) use 'BitVector''s
unsigned 'Ord' — there are no signed branches in v1.
-}
module Tamal.Branch
  ( BranchOp (..)
  , branchTaken
  ) where

import Clash.Prelude

data BranchOp = Beq | Bne | Bltu | Bgeu
  deriving stock (Generic, Show, Eq, Enum, Bounded)
  deriving anyclass NFDataX

-- Implemented by the human in Task 3.
branchTaken :: BranchOp -> BitVector 32 -> BitVector 32 -> Bool
branchTaken = errorX "Tamal.Branch.branchTaken: unimplemented (Task 3)"
```

- [ ] **Step 2: Wire the library module in `hdl/tamal.cabal`**

In the `library` `exposed-modules` list (currently ends at `Tamal.Trace`, line 103), add `Tamal.Branch`:

```
  exposed-modules:
    Tamal.Domain
    Tamal
    Tamal.Crc
    Tamal.Isa
    Tamal.Config
    Tamal.Bus.Serdes
    Tamal.Trace
    Tamal.Branch
```

- [ ] **Step 3: Confirm the library builds with the stub**

Run: `stack build`
Expected: PASS (the `errorX` stub type-checks; `-Wall` is on but not `-Werror`, so warnings do not fail the build).

- [ ] **Step 4: Create `hdl/tests/Test/Branch.hs`**

Note: literals use **no** numeric underscores — `NumericUnderscores` is not in the cabal `default-extensions`, so `0x8000_0000` would be a parse error. Write `0x80000000`.

```haskell
-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0

module Test.Branch (tests) where

import Clash.Prelude
import Test.Tasty
import Test.Tasty.HUnit
import Test.Tasty.Hedgehog (testProperty)
import Hedgehog (property, forAll, (===))
import qualified Hedgehog.Gen as Gen

import Tamal.Branch
import Test.Gen (genWord)

-- Independent reference for the four comparisons.
ref :: BranchOp -> BitVector 32 -> BitVector 32 -> Bool
ref Beq  a b = a == b
ref Bne  a b = a /= b
ref Bltu a b = a <  b
ref Bgeu a b = a >= b

tests :: TestTree
tests =
  testGroup "Branch"
    [ testProperty "branchTaken matches reference (all ops)" $ property $ do
        op <- forAll (Gen.element [minBound .. maxBound])
        a  <- forAll genWord
        b  <- forAll genWord
        branchTaken op a b === ref op a b
    , testProperty "Beq / Bne are complementary" $ property $ do
        a <- forAll genWord
        b <- forAll genWord
        branchTaken Beq a b === not (branchTaken Bne a b)
    , testProperty "Bltu / Bgeu are complementary" $ property $ do
        a <- forAll genWord
        b <- forAll genWord
        branchTaken Bltu a b === not (branchTaken Bgeu a b)
    , testProperty "Beq is reflexive" $ property $ do
        a <- forAll genWord
        branchTaken Beq a a === True
    , testCase "Bltu is unsigned: 0x7FFFFFFF < 0x80000000" $
        branchTaken Bltu 0x7FFFFFFF 0x80000000 @?= True
    , testCase "Bgeu is unsigned: 0xFFFFFFFF >= 0x00000000" $
        branchTaken Bgeu 0xFFFFFFFF 0x00000000 @?= True
    , testCase "Bltu is unsigned: 0x80000000 < 0x7FFFFFFF is False" $
        branchTaken Bltu 0x80000000 0x7FFFFFFF @?= False
    ]
```

- [ ] **Step 5: Wire the test module in `hdl/tamal.cabal` and `hdl/tests/unittests.hs`**

In `hdl/tamal.cabal`, add `Test.Branch` to the `test-suite test-library` `other-modules` list (currently ends at `Test.Trace`, line 144):

```
  other-modules:
    Test.Gen
    Test.Crc
    Test.Isa
    Test.Config
    Test.Serdes
    Test.Trace
    Test.Branch
```

In `hdl/tests/unittests.hs`, add the import (after `import qualified Test.Trace`, line 17):

```haskell
import qualified Test.Branch
```

and append `Test.Branch.tests` to the `testGroup "tamal"` list (after `Test.Trace.tests`, line 34):

```haskell
    , Test.Trace.tests
    , Test.Branch.tests
    ]
```

- [ ] **Step 6: Run the suite to confirm Branch is RED**

Run: `stack test --ta '-p "Branch"'`
Expected: FAIL — every Branch case errors with the `errorX` message `Tamal.Branch.branchTaken: unimplemented (Task 3)` (hedgehog forces the result and catches the exception). **Do not commit.** Proceed to Task 3.

---

## Task 3 — HUMAN: implement `branchTaken`

**Files:**
- Modify: `hdl/src/Tamal/Branch.hs` (replace the stub body)

This is yours to write. Below is the walkthrough of the Clash idioms, then the target body to type in, then the run + commit.

### Walkthrough (why each line is what it is)

- **`case op of` with four arms** — `BranchOp` is a plain 4-constructor enum; a total `case` synthesizes to a tiny mux. Clash requires totality (all four constructors), which the derived `Bounded`/`Enum` and the test's `[minBound .. maxBound]` also rely on.
- **`a == b` / `a /= b`** — `BitVector 32` has `Eq`; these are a 32-bit equality comparator and its negation.
- **`a < b` / `a >= b`** — the key decision (spec §3 #6, §8): `BitVector`'s `Ord` is **unsigned**. So `<` is exactly `BLTU` and `>=` is exactly `BGEU`. There is **no** sign interpretation here — that is what the `0x80000000` boundary test locks in. If you ever reached for `Signed 32` to compare, you would have accidentally implemented a *signed* branch, which v1 does not have.
- **Returns `Bool`, not a PC** — the comparator says only "taken?". The Engine owns `PC += signExtend off` when taken. Keeping this function value-only is what makes it a pure, testable leaf.

- [ ] **Step 1: Replace the stub with the real comparator**

In `hdl/src/Tamal/Branch.hs`, delete the `errorX` stub line and its comment, and write:

```haskell
branchTaken :: BranchOp -> BitVector 32 -> BitVector 32 -> Bool
branchTaken op a b = case op of
  Beq  -> a == b
  Bne  -> a /= b
  Bltu -> a <  b   -- unsigned: BitVector's Ord
  Bgeu -> a >= b   -- unsigned
```

- [ ] **Step 2: Run the Branch suite to verify GREEN**

Run: `stack test --ta '-p "Branch"'`
Expected: PASS — all four properties and three concrete cases green.

- [ ] **Step 3: Confirm nothing else regressed**

Run: `stack test`
Expected: PASS — the whole suite (smoke, Crc, Isa, Config, Serdes, Trace, Branch) green.

- [ ] **Step 4: Commit the Branch feature (scaffold + tests + impl together)**

```bash
git add hdl/src/Tamal/Branch.hs hdl/tests/Test/Branch.hs hdl/tamal.cabal hdl/tests/unittests.hs
git commit -m "feat(hdl): branch comparator (CTRL group) with hedgehog tests"
```

---

## Task 4: `Tamal.Alu` scaffold + `dataResult` + `Test.Alu` (RED)

**Files:**
- Create: `hdl/src/Tamal/Alu.hs` (real `dataResult`, stubbed `alu`)
- Create: `hdl/tests/Test/Alu.hs`
- Modify: `hdl/tamal.cabal`, `hdl/tests/unittests.hs`

Assistant work. `dataResult` (the wrapper glue) is written in full here — it compiles against the `alu` **stub** because the stub has the right type. The human implements the `alu` core in Task 5. Do **not** commit at the end of this task.

Remember the name-clash resolution: import the ISA **qualified as `Isa`** so `AluOp`'s bare `Add`/`Sub` do not collide with `Isa.Add`/`Isa.Sub`.

- [ ] **Step 1: Create `hdl/src/Tamal/Alu.hs` (real `dataResult`, stubbed `alu`)**

```haskell
-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0

{- |
DATA-group compute layer (ALU/branch design §5–7). Two layers:

  * 'alu' — a thin, total, @Op@-dispatched arithmetic/logic/shift core over
    register *values*. Shift amount is the low 5 bits of operand B.
  * 'dataResult' — the wrapper that resolves operand B (register value or
    sign-extended immediate), places 'LUI'/'MOV'/'LOAD_IMM' constants, and
    dispatches to 'alu'. It takes register VALUES, so it needs no register
    file; x0-hardwiring and writeback masking are the Engine's job.

The ISA is imported qualified as @Isa@ because 'AluOp'\'s @Add@/@Sub@ would
otherwise clash with 'Tamal.Isa.Instr'\'s @Add@/@Sub@ constructors.
-}
module Tamal.Alu
  ( AluOp (..)
  , alu
  , dataResult
  ) where

import Clash.Prelude
import Tamal.Isa (Instr)
import qualified Tamal.Isa as Isa

data AluOp = Add | Sub | And | Or | Xor | Sll | Srl | Sra
  deriving stock (Generic, Show, Eq, Enum, Bounded)
  deriving anyclass NFDataX

-- Thin, Op-dispatched core. Implemented by the human in Task 5.
alu :: AluOp -> BitVector 32 -> BitVector 32 -> BitVector 32
alu = errorX "Tamal.Alu.alu: unimplemented (Task 5)"

-- Complete DATA-group value semantics over register VALUES (rs1v, rs2v).
-- Total over Instr; non-DATA-compute constructors hit the documented default.
dataResult :: Instr -> BitVector 32 -> BitVector 32 -> BitVector 32
dataResult instr rs1v rs2v = case instr of
  Isa.LoadImm _ imm       -> signExtend imm
  Isa.Lui     _ imm20     -> (zeroExtend imm20 :: BitVector 32) `shiftL` 12
  Isa.Mov     _ _         -> rs1v
  Isa.Add     _ _ _       -> alu Add rs1v rs2v
  Isa.Addi    _ _ imm     -> alu Add rs1v (signExtend imm)
  Isa.Sub     _ _ _       -> alu Sub rs1v rs2v
  Isa.And_    _ _ _       -> alu And rs1v rs2v
  Isa.Andi    _ _ imm     -> alu And rs1v (signExtend imm)
  Isa.Or_     _ _ _       -> alu Or  rs1v rs2v
  Isa.Ori     _ _ imm     -> alu Or  rs1v (signExtend imm)
  Isa.Xor_    _ _ _       -> alu Xor rs1v rs2v
  Isa.Xori    _ _ imm     -> alu Xor rs1v (signExtend imm)
  Isa.Shift   _ _ shOp amt -> alu (toAluShift shOp) rs1v (zeroExtend amt)
  _                       -> 0   -- BUS / CTRL / RDSR: never routed here by the Engine
  where
    toAluShift :: BitVector 2 -> AluOp
    toAluShift = \case
      0b00 -> Sll
      0b01 -> Srl
      _    -> Sra   -- 0b10; 0b11 is unreachable (decode traps it, Task 1)
```

- [ ] **Step 2: Wire the library module in `hdl/tamal.cabal`**

Add `Tamal.Alu` to the `library` `exposed-modules` list (after `Tamal.Branch` from Task 2):

```
    Tamal.Trace
    Tamal.Branch
    Tamal.Alu
```

- [ ] **Step 3: Confirm the library builds with the `alu` stub**

Run: `stack build`
Expected: PASS. `dataResult` is fully written but compiles against the `alu` stub (the stub's type is correct). This also proves the qualified-import clash resolution works.

- [ ] **Step 4: Create `hdl/tests/Test/Alu.hs`**

Covers both layers (spec §11.1). No numeric underscores in literals. Immediates are generated with `genDefinedBitVector` (as `Test.Gen` does); 32-bit operands reuse `genWord`.

```haskell
-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0

module Test.Alu (tests) where

import Clash.Prelude
import Clash.Hedgehog.Sized.BitVector (genDefinedBitVector)
import Test.Tasty
import Test.Tasty.HUnit
import Test.Tasty.Hedgehog (testProperty)
import Hedgehog (Gen, property, forAll, (===))
import qualified Hedgehog.Gen as Gen

import Tamal.Alu
import qualified Tamal.Isa as Isa
import Test.Gen (genReg, genWord)

genImm :: Gen (BitVector 11)
genImm = genDefinedBitVector

genImm20 :: Gen (BitVector 20)
genImm20 = genDefinedBitVector

genAmt :: Gen (BitVector 5)
genAmt = genDefinedBitVector

-- Valid (non-reserved) shift ops, mirroring dataResult's toAluShift.
genShOp :: Gen (BitVector 2)
genShOp = Gen.element [0b00, 0b01, 0b10]

shiftOpToAlu :: BitVector 2 -> AluOp
shiftOpToAlu 0b00 = Sll
shiftOpToAlu 0b01 = Srl
shiftOpToAlu _    = Sra

tests :: TestTree
tests =
  testGroup "Alu"
    [ testGroup "alu core"
        [ testProperty "Add" $ property $ do
            a <- forAll genWord; b <- forAll genWord
            alu Add a b === a + b
        , testProperty "Sub" $ property $ do
            a <- forAll genWord; b <- forAll genWord
            alu Sub a b === a - b
        , testProperty "And" $ property $ do
            a <- forAll genWord; b <- forAll genWord
            alu And a b === a .&. b
        , testProperty "Or" $ property $ do
            a <- forAll genWord; b <- forAll genWord
            alu Or a b === a .|. b
        , testProperty "Xor" $ property $ do
            a <- forAll genWord; b <- forAll genWord
            alu Xor a b === a `xor` b
        , testProperty "Sub == Add of two's complement" $ property $ do
            a <- forAll genWord; b <- forAll genWord
            alu Sub a b === alu Add a (complement b + 1)
        , testProperty "Sll masks shift amount to low 5 bits" $ property $ do
            a <- forAll genWord; b <- forAll genWord
            alu Sll a b === alu Sll a (b .&. 0x1F)
        , testProperty "Srl masks shift amount to low 5 bits" $ property $ do
            a <- forAll genWord; b <- forAll genWord
            alu Srl a b === alu Srl a (b .&. 0x1F)
        , testProperty "Sra masks shift amount to low 5 bits" $ property $ do
            a <- forAll genWord; b <- forAll genWord
            alu Sra a b === alu Sra a (b .&. 0x1F)
        , testProperty "shift by 0 is identity (Sll/Srl/Sra)" $ property $ do
            a <- forAll genWord
            alu Sll a 0 === a
            alu Srl a 0 === a
            alu Sra a 0 === a
        , testProperty "Sra preserves the sign bit" $ property $ do
            a <- forAll genWord; b <- forAll genWord
            slice d31 d31 (alu Sra a b) === slice d31 d31 a
        , testCase "Sra 0x80000000 by 1 = 0xC0000000 (sign-fill)" $
            alu Sra 0x80000000 1 @?= 0xC0000000
        , testCase "Srl 0x80000000 by 1 = 0x40000000 (zero-fill)" $
            alu Srl 0x80000000 1 @?= 0x40000000
        ]
    , testGroup "dataResult wrapper"
        [ testProperty "Mov returns rs1v" $ property $ do
            rd <- forAll genReg; rs <- forAll genReg
            x <- forAll genWord; y <- forAll genWord
            dataResult (Isa.Mov rd rs) x y === x
        , testProperty "LoadImm sign-extends imm" $ property $ do
            rd <- forAll genReg; imm <- forAll genImm
            x <- forAll genWord; y <- forAll genWord
            dataResult (Isa.LoadImm rd imm) x y === signExtend imm
        , testProperty "Lui places imm20 at [31:12], low 12 zero" $ property $ do
            rd <- forAll genReg; i20 <- forAll genImm20
            x <- forAll genWord; y <- forAll genWord
            let r = dataResult (Isa.Lui rd i20) x y
            r === (zeroExtend i20 :: BitVector 32) `shiftL` 12
            (r .&. 0xFFF) === 0
        , testProperty "Addi = alu Add rs1v (signExtend imm)" $ property $ do
            rd <- forAll genReg; rs <- forAll genReg; imm <- forAll genImm
            x <- forAll genWord; y <- forAll genWord
            dataResult (Isa.Addi rd rs imm) x y === alu Add x (signExtend imm)
        , testProperty "Andi = alu And rs1v (signExtend imm)" $ property $ do
            rd <- forAll genReg; rs <- forAll genReg; imm <- forAll genImm
            x <- forAll genWord; y <- forAll genWord
            dataResult (Isa.Andi rd rs imm) x y === alu And x (signExtend imm)
        , testProperty "Ori = alu Or rs1v (signExtend imm)" $ property $ do
            rd <- forAll genReg; rs <- forAll genReg; imm <- forAll genImm
            x <- forAll genWord; y <- forAll genWord
            dataResult (Isa.Ori rd rs imm) x y === alu Or x (signExtend imm)
        , testProperty "Xori = alu Xor rs1v (signExtend imm)" $ property $ do
            rd <- forAll genReg; rs <- forAll genReg; imm <- forAll genImm
            x <- forAll genWord; y <- forAll genWord
            dataResult (Isa.Xori rd rs imm) x y === alu Xor x (signExtend imm)
        , testProperty "Add = alu Add rs1v rs2v" $ property $ do
            rd <- forAll genReg; a <- forAll genReg; b <- forAll genReg
            x <- forAll genWord; y <- forAll genWord
            dataResult (Isa.Add rd a b) x y === alu Add x y
        , testProperty "Sub = alu Sub rs1v rs2v" $ property $ do
            rd <- forAll genReg; a <- forAll genReg; b <- forAll genReg
            x <- forAll genWord; y <- forAll genWord
            dataResult (Isa.Sub rd a b) x y === alu Sub x y
        , testProperty "And_ = alu And rs1v rs2v" $ property $ do
            rd <- forAll genReg; a <- forAll genReg; b <- forAll genReg
            x <- forAll genWord; y <- forAll genWord
            dataResult (Isa.And_ rd a b) x y === alu And x y
        , testProperty "Or_ = alu Or rs1v rs2v" $ property $ do
            rd <- forAll genReg; a <- forAll genReg; b <- forAll genReg
            x <- forAll genWord; y <- forAll genWord
            dataResult (Isa.Or_ rd a b) x y === alu Or x y
        , testProperty "Xor_ = alu Xor rs1v rs2v" $ property $ do
            rd <- forAll genReg; a <- forAll genReg; b <- forAll genReg
            x <- forAll genWord; y <- forAll genWord
            dataResult (Isa.Xor_ rd a b) x y === alu Xor x y
        , testProperty "Shift = alu (toAluShift shOp) rs1v (zeroExtend amt)" $ property $ do
            rd <- forAll genReg; rs <- forAll genReg
            shOp <- forAll genShOp; amt <- forAll genAmt
            x <- forAll genWord; y <- forAll genWord
            dataResult (Isa.Shift rd rs shOp amt) x y
              === alu (shiftOpToAlu shOp) x (zeroExtend amt)
        ]
    ]
```

- [ ] **Step 5: Wire the test module in `hdl/tamal.cabal` and `hdl/tests/unittests.hs`**

In `hdl/tamal.cabal`, add `Test.Alu` to the `test-suite` `other-modules` list (after `Test.Branch`):

```
    Test.Trace
    Test.Branch
    Test.Alu
```

In `hdl/tests/unittests.hs`, add the import:

```haskell
import qualified Test.Alu
```

and append `Test.Alu.tests` to the tree (after `Test.Branch.tests`):

```haskell
    , Test.Branch.tests
    , Test.Alu.tests
    ]
```

- [ ] **Step 6: Run the suite to confirm Alu is RED**

Run: `stack test --ta '-p "Alu"'`
Expected: FAIL — the `alu core` group and every `dataResult` case that routes through `alu` error with `Tamal.Alu.alu: unimplemented (Task 5)`. (`Mov`, `LoadImm`, and `Lui` bypass `alu`, so those three `dataResult` properties pass even now — that is expected; the group as a whole is RED.) **Do not commit.** Proceed to Task 5.

---

## Task 5 — HUMAN: implement the `alu` core

**Files:**
- Modify: `hdl/src/Tamal/Alu.hs` (replace the `alu` stub body)

This is yours to write — the arithmetic/logic/shift heart. Walkthrough first, then the target body, then run + commit.

### Walkthrough (the Clash idioms that matter)

- **`case op of` over 8 arms** — `AluOp` has 8 constructors and no reserved case (the reserved shift `0b11` was trapped at *decode* in Task 1). So `alu` is **total** by construction — the Lion idiom "decode → `Either`, `alu` total". A total `case` on an enum synthesizes to a mux; no default arm needed.
- **`a + b`, `a - b`** — `BitVector 32` has a `Num` instance; `+`/`-` wrap mod 2³² (two's-complement wrap). No overflow handling — wrapping is the defined semantics (the `Sub == Add of complement+1` property checks this).
- **`.&.`, `.|.`, `` `xor` ``** — from the `Bits` instance. Note lowercase `xor` is the `Bits` method; do not confuse it with the `AluOp` constructor `Xor` (capital).
- **Shift amount = low 5 bits of B.** RISC-V-style masking so a shift by ≥ 32 is well-defined, never undefined. You compute it once in a `where` binding:
  - `truncateB b :: BitVector 5` keeps the **least-significant** 5 bits (the type annotation on the `unpack` below forces `truncateB`'s result width to 5).
  - `unpack (...) :: Unsigned 5` — convert those 5 bits to a number. Use `Unsigned 5`, **not** `BitVector 5`: `Unsigned` has `Integral`, so `fromIntegral` to `Int` works. `BitVector` is **not** `Integral` — never `fromIntegral` a `BitVector` (spec §5).
  - `fromIntegral (...) :: Int` — the `Int` shift count that `shiftL`/`shiftR` want.
- **`shiftL` — the same for all shift kinds** (zero enters at the bottom). `Sll -> a `shiftL` sh`.
- **`shiftR` on `BitVector` is LOGICAL (zero-fill).** So `Srl -> a `shiftR` sh` is the logical right shift directly.
- **Arithmetic right shift must detour through `Signed 32`.** `BitVector`'s `shiftR` will not sign-fill. So: `unpack a :: Signed 32` (reinterpret the bits as signed), `shiftR` (now arithmetic — sign-fills), then `pack` back to `BitVector 32`. That is `Sra -> pack (shiftR (unpack a :: Signed 32) sh)`. The `Sra preserves the sign bit` property and the `0x80000000 -> 0xC0000000` vector lock this in.

- [ ] **Step 1: Replace the `alu` stub with the real core**

In `hdl/src/Tamal/Alu.hs`, delete the `errorX` stub line and its comment, and write:

```haskell
alu :: AluOp -> BitVector 32 -> BitVector 32 -> BitVector 32
alu op a b = case op of
  Add -> a + b
  Sub -> a - b
  And -> a .&. b
  Or  -> a .|. b
  Xor -> a `xor` b
  Sll -> a `shiftL` sh
  Srl -> a `shiftR` sh                              -- logical: zero fill
  Sra -> pack (shiftR (unpack a :: Signed 32) sh)   -- arithmetic: sign fill
  where
    -- shift amount = low 5 bits of b, as a 0..31 Int
    sh :: Int
    sh = fromIntegral (unpack (truncateB b) :: Unsigned 5)
```

- [ ] **Step 2: Run the Alu suite to verify GREEN**

Run: `stack test --ta '-p "Alu"'`
Expected: PASS — both `alu core` and `dataResult wrapper` groups green.

- [ ] **Step 3: Confirm the whole suite is green**

Run: `stack test`
Expected: PASS — smoke, Crc, Isa, Config, Serdes, Trace, Branch, Alu all green.

- [ ] **Step 4: Commit the ALU feature (scaffold + wrapper + tests + core together)**

```bash
git add hdl/src/Tamal/Alu.hs hdl/tests/Test/Alu.hs hdl/tamal.cabal hdl/tests/unittests.hs
git commit -m "feat(hdl): DATA-group ALU (alu core + dataResult) with hedgehog tests"
```

---

## Task 6: Doc verification + full-suite / codegen verification

**Files:**
- Verify (edit only if drift): `docs/superpowers/specs/2026-07-01-tamal-isa-design.md`

Assistant work. Confirm the ISA design doc already reflects the spec §7.4 consequence, then run the full verification battery from spec §13.

- [ ] **Step 1: Verify the ISA design doc already says `li = LUI + ADDI` and `sext(imm)`**

Confirm these three points in `docs/superpowers/specs/2026-07-01-tamal-isa-design.md` (they were written this way when the ISA design was authored, so no edit is expected):
- line ~86: `li` pseudo-op "expanding to `LUI` + `ADDI`".
- line ~186: `LOAD_IMM` — `rd ← sext(imm[10:0])`.
- line ~187: `LUI` — "pairs with `ADDI` for 32-bit consts".

If any of these still says `ORI` or a bare `ext(imm)`, fix it to match (`li` = `LUI` + `ADDI`; `LOAD_IMM` = `sext(imm)`) and stage the doc. Otherwise, no edit.

- [ ] **Step 2: Full build**

Run: `stack build`
Expected: PASS (cold Clash/GHC builds are slow — expected; caching is load-bearing).

- [ ] **Step 3: Full test suite**

Run: `stack test`
Expected: PASS — all groups green, including the new `Branch` and `Alu` groups and the tightened `Isa` decode case.

- [ ] **Step 4: Clash codegen smoke**

Run: `stack run clash -- Tamal --verilog`
Expected: PASS — confirms the two new pure modules are Clash-clean even though `topEntity` does not reference them yet (this exercises library compilation under Clash, not new gateware).

- [ ] **Step 5: Commit the doc fix if Step 1 required one**

Only if Step 1 found drift:

```bash
git add docs/superpowers/specs/2026-07-01-tamal-isa-design.md
git commit -m "docs(spec): li = LUI + ADDI; LOAD_IMM sext(imm)"
```

If Step 1 required no edit, skip this commit.

---

## Done criteria

- `Tamal.Alu` (`AluOp`, `alu`, `dataResult`) and `Tamal.Branch` (`BranchOp`, `branchTaken`) exist under `hdl/src/Tamal/`, each with the SPDX/REUSE header.
- `decodeData` rejects the reserved `SHIFT` op `0b11` with `Left ReservedFieldNonZero`.
- `Test.Alu`, `Test.Branch`, and the new `Test.Isa` case pass under `stack test`; the full suite is green.
- `stack run clash -- Tamal --verilog` succeeds (codegen smoke).
- Commits: (1) decode trap, (2) Branch feature, (3) ALU feature, (+ optional doc fix).
- Out of scope, unchanged: register file, `Engine.step`, `RDSR`, BUS-op execution, signed branches, `li` constant-tiling (spec §2, §14).
