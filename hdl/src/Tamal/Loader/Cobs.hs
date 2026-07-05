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
  , EncSt
  , initEnc
  , cobsEncodeStep
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

-- | Encode mode: filling the current group, or emitting @code ++ group@.
data EncMode = EFilling | EEmitting
  deriving stock (Generic, Show, Eq)
  deriving anyclass (NFDataX)

{- | Encode state: the ≤254-byte group buffer + fill count, the emit index
(@0@ = code byte, @1..fill@ = data), a byte stashed when a full 254-group is
flushed (it starts the next group), whether a final empty group is still owed
(the last input byte was @0x00@), and whether the input stream has ended.
-}
data EncSt = EncSt
  { eMode :: EncMode
  , eBuf :: Vec 254 (BitVector 8)
  , eFill :: Unsigned 8
  , eIx :: Unsigned 8
  , ePend :: Maybe (BitVector 8, Bool)
  , eFinal :: Bool
  , eLast :: Bool
  }
  deriving stock (Generic, Show, Eq)
  deriving anyclass (NFDataX)

initEnc :: EncSt
initEnc = EncSt EFilling (repeat 0) 0 0 Nothing False False

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

{- | One encode step. Input @(maybe-(byte,is-last), downstream-ready)@; output
@(ready-in, maybe-COBS-byte, done)@. @ready-in@ is high only while filling (the
one state that consumes input). An output byte appears only when
downstream-ready. @done@ pulses when the whole frame's COBS output is emitted.
-}
cobsEncodeStep :: EncSt -> (Maybe (BitVector 8, Bool), Bool) -> (EncSt, (Bool, Maybe (BitVector 8), Bool))
cobsEncodeStep s (mIn, dsReady) = case eMode s of
  EFilling -> case mIn of
    Nothing -> (s, (True, Nothing, False))
    Just (b, lst)
      | eFill s == 254 ->
          -- Full group FIRST (before the @b == 0@ case): a 254-byte group flushes
          -- as a 0xFF continuation, then @b@ is reprocessed via 'ePend'. A zero
          -- landing here must terminate a *fresh* empty group, not fold into the
          -- full group (0xFF carries no implied zero) — see the pb==0 case below.
          ( s
              { eMode = EEmitting
              , eIx = 0
              , ePend = Just (b, lst)
              }
          , (True, Nothing, False)
          )
      | b == 0 ->
          ( s
              { eMode = EEmitting
              , eIx = 0
              , eFinal = lst
              , eLast = lst
              }
          , (True, Nothing, False)
          )
      | lst ->
          ( (store s b)
              { eMode = EEmitting
              , eIx = 0
              , eLast = True
              , eFinal = False
              }
          , (True, Nothing, False)
          )
      | otherwise ->
          (store s b, (True, Nothing, False))
  EEmitting
    | not dsReady -> (s, (False, Nothing, False))
    | eIx s <= eFill s ->
        let out :: BitVector 8
            out =
              if eIx s == 0
                then fromIntegral (eFill s) + 1
                else eBuf s !! (eIx s - 1)
         in (s{eIx = eIx s + 1}, (False, Just out, False))
    | eFinal s ->
        ( s
            { eFill = 0
            , eIx = 0
            , eFinal = False
            }
        , (False, Nothing, False)
        )
    | otherwise -> case ePend s of
        Just (pb, pl)
          | pb == 0 ->
              -- A zero that arrived on a full group: the full group was just
              -- emitted as a 0xFF continuation; now emit a *fresh* empty group
              -- for the zero itself. @eFinal@ owes the trailing empty group iff
              -- the zero was the last input byte (mirrors the b==0 EFilling case).
              ( s
                  { eMode = EEmitting
                  , eFill = 0
                  , eIx = 0
                  , ePend = Nothing
                  , eFinal = pl
                  , eLast = pl
                  }
              , (False, Nothing, False)
              )
          | pl ->
              ( (store s{eFill = 0} pb)
                  { eMode = EEmitting
                  , eIx = 0
                  , ePend = Nothing
                  , eLast = True
                  }
              , (False, Nothing, False)
              )
          | otherwise ->
              ( (store s{eFill = 0} pb)
                  { eMode = EFilling
                  , ePend = Nothing
                  }
              , (False, Nothing, False)
              )
        Nothing
          | eLast s -> (initEnc, (False, Nothing, True))
          | otherwise ->
              ( s
                  { eMode = EFilling
                  , eFill = 0
                  }
              , (False, Nothing, False)
              )
 where
  store st b = st{eBuf = replace (eFill st) b (eBuf st), eFill = eFill st + 1}
