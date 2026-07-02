-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0

{- |
Consistent Overhead Byte Stuffing (spec §5): removes every @0x00@ from a byte
sequence so a single @0x00@ can delimit frames. A dependency-free leaf — it
imports no other @Tamal.Wire.*@ module, and 'cobsDecode' reports malformed input
as 'Nothing' (the frame layer lifts that to @BadCobs@). This is a pure
@[BitVector 8]@ reference model; the loader implements the streaming form.
-}
module Tamal.Wire.Cobs
  ( cobsEncode
  ) where

import Clash.Prelude
import qualified Data.List as L

{- | COBS-encode a byte sequence into groups: each group is a code byte
@(len+1)@ followed by up to 254 non-zero data bytes. A @0x00@ closes a group
(consumed); a group full at 254 bytes is flushed (code @0xFF@). The output never
contains @0x00@ and does NOT include the frame delimiter (the frame layer appends
it).
-}
cobsEncode :: [BitVector 8] -> [BitVector 8]
cobsEncode = go []
 where
  go grp [] = emit grp
  go grp (b : bs)
    | b == 0 = emit grp <> go [] bs -- zero terminates the group (implied)
    | L.length grp == 254 = emit grp <> go [b] bs -- group full: flush, restart with b
    | otherwise = go (grp <> [b]) bs
  emit grp = fromIntegral (L.length grp + 1) : grp
