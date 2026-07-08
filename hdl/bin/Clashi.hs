-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-W-2.0

{- Wrapper around Clash's interactive REPL (clashi), with the tamal
   project in scope. Start it with:

       cabal run clashi

   This file is taken verbatim from the upstream clash-starters projects. -}

import Clash.Main (defaultMain)
import System.Environment (getArgs)
import Prelude

main :: IO ()
main = getArgs >>= defaultMain . ("--interactive" :)
