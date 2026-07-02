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

{- | Receiver FSM phase: idle (waiting for the start edge), confirming the start
bit, shifting in data bit @i@ (0..7), or checking the stop bit.
-}
data RxState = RxIdle | RxStart | RxData (Index 8) | RxStop
  deriving stock (Generic, Show, Eq)
  deriving anyclass (NFDataX)

-- | Everything the receiver must carry from one oversample tick to the next.
data RxS = RxS
  { rxState :: RxState
  -- ^ current frame phase
  , rxCnt :: Index 16
  -- ^ tick position within the current bit window (0..15)
  , rxShift :: BitVector 8
  -- ^ data bits assembled so far (LSB-first)
  , rxS7 :: Bit
  -- ^ line captured at tick 7 (for the majority vote)
  , rxS8 :: Bit
  -- ^ line captured at tick 8
  , rxS9 :: Bit
  -- ^ line captured at tick 9
  }
  deriving stock (Generic, Show, Eq)
  deriving anyclass (NFDataX)

{- | The receiver. Consumes the oversample @tick@ enable and the asynchronous RX
@line@, and yields two one-cycle strobes: @Just byte@ when a frame completes, and
a framing-error flag when a frame ends without a valid (high) stop bit.

The line is registered twice ('sync1' then 'synced') — a 2-flop synchronizer
against metastability — before the tick-gated FSM ('rxStep') ever looks at it.
-}
uartRx ::
  (HiddenClockResetEnable dom) =>
  Signal dom Bool ->
  Signal dom Bit ->
  (Signal dom (Maybe (BitVector 8)), Signal dom Bool)
uartRx tick rxLine = unbundle (mealy rxStep initRx (bundle (tick, synced)))
 where
  -- 2-flop synchronizer, clocked every cycle (not tick-gated); idle line is high.
  sync1 = register high rxLine
  synced = register high sync1
  initRx = RxS RxIdle 0 0 0 0 0

-- | Majority vote of three bits: high iff at least two of the three are high.
maj :: Bit -> Bit -> Bit -> Bit
maj a b c = (a .&. b) .|. (a .&. c) .|. (b .&. c)

{- | Stash the current line level into the tick-7/8/9 sample slots at the
matching counts; a no-op at every other count.
-}
captureSample :: RxS -> Bit -> RxS
captureSample s bit' = case rxCnt s of
  7 -> s{rxS7 = bit'}
  8 -> s{rxS8 = bit'}
  9 -> s{rxS9 = bit'}
  _ -> s -- actually unreachable

{- | Resolve a completed bit (called at count 15, once the three samples are in
hand) into the next state and any output. Votes the samples into @bit'@, then:
confirm or reject the start bit, shift a data bit in LSB-first, or test the stop
bit — emitting @Just byte@ on a good frame or flagging a framing error and
dropping the byte on a bad one. Always returns to 'RxIdle' from 'RxStop'.
-}
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

{- | One receiver step per oversample tick (the Mealy transition). Between ticks
the state is frozen. On a tick: in 'RxIdle', watch the synced line for the
falling start edge; otherwise capture a sample and, at the end of the bit window
(count 15 = 'maxBound'), hand off to 'decideBit', else just advance the counter.
-}
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
