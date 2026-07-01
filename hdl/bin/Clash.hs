-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0

{- Wrapper around Clash's batch compiler, with the tamal project in scope.

   Generate Verilog with:

       stack run clash -- Tamal --verilog

   The HDL lands in verilog/Tamal.topEntity/. This file is taken verbatim
   from the upstream clash-starters projects. -}

import Clash.Main (defaultMain)
import System.Environment (getArgs)
import Prelude

main :: IO ()
main = getArgs >>= defaultMain
