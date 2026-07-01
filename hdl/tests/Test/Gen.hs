module Test.Gen
  ( genBit
  , genByte
  ) where

import Clash.Prelude
import Clash.Hedgehog.Sized.BitVector (genDefinedBitVector)
import Hedgehog (Gen)
import qualified Hedgehog.Gen as Gen

-- | A single defined Bit (0 or 1).
genBit :: Gen Bit
genBit = Gen.element [0, 1]

-- | A defined 8-bit value.
genByte :: Gen (BitVector 8)
genByte = genDefinedBitVector
