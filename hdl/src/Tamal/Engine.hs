-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0
{-# LANGUAGE NumericUnderscores #-}

{- |
The tamal engine: a pure Mealy transition that composes every leaf into a
running eSPI shift engine (design doc 2026-07-02-tamal-engine-design.md).
'step' dispatches on 'phase' to per-phase helpers; all pin drives live in
'State' and are projected by 'busOut'.
-}
module Tamal.Engine
  ( Phase (..)
  , Pending (..)
  , State (..)
  , BusIn (..)
  , BusOut (..)
  , Ring (..)
  , initState
  , powerUpDefault
  , revisionWord
  , busOut
  , step
  ) where

import Clash.Prelude
import Tamal.Bus.Serdes (Lanes, hiZ)
import Tamal.Config (AlertSource (..), Config (..), IoMode (..), Role (..), Sck (..))
import Tamal.Isa (Reg)
import Tamal.RegFile (Regs, initRegs)

-- | Program-address width (word index): 1024-word store (§ D3).
type AW = 10

data Phase
  = Idle
  | Preamble
  | Fetch
  | Exec
  | BusBeat
  | TraceEmit
  | WaitAlert
  | Halted
  deriving stock (Generic, Show, Eq)
  deriving anyclass (NFDataX)

data Pending
  = PendNone
  | PendGet Reg (Unsigned 4) Bool -- rd, nbits, update RX CRC?
  | PendMark (BitVector 32) -- MARK payload emitted by TraceEmit
  deriving stock (Generic, Show, Eq)
  deriving anyclass (NFDataX)

data State = State
  { phase :: Phase
  , pc :: Unsigned AW
  , regs :: Regs
  , cfg :: Config
  , rxCrc :: BitVector 8
  , ringPtr :: Unsigned 12
  , ovf :: Bool
  , csN :: Bit
  , sck :: Bit
  , rstN :: Bit
  , lanes :: Lanes
  , busPhase :: Index 5
  , beatIx :: Unsigned 4
  , beatTot :: Unsigned 4
  , shifter :: BitVector 8
  , pending :: Pending
  , waitTimer :: BitVector 9
  }
  deriving stock (Generic, Show, Eq)
  deriving anyclass (NFDataX)

data BusIn = BusIn
  { instrWord :: BitVector 32
  , ioIn :: Vec 4 Bit
  , alertIn :: Bit
  , startIn :: Bool
  }
  deriving stock (Generic, Show, Eq)
  deriving anyclass (NFDataX)

data BusOut = BusOut
  { pcOut :: Unsigned AW
  , csOut :: Bit
  , sckOut :: Bit
  , rstOut :: Bit
  , lanesOut :: Lanes
  , haltedOut :: Bool
  }
  deriving stock (Generic, Show, Eq)
  deriving anyclass (NFDataX)

data Ring = Ring
  { rAddr :: Unsigned 12
  , rData :: BitVector 32
  }
  deriving stock (Generic, Show, Eq)
  deriving anyclass (NFDataX)

-- | Power-up config default (§7.2): controller / x1 / 20 MHz / ALERT# pin.
powerUpDefault :: Config
powerUpDefault = Config Controller X1 Sck20 AlertPin

-- | REVISION word [major8 | minor8 | patch16] = v0.1.0.
revisionWord :: BitVector 32
revisionWord = 0x00_01_0000

-- | Power-up state: Idle, pins safe, config default, ring pointer at 1.
initState :: State
initState =
  State
    { phase = Idle
    , pc = 0
    , regs = initRegs
    , cfg = powerUpDefault
    , rxCrc = 0
    , ringPtr = 1
    , ovf = False
    , csN = 1
    , sck = 0
    , rstN = 1
    , lanes = hiZ
    , busPhase = 0
    , beatIx = 0
    , beatTot = 0
    , shifter = 0
    , pending = PendNone
    , waitTimer = 0
    }

-- | Project the registered pin state to the output record.
busOut :: State -> BusOut
busOut s =
  BusOut
    { pcOut = pc s
    , csOut = csN s
    , sckOut = sck s
    , rstOut = rstN s
    , lanesOut = lanes s
    , haltedOut = phase s == Halted
    }

-- | Soft-init on 'startIn' (§8 / D9): reset run state.
softInit :: State
softInit = initState{phase = Preamble}

step :: State -> BusIn -> (State, BusOut, Maybe Ring)
step s inp = case phase s of
  Idle -> stepIdle s inp
  Halted -> stepHalted s inp
  Preamble -> stepPreamble s inp
  Fetch -> stepFetch s inp
  _ -> (s, busOut s, Nothing) -- filled in by later tasks

stepIdle :: State -> BusIn -> (State, BusOut, Maybe Ring)
stepIdle s inp
  | startIn inp = (softInit, busOut s, Nothing)
  | otherwise = (s, busOut s, Nothing)

stepHalted :: State -> BusIn -> (State, BusOut, Maybe Ring)
stepHalted s inp
  | startIn inp = (softInit, busOut s, Nothing)
  | otherwise = (s, busOut s, Nothing)

stepPreamble :: State -> BusIn -> (State, BusOut, Maybe Ring)
stepPreamble s _ = (s{phase = Fetch}, busOut s, Just (Ring 0 revisionWord))

stepFetch :: State -> BusIn -> (State, BusOut, Maybe Ring)
stepFetch s _ = (s{phase = Exec}, busOut s, Nothing)
