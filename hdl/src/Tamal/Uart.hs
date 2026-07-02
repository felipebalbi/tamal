-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0
{-# LANGUAGE NumericUnderscores #-}

{- |
UART top: shares one NCO oversample tick into the receiver and transmitter.
The topEntity calls @uart (SNat \@2_000_000)@ and wires the pins. See the UART
design, section 8.
-}
module Tamal.Uart
  ( uart
  ) where

import Clash.Prelude

import Tamal.Uart.BaudGen (oversampleTick)
import Tamal.Uart.Rx (uartRx)
import Tamal.Uart.Tx (uartTx)

uart ::
  forall baud dom.
  (HiddenClockResetEnable dom, KnownDomain dom, KnownNat baud) =>
  SNat baud ->
  Signal dom Bit ->
  Signal dom (Maybe (BitVector 8)) ->
  ( Signal dom (Maybe (BitVector 8))
  , Signal dom Bool
  , Signal dom Bit
  , Signal dom Bool
  )
uart baud rxLine txByte = (rxByte, rxErr, txLine, txReady)
 where
  tick = oversampleTick baud
  (rxByte, rxErr) = uartRx tick rxLine
  (txLine, txReady) = uartTx tick txByte
