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
import Tamal.Alu (dataResult)
import qualified Tamal.Branch as Br
import Tamal.Bus.Serdes (Lanes, hiZ)
import Tamal.Config (AlertSource (..), Config (..), IoMode (..), Role (..), Sck (..), decodeConfig)
import Tamal.Isa (Instr (..), Reg, decode)
import Tamal.RegFile (Regs, initRegs, readReg, writeReg)

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
  Exec -> stepExec s inp
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

stepExec :: State -> BusIn -> (State, BusOut, Maybe Ring)
stepExec s inp = case decode (instrWord inp) of
  Left _ -> haltWith True 1 0 (safePins s) -- decode error -> reason 1
  Right i -> execInstr i s inp

-- | Advance to the next sequential instruction.
advance :: State -> State
advance s = s{phase = Fetch, pc = pc s + 1}

{- | Fixed terminator slot (top of the ring address space; the
top-shell sizes it).
-}
termAddr :: Unsigned 12
termAddr = maxBound

-- | Drive pins safe (used by TRAP): CS# high, lanes hi-Z, SCK low, RESET# high.
safePins :: State -> State
safePins s = s{csN = 1, sck = 0, rstN = 1, lanes = hiZ}

{- | Emit the HALT terminator (§7.4). Builds the word directly (mirrors
'Tamal.Trace.encodeRecord'’s Halt layout) so 'step' stays synthesizable.
-}
haltWith :: Bool -> BitVector 3 -> BitVector 8 -> State -> (State, BusOut, Maybe Ring)
haltWith trap reason status s =
  let s' = s{phase = Halted}
      w = bitCoerce (0b11 :: BitVector 2, 0 :: BitVector 17, reason, trap, (ovf s), status)
   in (s', busOut s', Just (Ring termAddr w))

execInstr :: Instr -> State -> BusIn -> (State, BusOut, Maybe Ring)
execInstr i s inp = case i of
  LoadImm rd _ -> dataWb rd
  Lui rd _ -> dataWb rd
  Mov rd _ -> dataWb rd
  Add rd _ _ -> dataWb rd
  Addi rd _ _ -> dataWb rd
  Sub rd _ _ -> dataWb rd
  And_ rd _ _ -> dataWb rd
  Andi rd _ _ -> dataWb rd
  Or_ rd _ _ -> dataWb rd
  Ori rd _ _ -> dataWb rd
  Xor_ rd _ _ -> dataWb rd
  Xori rd _ _ -> dataWb rd
  Shift rd _ _ _ -> dataWb rd
  Halt st -> haltWith False 0 st s
  Rdsr rd srn
    | srn == 0 ->
        let s' = (advance s){regs = writeReg (regs s) rd (zeroExtend (rxCrc s))}
         in (s', busOut s', Nothing)
    | otherwise -> haltWith True 3 0 (safePins s) -- reserved sr# -> reason 3
  Beq a b off -> branch Br.Beq a b off
  Bne a b off -> branch Br.Bne a b off
  Bltu a b off -> branch Br.Bltu a b off
  Bgeu a b off -> branch Br.Bgeu a b off
  CsAssert -> pinOp (s{csN = 0})
  CsDeassert -> pinOp (s{csN = 1, lanes = hiZ})
  RstAssert -> pinOp (s{rstN = 0})
  RstDeassert -> pinOp (s{rstN = 1})
  CrcReset -> pinOp (s{rxCrc = 0})
  SetConfig p -> case decodeConfig p of
    Right c -> pinOp (s{cfg = c})
    Left _ -> haltWith True 2 0 (safePins s) -- unsupported config -> reason 2
  GetAlert rd ->
    let b =
          if cfgAlertSource (cfg s) == AlertPin
            then alertIn inp
            else ioIn inp !! 1
        s' = (advance s){regs = writeReg (regs s) rd (zeroExtend (pack b))}
     in (s', busOut s', Nothing)
  _ -> (advance s, busOut s, Nothing) -- other opcodes: later tasks
 where
  rs1v = readReg (regs s) (operandRs1 i)
  rs2v = readReg (regs s) (operandRs2 i)
  dataWb rd =
    let s' = (advance s){regs = writeReg (regs s) rd (dataResult i rs1v rs2v)}
     in (s', busOut s', Nothing)
  branch op a b off =
    let taken = Br.branchTaken op (readReg (regs s) a) (readReg (regs s) b)
        -- offset is 11-bit signed; PC is AW=10-bit. Take the low AW bits;
        -- pc + off ≡ pc + (off mod 2^AW) (mod 2^AW)
        offAw = unpack (truncateB off) :: Unsigned AW
        s'
          | taken = s{phase = Fetch, pc = pc s + offAw}
          | otherwise = advance s
     in (s', busOut s', Nothing)

operandRs1 :: Instr -> Reg
operandRs1 = \case
  Mov _ a -> a
  Add _ a _ -> a
  Addi _ a _ -> a
  Sub _ a _ -> a
  And_ _ a _ -> a
  Andi _ a _ -> a
  Or_ _ a _ -> a
  Ori _ a _ -> a
  Xor_ _ a _ -> a
  Xori _ a _ -> a
  Shift _ a _ _ -> a
  Beq a _ _ -> a
  Bne a _ _ -> a
  Bltu a _ _ -> a
  Bgeu a _ _ -> a
  PutByteReg a -> a
  PutBitsReg a _ -> a
  TarReg a -> a
  Mark _ a -> a
  _ -> 0

operandRs2 :: Instr -> Reg
operandRs2 = \case
  Add _ _ b -> b
  Sub _ _ b -> b
  And_ _ _ b -> b
  Or_ _ _ b -> b
  Xor_ _ _ b -> b
  Beq _ b _ -> b
  Bne _ b _ -> b
  Bltu _ b _ -> b
  Bgeu _ b _ -> b
  _ -> 0

{- | A pin/state op: apply the state update, then advance to the next
instruction.
-}
pinOp :: State -> (State, BusOut, Maybe Ring)
pinOp s = let s' = advance s in (s', busOut s', Nothing)
