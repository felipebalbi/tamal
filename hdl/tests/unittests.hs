module Main (main) where

import Prelude
import Test.Tasty
import Test.Tasty.HUnit

-- Placeholder tasty suite so `stack test` / `make test` is wired up from the
-- start. Real component tests (instruction encoder, assembler, eSPI cycle
-- decode, verdict engine) replace this as the gateware grows.
main :: IO ()
main = defaultMain tests

tests :: TestTree
tests =
        testGroup
                "tamal"
                [ testCase "scaffold placeholder" (True @?= True)
                ]
