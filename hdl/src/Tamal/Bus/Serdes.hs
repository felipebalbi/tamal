{- |
Pure single-I/O (x1) byte serialisation and the turnaround (TAR) beat
vector (spec §5). A 'Lanes' value is the per-beat drive state of the four
I/O lanes: @(output value, output enable)@. @oe = 0@ means tri-stated.

x1 rules: PUT drives the data bit on IO[0], MSB first, with IO[1..3]
tri-stated; GET samples IO[1] with all engine drivers tri-stated. Dual/
quad maps land in Phase 3.
-}
module Tamal.Bus.Serdes
  ( Lane
  , Lanes
  , hiZ
  , driveHigh
  , serializeX1
  , deserializeX1
  , tarBeat
  ) where

import Clash.Prelude

type Lane  = (Bit, Bit)   -- (output value, output enable)
type Lanes = Vec 4 Lane

-- | All four lanes tri-stated.
hiZ :: Lanes
hiZ = repeat (0, 0)

-- | All four lanes actively driven to logic 1 (TAR first clock).
driveHigh :: Lanes
driveHigh = repeat (1, 1)

-- | One byte -> eight beats, MSB first, driving IO[0] only.
serializeX1 :: BitVector 8 -> Vec 8 Lanes
serializeX1 b = map beat (unpack b :: Vec 8 Bit)
  where
    beat :: Bit -> Lanes
    beat bit = (bit, 1) :> (0, 0) :> (0, 0) :> (0, 0) :> Nil

-- | Eight IO[1] samples (MSB first) -> one byte.
deserializeX1 :: Vec 8 Bit -> BitVector 8
deserializeX1 = pack

-- | TAR beat @i@: beat 0 drives all lanes high, subsequent beats tri-state.
tarBeat :: Unsigned 4 -> Lanes
tarBeat i = if i == 0 then driveHigh else hiZ
