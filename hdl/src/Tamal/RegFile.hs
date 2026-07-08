-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-W-2.0

{- |
16×32 register file (register-file design). A pure leaf: 'Regs' is an opaque
'NFDataX' value the Engine will hold in its Mealy 'State'; 'readReg'/'writeReg'
are pure and combinational, and 'x0' is hardwired to 0. See
docs/superpowers/specs/2026-07-01-tamal-register-file-design.md.
-}
module Tamal.RegFile
  ( Regs
  , initRegs
  , readReg
  , writeReg
  ) where

import Clash.Prelude
import Tamal.Isa (Reg)

{- | The 16-entry register bank, opaque so it can only be touched through
'initRegs' / 'readReg' / 'writeReg'. The Engine holds one in its Mealy state.
-}
newtype Regs = Regs (Vec 16 (BitVector 32))
  deriving stock (Generic, Show, Eq)
  deriving anyclass (NFDataX)

-- | Power-up contents: all 16 registers zeroed.
initRegs :: Regs
initRegs = Regs (repeat 0)

{- | Physical slot for a 5-bit selector: the low 4 bits. Out-of-window selectors
(x16..x31) alias their low-4 twin, keeping the leaf total.
-}
regIndex :: Reg -> Index 16
regIndex r = unpack (truncateB r)

-- | Read a register value; @x0@ (index 0) reads 0 regardless of slot contents.
readReg :: Regs -> Reg -> BitVector 32
readReg (Regs v) r
  | idx == 0 = 0 -- x0 is hardwired to 0.
  | otherwise = v !! idx
 where
  idx = regIndex r

-- | Write a register value; writes to @x0@ (index 0) are discarded.
writeReg :: Regs -> Reg -> BitVector 32 -> Regs
writeReg regs@(Regs v) r x
  | idx == 0 = regs -- writes to x0 are ignored!
  | otherwise = Regs (replace idx x v)
 where
  idx = regIndex r
