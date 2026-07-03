-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0

{- |
Streaming COBS codec (spec §9): the per-cycle step functions the loader embeds.
Pure — no CRC, no delimiter (those live in the loader's frame layer, exactly as
'Tamal.Wire' sits above 'Tamal.Wire.Cobs'). Each step's iterated output equals
the pure 'Tamal.Wire.Cobs' reference.
-}
module Tamal.Loader.Cobs
  ( DecSt
  , initDec
  , cobsDecodeStep
  ) where

import Clash.Prelude

{- | Decode state: data bytes remaining in the current group (@0@ = expecting a
code byte), whether that group's code was @255@ (a full group injects no zero),
whether we owe an injected zero before the next code byte, and whether any byte
has arrived this frame.
-}
data DecSt = DecSt
  { dCnt :: Unsigned 8
  , dFull :: Bool
  , dPend :: Bool
  , dGot :: Bool
  }
  deriving stock (Generic, Show, Eq)
  deriving anyclass (NFDataX)

initDec :: DecSt
initDec = DecSt 0 False False False

{- | One decode step. Input @(maybe-COBS-byte, frame-end)@; the loader pulses
frame-end when it sees the @0x00@ delimiter. Output @(maybe-decoded-byte, done,
malformed)@. On frame-end the state resets and @done@ pulses; @malformed@ is set
if the frame was empty or a group was left truncated.
-}
cobsDecodeStep :: DecSt -> (Maybe (BitVector 8), Bool) -> (DecSt, (Maybe (BitVector 8), Bool, Bool))
cobsDecodeStep s (mIn, frameEnd)
  | frameEnd = (initDec, (Nothing, True, not (dGot s) || dCnt s /= 0))
  | otherwise = case mIn of
      Nothing -> (s, (Nothing, False, False))
      Just b
        | dCnt s == 0 ->
            let (out, s1) =
                  if dPend s
                    then (Just 0, s{dPend = False, dGot = True})
                    else (Nothing, s{dGot = True})
                s2 = startGroup s1 b
             in (s2, (out, False, False))
        | otherwise ->
            let cnt' = dCnt s - 1
                s' =
                  s
                    { dCnt = cnt'
                    , dGot = True
                    , dPend =
                        if cnt' == 0
                          then not (dFull s)
                          else dPend s
                    }
             in (s', (Just b, False, False))
 where
  startGroup st c =
    let full = c == 255
        n = (unpack c :: Unsigned 8) - 1 -- c is 1..255 (never 0), so n is 0..254
     in if n == 0
          then st{dCnt = 0, dFull = full, dPend = not full}
          else st{dCnt = n, dFull = full}
