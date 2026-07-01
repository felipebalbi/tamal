-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0

{- |
CTRL-group branch comparator (ALU/branch design §8). Pure, combinational,
single-cycle. 'branchTaken' returns only "taken?"; the PC / branch-offset math
is the Engine's job. Unsigned compares (@Bltu@/@Bgeu@) use 'BitVector''s
unsigned 'Ord' — there are no signed branches in v1.
-}
module Tamal.Branch
  ( BranchOp (..)
  , branchTaken
  ) where

import Clash.Prelude

data BranchOp = Beq | Bne | Bltu | Bgeu
  deriving stock (Generic, Show, Eq, Enum, Bounded)
  deriving anyclass (NFDataX)

branchTaken :: BranchOp -> BitVector 32 -> BitVector 32 -> Bool
branchTaken op r1 r2 = case op of
  Beq -> r1 == r2
  Bne -> r1 /= r2
  Bltu -> r1 < r2
  Bgeu -> r1 >= r2
