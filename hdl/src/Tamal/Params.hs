-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-W-2.0

{- |
Shared machine parameters: the block-RAM address widths that flank the engine.
This is a dependency-free leaf (imports only 'Clash.Prelude') so any component —
the engine, the two BRAM wrappers ('Tamal.Mem'), the loader, and the trace model
('Tamal.Trace') — can name the same width without importing one another. Widening
the instruction space (a bigger PC) or resizing the ring is then a single edit
here rather than a hunt for hand-copied @Unsigned 10@/@Unsigned 12@ literals.
-}
module Tamal.Params
  ( AW
  , RW
  ) where

import Clash.Prelude (Nat)

{- | Instruction-address width (word index). @2^AW@ words in the instruction
store; also the program-counter width. 1024 words at @AW = 10@.
-}
type AW = 10 :: Nat

{- | Ring/trace-address width. @2^RW@ words in the result ring, the top slot of
which is the reserved HALT terminator. 4096 words at @RW = 12@.
-}
type RW = 12 :: Nat
