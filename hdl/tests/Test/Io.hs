-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0

module Test.Io (tests) where

import Clash.Prelude
import qualified Data.List as L
import qualified Hedgehog as H
import qualified Hedgehog.Gen as Gen
import qualified Hedgehog.Range as Range
import Test.Tasty
import Test.Tasty.HUnit
import Test.Tasty.Hedgehog (testProperty)

import Tamal.Domain (Dom100)
import Tamal.Io (alertSync)

{- | Reference: a 2-flop synchronizer delays @raw@ by exactly two cycles, led by
two idle-high samples (the init-high flops before the input propagates). With
'simAlert' leading the stream by one idle cycle (below), the clean model holds:
@1 : 1 : xs@.
-}
refAlert :: [Bit] -> [Bit]
refAlert xs = L.take (L.length xs) (1 : 1 : xs)

{- | Sample 'alertSync' over a raw ALERT# stream. 'sampleN' asserts the Dom100
async reset on cycle 0, which swallows the first input sample and adds a lead
reset sample (see @hdl/PLAN.md@). So we feed one throwaway idle-high cycle up
front and 'L.drop 1' it — the repo's standard @sampleN@ idiom — leaving the real
stream aligned to the clean 'refAlert' model.
-}
simAlert :: [Bit] -> [Bit]
simAlert xs =
  L.drop 1
    $ sampleN
      (L.length xs + 1)
      (alertSync (fromList (1 : xs <> L.repeat 1)) :: Signal Dom100 Bit)

-- | A 0/1 'Bit' generator.
genBit :: H.Gen Bit
genBit = Gen.element [0, 1]

tests :: TestTree
tests =
  testGroup
    "Io"
    [ testCase "alertSync: 2-cycle lag, idle-high lead"
        $ simAlert [1, 1, 0, 0, 0, 1, 1]
        @?= [1, 1, 1, 1, 0, 0, 0]
    , testCase "alertSync: single-cycle assertion survives"
        $ simAlert [1, 1, 0, 1, 1, 1]
        @?= [1, 1, 1, 1, 0, 1]
    , testProperty "alertSync: matches the 2-cycle-delay model" $ H.property $ do
        xs <- H.forAll (Gen.list (Range.linear 0 48) genBit)
        simAlert xs H.=== refAlert xs
    ]
