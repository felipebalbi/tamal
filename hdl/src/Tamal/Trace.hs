-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-W-2.0

{- |
Result-ring record encoding and the overflow-safe push model (spec §8).
Records are whole 32-bit words: CAPTURE (1 word, tag 00), MARK (2 words,
tag 10), HALT (1 word, tag 11). 'ringPush' writes a record atomically or
drops it, setting a sticky overflow flag; it never writes past the record
limit (the HALT terminator slot beyond it is reserved).
-}
module Tamal.Trace
  ( Record (..)
  , encodeRecord
  , ringPush
  ) where

import Clash.Prelude
import qualified Data.List as L

import Tamal.Params (RW)

{- | A result-ring record. @Capture@ reports a sampled byte (with its valid bit
count), @Mark@ carries a host↔trace correlation label plus a register payload,
and @Halt@ terminates a run (with the sticky trap & overflow flags, 3-bit reason
and a status byte).
-}
data Record
  = Capture (BitVector 4) (BitVector 8) -- nbits (1..8), sampled byte
  | Mark (BitVector 14) (BitVector 32) -- label, payload
  | Halt Bool (BitVector 3) Bool (BitVector 8) -- trap, reason, overflow, status
  deriving stock (Generic, Show, Eq)
  deriving anyclass (NFDataX)

{- | Encode a record to its 32-bit words (reference model; the engine's
synthesizable word emitter in Plan A2 matches these layouts).
-}
encodeRecord :: Record -> [BitVector 32]
encodeRecord = \case
  Capture n b -> [bitCoerce (0b00 :: BitVector 2, 0 :: BitVector 18, n, b)]
  Mark lbl pl -> [bitCoerce (0b10 :: BitVector 2, 0 :: BitVector 16, lbl), pl]
  Halt trp rsn ovf st -> [bitCoerce (0b11 :: BitVector 2, 0 :: BitVector 17, rsn, trp, ovf, st)]

{- | Atomically push a record's words. Given the current write pointer, the
last usable record slot (@limit@), and prior overflow, either write all
words (advancing the pointer) or drop them and latch overflow.
-}
ringPush ::
  -- | current write pointer
  Unsigned RW ->
  -- | last usable record-slot index (limit)
  Unsigned RW ->
  -- | prior sticky overflow
  Bool ->
  -- | words of one record
  [BitVector 32] ->
  (Unsigned RW, Bool, [BitVector 32])
ringPush ptr limit ovf ws
  | ovf = (ptr, True, [])
  | fits = (ptr + count, False, ws)
  | otherwise = (ptr, True, [])
 where
  count = fromIntegral (L.length ws)
  -- last index this record would occupy is ptr + count - 1; must be <= limit
  fits = L.length ws > 0 && (ptr + count - 1) <= limit
