-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0

module Test.Wire (tests) where

import Clash.Prelude
import qualified Data.List as L
import Hedgehog (Gen, forAll, property, (===))
import qualified Hedgehog.Gen as Gen
import qualified Hedgehog.Range as Range
import Test.Tasty
import Test.Tasty.HUnit
import Test.Tasty.Hedgehog (testProperty)

import Tamal.Wire
import Tamal.Wire.Cobs (cobsDecode, cobsEncode)
import Test.Gen (genByte, genWord)

tests :: TestTree
tests =
  testGroup
    "Wire"
    [ testCase "wordToBytesLE 0xAABBCCDD == [DD,CC,BB,AA]"
        $ toList (wordToBytesLE 0xAABBCCDD)
        @?= [0xDD, 0xCC, 0xBB, 0xAA]
    , testProperty "bytesToWordLE . wordToBytesLE == id" $ property $ do
        w <- forAll genWord
        bytesToWordLE (wordToBytesLE w) === w
    , testCase "cobsEncode [0x00] == [0x01,0x01]"
        $ cobsEncode [0x00]
        @?= [0x01, 0x01]
    , testCase "cobsEncode [11,22,00,33] == [03,11,22,02,33]"
        $ cobsEncode [0x11, 0x22, 0x00, 0x33]
        @?= [0x03, 0x11, 0x22, 0x02, 0x33]
    , testCase "cobsEncode [11,00,00,00] == [02,11,01,01,01]"
        $ cobsEncode [0x11, 0x00, 0x00, 0x00]
        @?= [0x02, 0x11, 0x01, 0x01, 0x01]
    , testCase "cobsEncode [] == [0x01]"
        $ cobsEncode []
        @?= [0x01]
    , testCase "cobsEncode 254 non-zero bytes == 0xFF-led, no trailing 0x01"
        $ cobsEncode run254
        @?= (0xFF : run254)
    , testCase "cobsEncode 255 non-zero bytes == 0xFF group + 0x02 group"
        $ cobsEncode run255
        @?= (0xFF : run254) <> [0x02, 255]
    , testProperty "cobsEncode output contains no 0x00" $ property $ do
        xs <- forAll (Gen.list (Range.linear 0 300) genByteZeros)
        L.elem 0 (cobsEncode xs) === False
    , testProperty "cobsDecode . cobsEncode == Just" $ property $ do
        xs <- forAll (Gen.list (Range.linear 0 300) genByteZeros)
        cobsDecode (cobsEncode xs) === Just xs
    , testCase "cobsDecode truncated group -> Nothing"
        $ cobsDecode [0x05, 0x11]
        @?= Nothing
    , testCase "cobsDecode interior zero -> Nothing"
        $ cobsDecode [0x03, 0x11, 0x00]
        @?= Nothing
    , testCase "cobsDecode [] -> Nothing"
        $ cobsDecode []
        @?= (Nothing :: Maybe [BitVector 8])
    ]

-- A zero-dense byte generator: stresses COBS group boundaries far harder than
-- the ~1/256 zeros a uniform generator produces. Reuses Test.Gen's genByte.
genByteZeros :: Gen (BitVector 8)
genByteZeros = Gen.frequency [(1, pure 0), (4, genByte)]

-- 254 distinct non-zero bytes (1..254) — the maximal COBS group.
run254 :: [BitVector 8]
run254 = L.map fromIntegral [1 .. 254 :: Int]

-- 255 non-zero bytes — one past the maximal group (forces a second group).
run255 :: [BitVector 8]
run255 = L.map fromIntegral [1 .. 255 :: Int]
