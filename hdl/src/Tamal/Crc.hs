{- |
RX CRC-8 primitive used by the tamal engine's reception path.

Parameters (eSPI / SMBus PEC): polynomial @0x07@ (x^8 + x^2 + x + 1),
initial value @0x00@, most-significant-bit first, no input/output
reflection, no final XOR. The residue of a correct message followed by
its CRC byte is @0x00@.
-}
module Tamal.Crc
  ( crc8Update
  ) where

import Clash.Prelude

-- | Fold one byte into the running CRC-8, processing bit 7 down to bit 0.
crc8Update :: BitVector 8 -> BitVector 8 -> BitVector 8
crc8Update crc byte = foldl step crc (unpack byte :: Vec 8 Bit)
  where
    step :: BitVector 8 -> Bit -> BitVector 8
    step c inBit
      | feedbackBit == high = shifted `xor` 0x07
      | otherwise           = shifted
      where
        feedbackBit = msb c `xor` inBit
        shifted     = c `shiftL` 1
