-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0

{- |
The tamal wire format (design doc 2026-07-02-tamal-wire-format-design.md): the
transport-agnostic control/result byte framing. This module is the frame +
message layer over the 'Tamal.Wire.Cobs' leaf; it is a pure @[BitVector 8]@
reference model (not synthesizable — like 'Tamal.Trace.encodeRecord'). The
streaming realization is the piece-3 loader.
-}
module Tamal.Wire
  ( wordToBytesLE
  , bytesToWordLE
  ) where

import Clash.Prelude

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
