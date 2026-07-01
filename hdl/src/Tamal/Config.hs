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

data Role        = Controller | Target        deriving stock (Generic, Show, Eq) deriving anyclass NFDataX
data IoMode      = X1 | X2 | X4               deriving stock (Generic, Show, Eq) deriving anyclass NFDataX
data Sck         = Sck20 | Sck33 | Sck50 | Sck66 deriving stock (Generic, Show, Eq) deriving anyclass NFDataX
data AlertSource = AlertPin | AlertIo1        deriving stock (Generic, Show, Eq) deriving anyclass NFDataX

data Config = Config
  { cfgRole        :: Role
  , cfgIoMode      :: IoMode
  , cfgSck         :: Sck
  , cfgAlertSource :: AlertSource
  }
  deriving stock (Generic, Show, Eq)
  deriving anyclass NFDataX

data ConfigError
  = UnsupportedRole
  | UnsupportedIoMode
  | UnsupportedSck
  deriving stock (Generic, Show, Eq)
  deriving anyclass NFDataX

-- payload[5]=role, [4:3]=io_mode, [2:1]=sck, [0]=alert_source
decodeConfig :: BitVector 6 -> Either ConfigError Config
decodeConfig p =
  case (role, io, sck) of
    (0b0, 0b00, 0b00) -> Right (Config Controller X1 Sck20 alertSrc)
    (0b1, _,    _   ) -> Left UnsupportedRole
    (_,   io',  _   ) | io' /= 0b00 -> Left UnsupportedIoMode
    _                 -> Left UnsupportedSck
  where
    (role, io, sck, alert) = bitCoerce p :: (BitVector 1, BitVector 2, BitVector 2, BitVector 1)
    alertSrc = if alert == 0 then AlertPin else AlertIo1
