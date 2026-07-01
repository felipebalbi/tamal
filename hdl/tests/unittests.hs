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
    ]
