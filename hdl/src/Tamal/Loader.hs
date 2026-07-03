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

-- | What the top feeds the loader each cycle.
data LoaderIn = LoaderIn
  { rxByte :: Maybe (BitVector 8)
  , txReady :: Bool
  , halted :: Bool
  , ringPtrIn :: Unsigned 12
  , ringData :: BitVector 32
  }
  deriving stock (Generic, Show, Eq)
  deriving anyclass (NFDataX)

{- | The loader's outputs: UART TX, instr-BRAM write, ring-BRAM read address,
and the engine start pulse.
-}
data LoaderOut = LoaderOut
  { txByte :: Maybe (BitVector 8)
  , instrWr :: Maybe (Unsigned 10, BitVector 32)
  , ringAddr :: Unsigned 12
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
  , lAddr :: Unsigned 10 -- next instr write slot
  , lFull :: Bool -- instr store overflowed (>1024 words)
  , lDrn :: DrainPhase
  , lWord :: BitVector 32 -- ring word being emitted
  , lWIx :: Unsigned 2 -- LE byte of lWord (0..3)
  , lCrcTx :: BitVector 8 -- running CRC over the drain
  , lDrCnt :: Unsigned 12 -- ring record index being fetched
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
confirm :: LoaderSt -> BitVector 8 -> (LoaderSt, Maybe (Unsigned 10, BitVector 32))
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

-- | Drain: completed in Task 5.
drainStep :: LoaderSt -> LoaderIn -> (LoaderSt, LoaderOut)
drainStep s _ = (resetFrame s{lPhase = RxControl}, idleOut)
