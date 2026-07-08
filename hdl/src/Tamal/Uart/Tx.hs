-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-W-2.0

{- |
8N1 UART transmitter. Line idles high; each bit is held for 16 oversample ticks.
Accepts a byte when idle (ready high) via a one-cycle handshake. See the UART
design, section 7.
-}
module Tamal.Uart.Tx
  ( uartTx
  ) where

import Clash.Prelude

{- | Transmitter FSM phase: idle, driving the start bit, driving data bit @i@
(0..7), or driving the stop bit.
-}
data TxState = TxIdle | TxStart | TxData (Index 8) | TxStop
  deriving stock (Generic, Show, Eq)
  deriving anyclass (NFDataX)

-- | Everything the transmitter must carry from one oversample tick to the next.
data TxS = TxS
  { txState :: TxState
  -- ^ current frame phase
  , txShift :: BitVector 8
  -- ^ latched byte, shifted right so 'lsb' walks LSB-first
  , txCnt :: Index 16
  -- ^ tick position within the current bit (0..15)
  }
  deriving stock (Generic, Show, Eq)
  deriving anyclass (NFDataX)

{- | The transmitter. Consumes the oversample @tick@ enable and a @Just byte@
request; drives the serial @line@ (idles high) and a @ready@ flag (high only when
idle). The caller presents @Just byte@ on a cycle when @ready@ is high and the
byte is latched and serialised; requests offered mid-frame are ignored.
-}
uartTx ::
  (HiddenClockResetEnable dom) =>
  Signal dom Bool ->
  Signal dom (Maybe (BitVector 8)) ->
  (Signal dom Bit, Signal dom Bool)
uartTx tick mbyte = unbundle (mealy txStep initTx (bundle (tick, mbyte)))
 where
  initTx :: TxS
  initTx = TxS TxIdle 0 0

{- | One transmitter step (the Mealy transition). @line@ and @ready@ are Moore
outputs — pure functions of the /current/ state, valid every cycle. The next
state accepts a byte immediately when idle (not tick-gated: that is the
handshake), and otherwise advances one bit every 16 ticks via 'txAdvance'.
-}
txStep :: TxS -> (Bool, Maybe (BitVector 8)) -> (TxS, (Bit, Bool))
txStep s (tick, mbyte) = (s', (line, ready))
 where
  ready = txState s == TxIdle
  line = case txState s of
    TxIdle -> high -- idle line is high
    TxStart -> low -- start bit
    TxData _ -> lsb (txShift s) -- current data bit, LSB-first
    TxStop -> high -- stop bit
  s'
    | TxIdle <- txState s = case mbyte of
        Just b -> s{txState = TxStart, txShift = b, txCnt = 0}
        Nothing -> s
    | not tick = s
    | txCnt s /= maxBound = s{txCnt = txCnt s + 1}
    | otherwise = txAdvance s

{- | Advance to the next frame phase at the end of a bit (called when the tick
counter wraps): start → data 0 → … → data 7 → stop → idle. Between data bits the
latched byte is shifted right so 'lsb' presents the next bit.
-}
txAdvance :: TxS -> TxS
txAdvance s = case txState s of
  TxStart -> s{txState = TxData 0, txCnt = 0}
  TxData i
    | i == maxBound -> s{txState = TxStop, txCnt = 0}
    | otherwise -> s{txState = TxData (i + 1), txShift = txShift s `shiftR` 1, txCnt = 0}
  TxStop -> s{txState = TxIdle, txCnt = 0}
  TxIdle -> s
