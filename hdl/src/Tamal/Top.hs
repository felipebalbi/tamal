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
import Tamal.Engine
  ( BusIn (..)
  , BusOut (..)
  , Ring (..)
  , State
  , initState
  , step
  )
import Tamal.Loader
  ( LoaderIn (..)
  , LoaderOut (..)
  , loader
  )
import Tamal.Mem
  ( instrRam
  , ringRam
  )
import Tamal.Params (RW)
import Tamal.Uart (uart)

-- | The mealy adapter: re-associates 'step' so it lifts with 'mealy'.
stepM :: State -> BusIn -> (State, (BusOut, Maybe Ring))
stepM s i = (s', (bo, mr))
 where
  (s', bo, mr) = step s i

-- | Project the engine's ring write to the BRAM write-port tuple.
ringWrite :: Maybe Ring -> Maybe (Unsigned RW, BitVector 32)
ringWrite = fmap (\(Ring a d) -> (a, d))

-- | Lifecycle state shown on the status LED.
data RigState = Waiting | Running | Done
  deriving stock (Generic, Show, Eq)
  deriving anyclass (NFDataX)

-- | Pure status -> LED level over a free-running counter.
ledPattern :: RigState -> Unsigned 26 -> Bit
ledPattern Waiting c = msb c
ledPattern Running c = msb (c `shiftL` 3)
ledPattern Done _ = high

-- | Derive the LED state from the running latch + halted flag.
rigState :: Bool -> Bool -> RigState
rigState _ True = Done
rigState True False = Running
rigState False False = Waiting

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
system rxLine ioIn alertIn = (txLine, lanesO, csO, sckO, rstO, ledOut)
 where
  -- UART @ 2MBaud
  (rxByte, _rxErr, txLine, txReady) = uart (SNat @2_000_000) rxLine txByteL

  -- Loader FSM
  lOut = loader (LoaderIn <$> rxByte <*> txReady <*> halted <*> ringPtrO <*> ringData)
  txByteL = txByte <$> lOut
  instrWrL = instrWr <$> lOut
  ringAddrL = ringAddr <$> lOut
  startO = startOut <$> lOut

  -- Memories
  instrWord = instrRam pcO instrWrL
  ringData = ringRam ringAddrL (ringWrite <$> maybeRing)

  -- Engine
  (busOut, maybeRing) = unbundle (mealy stepM initState busInS)
  busInS = BusIn <$> instrWord <*> ioIn <*> alertIn <*> startO
  pcO = pcOut <$> busOut
  lanesO = lanesOut <$> busOut
  csO = csOut <$> busOut
  sckO = sckOut <$> busOut
  rstO = rstOut <$> busOut
  halted = haltedOut <$> busOut
  ringPtrO = ringPtrOut <$> busOut

  -- Status LED
  running = register False (mux startO (pure True) (mux halted (pure False) running))
  ledCnt = register (0 :: Unsigned 26) (ledCnt + 1)
  ledOut = ledPattern <$> (rigState <$> running <*> halted) <*> ledCnt
