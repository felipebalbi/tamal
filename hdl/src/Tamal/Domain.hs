-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0
{-# LANGUAGE NumericUnderscores #-}
{-# OPTIONS_GHC -Wno-orphans #-}

{- |
Clock domain for the tamal gateware on the Digilent Arty A7-100T.

The board's 100 MHz oscillator (CLK100MHZ, pin E3) drives the single system
domain. The default 'vSystem' reset (asynchronous, active-high) is kept as-is:
the top entity ties it permanently de-asserted, so it never reaches hardware
and Clash emits no reset port.
-}
module Tamal.Domain where

import Clash.Prelude

-- | 100 MHz system clock of the Arty A7-100T (CLK100MHZ, pin E3).
createDomain
  vSystem
    { vName = "Dom100"
    , vPeriod = hzToPeriod 100_000_000
    }

-- | 50 MHz reference domain for the C5G oscillator (CLOCK_50_B5B, pin R20). It
-- is the input to the Cyclone V PLL ('Tamal.Board.CycloneV'); the PLL multiplies
-- it to the design's 100 MHz 'Dom100'. Asynchronous + ActiveHigh (the 'vSystem'
-- default), which satisfies 'alteraPllSync''s @HasAsynchronousReset@ requirement.
createDomain
  vSystem
    { vName = "DomInput50"
    , vPeriod = hzToPeriod 50_000_000
    }
