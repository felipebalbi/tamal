-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0

module Test.Wire (tests) where

import Clash.Prelude
import Hedgehog (forAll, property, (===))
import Test.Tasty
import Test.Tasty.HUnit
import Test.Tasty.Hedgehog (testProperty)

import Tamal.Wire
import Test.Gen (genWord)

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
    ]
