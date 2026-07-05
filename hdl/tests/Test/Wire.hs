-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0

module Test.Wire (tests) where

import Clash.Prelude
import qualified Data.List as L
import Hedgehog (Gen, forAll, property, (/==), (===))
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
    , -- Regression: a full 254-byte group terminated by a zero must flush as a
      -- 0xFF continuation and then emit a *separate* empty group for the zero
      -- (…FF,<254>,01,01). Folding the zero into the full group (code 0xFF, which
      -- carries no implied zero) silently drops it on decode.
      testCase "cobsEncode 254 non-zero then 0x00 == FF,<254>,01,01"
        $ cobsEncode (run254 <> [0x00])
        @?= (0xFF : run254) <> [0x01, 0x01]
    , testCase "cobsDecode . cobsEncode round-trips 254 non-zero then 0x00"
        $ cobsDecode (cobsEncode (run254 <> [0x00]))
        @?= Just (run254 <> [0x00])
    , testProperty "cobsEncode output contains no 0x00" $ property $ do
        xs <- forAll (Gen.list (Range.linear 0 300) genByteZeros)
        L.elem 0 (cobsEncode xs) === False
    , testProperty "cobsDecode . cobsEncode == Just" $ property $ do
        xs <- forAll (Gen.list (Range.linear 0 300) genByteZeros)
        cobsDecode (cobsEncode xs) === Just xs
    , -- The zero-dense generator above cannot produce a 254-long non-zero run,
      -- so it never exercises the full-group / trailing-zero boundary. genRuns
      -- does: round-trip and the no-0x00 invariant must hold there too.
      testProperty "cobsDecode . cobsEncode == Just (boundary runs)" $ property $ do
        xs <- forAll genRuns
        cobsDecode (cobsEncode xs) === Just xs
    , testProperty "cobsEncode output contains no 0x00 (boundary runs)" $ property $ do
        xs <- forAll genRuns
        L.elem 0 (cobsEncode xs) === False
    , testCase "cobsDecode truncated group -> Nothing"
        $ cobsDecode [0x05, 0x11]
        @?= Nothing
    , testCase "cobsDecode interior zero -> Nothing"
        $ cobsDecode [0x03, 0x11, 0x00]
        @?= Nothing
    , testCase "cobsDecode [] -> Nothing"
        $ cobsDecode []
        @?= (Nothing :: Maybe [BitVector 8])
    , testCase "crc8 matches CRC-8/SMBUS check vector (0xF4)"
        $ crc8 [fromIntegral (fromEnum c) | c <- "123456789"]
        @?= 0xF4
    , testProperty "frameDecode . frameEncode == Right" $ property $ do
        xs <- forAll (Gen.list (Range.linear 0 64) genByte)
        frameDecode (frameEncode xs) === Right xs
    , testCase "frame ends in exactly one 0x00, none interior" $ do
        let f = frameEncode [0x01, 0x00, 0x02]
        L.last f @?= 0
        L.length (L.filter (== 0) f) @?= 1
    , testProperty "single-byte corruption is never a silent success" $ property $ do
        xs <- forAll (Gen.list (Range.linear 1 32) genByte)
        let f = frameEncode xs
        i <- forAll (Gen.int (Range.linear 0 (L.length f - 1)))
        let f' = [if j == i then x `xor` 1 else x | (j, x) <- L.zip [0 ..] f]
        frameDecode f' /== Right xs
    , testProperty "decodeControl . encodeControl (LoadProgram) == Right" $ property $ do
        ws <- forAll (Gen.list (Range.linear 0 32) genWord)
        decodeControl (encodeControl (LoadProgram ws)) === Right (LoadProgram ws)
    , testCase "decodeControl . encodeControl Trigger == Right Trigger"
        $ decodeControl (encodeControl Trigger)
        @?= Right Trigger
    , testCase "unknown opcode -> UnknownOpcode"
        $ decodeControl (frameEncode [0x7E, 0xAA])
        @?= Left (UnknownOpcode 0x7E)
    , testCase "LOAD payload not a multiple of 4 -> BadPayloadLen"
        $ decodeControl (frameEncode [0x01, 0xAA, 0xBB])
        @?= Left BadPayloadLen
    , testCase "empty logical frame -> ShortFrame"
        $ decodeControl (frameEncode [])
        @?= Left ShortFrame
    , testProperty "decodeResult . encodeResult == Right" $ property $ do
        ws <- forAll (Gen.list (Range.linear 0 64) genWord)
        decodeResult (encodeResult ws) === Right ws
    , testCase "result frame round-trips a REVISION-led word stream"
        $ decodeResult (encodeResult [0x00010000, 0xAABBCCDD, 0xC0000011])
        @?= Right [0x00010000, 0xAABBCCDD, 0xC0000011]
    , testCase "control opcode is rejected by decodeResult"
        $ decodeResult (encodeControl Trigger)
        @?= Left (UnknownOpcode 0x02)
    ]

-- A zero-dense byte generator: many group boundaries via frequent zeros, but
-- (by design) only short non-zero runs — it cannot reach the 254-byte full-group
-- cap. Reuses Test.Gen's genByte.
genByteZeros :: Gen (BitVector 8)
genByteZeros = Gen.frequency [(1, pure 0), (4, genByte)]

-- A full-group-boundary generator: concatenated runs of non-zero bytes (each up
-- to 260 long, so runs cross the 254-byte cap), each optionally followed by a
-- single zero. This is what actually exercises the full-group / trailing-zero
-- interaction the zero-dense generator above cannot reach.
genRuns :: Gen [BitVector 8]
genRuns = fmap L.concat (Gen.list (Range.linear 0 4) chunk)
 where
  chunk = do
    n <- Gen.int (Range.linear 0 260)
    run <- Gen.list (Range.singleton n) genNonZero
    z <- Gen.bool
    pure (run <> if z then [0] else [])
  genNonZero = fmap fromIntegral (Gen.int (Range.linear 1 255))

-- 254 distinct non-zero bytes (1..254) — the maximal COBS group.
run254 :: [BitVector 8]
run254 = L.map fromIntegral [1 .. 254 :: Int]

-- 255 non-zero bytes — one past the maximal group (forces a second group).
run255 :: [BitVector 8]
run255 = L.map fromIntegral [1 .. 255 :: Int]
