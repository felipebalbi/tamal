-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0
{-# LANGUAGE NumericUnderscores #-}

module Test.Uart (tests) where

import Clash.Prelude
import qualified Data.List as L
import Test.Tasty
import Test.Tasty.HUnit

import Tamal.Domain (Dom100)
import Tamal.Uart.BaudGen (oversampleTick)

-- | Ticks emitted over the first n system-clock cycles at 2 Mbaud.
baudTicks :: Int -> [Bool]
baudTicks n = sampleN n (oversampleTick (SNat @2_000_000) :: Signal Dom100 Bool)

tests :: TestTree
tests =
  testGroup
    "Uart"
    [ testCase "oversample tick rate is 16x baud (~32 MHz avg)"
        $
        -- Over N cycles expect N * (16*2e6)/100e6 = N * 0.32 ticks.
        let n = 10000
            c = L.length (L.filter id (baudTicks n))
         in assertBool ("tick count = " <> show c <> ", expected ~3200") (abs (c - 3200) <= 2)
    ]
