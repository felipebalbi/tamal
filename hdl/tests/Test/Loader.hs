-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0
{-# LANGUAGE NumericUnderscores #-}

module Test.Loader (tests) where

import Clash.Prelude
import Test.Tasty
import Test.Tasty.HUnit

import Tamal.Engine (State (..), busOut, initState, ringPtrOut)

tests :: TestTree
tests =
  testGroup
    "Loader"
    [ testGroup
        "engine ringPtrOut projection"
        [ testCase "initState projects ringPtr = 1"
            $ ringPtrOut (busOut initState)
            @?= 1
        , testCase "busOut projects the State ringPtr verbatim"
            $ ringPtrOut (busOut initState{ringPtr = 42})
            @?= 42
        , testCase "projects the top-of-ring value"
            $ ringPtrOut (busOut initState{ringPtr = maxBound})
            @?= (maxBound :: Unsigned 12)
        ]
    ]
