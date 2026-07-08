-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-W-2.0

module Test.Crc (tests) where

import Clash.Prelude
import qualified Data.List as L
import Hedgehog (forAll, property, (===))
import qualified Hedgehog.Gen as Gen
import qualified Hedgehog.Range as Range
import Test.Tasty
import Test.Tasty.HUnit
import Test.Tasty.Hedgehog (testProperty)

import Tamal.Crc (crc8Update)
import Test.Gen (genByte)

-- | Fold the CRC over a whole message (test-side helper).
crc8 :: [BitVector 8] -> BitVector 8
crc8 = L.foldl' crc8Update 0

tests :: TestTree
tests =
  testGroup
    "Crc"
    [ testCase "crc8Update 0 0x01 == 0x07"
        $ crc8Update 0 0x01
        @?= 0x07
    , testCase "CRC-8/SMBUS check \"123456789\" == 0xF4"
        $ crc8 [fromIntegral (fromEnum c) | c <- "123456789"]
        @?= 0xF4
    , testProperty "residue law: crc8 (msg <> [crc8 msg]) == 0" $ property $ do
        msg <- forAll (Gen.list (Range.linear 0 32) genByte)
        crc8 (msg <> [crc8 msg]) === 0
    ]

-- Note: `Clash.Prelude` re-exports `map`/`(++)` as the `Vec` versions, so the
-- list glue uses a list comprehension and `(<>)` (list `Semigroup`) rather than
-- `map`/`(++)`.
