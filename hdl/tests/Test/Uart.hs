-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0
{-# LANGUAGE NumericUnderscores #-}

module Test.Uart (tests) where

import Clash.Prelude
import qualified Data.List as L
import Hedgehog (forAll, property, (===))
import Test.Tasty
import Test.Tasty.HUnit
import Test.Tasty.Hedgehog (testProperty)

import Tamal.Domain (Dom100)
import Tamal.Uart (uart)
import Tamal.Uart.BaudGen (oversampleTick)
import Tamal.Uart.Rx (uartRx)
import Tamal.Uart.Tx (uartTx)
import Test.Gen (genByte)

-- | Ticks emitted over the first n system-clock cycles at 2 Mbaud.
baudTicks :: Int -> [Bool]
baudTicks n = sampleN n (oversampleTick (SNat @2_000_000) :: Signal Dom100 Bool)

-- | One line sample per bit-cell position; LSB-first data. 16 samples per bit.
bitAt :: BitVector 8 -> Int -> Bit
bitAt b i = if testBit b i then high else low

-- | Full 8N1 line waveform (16 samples/bit): start, 8 data LSB-first, stop.
frame :: BitVector 8 -> Bit -> [Bit]
frame b stop =
  L.replicate 16 low
    <> L.concatMap (\i -> L.replicate 16 (bitAt b i)) [0 .. 7]
    <> L.replicate 16 stop

{- | A data-bit cell that carries its true value only in the /center/ (sample
offsets 6..10) and the complement at the edges. A receiver that samples the bit
center recovers the value; one that samples the bit boundary (e.g. a half-bit
early @rxCnt@ init after the start edge) recovers the complement.
-}
centerCell :: Bit -> [Bit]
centerCell v = [if 6 <= j && j <= 10 then v else complement v | j <- [0 .. 15 :: Int]]

{- | An 8N1 frame whose data cells only reveal the byte when sampled at the bit
center (see 'centerCell'): start (all low), 8 center-weighted data bits LSB-first,
stop (high). Used to pin center sampling against a boundary-sampling regression.
-}
centerFrame :: BitVector 8 -> [Bit]
centerFrame b =
  L.replicate 16 low
    <> L.concatMap (centerCell . bitAt b) [0 .. 7]
    <> L.replicate 16 high

-- | Flip the sample at index k (glitch injection).
flipAt :: Int -> [Bit] -> [Bit]
flipAt k xs = [if j == k then complement x else x | (j, x) <- L.zip ([0 ..] :: [Int]) xs]

-- | Drive uartRx with tick = always-true and a crafted line; collect (byte, err).
runRx :: [Bit] -> [(Maybe (BitVector 8), Bool)]
runRx samples =
  sampleN
    (8 + L.length samples + 24)
    (bundle (uartRx (pure True) lineSig) :: Signal Dom100 (Maybe (BitVector 8), Bool))
 where
  lineSig = fromList (L.replicate 8 high <> samples <> L.repeat high)

recovered :: [(Maybe (BitVector 8), Bool)] -> [BitVector 8]
recovered xs = [b | (Just b, _) <- xs]

anyErr :: [(Maybe (BitVector 8), Bool)] -> Bool
anyErr xs = L.or [e | (_, e) <- xs]

-- | TX line fed straight into RX, both at tick = always-true (16 cycles/bit).
fastLoop ::
  (HiddenClockResetEnable dom) =>
  Signal dom (Maybe (BitVector 8)) ->
  Signal dom (Maybe (BitVector 8))
fastLoop txByte = rxByte
 where
  tick = pure True
  (txLine, _txReady) = uartTx tick txByte
  (rxByte, _rxErr) = uartRx tick txLine

runFastLoop :: [Maybe (BitVector 8)] -> Int -> [Maybe (BitVector 8)]
runFastLoop ins n =
  sampleN n (fastLoop (fromList (ins <> L.repeat Nothing)) :: Signal Dom100 (Maybe (BitVector 8)))

-- | Full UART with the real NCO tick (50 cycles/bit at 2 Mbaud); TX line looped to RX.
fullLoop ::
  (HiddenClockResetEnable dom) =>
  Signal dom (Maybe (BitVector 8)) ->
  Signal dom (Maybe (BitVector 8))
fullLoop txByte = rxByte
 where
  (rxByte, _err, txLine, _rdy) = uart (SNat @2_000_000) rxLine txByte
  rxLine = txLine

runFullLoop :: [Maybe (BitVector 8)] -> Int -> [Maybe (BitVector 8)]
runFullLoop ins n =
  sampleN n (fullLoop (fromList (ins <> L.repeat Nothing)) :: Signal Dom100 (Maybe (BitVector 8)))

tests :: TestTree
tests =
  testGroup
    "Uart"
    [ testCase "oversample tick rate is 16x baud (~32 MHz avg)"
        $
        -- Over N cycles expect N * (16*2e6)/100e6 = N * 0.32 ticks.
        let n = 10000
            c = L.length (L.filter id (baudTicks n))
         in assertBool ("tick count = " <> show c <> ", expected ~3200") (abs (c - 3200) <= 2)
    , testProperty "RX decodes a clean 8N1 frame" $ property $ do
        b <- forAll genByte
        let out = runRx (frame b high)
        recovered out === [b]
        anyErr out === False
    , testProperty "RX flags a framing error on a low stop bit" $ property $ do
        b <- forAll genByte
        let out = runRx (frame b low)
        recovered out === []
        anyErr out === True
    , testProperty "RX majority vote rejects a single glitch" $ property $ do
        b <- forAll genByte
        let out = runRx (flipAt (16 + 3 * 16 + 8) (frame b high))
        recovered out === [b]
    , testProperty "RX samples the bit center, not the edge" $ property $ do
        -- Regression guard: after the start edge the sample point must land at
        -- each data bit's center (offsets 7/8/9), not its leading boundary. A
        -- half-bit-early rxCnt init recovers the complement and fails here,
        -- while the constant-cell tests above cannot tell center from edge.
        b <- forAll genByte
        let out = runRx (centerFrame b)
        recovered out === [b]
        anyErr out === False
    , testProperty "TX->RX fast loopback recovers the byte" $ property $ do
        b <- forAll genByte
        let out = runFastLoop [Nothing, Just b] 400
        [b' | Just b' <- out] === [b]
    , testProperty "TX ignores input while busy (ready gating)" $ property $ do
        b <- forAll genByte
        let out = runFastLoop (Nothing : L.replicate 100 (Just b)) 400
        [b' | Just b' <- out] === [b]
    , testProperty "full UART loopback (real NCO) recovers the byte" $ property $ do
        b <- forAll genByte
        let out = runFullLoop (Nothing : L.replicate 60 (Just b)) 1200
        [b' | Just b' <- out] === [b]
    ]
