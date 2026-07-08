-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-W-2.0
{-# LANGUAGE NumericUnderscores #-}
{-# OPTIONS_GHC -Wno-orphans #-}

{- |
Clock domains for the tamal gateware, shared by both board targets.

'Dom100' is the 100 MHz system domain the whole design runs on: on the Digilent
Arty A7-100T it is the board oscillator directly (CLK100MHZ, pin E3); on the
Terasic Cyclone V GX Starter Kit it is the output of an Altera PLL
('Tamal.Board.CycloneV'). 'DomInput50' is the C5G's 50 MHz PLL-input domain
(CLOCK_50_B5B, pin R20).

The default 'vSystem' reset (asynchronous, active-high) is kept as-is: the board
shells tie it permanently de-asserted, so it never reaches hardware and Clash
emits no reset port.
-}
module Tamal.Domain where

import Clash.Prelude

-- | 100 MHz system clock of the Arty A7-100T (CLK100MHZ, pin E3).
createDomain
  vSystem
    { vName = "Dom100"
    , vPeriod = hzToPeriod 100_000_000
    }

{- | 50 MHz reference domain for the C5G oscillator (CLOCK_50_B5B, pin R20). It
is the input to the Cyclone V PLL ('Tamal.Board.CycloneV'); the PLL multiplies
it to the design's 100 MHz 'Dom100'. Asynchronous + ActiveHigh (the 'vSystem'
default), which satisfies 'alteraPllSync''s @HasAsynchronousReset@ requirement.
-}
createDomain
  vSystem
    { vName = "DomInput50"
    , vPeriod = hzToPeriod 50_000_000
    }
