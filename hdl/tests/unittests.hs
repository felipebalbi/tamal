module Main (main) where

import Prelude
import Clash.Prelude (pack, unpack, BitVector)
import Test.Tasty
import Test.Tasty.Hedgehog (testProperty)
import Hedgehog (property, forAll, (===))

import Test.Gen (genByte)

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
    ]
