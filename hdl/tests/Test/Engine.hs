-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0

module Test.Engine (tests) where

import Clash.Prelude
import Test.Tasty
import Test.Tasty.HUnit

import Tamal.Bus.Serdes (hiZ)
import Tamal.Engine

tests :: TestTree
tests =
  testGroup
    "Engine"
    [ testCase "initState: Idle, pins safe" $ do
        phase initState @?= Idle
        csN initState @?= 1
        sck initState @?= 0
        rstN initState @?= 1
        lanes initState @?= hiZ
    , testCase "Idle without start stays Idle"
        $ let (s', _, r) = step initState (BusIn 0 (repeat 0) 0 False)
           in (phase s', r) @?= (Idle, Nothing)
    , testCase "start soft-inits into Preamble"
        $ let (s', _, _) = step initState (BusIn 0 (repeat 0) 0 True)
           in phase s' @?= Preamble
    ]
