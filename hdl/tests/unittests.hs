-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0

module Main (main) where

import Prelude
import Clash.Prelude (pack, unpack, BitVector)
import Test.Tasty
import Test.Tasty.Hedgehog (testProperty)
import Hedgehog (property, forAll, (===))

import Test.Gen (genByte)
import qualified Test.Crc
import qualified Test.Isa
import qualified Test.Config
import qualified Test.Serdes
import qualified Test.Trace
import qualified Test.Branch

main :: IO ()
main = defaultMain tests

tests :: TestTree
tests =
  testGroup "tamal"
    [ testGroup "smoke"
        [ testProperty "pack/unpack byte round-trips" $ property $ do
            b <- forAll genByte
            unpack (pack b) === (b :: BitVector 8)
        ]
    , Test.Crc.tests
    , Test.Isa.tests
    , Test.Config.tests
    , Test.Serdes.tests
    , Test.Trace.tests
    , Test.Branch.tests
    ]
