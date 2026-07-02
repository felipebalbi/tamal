-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0

{- |
8N1 UART transmitter. Line idles high; each bit is held for 16 oversample ticks.
Accepts a byte when idle (ready high) via a one-cycle handshake. See the UART
design, section 7.
-}
module Tamal.Uart.Tx
  ( uartTx
  ) where

import Clash.Prelude

data TxState = TxIdle | TxStart | TxData (Index 8) | TxStop
  deriving stock (Generic, Show, Eq)
  deriving anyclass (NFDataX)

data TxS = TxS
  { txState :: TxState
  , txShift :: BitVector 8
  , txCnt :: Index 16
  }
  deriving stock (Generic, Show, Eq)
  deriving anyclass (NFDataX)

uartTx ::
  (HiddenClockResetEnable dom) =>
  Signal dom Bool ->
  Signal dom (Maybe (BitVector 8)) ->
  (Signal dom Bit, Signal dom Bool)
uartTx tick mbyte = unbundle (mealy txStep initTx (bundle (tick, mbyte)))
 where
  initTx :: TxS
  initTx = TxS TxIdle 0 0

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

txAdvance :: TxS -> TxS
txAdvance s = case txState s of
  TxStart -> s{txState = TxData 0, txCnt = 0}
  TxData i
    | i == maxBound -> s{txState = TxStop, txCnt = 0}
    | otherwise -> s{txState = TxData (i + 1), txShift = txShift s `shiftR` 1, txCnt = 0}
  TxStop -> s{txState = TxIdle, txCnt = 0}
  TxIdle -> s
