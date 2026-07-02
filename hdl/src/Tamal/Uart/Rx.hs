-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0

{- |
8N1 UART receiver. Runs one FSM step per oversample tick; a 2-flop synchronizer
guards the async line. Each bit is 16 ticks; the bit is decided by a 3-sample
majority vote (ticks 7/8/9). See the UART design, section 6.
-}
module Tamal.Uart.Rx
  ( uartRx
  ) where

import Clash.Prelude

data RxState = RxIdle | RxStart | RxData (Index 8) | RxStop
  deriving stock (Generic, Show, Eq)
  deriving anyclass (NFDataX)

data RxS = RxS
  { rxState :: RxState
  , rxCnt :: Index 16
  , rxShift :: BitVector 8
  , rxS7 :: Bit
  , rxS8 :: Bit
  , rxS9 :: Bit
  }
  deriving stock (Generic, Show, Eq)
  deriving anyclass (NFDataX)

uartRx ::
  (HiddenClockResetEnable dom) =>
  Signal dom Bool ->
  Signal dom Bit ->
  (Signal dom (Maybe (BitVector 8)), Signal dom Bool)
uartRx tick rxLine = unbundle (mealy rxStep initRx (bundle (tick, synced)))
 where
  sync1 = register high rxLine
  synced = register high sync1
  initRx = RxS RxIdle 0 0 0 0 0

maj :: Bit -> Bit -> Bit -> Bit
maj a b c = (a .&. b) .|. (a .&. c) .|. (b .&. c)

captureSample :: RxS -> Bit -> RxS
captureSample s bit' = case rxCnt s of
  7 -> s{rxS7 = bit'}
  8 -> s{rxS8 = bit'}
  9 -> s{rxS9 = bit'}
  _ -> s -- actually unreachable

decideBit :: RxS -> (RxS, (Maybe (BitVector 8), Bool))
decideBit s = case rxState s of
  RxStart
    | bit' == low -> (s{rxState = RxData 0, rxCnt = 0}, (Nothing, False))
    | otherwise -> (s{rxState = RxIdle, rxCnt = 0}, (Nothing, False))
  RxData i ->
    let sh =
          if bit' == high
            then setBit (rxShift s) (fromEnum i)
            else clearBit (rxShift s) (fromEnum i)
        s' = s{rxShift = sh, rxCnt = 0}
     in if i == maxBound
          then (s'{rxState = RxStop}, (Nothing, False))
          else (s'{rxState = RxData (i + 1)}, (Nothing, False))
  RxStop
    | bit' == high -> (s{rxState = RxIdle, rxCnt = 0}, (Just (rxShift s), False))
    | otherwise -> (s{rxState = RxIdle, rxCnt = 0}, (Nothing, True))
  RxIdle -> (s, (Nothing, False)) -- actually unreachable
 where
  bit' = maj (rxS7 s) (rxS8 s) (rxS9 s)

rxStep :: RxS -> (Bool, Bit) -> (RxS, (Maybe (BitVector 8), Bool))
rxStep s (tick, line)
  | not tick = (s, (Nothing, False)) -- only move on oversample ticks
  | otherwise = case rxState s of
      RxIdle
        | line == low -> (s{rxState = RxStart, rxCnt = 0}, (Nothing, False))
        | otherwise -> (s, (Nothing, False))
      _ ->
        -- RxStart / RxData / RxStop
        let s1 = captureSample s line
         in if rxCnt s == maxBound
              then decideBit s1
              else (s1{rxCnt = rxCnt s + 1}, (Nothing, False))
