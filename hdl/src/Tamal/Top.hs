-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0
{-# LANGUAGE NumericUnderscores #-}

{- |
The tamal design minus pin binding (design doc 2026-07-03-tamal-topentity-design.md):
'system' wires the BRAMs, loader, UART, and engine (@mealy stepM initState@) over
plain 'Signal's — no 'BiSignal', so the whole integration is cosim-testable. The
'Tamal' shell binds the clock + 'espiPads' + pins around it. Pure helpers 'stepM',
'ringWrite', 'rigState', and 'ledPattern' are hedgehog-tested.
-}
module Tamal.Top
  ( system
  , stepM
  , ringWrite
  , RigState (..)
  , rigState
  , ledPattern
  ) where

import Clash.Prelude

import Tamal.Bus.Serdes (Lanes)
import Tamal.Engine (BusIn (..), BusOut (..), Ring (..), State, initState, step)

-- | The mealy adapter: re-associates 'step' so it lifts with 'mealy'.
stepM :: State -> BusIn -> (State, (BusOut, Maybe Ring))
stepM = undefined

-- | Project the engine's ring write to the BRAM write-port tuple.
ringWrite :: Maybe Ring -> Maybe (Unsigned 12, BitVector 32)
ringWrite = undefined

-- | Lifecycle state shown on the status LED.
data RigState = Waiting | Running | Done
  deriving stock (Generic, Show, Eq)
  deriving anyclass (NFDataX)

-- | Pure status -> LED level over a free-running counter.
ledPattern :: RigState -> Unsigned 26 -> Bit
ledPattern = undefined

-- | Derive the LED state from the running latch + halted flag.
rigState :: Bool -> Bool -> RigState
rigState = undefined

-- | The whole design minus pin binding.
system ::
  (HiddenClockResetEnable dom) =>
  Signal dom Bit -> -- uart RX line
  Signal dom (Vec 4 Bit) -> -- ioIn
  Signal dom Bit -> -- alertIn
  ( Signal dom Bit -- uart TX line
  , Signal dom Lanes -- lanesOut
  , Signal dom Bit -- csOut
  , Signal dom Bit -- sckOut
  , Signal dom Bit -- rstOut
  , Signal dom Bit -- led
  )
system = undefined
