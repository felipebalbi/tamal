-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-W-2.0

{- Wrapper around Clash's batch compiler, with the tamal project in scope.

   Generate Verilog with:

       cabal run clash -- Tamal.Board.ArtyA7 --verilog
       cabal run clash -- Tamal.Board.CycloneV --verilog

   The HDL lands in verilog/Tamal.Board.ArtyA7.topEntity/ (or
   verilog/Tamal.Board.CycloneV.topEntity/). This file is taken verbatim
   from the upstream clash-starters projects. -}

import Clash.Main (defaultMain)
import System.Environment (getArgs)
import Prelude

main :: IO ()
main = getArgs >>= defaultMain
