-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0

module Test.RegFile (tests) where

import Clash.Prelude
import Hedgehog (Gen, forAll, property, (===))
import qualified Hedgehog.Gen as Gen
import qualified Hedgehog.Range as Range
import Test.Tasty
import Test.Tasty.HUnit
import Test.Tasty.Hedgehog (testProperty)

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
  testGroup
    "RegFile"
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
          forAll
            $ Gen.filter
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
    , testCase "x16 aliases x0 (write discarded)"
        $ writeReg initRegs 16 42
        @?= initRegs
    ]
