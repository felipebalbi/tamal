-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0

module Main (main) where

import Clash.Prelude (BitVector, pack, unpack)
import Hedgehog (forAll, property, (===))
import Test.Tasty
import Test.Tasty.Hedgehog (testProperty)
import Prelude

import qualified Test.Alu
import qualified Test.Branch
import qualified Test.Config
import qualified Test.Crc
import Test.Gen (genByte)
import qualified Test.Isa
import qualified Test.RegFile
import qualified Test.Serdes
import qualified Test.Trace
import qualified Test.Uart

main :: IO ()
main = defaultMain tests

tests :: TestTree
tests =
  testGroup
    "tamal"
    [ testGroup
        "smoke"
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
    , Test.Alu.tests
    , Test.RegFile.tests
    , Test.Uart.tests
    ]
