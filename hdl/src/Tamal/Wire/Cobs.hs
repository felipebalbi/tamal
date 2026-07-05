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
  , cobsDecode
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
    -- Group full FIRST: a 254-byte group flushes as a 0xFF continuation (no
    -- implied zero), then @b@ is reprocessed in a fresh group. Checking this
    -- before the @b == 0@ case is load-bearing: a zero arriving on a full group
    -- must terminate a *fresh* empty group (…FF,<254>,01,…) — folding it into the
    -- full group as code 0xFF would drop the zero on decode.
    | L.length grp == 254 = emit grp <> go [] (b : bs) -- group full: flush, reprocess b
    | b == 0 = emit grp <> go [] bs -- zero terminates the group (implied)
    | otherwise = go (grp <> [b]) bs
  emit grp = fromIntegral (L.length grp + 1) : grp

{- | COBS-decode a delimiter-stripped byte sequence, or 'Nothing' if malformed
(a code byte demanding more bytes than remain, any @0x00@ in the input, or an
empty list). Inverse of 'cobsEncode': @cobsDecode (cobsEncode x) == Just x@.
-}
cobsDecode :: [BitVector 8] -> Maybe [BitVector 8]
cobsDecode bytes
  | L.null bytes = Nothing -- empty is not a valid frame
  | L.elem 0 bytes = Nothing -- a valid (delimiter-stripped) frame has no 0x00
  | otherwise = go bytes
 where
  go [] = Just [] -- unreachable given the guards. Here merely to make sure `go` is a total function
  go (code : rest) =
    let n = fromIntegral code - 1 -- data bytes in this group
     in do
          (grp, more) <- takeExactly n rest
          if L.null more
            then Just grp -- final group: no trailing zero
            else do
              decoded <- go more
              -- a non-full group (<255) had a zero the encoder consumed
              Just (grp <> (if code == 255 then [] else [0]) <> decoded)

-- | Split off exactly @k@ elements, or 'Nothing' if fewer than @k@ remain.
takeExactly :: Int -> [a] -> Maybe ([a], [a])
takeExactly 0 xs = Just ([], xs)
takeExactly _ [] = Nothing
takeExactly k (x : xs) = do
  (ys, zs) <- takeExactly (k - 1) xs
  Just (x : ys, zs)
