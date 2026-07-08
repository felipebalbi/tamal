-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-W-2.0

{- | The tamal wire format (design doc 2026-07-02-tamal-wire-format-design.md):
the transport-agnostic control/result byte framing. This module is the frame +
message layer over the 'Tamal.Wire.Cobs' leaf; it is a pure @[BitVector 8]@
reference model (not synthesizable — like 'Tamal.Trace.encodeRecord'). The
streaming realization is the piece-3 loader.
-}
module Tamal.Wire
  ( WireError (..)
  , ControlMsg (..)
  , wordToBytesLE
  , bytesToWordLE
  , crc8
  , frameEncode
  , frameDecode
  , encodeControl
  , decodeControl
  , encodeResult
  , decodeResult
  ) where

import Clash.Prelude

import qualified Data.List as L

import Tamal.Crc (crc8Update)
import Tamal.Wire.Cobs (cobsDecode, cobsEncode)

-- | Control-plane messages (host -> FPGA).
data ControlMsg
  = -- | instruction words to load into the instr BRAM
    LoadProgram [BitVector 32]
  | -- | Start-of-run pulse
    Trigger
  deriving stock (Generic, Show, Eq)
  deriving anyclass (NFDataX)

-- | Why a frame failed to decode.
data WireError
  = -- | CRC byte did not match the recomputed CRC
    BadCrc
  | -- | malformed COBS or missing delimiter
    BadCobs
  | -- | opcode not in the v1 set
    UnknownOpcode (BitVector 8)
  | -- | decoded frame lacks an opcode and/or CRC byte
    ShortFrame
  | -- | payload length invalid for the opcode
    BadPayloadLen
  deriving stock (Generic, Show, Eq)
  deriving anyclass (NFDataX)

{- | Split a 32-bit word into four little-endian bytes (LSB first; ISA §4).
@0xAABBCCDD -> <0xDD, 0xCC, 0xBB, 0xAA>@.
-}
wordToBytesLE :: BitVector 32 -> Vec 4 (BitVector 8)
wordToBytesLE = reverse . unpack

{- | Reassemble a little-endian byte quartet into its 32-bit word (inverse of
'wordToBytesLE').
-}
bytesToWordLE :: Vec 4 (BitVector 8) -> BitVector 32
bytesToWordLE = pack . reverse

{- | Fold the CRC-8 (poly 0x07, init 0x00, MSB-first - reuses 'Tamal.Crc') over
a whole byte sequence.
-}
crc8 :: [BitVector 8] -> BitVector 8
crc8 = L.foldl' crc8Update 0

{- | Wrap a logical frame (opcode ++ payload) for the wire: append its CRC,
COBS-encode, then append the 0x00 delimiter (§4, §8).
-}
frameEncode :: [BitVector 8] -> [BitVector 8]
frameEncode logical = cobsEncode (logical <> [crc8 logical]) <> [0]

{- | Unwrap a wire frame back to its logical bytes: strip the delimiter,
COBS-decode, split off the trailing CRC byte, and verify it.
-}
frameDecode :: [BitVector 8] -> Either WireError [BitVector 8]
frameDecode wire = do
  stripped <- stripDelim wire
  content <- maybe (Left BadCobs) Right (cobsDecode stripped)
  (logical, crc) <- splitLastByte content
  if crc8 logical == crc
    then Right logical
    else Left BadCrc

-- | Remove the trailing 0x00 delimiter (or fail).
stripDelim :: [BitVector 8] -> Either WireError [BitVector 8]
stripDelim [] = Left ShortFrame
stripDelim xs
  | L.last xs == 0 = Right (L.init xs) -- remove the last byte
  | otherwise = Left BadCobs

-- | Separate a non-empty byte list into its leading bytes and its last byte.
splitLastByte :: [BitVector 8] -> Either WireError ([BitVector 8], BitVector 8)
splitLastByte [] = Left ShortFrame
splitLastByte xs = Right (L.init xs, L.last xs)

-- | Frame opcodes (§8.1). 0x00 is the delimiter, never an opcode.
opLoadProgram :: BitVector 8
opLoadProgram = 0x01

opTrigger :: BitVector 8
opTrigger = 0x02

opTraceDrain :: BitVector 8
opTraceDrain = 0x81

-- | Encode a control message to its wire frame.
encodeControl :: ControlMsg -> [BitVector 8]
encodeControl (LoadProgram ws) =
  frameEncode (opLoadProgram : L.concatMap (toList . wordToBytesLE) ws)
encodeControl Trigger = frameEncode [opTrigger]

-- | Decode a wire frame into a control message.
decodeControl :: [BitVector 8] -> Either WireError ControlMsg
decodeControl wire = do
  logical <- frameDecode wire
  case logical of
    [] -> Left ShortFrame
    (op : payload)
      | op == opLoadProgram -> LoadProgram <$> bytesToWords payload
      | op == opTrigger ->
          if L.null payload
            then Right Trigger
            else Left BadPayloadLen
      | otherwise -> Left (UnknownOpcode op)

{- | Regroup a byte payload into little-endian 32-bit words; fail unless the
length is a multiple of four.
-}
bytesToWords :: [BitVector 8] -> Either WireError [BitVector 32]
bytesToWords [] = Right []
bytesToWords (a : b : c : d : rest) =
  (bytesToWordLE (a :> b :> c :> d :> Nil) :) <$> bytesToWords rest
bytesToWords _ = Left BadPayloadLen

{- | Encode a drained ring word-stream (REVISION ++ records ++ HALT terminator,
§8.3) as a TRACE_DRAIN wire frame.
-}
encodeResult ::
  [BitVector 32] ->
  [BitVector 8]
encodeResult ws = frameEncode (opTraceDrain : L.concatMap (toList . wordToBytesLE) ws)

-- | Decode a TRACE_DRAIN wire frame back to its ring word-stream.
decodeResult :: [BitVector 8] -> Either WireError [BitVector 32]
decodeResult wire = do
  logical <- frameDecode wire
  case logical of
    [] -> Left ShortFrame
    (op : payload)
      | op == opTraceDrain -> bytesToWords payload
      | otherwise -> Left (UnknownOpcode op)
