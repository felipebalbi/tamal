-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0
{-# LANGUAGE NumericUnderscores #-}

module Test.Loader (tests) where

import Clash.Prelude
import qualified Data.List as L
import Hedgehog (Gen, forAll, property, (===))
import qualified Hedgehog.Gen as Gen
import qualified Hedgehog.Range as Range
import Test.Tasty
import Test.Tasty.HUnit
import Test.Tasty.Hedgehog (testProperty)

import Tamal.Engine (State (..), busOut, initState, ringPtrOut)
import Tamal.Loader.Cobs (DecSt, EncSt, cobsDecodeStep, cobsEncodeStep, initDec, initEnc)
import Tamal.Wire.Cobs (cobsEncode)

tests :: TestTree
tests =
  testGroup
    "Loader"
    [ testGroup
        "engine ringPtrOut projection"
        [ testCase "initState projects ringPtr = 1"
            $ ringPtrOut (busOut initState)
            @?= 1
        , testCase "busOut projects the State ringPtr verbatim"
            $ ringPtrOut (busOut initState{ringPtr = 42})
            @?= 42
        , testCase "projects the top-of-ring value"
            $ ringPtrOut (busOut initState{ringPtr = maxBound})
            @?= (maxBound :: Unsigned 12)
        ]
    , decodeTests
    , encodeTests
    ]

-- | A zero-dense byte generator: stresses COBS group boundaries (~1/4 zeros).
genByteZeros :: Gen (BitVector 8)
genByteZeros = Gen.frequency [(1, pure 0), (4, genB)]
 where
  genB = fromIntegral <$> Gen.int (Range.linear 0 255)

{- | Pure driver for the decode step: feed the COBS bytes (delimiter stripped),
then a frame-end pulse; collect the decoded bytes and the malformed flag.
-}
decDrive :: [BitVector 8] -> ([BitVector 8], Bool)
decDrive enc = go initDec ([(Just b, False) | b <- enc] <> [(Nothing, True)])
 where
  go :: DecSt -> [(Maybe (BitVector 8), Bool)] -> ([BitVector 8], Bool)
  go _ [] = ([], False)
  go st (i : is) =
    let (st', (mo, done, bad)) = cobsDecodeStep st i
     in if done
          then ([], bad)
          else let (os, b) = go st' is in (maybe os (: os) mo, b)

decodeTests :: TestTree
decodeTests =
  testGroup
    "Loader.Cobs decode"
    [ testProperty "streaming decode of cobsEncode x reconstructs x" $ property $ do
        x <- forAll (Gen.list (Range.linear 0 300) genByteZeros)
        let (dec, bad) = decDrive (cobsEncode x)
        dec === x
        bad === False
    , testCase "decode reconstructs the [0x00] vector"
        $ decDrive (cobsEncode [0x00])
        @?= ([0x00], False)
    , testCase "decode reconstructs the 254/255 boundary vectors" $ do
        decDrive (cobsEncode (L.map fromIntegral [1 .. 254 :: Int]))
          @?= (L.map fromIntegral [1 .. 254 :: Int], False)
        decDrive (cobsEncode (L.map fromIntegral [1 .. 255 :: Int]))
          @?= (L.map fromIntegral [1 .. 255 :: Int], False)
    , testCase "truncated group -> malformed"
        $ snd (decDrive [0x05, 0x11])
        @?= True
    , testCase "empty frame (delimiter only) -> malformed"
        $ snd (decDrive [])
        @?= True
    , testCase "code byte with no data -> malformed"
        $ snd (decDrive [0x03])
        @?= True
    ]

{- | Pure driver for the encode step: feed logical bytes (last one flagged),
downstream always ready; advance the input only when readyIn (byte consumed).
Collects the emitted COBS bytes (no delimiter).
-}
encDrive :: [BitVector 8] -> [BitVector 8]
encDrive [] = []
encDrive xs = go initEnc [(x, i == L.length xs - 1) | (i, x) <- L.zip [0 ..] xs]
 where
  go :: EncSt -> [(BitVector 8, Bool)] -> [BitVector 8]
  go st inp =
    let (feed, rest0) = case inp of
          (t : ts) -> (Just t, ts)
          [] -> (Nothing, [])
        (st', (readyIn, mo, done)) = cobsEncodeStep st (feed, True)
        inp' = if readyIn then rest0 else inp
     in if done then [] else maybe (go st' inp') (: go st' inp') mo

encodeTests :: TestTree
encodeTests =
  testGroup
    "Loader.Cobs encode"
    [ testProperty "streaming encode equals cobsEncode (non-empty)" $ property $ do
        x <- forAll (Gen.list (Range.linear 1 300) genByteZeros)
        encDrive x === cobsEncode x
    , testCase "encode the [11,00,00,00] vector"
        $ encDrive [0x11, 0x00, 0x00, 0x00]
        @?= [0x02, 0x11, 0x01, 0x01, 0x01]
    , testCase "encode the 254/255 boundary" $ do
        encDrive (L.map fromIntegral [1 .. 254 :: Int])
          @?= (0xFF : L.map fromIntegral [1 .. 254 :: Int])
        encDrive (L.map fromIntegral [1 .. 255 :: Int])
          @?= (0xFF : L.map fromIntegral [1 .. 254 :: Int]) <> [0x02, 255]
    , testProperty "encode then decode round-trips (both streaming)" $ property $ do
        x <- forAll (Gen.list (Range.linear 1 300) genByteZeros)
        decDrive (encDrive x) === (x, False)
    ]
