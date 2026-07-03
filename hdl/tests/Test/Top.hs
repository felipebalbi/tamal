-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0
{-# LANGUAGE NumericUnderscores #-}

module Test.Top (tests) where

import Clash.Prelude
import qualified Hedgehog as H
import qualified Hedgehog.Gen as Gen
import qualified Hedgehog.Range as Range
import Test.Tasty
import Test.Tasty.HUnit
import Test.Tasty.Hedgehog (testProperty)

import Tamal.Engine (BusIn (..), Ring (..), initState, step)
import Tamal.Top (RigState (..), ledPattern, rigState, ringWrite, stepM)
import Test.Gen (genBit, genWord)

-- | A random BusIn: instr word, four sampled IO bits, ALERT#, start.
genBusIn :: H.Gen BusIn
genBusIn =
  BusIn
    <$> genWord
    <*> ((\a b c d -> a :> b :> c :> d :> Nil) <$> genBit <*> genBit <*> genBit <*> genBit)
    <*> genBit
    <*> Gen.bool

tests :: TestTree
tests =
  testGroup
    "Top"
    [ -- stepM: re-associates step, nothing more
      testProperty "stepM = step re-associated" $ H.property $ do
        i <- H.forAll genBusIn
        let (s', bo, mr) = step initState i
        stepM initState i H.=== (s', (bo, mr))
    , -- ringWrite: unwrap the Ring record to the BRAM tuple
      testCase "ringWrite Nothing = Nothing"
        $ ringWrite Nothing
        @?= Nothing
    , testProperty "ringWrite (Just Ring) = Just (addr,data)" $ H.property $ do
        a <- H.forAll (fromIntegral <$> Gen.int (Range.linear 0 4095))
        d <- H.forAll genWord
        ringWrite (Just (Ring a d)) H.=== Just (a, d)
    , -- rigState truth table
      testCase "rigState: halted -> Done (regardless of running)" $ do
        rigState False True @?= Done
        rigState True True @?= Done
    , testCase "rigState: running & not halted -> Running"
        $ rigState True False
        @?= Running
    , testCase "rigState: idle -> Waiting"
        $ rigState False False
        @?= Waiting
    , -- ledPattern: Done solid; Running faster than Waiting
      testCase "ledPattern Done is solid on" $ do
        ledPattern Done 0 @?= high
        ledPattern Done maxBound @?= high
    , testCase "ledPattern Waiting toggles on bit 25 (slow)" $ do
        ledPattern Waiting 0 @?= low
        ledPattern Waiting 0x2000000 @?= high -- 2^25
    , testCase "ledPattern Running toggles on bit 22 (faster than Waiting)" $ do
        ledPattern Running 0 @?= low
        ledPattern Running 0x400000 @?= high -- 2^22
        ledPattern Waiting 0x400000 @?= low -- same count: Waiting still low => Running is faster
    ]
