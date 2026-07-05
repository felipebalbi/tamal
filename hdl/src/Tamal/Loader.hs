-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0

{- |
The impure loader (design doc 2026-07-02-tamal-loader-design.md): the
@RxControl -> Run -> Drain@ lifecycle FSM bridging the UART, the two BRAMs, and
the engine's @startIn@/@haltedOut@. It is the streaming realization of the pure
'Tamal.Wire' model; the streaming COBS lives in 'Tamal.Loader.Cobs'.
-}
module Tamal.Loader
  ( LoaderIn (..)
  , LoaderOut (..)
  , loader
  ) where

import Clash.Prelude
import Data.Maybe (fromMaybe, isJust)

import Tamal.Crc (crc8Update)
import Tamal.Loader.Cobs
import Tamal.Params (AW, RW)

-- | What the top feeds the loader each cycle.
data LoaderIn = LoaderIn
  { rxByte :: Maybe (BitVector 8)
  , txReady :: Bool
  , halted :: Bool
  , ringPtrIn :: Unsigned RW
  , ringData :: BitVector 32
  }
  deriving stock (Generic, Show, Eq)
  deriving anyclass (NFDataX)

{- | The loader's outputs: UART TX, instr-BRAM write, ring-BRAM read address,
and the engine start pulse.
-}
data LoaderOut = LoaderOut
  { txByte :: Maybe (BitVector 8)
  , instrWr :: Maybe (Unsigned AW, BitVector 32)
  , ringAddr :: Unsigned RW
  , startOut :: Bool
  }
  deriving stock (Generic, Show, Eq)
  deriving anyclass (NFDataX)

data Lifecycle = RxControl | Run | Drain
  deriving stock (Generic, Show, Eq)
  deriving anyclass (NFDataX)

data DrainPhase = DrOpcode | DrFetch | DrLatch | DrWordByte | DrCrcByte | DrDrainOut | DrDelim
  deriving stock (Generic, Show, Eq)
  deriving anyclass (NFDataX)

data LoaderSt = LoaderSt
  { lPhase :: Lifecycle
  , lDec :: DecSt
  , lEnc :: EncSt
  , lHeld :: Maybe (BitVector 8) -- one-byte holdback (separates the trailing CRC)
  , lCrcRx :: BitVector 8 -- running CRC over confirmed bytes
  , lHaveOp :: Bool -- opcode confirmed yet?
  , lOpcode :: BitVector 8
  , lByteIx :: Unsigned 2 -- payload byte within the current word (0..3)
  , lWordAcc :: BitVector 32 -- LE word being assembled
  , lHadPay :: Bool -- any payload byte seen (TRIGGER must have none)
  , lAddr :: Unsigned AW -- next instr write slot
  , lFull :: Bool -- instr store overflowed (> 2^AW words)
  , lDrn :: DrainPhase
  , lWord :: BitVector 32 -- ring word being emitted
  , lWIx :: Unsigned 2 -- LE byte of lWord (0..3)
  , lCrcTx :: BitVector 8 -- running CRC over the drain
  , lDrCnt :: Unsigned RW -- ring record index being fetched
  , lTerm :: Bool -- fetching/emitting the terminator word
  }
  deriving stock (Generic, Show, Eq)
  deriving anyclass (NFDataX)

initLoader :: LoaderSt
initLoader =
  LoaderSt
    { lPhase = RxControl
    , lDec = initDec
    , lEnc = initEnc
    , lHeld = Nothing
    , lCrcRx = 0
    , lHaveOp = False
    , lOpcode = 0
    , lByteIx = 0
    , lWordAcc = 0
    , lHadPay = False
    , lAddr = 0
    , lFull = False
    , lDrn = DrOpcode
    , lWord = 0
    , lWIx = 0
    , lCrcTx = 0
    , lDrCnt = 0
    , lTerm = False
    }

idleOut :: LoaderOut
idleOut = LoaderOut{txByte = Nothing, instrWr = Nothing, ringAddr = 0, startOut = False}

-- | The loader: a plain 'mealy' over the pure 'loaderStep' (matching the engine lift).
loader :: (HiddenClockResetEnable dom) => Signal dom LoaderIn -> Signal dom LoaderOut
loader = mealy loaderStep initLoader

loaderStep :: LoaderSt -> LoaderIn -> (LoaderSt, LoaderOut)
loaderStep s inp = case lPhase s of
  RxControl -> rxStep s inp
  Run -> runStep s inp
  Drain -> drainStep s inp

-- | Run: idle until the engine halts, then start the drain.
runStep :: LoaderSt -> LoaderIn -> (LoaderSt, LoaderOut)
runStep s inp
  | halted inp =
      ( s{lPhase = Drain, lEnc = initEnc, lDrn = DrOpcode, lCrcTx = 0, lDrCnt = 0, lWIx = 0, lTerm = False}
      , idleOut
      )
  | otherwise = (s, idleOut)

-- | RxControl: stream-decode a control frame; on the delimiter, verify + dispatch.
rxStep :: LoaderSt -> LoaderIn -> (LoaderSt, LoaderOut)
rxStep s inp =
  let din = case rxByte inp of
        Just 0 -> (Nothing, True) -- delimiter => frame end
        Just b -> (Just b, False)
        Nothing -> (Nothing, False)
      (dec', (mDec, done, bad)) = cobsDecodeStep (lDec s) din
      s1 = s{lDec = dec'}
   in if done
        then finalize s1 bad
        else case mDec of
          Nothing -> (s1, idleOut)
          Just d ->
            let (s2, mw) = case lHeld s1 of
                  Just h -> confirm s1 h
                  Nothing -> (s1, Nothing)
             in (s2{lHeld = Just d}, idleOut{instrWr = mw})

{- | Confirm a held (definitely-not-CRC) byte: fold CRC, route as opcode or a
payload byte, assembling LE words and writing them (write-through) for LOAD.
-}
confirm :: LoaderSt -> BitVector 8 -> (LoaderSt, Maybe (Unsigned AW, BitVector 32))
confirm s h
  | not (lHaveOp s) =
      ( s
          { lHaveOp = True
          , lOpcode = h
          , lCrcRx = crc8Update (lCrcRx s) h
          , lAddr = if h == 0x01 then 0 else lAddr s
          , lFull = if h == 0x01 then False else lFull s
          }
      , Nothing
      )
  | otherwise =
      let crc' = crc8Update (lCrcRx s) h
          acc' = lWordAcc s .|. (zeroExtend h `shiftL` (8 * fromIntegral (lByteIx s)))
          isLoad = lOpcode s == 0x01
       in if lByteIx s == 3
            then
              let doWrite = isLoad && not (lFull s)
                  (addr', full') = if lAddr s == maxBound then (lAddr s, True) else (lAddr s + 1, lFull s)
               in ( s
                      { lCrcRx = crc'
                      , lWordAcc = 0
                      , lByteIx = 0
                      , lHadPay = True
                      , lAddr = if isLoad then addr' else lAddr s
                      , lFull = if isLoad then full' else lFull s
                      }
                  , if doWrite then Just (lAddr s, acc') else Nothing
                  )
            else
              ( s{lCrcRx = crc', lWordAcc = acc', lByteIx = lByteIx s + 1, lHadPay = True}
              , Nothing
              )

{- | Frame end: the held byte is the CRC candidate. A good TRIGGER pulses
startOut and enters Run; a good LOAD's words are already written; anything
else is discarded (D4/D5). Reset the frame-parse state either way.
-}
finalize :: LoaderSt -> Bool -> (LoaderSt, LoaderOut)
finalize s bad =
  let crcCand = fromMaybe 0 (lHeld s)
      crcGood = not bad && isJust (lHeld s) && lHaveOp s && lCrcRx s == crcCand
      trigOk = crcGood && lOpcode s == 0x02 && not (lHadPay s)
      s0 = resetFrame s
   in if trigOk
        then (s0{lPhase = Run}, idleOut{startOut = True})
        else (s0, idleOut)

resetFrame :: LoaderSt -> LoaderSt
resetFrame s =
  s
    { lDec = initDec
    , lHeld = Nothing
    , lCrcRx = 0
    , lHaveOp = False
    , lOpcode = 0
    , lByteIx = 0
    , lWordAcc = 0
    , lHadPay = False
    }

-- | Drain: emit one TRACE_DRAIN frame from the ring, then return to RxControl.
drainStep :: LoaderSt -> LoaderIn -> (LoaderSt, LoaderOut)
drainStep s inp = case lDrn s of
  DrOpcode ->
    feedByte s inp 0x81 False (\s' -> s'{lDrn = DrFetch, lDrCnt = 0, lTerm = False})
  DrWordByte ->
    feedByte s inp (leByte (lWord s) (lWIx s)) False (afterWordByte inp)
  DrCrcByte ->
    feedByte s inp (lCrcTx s) True (\s' -> s'{lDrn = DrDrainOut})
  DrFetch ->
    let addr = if lTerm s then maxBound else lDrCnt s
        (enc', (_, mOut, _)) = cobsEncodeStep (lEnc s) (Nothing, txReady inp)
     in (s{lEnc = enc', lDrn = DrLatch}, idleOut{txByte = mOut, ringAddr = addr})
  DrLatch ->
    let addr = if lTerm s then maxBound else lDrCnt s
        (enc', (_, mOut, _)) = cobsEncodeStep (lEnc s) (Nothing, txReady inp)
     in ( s{lEnc = enc', lWord = ringData inp, lWIx = 0, lDrn = DrWordByte}
        , idleOut{txByte = mOut, ringAddr = addr}
        )
  DrDrainOut ->
    let (enc', (_, mOut, encDone)) = cobsEncodeStep (lEnc s) (Nothing, txReady inp)
     in (s{lEnc = enc', lDrn = if encDone then DrDelim else DrDrainOut}, idleOut{txByte = mOut})
  DrDelim ->
    if txReady inp
      then (resetFrame s{lPhase = RxControl}, idleOut{txByte = Just 0})
      else (s, idleOut{txByte = Nothing})
 where
  afterWordByte i s'
    | lWIx s' /= 3 = s'{lWIx = lWIx s' + 1}
    | lTerm s' = s'{lDrn = DrCrcByte}
    | lDrCnt s' + 1 >= ringPtrIn i = s'{lTerm = True, lDrn = DrFetch}
    | otherwise = s'{lDrCnt = lDrCnt s' + 1, lDrn = DrFetch}

{- | Present a logical byte to the encoder; when consumed (readyIn), fold it into
the drain CRC (except the CRC byte itself, flagged @lst@) and advance the
generator. Route the encoder's output to txByte.
-}
feedByte ::
  LoaderSt -> LoaderIn -> BitVector 8 -> Bool -> (LoaderSt -> LoaderSt) -> (LoaderSt, LoaderOut)
feedByte s inp b lst advance =
  let (enc', (readyIn, mOut, _)) = cobsEncodeStep (lEnc s) (Just (b, lst), txReady inp)
      s1 = s{lEnc = enc'}
      s2 =
        if readyIn
          then advance s1{lCrcTx = if lst then lCrcTx s1 else crc8Update (lCrcTx s1) b}
          else s1
   in (s2, idleOut{txByte = mOut})

-- | The little-endian byte @i@ (0..3) of a 32-bit word.
leByte :: BitVector 32 -> Unsigned 2 -> BitVector 8
leByte w i = case i of
  0 -> slice d7 d0 w
  1 -> slice d15 d8 w
  2 -> slice d23 d16 w
  _ -> slice d31 d24 w
