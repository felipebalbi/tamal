# Tamal ISA Pure Core (Plan A1) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the property-tested pure cores of the tamal SPI-engine ISA — instruction encode/decode, RX CRC-8, config decode, x1 byte serdes + TAR, and result-ring record encoding — plus wire hedgehog into the HDL test suite and stand up HDL+Rust CI.

**Architecture:** Each core is a small, total, synthesizable-where-relevant Haskell/Clash function in its own module under `hdl/src/Tamal/`, with a matching hedgehog property-test module under `hdl/tests/Test/`. No engine FSM, no top-entity/IOBUF/UART, no Vivado — those are later plans (A2 = `Engine.step`; B = top-entity shell). This plan produces a verified library that `stack test` and `cargo test` fully exercise.

**Tech Stack:** Clash 1.10 (Haskell/GHC via stack, LTS 24.38), hedgehog 1.5 + tasty-hedgehog 1.4.0.2 + clash-prelude-hedgehog 1.10.0 (already pinned in `hdl/stack.yaml`), tasty 1.5.4; GitHub Actions for CI (haskell-actions/setup + dtolnay/rust-toolchain).

**Reference spec:** `docs/superpowers/specs/2026-07-01-tamal-isa-design.md` (§4 instruction word, §5 BUS, §6 CTRL/DATA, §7 config/CRC, §8 result-ring).

**Conventions:**
- Haskell source uses **spaces** (match existing `hdl/src/Tamal/Domain.hs` style).
- Every new library module is added to `exposed-modules` in `hdl/tamal.cabal`.
- Every new test module is added to `other-modules` in the `test-library` stanza and aggregated in `hdl/tests/unittests.hs`.
- All commands run from `hdl/` unless stated. `stack test` runs the suite; `stack build` compiles the library.
- ADTs derive `(Generic, Show, Eq)` via `stock` and `NFDataX` via `anyclass` (Clash idiom, per Lion).

---

## File Structure

Created by this plan:

```
hdl/src/Tamal/
  Isa.hs          instruction ADT + encode/decode + DecodeError            [pure, synth]
  Crc.hs          crc8Update (RX CRC-8, poly 0x07, init 0x00, MSB-first)   [pure, synth]
  Config.hs       Config type + decodeConfig (v1-live validation)          [pure, synth]
  Bus/Serdes.hs   x1 byte serialize/deserialize + TAR beat + Lanes         [pure, synth]
  Trace.hs        Record type + encodeRecord + ringPush model              [pure]
hdl/tests/
  unittests.hs    tasty aggregator (modified)
  Test/Crc.hs
  Test/Isa.hs
  Test/Config.hs
  Test/Serdes.hs
  Test/Trace.hs
  Test/Gen.hs     shared hedgehog generators
.github/workflows/ci.yml   HDL + Rust CI
```

Modified:
- `hdl/tamal.cabal` — add modules + hedgehog deps.
- `hdl/tests/unittests.hs` — aggregate the new test groups.

---

## Task 1: Wire hedgehog into the test suite

**Files:**
- Modify: `hdl/tamal.cabal` (test-suite `test-library` stanza)
- Modify: `hdl/tests/unittests.hs`
- Create: `hdl/tests/Test/Gen.hs`

- [ ] **Step 1: Add hedgehog deps + test module list to the cabal test-suite**

In `hdl/tamal.cabal`, replace the `test-suite test-library` stanza's `build-depends` block and add an `other-modules` field. The stanza becomes:

```cabal
test-suite test-library
  import: common-options
  default-language: Haskell2010
  hs-source-dirs: tests
  type: exitcode-stdio-1.0
  ghc-options: -threaded
  main-is: unittests.hs
  other-modules:
    Test.Gen
  build-depends:
    tamal,
    tasty >= 1.2 && < 1.6,
    tasty-hunit,
    tasty-hedgehog,
    hedgehog,
    clash-prelude-hedgehog
```

- [ ] **Step 2: Create the shared generators module with one smoke generator**

Create `hdl/tests/Test/Gen.hs`:

```haskell
module Test.Gen
  ( genBit
  , genByte
  ) where

import Clash.Prelude
import Clash.Hedgehog.Sized.BitVector (genDefinedBitVector)
import Hedgehog (Gen)
import qualified Hedgehog.Gen as Gen

-- | A single defined Bit (0 or 1).
genBit :: Gen Bit
genBit = Gen.element [0, 1]

-- | A defined 8-bit value.
genByte :: Gen (BitVector 8)
genByte = genDefinedBitVector
```

- [ ] **Step 3: Rewrite the aggregator with a hedgehog smoke property**

Replace the entire contents of `hdl/tests/unittests.hs`:

```haskell
module Main (main) where

import Prelude
import Clash.Prelude (pack, unpack, BitVector)
import Test.Tasty
import Test.Tasty.Hedgehog (testProperty)
import Hedgehog (property, forAll, (===))

import Test.Gen (genByte)

main :: IO ()
main = defaultMain tests

tests :: TestTree
tests =
  testGroup "tamal"
    [ testGroup "smoke"
        [ testProperty "pack/unpack byte round-trips" $ property $ do
            b <- forAll genByte
            unpack (pack b) === (b :: BitVector 8)
        ]
    ]
```

- [ ] **Step 4: Run the suite to verify hedgehog is wired**

Run: `stack test`
Expected: builds, then `tamal > smoke > pack/unpack byte round-trips: OK` and the suite passes. (First run triggers a cold GHC/Clash build — this is slow but one-time.)

- [ ] **Step 5: Commit**

```bash
git add hdl/tamal.cabal hdl/tests/unittests.hs hdl/tests/Test/Gen.hs
git commit -m "test(hdl): wire hedgehog + tasty-hedgehog into the suite"
```

---

## Task 2: RX CRC-8 core (`Tamal.Crc`)

CRC-8, poly `0x07`, init `0x00`, MSB-first, no reflection/xorout (spec §7.4). Known-answer vectors: `crc8Update 0 0x01 == 0x07`; CRC-8/SMBUS check over ASCII `"123456789"` == `0xF4`; residue law `foldl crc8Update 0 (msg ++ [foldl crc8Update 0 msg]) == 0`.

**Files:**
- Create: `hdl/src/Tamal/Crc.hs`
- Modify: `hdl/tamal.cabal` (add `Tamal.Crc` to `exposed-modules`)
- Create: `hdl/tests/Test/Crc.hs`
- Modify: `hdl/tests/unittests.hs` (aggregate), `hdl/tamal.cabal` (add `Test.Crc` to `other-modules`)

- [ ] **Step 1: Write the failing test**

Create `hdl/tests/Test/Crc.hs`:

```haskell
module Test.Crc (tests) where

import Clash.Prelude
import qualified Data.List as L
import Test.Tasty
import Test.Tasty.HUnit
import Test.Tasty.Hedgehog (testProperty)
import Hedgehog (property, forAll, (===))
import qualified Hedgehog.Gen as Gen
import qualified Hedgehog.Range as Range

import Tamal.Crc (crc8Update)
import Test.Gen (genByte)

-- | Fold the CRC over a whole message (test-side helper).
crc8 :: [BitVector 8] -> BitVector 8
crc8 = L.foldl' crc8Update 0

tests :: TestTree
tests =
  testGroup "Crc"
    [ testCase "crc8Update 0 0x01 == 0x07" $
        crc8Update 0 0x01 @?= 0x07
    , testCase "CRC-8/SMBUS check \"123456789\" == 0xF4" $
        crc8 [ fromIntegral (fromEnum c) | c <- "123456789" ] @?= 0xF4
    , testProperty "residue law: crc8 (msg <> [crc8 msg]) == 0" $ property $ do
        msg <- forAll (Gen.list (Range.linear 0 32) genByte)
        crc8 (msg <> [crc8 msg]) === 0
    ]
```

Note: `Clash.Prelude` re-exports `map`/`(++)` as the `Vec` versions, so the
list glue uses a list comprehension and `(<>)` (list `Semigroup`) rather than
`map`/`(++)`.

- [ ] **Step 2: Aggregate the group and run to verify it fails**

In `hdl/tests/unittests.hs` add `import qualified Test.Crc` and put `Test.Crc.tests` in the top-level list:

```haskell
import qualified Test.Crc
```
```haskell
tests =
  testGroup "tamal"
    [ testGroup "smoke"
        [ testProperty "pack/unpack byte round-trips" $ property $ do
            b <- forAll genByte
            unpack (pack b) === (b :: BitVector 8)
        ]
    , Test.Crc.tests
    ]
```

Add `Test.Crc` to `other-modules` in `hdl/tamal.cabal`:

```cabal
  other-modules:
    Test.Gen
    Test.Crc
```

Run: `stack test`
Expected: FAIL to compile — `Tamal.Crc` / `crc8Update` not found.

- [ ] **Step 3: Implement `Tamal.Crc`**

Create `hdl/src/Tamal/Crc.hs`:

```haskell
{- |
RX CRC-8 primitive used by the tamal engine's reception path.

Parameters (eSPI / SMBus PEC): polynomial @0x07@ (x^8 + x^2 + x + 1),
initial value @0x00@, most-significant-bit first, no input/output
reflection, no final XOR. The residue of a correct message followed by
its CRC byte is @0x00@.
-}
module Tamal.Crc
  ( crc8Update
  ) where

import Clash.Prelude

-- | Fold one byte into the running CRC-8, processing bit 7 down to bit 0.
crc8Update :: BitVector 8 -> BitVector 8 -> BitVector 8
crc8Update crc0 byte = foldl step crc0 (unpack byte :: Vec 8 Bit)
  where
    step :: BitVector 8 -> Bit -> BitVector 8
    step c inBit =
      let feedback = msb c `xor` inBit
          shifted  = c `shiftL` 1
      in if feedback == 1 then shifted `xor` 0x07 else shifted
```

Add `Tamal.Crc` to the library `exposed-modules` in `hdl/tamal.cabal`:

```cabal
  exposed-modules:
    Tamal.Domain
    Tamal
    Tamal.Crc
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `stack test`
Expected: PASS — `Crc` group green (both known-answer cases and the residue property).

- [ ] **Step 5: Commit**

```bash
git add hdl/src/Tamal/Crc.hs hdl/tests/Test/Crc.hs hdl/tests/unittests.hs hdl/tamal.cabal
git commit -m "feat(hdl): RX CRC-8 core with known-answer + residue tests"
```

---

## Task 3: Instruction word framework + BUS-group encode/decode (`Tamal.Isa`)

Instruction word (spec §4): `group[31:30] sub[29:26] rd[25:21] rs1[20:16] rs2[15:11] imm[10:0]` (2+4+5+5+5+11 = 32). This task defines the full `Instr` ADT (all Phase-1 opcodes), the `bitCoerce`-based field split/join, and `encode`/`decode` for the **BUS group**. CTRL/DATA decoding returns `Left OpcodeUnimplemented` until Tasks 4–5.

**Files:**
- Create: `hdl/src/Tamal/Isa.hs`
- Modify: `hdl/tamal.cabal` (`exposed-modules` += `Tamal.Isa`; `other-modules` += `Test.Isa`)
- Create: `hdl/tests/Test/Isa.hs`
- Modify: `hdl/tests/unittests.hs`

- [ ] **Step 1: Write the failing test (BUS round-trip + reserved-field trap)**

Create `hdl/tests/Test/Isa.hs`:

```haskell
module Test.Isa (tests) where

import Clash.Prelude
import Test.Tasty
import Test.Tasty.HUnit
import Test.Tasty.Hedgehog (testProperty)
import Hedgehog (property, forAll, (===))

import Tamal.Isa
import Test.Gen (genBusInstr)

tests :: TestTree
tests =
  testGroup "Isa"
    [ testProperty "BUS: decode . encode == Right" $ property $ do
        i <- forAll genBusInstr
        decode (encode i) === Right i
    , testProperty "canonical: decode w == Right i ==> encode i == w" $ property $ do
        i <- forAll genBusInstr
        let w = encode i
        decode w === Right i
        (encode <$> decode w) === Right w
    , testCase "reserved non-zero field traps (CS_ASSERT with junk imm)" $
        -- CS_ASSERT = group 00, sub 0x0, all operand bits reserved; set imm bit 0.
        decode (0b00 `busWord` 0x0 + 1) @?= Left ReservedFieldNonZero
    ]
  where
    -- helper: build a BUS word from (subOpcode) with all-zero operands
    busWord :: BitVector 2 -> BitVector 4 -> BitVector 32
    busWord g s = bitCoerce (g, s, 0 :: BitVector 5, 0 :: BitVector 5, 0 :: BitVector 5, 0 :: BitVector 11)
```

- [ ] **Step 2: Replace `Test.Gen` with the complete generator module**

Rewrite `hdl/tests/Test/Gen.hs` in full. This defines **all** group generators up front (Tasks 4–5 reuse `genCtrlInstr`/`genDataInstr`/`genInstr` without editing this file). The `Index 8` count is generated via `unpack` of a 3-bit vector to avoid depending on the `genIndex` API surface.

```haskell
module Test.Gen
  ( genBit
  , genByte
  , genReg
  , genIndex8
  , genBusInstr
  , genCtrlInstr
  , genDataInstr
  , genInstr
  ) where

import Clash.Prelude
import Clash.Hedgehog.Sized.BitVector (genDefinedBitVector)
import Hedgehog (Gen)
import qualified Hedgehog.Gen as Gen

import Tamal.Isa

genBit :: Gen Bit
genBit = Gen.element [0, 1]

genByte :: Gen (BitVector 8)
genByte = genDefinedBitVector

genReg :: Gen (BitVector 5)
genReg = genDefinedBitVector

-- | Count minus one (0..7 -> 1..8 bits) for PUT_BITS/GET_BITS.
genIndex8 :: Gen (Index 8)
genIndex8 = unpack <$> (genDefinedBitVector :: Gen (BitVector 3))

genBusInstr :: Gen Instr
genBusInstr = Gen.choice
  [ pure CsAssert
  , pure CsDeassert
  , PutByteImm <$> genByte
  , PutByteReg <$> genReg
  , GetByte    <$> genReg
  , PutBitsImm <$> genIndex8 <*> genByte
  , PutBitsReg <$> genReg <*> genIndex8
  , GetBits    <$> genReg <*> genIndex8
  , TarImm     <$> (genDefinedBitVector :: Gen (BitVector 4))
  , TarReg     <$> genReg
  , pure RstAssert
  , pure RstDeassert
  , GetAlert   <$> genReg
  ]

genCtrlInstr :: Gen Instr
genCtrlInstr = Gen.choice
  [ Halt      <$> genByte
  , Beq       <$> genReg <*> genReg <*> genOff
  , Bne       <$> genReg <*> genReg <*> genOff
  , Bltu      <$> genReg <*> genReg <*> genOff
  , Bgeu      <$> genReg <*> genReg <*> genOff
  , WaitOn    <$> genReg
              <*> (genDefinedBitVector :: Gen (BitVector 2))
              <*> (genDefinedBitVector :: Gen (BitVector 9))
  , SetConfig <$> (genDefinedBitVector :: Gen (BitVector 6))
  , Mark      <$> genOff <*> genReg
  , pure CrcReset
  ]
  where genOff = genDefinedBitVector :: Gen (BitVector 11)

genDataInstr :: Gen Instr
genDataInstr = Gen.choice
  [ LoadImm <$> genReg <*> genI
  , Lui     <$> genReg <*> (genDefinedBitVector :: Gen (BitVector 20))
  , Mov     <$> genReg <*> genReg
  , Add     <$> genReg <*> genReg <*> genReg
  , Addi    <$> genReg <*> genReg <*> genI
  , Sub     <$> genReg <*> genReg <*> genReg
  , And_    <$> genReg <*> genReg <*> genReg
  , Andi    <$> genReg <*> genReg <*> genI
  , Or_     <$> genReg <*> genReg <*> genReg
  , Ori     <$> genReg <*> genReg <*> genI
  , Xor_    <$> genReg <*> genReg <*> genReg
  , Xori    <$> genReg <*> genReg <*> genI
  , Shift   <$> genReg <*> genReg <*> genShOp <*> (genDefinedBitVector :: Gen (BitVector 5))
  , Rdsr    <$> genReg <*> (genDefinedBitVector :: Gen (BitVector 5))
  ]
  where
    genI    = genDefinedBitVector :: Gen (BitVector 11)
    genShOp = Gen.element [0b00, 0b01, 0b10] :: Gen (BitVector 2)

genInstr :: Gen Instr
genInstr = Gen.choice [genBusInstr, genCtrlInstr, genDataInstr]
```

- [ ] **Step 3: Aggregate + run to verify it fails**

In `hdl/tests/unittests.hs` add `import qualified Test.Isa` and append `Test.Isa.tests` to the list. Add `Test.Isa` to `other-modules` in `hdl/tamal.cabal`.

Run: `stack test`
Expected: FAIL to compile — `Tamal.Isa` not found.

- [ ] **Step 4: Implement `Tamal.Isa` (framework + BUS group)**

Create `hdl/src/Tamal/Isa.hs`:

```haskell
{- |
Tamal instruction encoding (spec §4–6). 32-bit fixed-width words:

@
 31 30 | 29 .. 26 | 25 .. 21 | 20 .. 16 | 15 .. 11 | 10 .. 0
 group |   sub    |    rd    |   rs1    |   rs2    |  imm
@

This module owns the @Instr@ ADT and total @encode@ / @decode@. Fields
unused by an opcode are reserved-must-be-zero; a non-zero reserved field
decodes to @Left ReservedFieldNonZero@.
-}
module Tamal.Isa
  ( Instr (..)
  , DecodeError (..)
  , Reg
  , encode
  , decode
  ) where

import Clash.Prelude

type Reg = BitVector 5

data Instr
  -- BUS group (group 00)
  = CsAssert
  | CsDeassert
  | PutByteImm (BitVector 8)
  | PutByteReg Reg
  | GetByte    Reg
  | PutBitsImm (Index 8) (BitVector 8)   -- n-1, bits (n = count in 1..8)
  | PutBitsReg Reg (Index 8)
  | GetBits    Reg (Index 8)
  | TarImm     (BitVector 4)
  | TarReg     Reg
  | RstAssert
  | RstDeassert
  | GetAlert   Reg
  -- CTRL group (group 01) — decoding lands in Task 4
  | Halt    (BitVector 8)
  | Beq  Reg Reg (BitVector 11)
  | Bne  Reg Reg (BitVector 11)
  | Bltu Reg Reg (BitVector 11)
  | Bgeu Reg Reg (BitVector 11)
  | WaitOn Reg (BitVector 2) (BitVector 9)   -- rd, cond, timeout
  | SetConfig (BitVector 6)
  | Mark (BitVector 11) Reg                  -- label, payload reg
  | CrcReset
  -- DATA group (group 10) — decoding lands in Task 5
  | LoadImm Reg (BitVector 11)
  | Lui     Reg (BitVector 20)
  | Mov     Reg Reg
  | Add  Reg Reg Reg
  | Addi Reg Reg (BitVector 11)
  | Sub  Reg Reg Reg
  | And_ Reg Reg Reg
  | Andi Reg Reg (BitVector 11)
  | Or_  Reg Reg Reg
  | Ori  Reg Reg (BitVector 11)
  | Xor_ Reg Reg Reg
  | Xori Reg Reg (BitVector 11)
  | Shift Reg Reg (BitVector 2) (BitVector 5)  -- rd, rs1, op(SLL/SRL/SRA/rsv), amt
  | Rdsr  Reg (BitVector 5)                    -- rd, sr#
  deriving stock (Generic, Show, Eq)
  deriving anyclass NFDataX

data DecodeError
  = ReservedFieldNonZero
  | OpcodeUnimplemented
  | IllegalOpcode
  deriving stock (Generic, Show, Eq)
  deriving anyclass NFDataX

-- Field split/join. group=2, sub=4, rd=5, rs1=5, rs2=5, imm=11 (sum 32).
type Fields = (BitVector 2, BitVector 4, BitVector 5, BitVector 5, BitVector 5, BitVector 11)

split :: BitVector 32 -> Fields
split = bitCoerce

joinW :: Fields -> BitVector 32
joinW = bitCoerce

-- Sub-field helpers for the 11-bit imm.
-- PUT_BITS/GET_BITS: imm[10:8] = n-1, imm[7:0] = bits.
bitsField :: BitVector 11 -> (BitVector 3, BitVector 8)
bitsField = bitCoerce

mkBitsImm :: Index 8 -> BitVector 8 -> BitVector 11
mkBitsImm n b = bitCoerce (pack n, b)

encode :: Instr -> BitVector 32
encode = \case
  -- BUS group (00)
  CsAssert        -> joinW (0b00, 0x0, 0, 0, 0, 0)
  CsDeassert      -> joinW (0b00, 0x1, 0, 0, 0, 0)
  PutByteImm b    -> joinW (0b00, 0x2, 0, 0, 0, zeroExtend b)
  PutByteReg rs   -> joinW (0b00, 0x3, 0, rs, 0, 0)
  GetByte rd      -> joinW (0b00, 0x4, rd, 0, 0, 0)
  PutBitsImm n b  -> joinW (0b00, 0x5, 0, 0, 0, mkBitsImm n b)
  PutBitsReg rs n -> joinW (0b00, 0x6, 0, rs, 0, mkBitsImm n 0)
  GetBits rd n    -> joinW (0b00, 0x7, rd, 0, 0, mkBitsImm n 0)
  TarImm n        -> joinW (0b00, 0x8, 0, 0, 0, zeroExtend n)
  TarReg rs       -> joinW (0b00, 0x9, 0, rs, 0, 0)
  RstAssert       -> joinW (0b00, 0xA, 0, 0, 0, 0)
  RstDeassert     -> joinW (0b00, 0xB, 0, 0, 0, 0)
  GetAlert rd     -> joinW (0b00, 0xC, rd, 0, 0, 0)
  -- CTRL / DATA groups encode in Tasks 4–5; provide encodings now so the
  -- ADT is total (decode for these lands later).
  _               -> encodeRest

-- placeholder branch replaced in Task 4/5; never hit by BUS tests.
encodeRest :: BitVector 32
encodeRest = joinW (0b11, 0xF, 0, 0, 0, 0)

decode :: BitVector 32 -> Either DecodeError Instr
decode w =
  case grp of
    0b00 -> decodeBus sub rd rs1 rs2 imm
    0b01 -> Left OpcodeUnimplemented   -- Task 4
    0b10 -> Left OpcodeUnimplemented   -- Task 5
    _    -> Left IllegalOpcode         -- group 11 reserved
  where
    (grp, sub, rd, rs1, rs2, imm) = split w

-- Accept an instruction only when its reserved fields are all zero.
only :: Bool -> Instr -> Either DecodeError Instr
only ok r = if ok then Right r else Left ReservedFieldNonZero

decodeBus
  :: BitVector 4 -> BitVector 5 -> BitVector 5 -> BitVector 5 -> BitVector 11
  -> Either DecodeError Instr
decodeBus sub rd rs1 rs2 imm =
  case sub of
    0x0 -> only (z rd && z rs1 && z rs2 && z imm)        CsAssert
    0x1 -> only (z rd && z rs1 && z rs2 && z imm)        CsDeassert
    0x2 -> only (z rd && z rs1 && z rs2 && immHi8 == 0)  (PutByteImm (truncateB imm))
    0x3 -> only (z rd && z rs2 && z imm)                 (PutByteReg rs1)
    0x4 -> only (z rs1 && z rs2 && z imm)                (GetByte rd)
    0x5 -> only (z rd && z rs1 && z rs2)                 (PutBitsImm nBits bBits)
    0x6 -> only (z rd && z rs2 && bBits == 0)            (PutBitsReg rs1 nBits)
    0x7 -> only (z rs1 && z rs2 && bBits == 0)           (GetBits rd nBits)
    0x8 -> only (z rd && z rs1 && z rs2 && immHi4 == 0)  (TarImm (truncateB imm))
    0x9 -> only (z rd && z rs2 && z imm)                 (TarReg rs1)
    0xA -> only (z rd && z rs1 && z rs2 && z imm)        RstAssert
    0xB -> only (z rd && z rs1 && z rs2 && z imm)        RstDeassert
    0xC -> only (z rs1 && z rs2 && z imm)                (GetAlert rd)
    _   -> Left IllegalOpcode
  where
    z :: KnownNat n => BitVector n -> Bool
    z = (== 0)
    (n3, bBits) = bitsField imm
    nBits  = unpack n3 :: Index 8
    immHi8 = slice d10 d8 imm    -- imm[10:8]  (BitVector 3) reserved for PUT_BYTE
    immHi4 = slice d10 d4 imm    -- imm[10:4]  (BitVector 7) reserved for TAR
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `stack test`
Expected: PASS — `Isa` group green (BUS round-trip, canonical, reserved-trap).

- [ ] **Step 6: Commit**

```bash
git add hdl/src/Tamal/Isa.hs hdl/tests/Test/Isa.hs hdl/tests/Test/Gen.hs hdl/tests/unittests.hs hdl/tamal.cabal
git commit -m "feat(hdl): Isa framework + BUS-group encode/decode"
```

---

## Task 4: CTRL-group encode/decode (`Tamal.Isa`)

Extend `encode`/`decode` with the CTRL group (spec §6.1). Layouts:
`HALT` imm[7:0]=status; `BEQ/BNE/BLTU/BGEU` rs1,rs2,off=imm[10:0]; `WAIT_ON` rd, cond=imm[10:9], timeout=imm[8:0]; `SET_CONFIG` payload=imm[5:0]; `MARK` label=imm[10:0], rs1=payload; `CRC_RESET` no operands.

**Files:**
- Modify: `hdl/src/Tamal/Isa.hs`
- Modify: `hdl/tests/Test/Isa.hs`

- [ ] **Step 1: Add a CTRL round-trip property to `Test.Isa`**

`genCtrlInstr` already exists in `Test.Gen` (Task 3). Extend the `Test.Isa` import and add a property. Change the import line to:

```haskell
import Test.Gen (genBusInstr, genCtrlInstr)
```

Add a property to the `testGroup "Isa"` list:

```haskell
    , testProperty "CTRL: decode . encode == Right" $ property $ do
        i <- forAll genCtrlInstr
        decode (encode i) === Right i
```

- [ ] **Step 2: Run to verify it fails**

Run: `stack test`
Expected: FAIL — CTRL instrs currently `encode` to `encodeRest` and `decode` returns `Left OpcodeUnimplemented`.

- [ ] **Step 3: Implement CTRL encode + decode**

In `hdl/src/Tamal/Isa.hs`, replace the `_ -> encodeRest` arm of `encode` with the CTRL arms (and keep a DATA fallthrough for Task 5):

```haskell
  -- CTRL group (01)
  Halt s          -> joinW (0b01, 0x0, 0, 0, 0, zeroExtend s)
  Beq  a b off    -> joinW (0b01, 0x1, 0, a, b, off)
  Bne  a b off    -> joinW (0b01, 0x2, 0, a, b, off)
  Bltu a b off    -> joinW (0b01, 0x3, 0, a, b, off)
  Bgeu a b off    -> joinW (0b01, 0x4, 0, a, b, off)
  WaitOn rd c t   -> joinW (0b01, 0x5, rd, 0, 0, bitCoerce (c, t))
  SetConfig p     -> joinW (0b01, 0x6, 0, 0, 0, zeroExtend p)
  Mark lbl rs     -> joinW (0b01, 0x7, 0, rs, 0, lbl)
  CrcReset        -> joinW (0b01, 0x8, 0, 0, 0, 0)
  _               -> encodeRest
```

Replace the `0b01 -> Left OpcodeUnimplemented` arm of `decode` with `0b01 -> decodeCtrl sub rd rs1 rs2 imm`, and add:

```haskell
decodeCtrl
  :: BitVector 4 -> BitVector 5 -> BitVector 5 -> BitVector 5 -> BitVector 11
  -> Either DecodeError Instr
decodeCtrl sub rd rs1 rs2 imm =
  case sub of
    0x0 -> only (z rd && z rs1 && z rs2 && immHi8 == 0)  (Halt (truncateB imm))
    0x1 -> only (z rd)                                   (Beq  rs1 rs2 imm)
    0x2 -> only (z rd)                                   (Bne  rs1 rs2 imm)
    0x3 -> only (z rd)                                   (Bltu rs1 rs2 imm)
    0x4 -> only (z rd)                                   (Bgeu rs1 rs2 imm)
    0x5 -> only (z rs1 && z rs2)                         (WaitOn rd cond timeout)
    0x6 -> only (z rd && z rs1 && z rs2 && immHi6 == 0)  (SetConfig (truncateB imm))
    0x7 -> only (z rd && z rs2)                          (Mark imm rs1)
    0x8 -> only (z rd && z rs1 && z rs2 && z imm)        CrcReset
    _   -> Left IllegalOpcode
  where
    z :: KnownNat n => BitVector n -> Bool
    z = (== 0)
    (cond, timeout) = bitCoerce imm :: (BitVector 2, BitVector 9)
    immHi8 = slice d10 d8 imm    -- HALT: imm[10:8]  (BitVector 3) reserved
    immHi6 = slice d10 d6 imm    -- SET_CONFIG: imm[10:6] (BitVector 5) reserved
```

- [ ] **Step 4: Run to verify pass**

Run: `stack test`
Expected: PASS — CTRL round-trip green (BUS still green).

- [ ] **Step 5: Commit**

```bash
git add hdl/src/Tamal/Isa.hs hdl/tests/Test/Isa.hs hdl/tests/Test/Gen.hs
git commit -m "feat(hdl): CTRL-group encode/decode"
```

---

## Task 5: DATA-group encode/decode (`Tamal.Isa`)

Extend with the DATA group (spec §6.2). Layouts: reg-reg ops use rd,rs1,rs2 (imm=0); imm ops use rd,rs1,imm[10:0] (rs2=0); `LUI` rd + imm20 packed into rs1++rs2++imm low 20 (bit20 reserved); `SHIFT` rd,rs1,op=imm[10:9],amt=imm[4:0]; `RDSR` rd,sr#=imm[4:0].

**Files:**
- Modify: `hdl/src/Tamal/Isa.hs`
- Modify: `hdl/tests/Test/Isa.hs`

- [ ] **Step 1: Add DATA + all-groups round-trip properties to `Test.Isa`**

`genDataInstr` and `genInstr` already exist in `Test.Gen` (Task 3). Change the `Test.Isa` import line to:

```haskell
import Test.Gen (genBusInstr, genCtrlInstr, genDataInstr, genInstr)
```

Add to the `testGroup "Isa"` list:

```haskell
    , testProperty "DATA: decode . encode == Right" $ property $ do
        i <- forAll genDataInstr
        decode (encode i) === Right i
    , testProperty "any valid instr: encode . decode == id" $ property $ do
        i <- forAll genInstr
        (encode <$> decode (encode i)) === Right (encode i)
```

- [ ] **Step 2: Run to verify it fails**

Run: `stack test`
Expected: FAIL — DATA instrs `encode` to `encodeRest`, `decode` returns `Left OpcodeUnimplemented`.

- [ ] **Step 3: Implement DATA encode + decode; delete `encodeRest`**

In `hdl/src/Tamal/Isa.hs`, replace the trailing `_ -> encodeRest` arm of `encode` with explicit DATA arms and remove the `encodeRest` definition:

```haskell
  -- DATA group (10)
  LoadImm rd i    -> joinW (0b10, 0x0, rd, 0, 0, i)
  Lui rd i20      -> let (rs1', rs2', imm') = splitImm20 i20
                     in joinW (0b10, 0x1, rd, rs1', rs2', imm')
  Mov rd rs       -> joinW (0b10, 0x2, rd, rs, 0, 0)
  Add rd a b      -> joinW (0b10, 0x3, rd, a, b, 0)
  Addi rd a i     -> joinW (0b10, 0x4, rd, a, 0, i)
  Sub rd a b      -> joinW (0b10, 0x5, rd, a, b, 0)
  And_ rd a b     -> joinW (0b10, 0x6, rd, a, b, 0)
  Andi rd a i     -> joinW (0b10, 0x7, rd, a, 0, i)
  Or_ rd a b      -> joinW (0b10, 0x8, rd, a, b, 0)
  Ori rd a i      -> joinW (0b10, 0x9, rd, a, 0, i)
  Xor_ rd a b     -> joinW (0b10, 0xA, rd, a, b, 0)
  Xori rd a i     -> joinW (0b10, 0xB, rd, a, 0, i)
  Shift rd a op a5 -> joinW (0b10, 0xC, rd, a, 0, bitCoerce (op, 0 :: BitVector 4, a5))
  Rdsr rd sr      -> joinW (0b10, 0xD, rd, 0, 0, zeroExtend sr)
```

Add the LUI imm20 pack/split helpers near the field helpers:

```haskell
-- LUI imm20 occupies rs1 ++ rs2 ++ imm low 20 bits; bit 20 reserved 0.
splitImm20 :: BitVector 20 -> (BitVector 5, BitVector 5, BitVector 11)
splitImm20 i20 = bitCoerce ((0 :: BitVector 1) ++# i20)

joinImm20 :: BitVector 5 -> BitVector 5 -> BitVector 11 -> (BitVector 1, BitVector 20)
joinImm20 rs1 rs2 imm = bitCoerce (rs1 ++# rs2 ++# imm)
```

Replace the `0b10 -> Left OpcodeUnimplemented` arm of `decode` with `0b10 -> decodeData sub rd rs1 rs2 imm`, and add:

```haskell
decodeData
  :: BitVector 4 -> BitVector 5 -> BitVector 5 -> BitVector 5 -> BitVector 11
  -> Either DecodeError Instr
decodeData sub rd rs1 rs2 imm =
  case sub of
    0x0 -> only (z rs1 && z rs2)                  (LoadImm rd imm)
    0x1 -> only (hi == 0)                         (Lui rd i20)
    0x2 -> only (z rs2 && z imm)                  (Mov rd rs1)
    0x3 -> only (z imm)                           (Add rd rs1 rs2)
    0x4 -> only (z rs2)                           (Addi rd rs1 imm)
    0x5 -> only (z imm)                           (Sub rd rs1 rs2)
    0x6 -> only (z imm)                           (And_ rd rs1 rs2)
    0x7 -> only (z rs2)                           (Andi rd rs1 imm)
    0x8 -> only (z imm)                           (Or_ rd rs1 rs2)
    0x9 -> only (z rs2)                           (Ori rd rs1 imm)
    0xA -> only (z imm)                           (Xor_ rd rs1 rs2)
    0xB -> only (z rs2)                           (Xori rd rs1 imm)
    0xC -> only (z rs2 && shMid == 0)             (Shift rd rs1 shOp shAmt)
    0xD -> only (z rs1 && z rs2 && immHi5 == 0)   (Rdsr rd (truncateB imm))
    _   -> Left IllegalOpcode
  where
    z :: KnownNat n => BitVector n -> Bool
    z = (== 0)
    (hi, i20)            = joinImm20 rs1 rs2 imm
    (shOp, shMid, shAmt) = bitCoerce imm :: (BitVector 2, BitVector 4, BitVector 5)
    immHi5 = slice d10 d5 imm    -- RDSR imm[10:5] (BitVector 6) reserved
```

Add `joinImm20` type is `-> (BitVector 1, BitVector 20)`; `hi` is the reserved bit checked in decode.

- [ ] **Step 4: Run to verify pass**

Run: `stack test`
Expected: PASS — all three groups round-trip; `encode . decode == id` for random valid instrs.

- [ ] **Step 5: Commit**

```bash
git add hdl/src/Tamal/Isa.hs hdl/tests/Test/Isa.hs hdl/tests/Test/Gen.hs
git commit -m "feat(hdl): DATA-group encode/decode; Isa complete for Phase 1"
```

---

## Task 6: Config decode (`Tamal.Config`)

Decode the 6-bit `SET_CONFIG` payload (spec §7.2): `role[5]`, `io_mode[4:3]`, `sck[2:1]`, `alert_source[0]`. v1 accepts only controller / x1 / 20 MHz; unimplemented values → `Left`.

**Files:**
- Create: `hdl/src/Tamal/Config.hs`
- Modify: `hdl/tamal.cabal` (`exposed-modules` += `Tamal.Config`; `other-modules` += `Test.Config`)
- Create: `hdl/tests/Test/Config.hs`
- Modify: `hdl/tests/unittests.hs`

- [ ] **Step 1: Write the failing test**

Create `hdl/tests/Test/Config.hs`:

```haskell
module Test.Config (tests) where

import Clash.Prelude
import Test.Tasty
import Test.Tasty.HUnit

import Tamal.Config

tests :: TestTree
tests =
  testGroup "Config"
    [ testCase "v1 default payload decodes" $
        -- role=0, io=00, sck=00, alert=0  ->  all-zero payload
        decodeConfig 0b000000 @?= Right (Config Controller X1 Sck20 AlertPin)
    , testCase "alert_source=io1 accepted" $
        decodeConfig 0b000001 @?= Right (Config Controller X1 Sck20 AlertIo1)
    , testCase "target role rejected in v1" $
        decodeConfig 0b100000 @?= Left UnsupportedRole
    , testCase "dual I/O rejected in v1" $
        decodeConfig 0b001000 @?= Left UnsupportedIoMode
    , testCase "33 MHz rejected in v1" $
        decodeConfig 0b000010 @?= Left UnsupportedSck
    ]
```

- [ ] **Step 2: Aggregate + run to verify it fails**

Add `import qualified Test.Config` and `Test.Config.tests` to `hdl/tests/unittests.hs`; add `Test.Config` to `other-modules`.

Run: `stack test`
Expected: FAIL to compile — `Tamal.Config` not found.

- [ ] **Step 3: Implement `Tamal.Config`**

Create `hdl/src/Tamal/Config.hs`:

```haskell
{- |
Engine configuration decoded from the @SET_CONFIG@ payload (spec §7.2).
v1 implements only controller role, single I/O, and 20 MHz SCK; any other
selection is a decode error (the engine turns these into a TRAP).
-}
module Tamal.Config
  ( Role (..)
  , IoMode (..)
  , Sck (..)
  , AlertSource (..)
  , Config (..)
  , ConfigError (..)
  , decodeConfig
  ) where

import Clash.Prelude

data Role        = Controller | Target        deriving stock (Generic, Show, Eq) deriving anyclass NFDataX
data IoMode      = X1 | X2 | X4               deriving stock (Generic, Show, Eq) deriving anyclass NFDataX
data Sck         = Sck20 | Sck33 | Sck50 | Sck66 deriving stock (Generic, Show, Eq) deriving anyclass NFDataX
data AlertSource = AlertPin | AlertIo1        deriving stock (Generic, Show, Eq) deriving anyclass NFDataX

data Config = Config
  { cfgRole        :: Role
  , cfgIoMode      :: IoMode
  , cfgSck         :: Sck
  , cfgAlertSource :: AlertSource
  }
  deriving stock (Generic, Show, Eq)
  deriving anyclass NFDataX

data ConfigError
  = UnsupportedRole
  | UnsupportedIoMode
  | UnsupportedSck
  deriving stock (Generic, Show, Eq)
  deriving anyclass NFDataX

-- payload[5]=role, [4:3]=io_mode, [2:1]=sck, [0]=alert_source
decodeConfig :: BitVector 6 -> Either ConfigError Config
decodeConfig p =
  case (role, io, sck) of
    (0b0, 0b00, 0b00) -> Right (Config Controller X1 Sck20 alertSrc)
    (0b1, _,    _   ) -> Left UnsupportedRole
    (_,   io',  _   ) | io' /= 0b00 -> Left UnsupportedIoMode
    _                 -> Left UnsupportedSck
  where
    (role, io, sck, alert) = bitCoerce p :: (BitVector 1, BitVector 2, BitVector 2, BitVector 1)
    alertSrc = if alert == 0 then AlertPin else AlertIo1
```

Note: helper `Config Controller X1 Sck20 alertSrc` uses positional fields — order matches the record declaration (`cfgRole cfgIoMode cfgSck cfgAlertSource`).

Add `Tamal.Config` to `exposed-modules` in `hdl/tamal.cabal`.

- [ ] **Step 4: Run to verify pass**

Run: `stack test`
Expected: PASS — `Config` group green.

- [ ] **Step 5: Commit**

```bash
git add hdl/src/Tamal/Config.hs hdl/tests/Test/Config.hs hdl/tests/unittests.hs hdl/tamal.cabal
git commit -m "feat(hdl): SET_CONFIG payload decode with v1 validation"
```

---

## Task 7: x1 byte serdes + TAR (`Tamal.Bus.Serdes`)

The pure lane logic (spec §5.2, §5.4, §5.5). `Lanes = Vec 4 (Bit, Bit)` where each element is `(output value, output enable)`. x1: PUT drives IO[0] MSB-first; GET samples IO[1]. TAR beat 0 drives all lanes high, later beats tri-state.

**Files:**
- Create: `hdl/src/Tamal/Bus/Serdes.hs`
- Modify: `hdl/tamal.cabal` (`exposed-modules` += `Tamal.Bus.Serdes`; `other-modules` += `Test.Serdes`)
- Create: `hdl/tests/Test/Serdes.hs`
- Modify: `hdl/tests/unittests.hs`

- [ ] **Step 1: Write the failing test**

Create `hdl/tests/Test/Serdes.hs`:

```haskell
module Test.Serdes (tests) where

import Clash.Prelude
import Test.Tasty
import Test.Tasty.HUnit
import Test.Tasty.Hedgehog (testProperty)
import Hedgehog (property, forAll, (===))

import Tamal.Bus.Serdes
import Test.Gen (genByte)

-- loopback: sample IO[0] (the driven lane) back as if it were IO[1]
io0 :: Lanes -> Bit
io0 lanes = fst (lanes !! (0 :: Index 4))

tests :: TestTree
tests =
  testGroup "Serdes"
    [ testProperty "x1 serialize/deserialize round-trips (loopback)" $ property $ do
        b <- forAll genByte
        deserializeX1 (map io0 (serializeX1 b)) === b
    , testCase "x1 serialize drives IO[0] MSB-first, tri-states IO[1..3]" $ do
        let beats = serializeX1 0b1000_0000        -- MSB set only
        io0 (head beats) @?= 1                       -- first beat carries the MSB
        io0 (last beats) @?= 0
        -- IO[1] output-enable is 0 (tri-stated) on every beat
        map (\l -> snd (l !! (1 :: Index 4))) beats @?= repeat 0
    , testCase "TAR beat 0 drives all lanes high; later beats tri-state" $ do
        tarBeat 0 @?= repeat (1, 1)
        tarBeat 1 @?= repeat (0, 0)
        tarBeat 5 @?= repeat (0, 0)
    ]
```

- [ ] **Step 2: Aggregate + run to verify it fails**

Add `import qualified Test.Serdes` and `Test.Serdes.tests` to `hdl/tests/unittests.hs`; add `Test.Serdes` to `other-modules`.

Run: `stack test`
Expected: FAIL to compile — `Tamal.Bus.Serdes` not found.

- [ ] **Step 3: Implement `Tamal.Bus.Serdes`**

Create `hdl/src/Tamal/Bus/Serdes.hs`:

```haskell
{- |
Pure single-I/O (x1) byte serialisation and the turnaround (TAR) beat
vector (spec §5). A 'Lanes' value is the per-beat drive state of the four
I/O lanes: @(output value, output enable)@. @oe = 0@ means tri-stated.

x1 rules: PUT drives the data bit on IO[0], MSB first, with IO[1..3]
tri-stated; GET samples IO[1] with all engine drivers tri-stated. Dual/
quad maps land in Phase 3.
-}
module Tamal.Bus.Serdes
  ( Lane
  , Lanes
  , hiZ
  , driveHigh
  , serializeX1
  , deserializeX1
  , tarBeat
  ) where

import Clash.Prelude

type Lane  = (Bit, Bit)   -- (output value, output enable)
type Lanes = Vec 4 Lane

-- | All four lanes tri-stated.
hiZ :: Lanes
hiZ = repeat (0, 0)

-- | All four lanes actively driven to logic 1 (TAR first clock).
driveHigh :: Lanes
driveHigh = repeat (1, 1)

-- | One byte -> eight beats, MSB first, driving IO[0] only.
serializeX1 :: BitVector 8 -> Vec 8 Lanes
serializeX1 b = map beat (unpack b :: Vec 8 Bit)
  where
    beat :: Bit -> Lanes
    beat bit = (bit, 1) :> (0, 0) :> (0, 0) :> (0, 0) :> Nil

-- | Eight IO[1] samples (MSB first) -> one byte.
deserializeX1 :: Vec 8 Bit -> BitVector 8
deserializeX1 = pack

-- | TAR beat @i@: beat 0 drives all lanes high, subsequent beats tri-state.
tarBeat :: Unsigned 4 -> Lanes
tarBeat i = if i == 0 then driveHigh else hiZ
```

Add `Tamal.Bus.Serdes` to `exposed-modules` in `hdl/tamal.cabal`.

- [ ] **Step 4: Run to verify pass**

Run: `stack test`
Expected: PASS — `Serdes` group green.

- [ ] **Step 5: Commit**

```bash
git add hdl/src/Tamal/Bus/Serdes.hs hdl/tests/Test/Serdes.hs hdl/tests/unittests.hs hdl/tamal.cabal
git commit -m "feat(hdl): x1 byte serdes + TAR beat vector"
```

---

## Task 8: Result-ring records + overflow model (`Tamal.Trace`)

Record encoding + atomic ring-push overflow model (spec §8). CAPTURE=1 word, MARK=2 words, HALT=1 word. `ringPush` writes a record atomically or drops it (sticky overflow), never exceeding the record limit (the HALT slot beyond `limit` is reserved).

**Files:**
- Create: `hdl/src/Tamal/Trace.hs`
- Modify: `hdl/tamal.cabal` (`exposed-modules` += `Tamal.Trace`; `other-modules` += `Test.Trace`)
- Create: `hdl/tests/Test/Trace.hs`
- Modify: `hdl/tests/unittests.hs`

- [ ] **Step 1: Write the failing test**

Create `hdl/tests/Test/Trace.hs`:

```haskell
module Test.Trace (tests) where

import Clash.Prelude
import qualified Data.List as L
import Test.Tasty
import Test.Tasty.HUnit

import Tamal.Trace

tests :: TestTree
tests =
  testGroup "Trace"
    [ testCase "CAPTURE encodes tag 00, nbits, byte" $
        encodeRecord (Capture 8 0xA5) @?= [0b00 `shiftL` 30 .|. (8 `shiftL` 8) .|. 0xA5]
    , testCase "MARK encodes 2 words: tag 10 + label, then payload" $
        encodeRecord (Mark 0x1234 0xDEADBEEF)
          @?= [ (0b10 `shiftL` 30) .|. 0x1234, 0xDEADBEEF ]
    , testCase "HALT encodes tag 11, overflow bit, status" $
        encodeRecord (Halt True 0x11) @?= [ (0b11 `shiftL` 30) .|. (1 `shiftL` 8) .|. 0x11 ]
    , testCase "ringPush past limit sets sticky overflow and drops" $ do
        -- limit 3: slots 0..3 usable, slot 4 = reserved HALT terminator.
        let step (ptr, ovf, acc) ws =
              let (ptr', ovf', wrote) = ringPush ptr 3 ovf ws
              in (ptr', ovf', acc <> wrote)
            recs = L.replicate 10 [0xC0DE :: BitVector 32]   -- ten 1-word records
            (finalPtr, finalOvf, written) = L.foldl' step (0, False, []) recs
        assertBool "ptr never past limit+1" (finalPtr <= 4)
        finalOvf @?= True
        assertBool "at most 4 words written" (L.length written <= 4)
    ]
```

- [ ] **Step 2: Aggregate + run to verify it fails**

Add `import qualified Test.Trace` and `Test.Trace.tests` to `hdl/tests/unittests.hs`; add `Test.Trace` to `other-modules`.

Run: `stack test`
Expected: FAIL to compile — `Tamal.Trace` not found.

- [ ] **Step 3: Implement `Tamal.Trace`**

Create `hdl/src/Tamal/Trace.hs`:

```haskell
{- |
Result-ring record encoding and the overflow-safe push model (spec §8).
Records are whole 32-bit words: CAPTURE (1 word, tag 00), MARK (2 words,
tag 10), HALT (1 word, tag 11). 'ringPush' writes a record atomically or
drops it, setting a sticky overflow flag; it never writes past the record
limit (the HALT terminator slot beyond it is reserved).
-}
module Tamal.Trace
  ( Record (..)
  , encodeRecord
  , ringPush
  ) where

import Clash.Prelude
import qualified Data.List as L

data Record
  = Capture (BitVector 4) (BitVector 8)   -- nbits (1..8), sampled byte
  | Mark    (BitVector 14) (BitVector 32) -- label, payload
  | Halt    Bool (BitVector 8)            -- overflow, status
  deriving stock (Generic, Show, Eq)
  deriving anyclass NFDataX

-- | Encode a record to its 32-bit words (reference model; the engine's
-- synthesizable word emitter in Plan A2 matches these layouts).
encodeRecord :: Record -> [BitVector 32]
encodeRecord = \case
  Capture n b   -> [ bitCoerce (0b00 :: BitVector 2, 0 :: BitVector 18, n, b) ]
  Mark lbl pl   -> [ bitCoerce (0b10 :: BitVector 2, 0 :: BitVector 16, lbl), pl ]
  Halt ovf st   -> [ bitCoerce (0b11 :: BitVector 2, 0 :: BitVector 21, ovf, st) ]

-- | Atomically push a record's words. Given the current write pointer, the
-- last usable record slot (@limit@), and prior overflow, either write all
-- words (advancing the pointer) or drop them and latch overflow.
ringPush
  :: Unsigned 12          -- ^ current write pointer
  -> Unsigned 12          -- ^ last usable record-slot index (limit)
  -> Bool                 -- ^ prior sticky overflow
  -> [BitVector 32]       -- ^ words of one record
  -> (Unsigned 12, Bool, [BitVector 32])
ringPush ptr limit ovf ws
  | ovf                                = (ptr, True, [])
  | fits                               = (ptr + count, False, ws)
  | otherwise                          = (ptr, True, [])
  where
    count = fromIntegral (L.length ws)
    -- last index this record would occupy is ptr + count - 1; must be <= limit
    fits  = L.length ws > 0 && (ptr + count - 1) <= limit
```

Add `Tamal.Trace` to `exposed-modules` in `hdl/tamal.cabal`.

- [ ] **Step 4: Run to verify pass**

Run: `stack test`
Expected: PASS — `Trace` group green.

- [ ] **Step 5: Commit**

```bash
git add hdl/src/Tamal/Trace.hs hdl/tests/Test/Trace.hs hdl/tests/unittests.hs hdl/tamal.cabal
git commit -m "feat(hdl): result-ring record encoding + overflow model"
```

---

## Task 9: CI — HDL (hedgehog) + Rust

Two-job GitHub Actions workflow (ubuntu, no Vivado). HDL: stack build + test + Clash codegen smoke, with caching. Rust: build + test + clippy + fmt.

**Files:**
- Create: `.github/workflows/ci.yml`

- [ ] **Step 1: Create the workflow**

Create `.github/workflows/ci.yml`:

```yaml
name: CI

on:
  push:
    branches: [main]
  pull_request:

jobs:
  hdl:
    name: HDL (Clash + hedgehog)
    runs-on: ubuntu-latest
    defaults:
      run:
        working-directory: hdl
    steps:
      - uses: actions/checkout@v4

      - uses: haskell-actions/setup@v2
        with:
          enable-stack: true
          stack-version: latest

      - name: Cache stack + GHC + Clash
        uses: actions/cache@v4
        with:
          path: |
            ~/.stack
            hdl/.stack-work
          key: stack-${{ runner.os }}-${{ hashFiles('hdl/stack.yaml.lock', 'hdl/tamal.cabal') }}
          restore-keys: |
            stack-${{ runner.os }}-

      - name: Build
        run: stack build --test --no-run-tests

      - name: Test (hedgehog)
        run: stack test

      - name: Clash -> Verilog codegen smoke
        run: stack run clash -- Tamal --verilog

  rust:
    name: Rust (workspace)
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy, rustfmt

      - uses: Swatinem/rust-cache@v2

      - name: Build
        run: cargo build --workspace --all-targets

      - name: Test
        run: cargo test --workspace

      - name: Clippy
        run: cargo clippy --workspace --all-targets -- -D warnings

      - name: Format check
        run: cargo fmt --all --check
```

- [ ] **Step 2: Validate the workflow YAML locally**

Run (from repo root): `pwsh -c "python -c 'import yaml,sys; yaml.safe_load(open(\"./.github/workflows/ci.yml\")); print(\"ok\")'"`
Expected: `ok` (no YAML syntax error). If Python/pyyaml is unavailable, visually confirm indentation is 2-space and consistent.

- [ ] **Step 3: Confirm the referenced commands work locally**

Run (from `hdl/`): `stack test` then `stack run clash -- Tamal --verilog`
Expected: `stack test` passes; the Clash run writes `verilog/Tamal.topEntity/…`.

Run (from repo root): `cargo build --workspace --all-targets`, `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all --check`
Expected: all succeed (scaffold crates compile; no clippy warnings; formatting clean). If `cargo fmt --all --check` reports diffs on the existing scaffold, run `cargo fmt --all` and include that formatting fix in this commit.

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: HDL (Clash + hedgehog) and Rust workspace jobs"
```

---

## Self-Review Notes

- **Spec coverage:** §4 instruction word → Tasks 3–5; §5 BUS opcodes → Task 3 (encode/decode) + Task 7 (serdes/TAR mechanics); §6 CTRL/DATA → Tasks 4–5; §7.2 config → Task 6; §7.4 RX CRC-8 → Task 2; §8 result-ring → Task 8; §11 hedgehog+CI → Tasks 1 & 9. `Engine.step` (§10 engine, §9 worked example) is intentionally deferred to Plan A2; the impure top-entity shell (UART/IOBUF/SCK-gen/Vivado/XDC) to Plan B.
- **Not in this plan:** register-file x16–x31 rejection (an Engine concern, A2); synthesizable per-word ring emitter (A2 — `encodeRecord`/`ringPush` here are the tested reference model); dual/quad serdes (Phase 3).
- **Type consistency:** `Lanes = Vec 4 (Bit, Bit)` used identically in Serdes + tests; `Reg = BitVector 5`; `Instr` constructor arities match generators; `decode`/`encode` are total inverses on the valid-instruction domain.
```
