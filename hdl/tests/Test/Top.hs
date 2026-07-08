-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-W-2.0
{-# LANGUAGE NumericUnderscores #-}

module Test.Top (tests) where

import Clash.Prelude
import qualified Data.List as L
import qualified Hedgehog as H
import qualified Hedgehog.Gen as Gen
import qualified Hedgehog.Range as Range
import Test.Tasty
import Test.Tasty.HUnit
import Test.Tasty.Hedgehog (testProperty)

import Tamal.Bus.Serdes (Lanes)
import Tamal.Domain (Dom100)
import Tamal.Engine (BusIn (..), Ring (..), initState, step)
import Tamal.Isa (Instr (..), encode)
import Tamal.Top (RigState (..), ledPattern, rigState, ringWrite, stepM, system)
import Tamal.Uart.BaudGen (oversampleTick)
import Tamal.Uart.Rx (uartRx)
import Tamal.Wire (ControlMsg (..), decodeResult, encodeControl)
import Test.Gen (genBit, genWord)

-- | A random BusIn: instr word, four sampled IO bits, ALERT#, start.
genBusIn :: H.Gen BusIn
genBusIn =
  BusIn
    <$> genWord
    <*> ((\a b c d -> a :> b :> c :> d :> Nil) <$> genBit <*> genBit <*> genBit <*> genBit)
    <*> genBit
    <*> Gen.bool

--------------------------------------------------------------------------------
-- whole-system cosim harness
--------------------------------------------------------------------------------

-- | 100 MHz / 2 Mbaud = 50 system cycles per UART bit.
cyclesPerBit :: Int
cyclesPerBit = 50

{- | Serialize bytes to a UART line waveform: 8N1, LSB-first, 50 cycles/bit, with
one idle bit-time between bytes (a realistic transmitter's inter-byte gap — the RX
needs it to resync; truly back-to-back bytes drop on the falling-edge resync).
Idle-high before/after.
-}
serialize :: [BitVector 8] -> [Bit]
serialize = L.concatMap serByte
 where
  serByte b =
    L.replicate cyclesPerBit low
      <> L.concatMap (\i -> L.replicate cyclesPerBit (if testBit b i then high else low)) [0 .. 7]
      <> L.replicate (2 * cyclesPerBit) high -- stop bit + one idle bit-time

{- | Decode a captured UART line back to bytes by running it through the real
'uartRx' (reuses tested framing rather than reimplementing it).
-}
deserialize :: [Bit] -> [BitVector 8]
deserialize samples =
  [ b
  | Just b <-
      sampleN
        (L.length samples)
        ( fst (uartRx (oversampleTick (SNat @2_000_000)) (fromList (samples <> L.repeat high))) ::
            Signal Dom100 (Maybe (BitVector 8))
        )
  ]

{- | Drive 'system': feed a serialized control stream on rxLine (idle-high after),
hold ioIn at 0 and ALERT# idle-high, run for @lead + length rx + nExtra@
cycles, and return the sampled (txLine, cs_n, sck, lanesOut).
-}
runSystem :: [Bit] -> Int -> ([Bit], [Bit], [Bit], [Lanes])
runSystem rxSamples nExtra =
  L.unzip4
    $ sampleN
      (leadN + L.length rxSamples + nExtra)
      ( let (txLine, lanesOut, csO, sckO, _rstO, _led) =
              system
                (fromList (L.replicate leadN high <> rxSamples <> L.repeat high))
                (pure (repeat 0))
                (pure 1)
         in bundle (txLine, csO, sckO, lanesOut) :: Signal Dom100 (Bit, Bit, Bit, Lanes)
      )
 where
  -- one idle bit-time up front so the Dom100 cycle-0 reset settles during idle,
  -- not on the first start bit (the sampleN reset idiom, cf. hdl/PLAN.md).
  leadN = cyclesPerBit

-- | Load a program + trigger, run, and return the decoded drain word-stream.
loadRunDrain :: [BitVector 32] -> Int -> Either String [BitVector 32]
loadRunDrain prog nExtra =
  case decodeResult (deserialize tx) of
    Right ws -> Right ws
    Left e -> Left (show e)
 where
  ctrl = encodeControl (LoadProgram prog) <> encodeControl Trigger
  (tx, _cs, _sck, _lanes) = runSystem (serialize ctrl) nExtra

tests :: TestTree
tests =
  testGroup
    "Top"
    [ -- stepM: re-associates step, nothing more
      testProperty "stepM = step re-associated" $ H.property $ do
        i <- H.forAll genBusIn
        let (s', bo, mr) = step initState i
        stepM initState i H.=== (s', (bo, mr))
    , -- ringWrite: unwrap the Ring record to the BRAM tuple
      testCase "ringWrite Nothing = Nothing"
        $ ringWrite Nothing
        @?= Nothing
    , testProperty "ringWrite (Just Ring) = Just (addr,data)" $ H.property $ do
        a <- H.forAll (fromIntegral <$> Gen.int (Range.linear 0 4095))
        d <- H.forAll genWord
        ringWrite (Just (Ring a d)) H.=== Just (a, d)
    , -- rigState truth table
      testCase "rigState: halted -> Done (regardless of running)" $ do
        rigState False True @?= Done
        rigState True True @?= Done
    , testCase "rigState: running & not halted -> Running"
        $ rigState True False
        @?= Running
    , testCase "rigState: idle -> Waiting"
        $ rigState False False
        @?= Waiting
    , -- ledPattern: Done solid; Running faster than Waiting
      testCase "ledPattern Done is solid on" $ do
        ledPattern Done 0 @?= high
        ledPattern Done maxBound @?= high
    , testCase "ledPattern Waiting toggles on bit 25 (slow)" $ do
        ledPattern Waiting 0 @?= low
        ledPattern Waiting 0x2000000 @?= high -- 2^25
    , testCase "ledPattern Running toggles on bit 22 (faster than Waiting)" $ do
        ledPattern Running 0 @?= low
        ledPattern Running 0x400000 @?= high -- 2^22
        ledPattern Waiting 0x400000 @?= low -- same count: Waiting still low => Running is faster
    , -- whole-system cosim: minimal program
      testCase "cosim: load [HALT], trigger -> drain = REVISION + HALT terminator"
        $ loadRunDrain [encode (Halt 0)] 20000
        @?= Right [0x0001_0000, 0xC000_0000]
    , -- whole-system cosim: eSPI pin activity + still drains
      testCase "cosim: PUT program asserts CS# and toggles SCK" $ do
        let prog =
              [ encode CsAssert
              , encode (PutByteImm 0xA5)
              , encode CsDeassert
              , encode (Halt 0)
              ]
            ctrl = encodeControl (LoadProgram prog) <> encodeControl Trigger
            (tx, cs, sck, _lanes) = runSystem (serialize ctrl) 20000
        assertBool "cs_n asserts low" (low `L.elem` cs)
        assertBool "sck toggles" (low `L.elem` sck && high `L.elem` sck)
        decodeResult (deserialize tx) @?= Right [0x0001_0000, 0xC000_0000]
    ]
