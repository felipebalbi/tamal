-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0
{-# LANGUAGE NumericUnderscores #-}

module Test.Loader (tests) where

import Clash.Prelude
import qualified Data.List as L
import Data.Maybe (mapMaybe)
import Hedgehog (Gen, forAll, property, (===))
import qualified Hedgehog.Gen as Gen
import qualified Hedgehog.Range as Range
import Test.Tasty
import Test.Tasty.HUnit
import Test.Tasty.Hedgehog (testProperty)

import Tamal.Domain (Dom100)
import Tamal.Engine (State (..), busOut, initState, ringPtrOut)
import Tamal.Loader (LoaderIn (..), LoaderOut (..), loader)
import Tamal.Loader.Cobs (DecSt, EncSt, cobsDecodeStep, cobsEncodeStep, initDec, initEnc)
import Tamal.Wire (ControlMsg (..), encodeControl, encodeResult)
import Tamal.Wire.Cobs (cobsEncode)
import Test.Gen (genWord)

tests :: TestTree
tests =
  testGroup
    "Loader"
    [ testGroup
        "engine ringPtrOut projection"
        [ testCase "initState projects ringPtr = 1"
            $ ringPtrOut (busOut initState)
            @?= 1
        , testCase "busOut projects the State ringPtr verbatim"
            $ ringPtrOut (busOut initState{ringPtr = 42})
            @?= 42
        , testCase "projects the top-of-ring value"
            $ ringPtrOut (busOut initState{ringPtr = maxBound})
            @?= (maxBound :: Unsigned 12)
        ]
    , decodeTests
    , encodeTests
    , rxTests
    , drainTests
    , robustTests
    ]

-- | A zero-dense byte generator: stresses COBS group boundaries (~1/4 zeros).
genByteZeros :: Gen (BitVector 8)
genByteZeros = Gen.frequency [(1, pure 0), (4, genB)]
 where
  genB = fromIntegral <$> Gen.int (Range.linear 0 255)

{- | A full-group-boundary generator: concatenated runs of non-zero bytes (each
up to 260 long, crossing the 254-byte cap), each optionally followed by a
single zero. genByteZeros cannot produce a 254-long non-zero run, so it never
reaches the full-group / trailing-zero interaction; this generator does.
-}
genRuns :: Gen [BitVector 8]
genRuns = fmap L.concat (Gen.list (Range.linear 0 4) chunk)
 where
  chunk = do
    n <- Gen.int (Range.linear 0 260)
    run <- Gen.list (Range.singleton n) genNonZero
    z <- Gen.bool
    pure (run <> if z then [0] else [])
  genNonZero = fmap fromIntegral (Gen.int (Range.linear 1 255))

{- | Pure driver for the decode step: feed the COBS bytes (delimiter stripped),
then a frame-end pulse; collect the decoded bytes and the malformed flag.
-}
decDrive :: [BitVector 8] -> ([BitVector 8], Bool)
decDrive enc = go initDec ([(Just b, False) | b <- enc] <> [(Nothing, True)])
 where
  go :: DecSt -> [(Maybe (BitVector 8), Bool)] -> ([BitVector 8], Bool)
  go _ [] = ([], False)
  go st (i : is) =
    let (st', (mo, done, bad)) = cobsDecodeStep st i
     in if done
          then ([], bad)
          else let (os, b) = go st' is in (maybe os (: os) mo, b)

decodeTests :: TestTree
decodeTests =
  testGroup
    "Loader.Cobs decode"
    [ testProperty "streaming decode of cobsEncode x reconstructs x" $ property $ do
        x <- forAll (Gen.list (Range.linear 0 300) genByteZeros)
        let (dec, bad) = decDrive (cobsEncode x)
        dec === x
        bad === False
    , testCase "decode reconstructs the [0x00] vector"
        $ decDrive (cobsEncode [0x00])
        @?= ([0x00], False)
    , testCase "decode reconstructs the 254/255 boundary vectors" $ do
        decDrive (cobsEncode (L.map fromIntegral [1 .. 254 :: Int]))
          @?= (L.map fromIntegral [1 .. 254 :: Int], False)
        decDrive (cobsEncode (L.map fromIntegral [1 .. 255 :: Int]))
          @?= (L.map fromIntegral [1 .. 255 :: Int], False)
    , testCase "truncated group -> malformed"
        $ snd (decDrive [0x05, 0x11])
        @?= True
    , testCase "empty frame (delimiter only) -> malformed"
        $ snd (decDrive [])
        @?= True
    , testCase "code byte with no data -> malformed"
        $ snd (decDrive [0x03])
        @?= True
    ]

{- | Pure driver for the encode step: feed logical bytes (last one flagged),
downstream always ready; advance the input only when readyIn (byte consumed).
Collects the emitted COBS bytes (no delimiter).
-}
encDrive :: [BitVector 8] -> [BitVector 8]
encDrive [] = []
encDrive xs = go initEnc [(x, i == L.length xs - 1) | (i, x) <- L.zip [0 ..] xs]
 where
  go :: EncSt -> [(BitVector 8, Bool)] -> [BitVector 8]
  go st inp =
    let (feed, rest0) = case inp of
          (t : ts) -> (Just t, ts)
          [] -> (Nothing, [])
        (st', (readyIn, mo, done)) = cobsEncodeStep st (feed, True)
        inp' = if readyIn then rest0 else inp
     in if done then [] else maybe (go st' inp') (: go st' inp') mo

encodeTests :: TestTree
encodeTests =
  testGroup
    "Loader.Cobs encode"
    [ testProperty "streaming encode equals cobsEncode (non-empty)" $ property $ do
        x <- forAll (Gen.list (Range.linear 1 300) genByteZeros)
        encDrive x === cobsEncode x
    , testCase "encode the [11,00,00,00] vector"
        $ encDrive [0x11, 0x00, 0x00, 0x00]
        @?= [0x02, 0x11, 0x01, 0x01, 0x01]
    , testCase "encode the 254/255 boundary" $ do
        encDrive (L.map fromIntegral [1 .. 254 :: Int])
          @?= (0xFF : L.map fromIntegral [1 .. 254 :: Int])
        encDrive (L.map fromIntegral [1 .. 255 :: Int])
          @?= (0xFF : L.map fromIntegral [1 .. 254 :: Int]) <> [0x02, 255]
    , -- Regression: a full 254-byte group terminated by a zero. The streaming
      -- encoder must match the pure oracle — flush 0xFF,<254> then a fresh empty
      -- group for the zero (…,01,01), not drop the zero.
      testCase "encode 254 non-zero then 0x00 == FF,<254>,01,01"
        $ encDrive (L.map fromIntegral [1 .. 254 :: Int] <> [0x00])
        @?= (0xFF : L.map fromIntegral [1 .. 254 :: Int]) <> [0x01, 0x01]
    , testProperty "streaming encode equals cobsEncode (boundary runs)" $ property $ do
        x <- forAll (Gen.filter (not . L.null) genRuns)
        encDrive x === cobsEncode x
    , testProperty "encode then decode round-trips (both streaming)" $ property $ do
        x <- forAll (Gen.list (Range.linear 1 300) genByteZeros)
        decDrive (encDrive x) === (x, False)
    , testProperty "encode then decode round-trips (both streaming, boundary runs)" $ property $ do
        x <- forAll (Gen.filter (not . L.null) genRuns)
        decDrive (encDrive x) === (x, False)
    ]

{- | Feed an rxByte stream (idle otherwise), collecting the instr-BRAM writes.
The engine/ring inputs are quiescent (halted low, ring empty). The stream is
led by one idle cycle: @sampleN@ asserts @resetGen@ on cycle 0 (Dom100 has an
async reset), so a byte fed at cycle 0 would be lost — the line idles first,
exactly as it does in hardware (cf. Test.Uart).
-}
simInstrWr :: [Maybe (BitVector 8)] -> [(Unsigned 10, BitVector 32)]
simInstrWr rxs =
  mapMaybe instrWr
    $ sampleN
      (L.length rxs + 9)
      (loader (fromList (fmap mkIn (Nothing : (rxs <> L.repeat Nothing)))) :: Signal Dom100 LoaderOut)
 where
  mkIn r = LoaderIn{rxByte = r, txReady = True, halted = False, ringPtrIn = 0, ringData = 0}

-- | Collect the startOut pulses over an rxByte stream (led by one idle cycle).
simStartOut :: [Maybe (BitVector 8)] -> [Bool]
simStartOut rxs =
  fmap startOut
    $ sampleN
      (L.length rxs + 9)
      (loader (fromList (fmap mkIn (Nothing : (rxs <> L.repeat Nothing)))) :: Signal Dom100 LoaderOut)
 where
  mkIn r = LoaderIn{rxByte = r, txReady = True, halted = False, ringPtrIn = 0, ringData = 0}

rxTests :: TestTree
rxTests =
  testGroup
    "Loader RX / load"
    [ testProperty "LOAD_PROGRAM writes the exact words at 0,1,2,.." $ property $ do
        ws <- forAll (Gen.list (Range.linear 0 20) genWord)
        let bytes = encodeControl (LoadProgram ws)
        simInstrWr (fmap Just bytes) === [(fromIntegral i, w) | (i, w) <- L.zip [0 :: Int ..] ws]
    , testCase "LOAD_PROGRAM does not pulse startOut" $ do
        let bytes = encodeControl (LoadProgram [0xDEAD_BEEF, 0x0000_0001])
        L.filter id (simStartOut (fmap Just bytes)) @?= []
    , testCase "TRIGGER pulses startOut exactly once, after the frame" $ do
        let bytes = encodeControl Trigger
        L.length (L.filter id (simStartOut (fmap Just bytes))) @?= 1
    ]

{- | The loader with the ring-BRAM read loop closed (1-cycle latency via register).
Feedback harnesses MUST be a function carrying (HiddenClockResetEnable dom) so
sampleN can supply the hidden clock/reset/enable (the Test.Uart fastLoop idiom);
a `where`-bound signal at the test level would have no clock in scope.
-}
drainRig ::
  (HiddenClockResetEnable dom) =>
  (Unsigned 12 -> BitVector 32) -> -- ring lookup (simulation-only, non-synthesizable)
  Unsigned 12 -> -- ringPtr
  Signal dom (Maybe (BitVector 8)) -> -- rxByte
  Signal dom Bool -> -- txReady
  Signal dom Bool -> -- halted
  Signal dom (Maybe (BitVector 8)) -- txByte
drainRig lookupRing ringPtrV rxs txr hlt = txByte <$> loaderOut
 where
  loaderOut = loader loaderIn
  ringDataS = register 0 (lookupRing <$> (ringAddr <$> loaderOut))
  loaderIn = LoaderIn <$> rxs <*> txr <*> hlt <*> pure ringPtrV <*> ringDataS

-- | Ring model: word[0..ringPtr-1] = records (word0 = REVISION); word[termAddr] = term.
ringModel :: [BitVector 32] -> BitVector 32 -> Unsigned 12 -> BitVector 32
ringModel records term a
  | a == maxBound = term
  | fromIntegral a < L.length records = records L.!! fromIntegral a
  | otherwise = 0

{- | Drive TRIGGER -> Run -> (halted) -> Drain; collect the drained byte stream.
@records@ are word[0..ringPtr-1] (word0 = REVISION); @term@ is the terminator.
The rxByte stream leads with one idle cycle (sampleN/resetGen cycle-0 hazard).
-}
simDrain :: [BitVector 32] -> BitVector 32 -> [Bool] -> [BitVector 8]
simDrain records term txReadyPat =
  mapMaybe id
    $ sampleN
      2500
      ( drainRig
          (ringModel records term)
          (fromIntegral (L.length records))
          (fromList rxs)
          (fromList (L.cycle txReadyPat))
          (fromList halteds) ::
          Signal Dom100 (Maybe (BitVector 8))
      )
 where
  trig = encodeControl Trigger
  rxs = Nothing : (fmap Just trig <> L.repeat Nothing)
  halteds = L.replicate (L.length trig + 7) False <> L.repeat True

drainTests :: TestTree
drainTests =
  testGroup
    "Loader TX / drain"
    [ testProperty "drain stream == encodeResult (records ++ terminator)" $ property $ do
        records <- forAll (Gen.list (Range.linear 1 24) genWord)
        term <- forAll genWord
        simDrain records term [True] === encodeResult (records <> [term])
    , testCase "minimal drain: REVISION + terminator only"
        $ simDrain [0x0001_0000] 0xC000_0000 [True]
        @?= encodeResult [0x0001_0000, 0xC000_0000]
    , testProperty "drain is byte-identical under txReady backpressure" $ property $ do
        records <- forAll (Gen.list (Range.linear 1 16) genWord)
        term <- forAll genWord
        simDrain records term [True, False, True, True, False]
          === encodeResult (records <> [term])
    ]

{- | Two TRIGGER/halt cycles: the second TRIGGER at cycle 1000 (well after drain 1),
halted held high, so the loader drains twice. Reuses drainRig + ringModel.
The rxByte stream leads with one idle cycle (sampleN/resetGen cycle-0 hazard).
-}
simDrainTwice :: [BitVector 32] -> BitVector 32 -> [BitVector 8]
simDrainTwice records term =
  mapMaybe id
    $ sampleN
      2500
      ( drainRig
          (ringModel records term)
          (fromIntegral (L.length records))
          (fromList rxs)
          (pure True)
          (fromList halteds) ::
          Signal Dom100 (Maybe (BitVector 8))
      )
 where
  trig = encodeControl Trigger
  rxs =
    Nothing
      : ( fmap Just trig
            <> L.replicate (1000 - L.length trig) Nothing
            <> fmap Just trig
            <> L.repeat Nothing
        )
  halteds = L.replicate 30 False <> L.repeat True

robustTests :: TestTree
robustTests =
  testGroup
    "Loader robustness + lifecycle"
    [ testProperty "single-byte corruption of a TRIGGER never triggers a run" $ property $ do
        -- TRIGGER is the frame that *would* pulse startOut; corrupting any byte
        -- (bad COBS/CRC, or an early 0x00) must be discarded (D4/D5) -> no pulse.
        let frame = encodeControl Trigger
        i <- forAll (Gen.int (Range.linear 0 (L.length frame - 1)))
        let frame' = [if j == i then b `xor` 1 else b | (j, b) <- L.zip [0 :: Int ..] frame]
        L.filter id (simStartOut (fmap Just frame')) === []
    , testCase "a clean TRIGGER still pulses exactly once (control)"
        $ L.length (L.filter id (simStartOut (fmap Just (encodeControl Trigger))))
        @?= 1
    , testCase "over-long LOAD saturates the write address at 1023" $ do
        let ws = L.map fromIntegral [1 .. 1100 :: Int] :: [BitVector 32]
            writes = simInstrWr (fmap Just (encodeControl (LoadProgram ws)))
        L.length writes @?= 1024
        fmap fst writes @?= [0 .. 1023]
    , testCase "two LOADs each write from address 0 (overwrite)" $ do
        let ws1 = [0x1111_1111, 0x2222_2222] :: [BitVector 32]
            ws2 = [0xAAAA_AAAA] :: [BitVector 32]
            bytes = encodeControl (LoadProgram ws1) <> encodeControl (LoadProgram ws2)
        simInstrWr (fmap Just bytes)
          @?= [(0, 0x1111_1111), (1, 0x2222_2222), (0, 0xAAAA_AAAA)]
    , testCase "re-runnable: two TRIGGER/halt cycles drain twice" $ do
        let recs = [0x0001_0000, 0x00AB_00CD] :: [BitVector 32]
            term = 0xC000_0000 :: BitVector 32
        simDrainTwice recs term @?= (encodeResult (recs <> [term]) <> encodeResult (recs <> [term]))
    ]
