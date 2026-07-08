-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-W-2.0

{- |
Engine configuration decoded from the @SET_CONFIG@ payload (spec §7.2).
v1 implements only controller role, single I/O, and 20 MHz SCK; any other
selection is a decode error (the engine turns these into a TRAP).
-}
module Tamal.Config
  ( Role (..)
  , IoMode (..)
  , Sck (..)
  , AlertSource (..)
  , Config (..)
  , ConfigError (..)
  , decodeConfig
  ) where

import Clash.Prelude

-- | Link role. v1 is controller-only; @Target@ is reserved.
data Role = Controller | Target
  deriving stock (Generic, Show, Eq)
  deriving anyclass (NFDataX)

-- | I/O width. v1 is single-lane (@X1@); @X2@/@X4@ land in Phase 3.
data IoMode = X1 | X2 | X4
  deriving stock (Generic, Show, Eq)
  deriving anyclass (NFDataX)

-- | SCK frequency selection. v1 accepts only 20 MHz (@Sck20@).
data Sck = Sck20 | Sck33 | Sck50 | Sck66
  deriving stock (Generic, Show, Eq)
  deriving anyclass (NFDataX)

-- | Where alerts are observed: the dedicated @ALERT#@ pin or in-band on IO[1].
data AlertSource = AlertPin | AlertIo1
  deriving stock (Generic, Show, Eq)
  deriving anyclass (NFDataX)

-- | The decoded engine configuration (one field per @SET_CONFIG@ sub-field).
data Config = Config
  { cfgRole :: Role
  , cfgIoMode :: IoMode
  , cfgSck :: Sck
  , cfgAlertSource :: AlertSource
  }
  deriving stock (Generic, Show, Eq)
  deriving anyclass (NFDataX)

-- | Why a @SET_CONFIG@ payload was rejected (each becomes a TRAP in the engine).
data ConfigError
  = UnsupportedRole
  | UnsupportedIoMode
  | UnsupportedSck
  deriving stock (Generic, Show, Eq)
  deriving anyclass (NFDataX)

{- | Decode a 6-bit @SET_CONFIG@ payload — @[5]=role, [4:3]=io_mode, [2:1]=sck,
[0]=alert_source@ — into a 'Config', rejecting any non-v1 selection.
-}
decodeConfig :: BitVector 6 -> Either ConfigError Config
decodeConfig p =
  case (role, io, sck) of
    (0b0, 0b00, 0b00) -> Right (Config Controller X1 Sck20 alertSrc)
    (0b1, _, _) -> Left UnsupportedRole
    (_, io', _) | io' /= 0b00 -> Left UnsupportedIoMode
    _ -> Left UnsupportedSck
 where
  (role, io, sck, alert) = bitCoerce p :: (BitVector 1, BitVector 2, BitVector 2, BitVector 1)
  alertSrc = if alert == 0 then AlertPin else AlertIo1
