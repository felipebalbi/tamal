-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0

module Test.Branch (tests) where

import Clash.Prelude
import Hedgehog (forAll, property, (===))
import qualified Hedgehog.Gen as Gen
import Test.Tasty
import Test.Tasty.HUnit
import Test.Tasty.Hedgehog (testProperty)

import Tamal.Branch
import Test.Gen (genWord)

-- Independent reference for the four comparisons.
ref :: BranchOp -> BitVector 32 -> BitVector 32 -> Bool
ref Beq a b = a == b
ref Bne a b = a /= b
ref Bltu a b = a < b
ref Bgeu a b = a >= b

tests :: TestTree
tests =
  testGroup
    "Branch"
    [ testProperty "branchTaken matches reference (all ops)" $ property $ do
        op <- forAll (Gen.element [minBound .. maxBound])
        a <- forAll genWord
        b <- forAll genWord
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
    , testCase "Bltu is unsigned: 0x7FFFFFFF < 0x80000000"
        $ branchTaken Bltu 0x7FFFFFFF 0x80000000
        @?= True
    , testCase "Bgeu is unsigned: 0xFFFFFFFF >= 0x00000000"
        $ branchTaken Bgeu 0xFFFFFFFF 0x00000000
        @?= True
    , testCase "Bltu is unsigned: 0x80000000 < 0x7FFFFFFF is False"
        $ branchTaken Bltu 0x80000000 0x7FFFFFFF
        @?= False
    ]
