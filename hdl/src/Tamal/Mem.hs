-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0

{- |
The two block-RAM memories that flank the engine (design doc
2026-07-02-tamal-bram-design.md): the instruction store the engine fetches from,
and the trace ring it writes records into. Both are thin 'blockRamPow2' wrappers
addressed with the engine's own 'Unsigned' widths. This module is a
dependency-free leaf: it does not import 'Tamal.Engine' (the topEntity projects
@Maybe Ring@ to the tuple write port).
-}
module Tamal.Mem
  ( instrRam
  , ringRam
  ) where

import Clash.Prelude

import Tamal.Params (AW, RW)

{- | Instruction store: 1024 words (2^10), zero-initialized. Read address is the
engine's registered PC (@pcOut :: Unsigned AW@); the write port is the loader's
@Maybe (addr, word)@. The output feeds @BusIn.instrWord@. The 1-cycle read
latency IS the engine's @Fetch@ bubble. @blockRamPow2@ uses only clock + enable
(no content reset), matching the no-reset power-up design.
-}
instrRam ::
  forall dom.
  (HiddenClockResetEnable dom) =>
  Signal dom (Unsigned AW) ->
  Signal dom (Maybe (Unsigned AW, BitVector 32)) ->
  Signal dom (BitVector 32)
instrRam = blockRamPow2 (repeat 0)

-- | Trace ring: 4096 words (2^12), zero-initialized. Implemented in Task 3.
ringRam ::
  (HiddenClockResetEnable dom) =>
  Signal dom (Unsigned RW) ->
  Signal dom (Maybe (Unsigned RW, BitVector 32)) ->
  Signal dom (BitVector 32)
ringRam = blockRamPow2 (repeat 0)
