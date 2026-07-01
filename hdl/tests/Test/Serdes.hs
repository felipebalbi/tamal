-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0
{-# LANGUAGE NumericUnderscores #-}

module Test.Serdes (tests) where

import Clash.Prelude
import Hedgehog (forAll, property, (===))
import Test.Tasty
import Test.Tasty.HUnit
import Test.Tasty.Hedgehog (testProperty)

import Tamal.Bus.Serdes
import Test.Gen (genByte)

-- loopback: sample IO[0] (the driven lane) back as if it were IO[1]
io0 :: Lanes -> Bit
io0 lanes = fst (lanes !! (0 :: Index 4))

tests :: TestTree
tests =
  testGroup
    "Serdes"
    [ testProperty "x1 serialize/deserialize round-trips (loopback)" $ property $ do
        b <- forAll genByte
        deserializeX1 (map io0 (serializeX1 b)) === b
    , testCase "x1 serialize drives IO[0] MSB-first, tri-states IO[1..3]" $ do
        let beats = serializeX1 0b1000_0000 -- MSB set only
        io0 (head beats) @?= 1 -- first beat carries the MSB
        io0 (last beats) @?= 0
        -- IO[1] output-enable is 0 (tri-stated) on every beat
        map (\l -> snd (l !! (1 :: Index 4))) beats @?= repeat 0
    , testCase "TAR beat 0 drives all lanes high; later beats tri-state" $ do
        tarBeat 0 @?= repeat (1, 1)
        tarBeat 1 @?= repeat (0, 0)
        tarBeat 5 @?= repeat (0, 0)
    ]
