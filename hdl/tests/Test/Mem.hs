-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0
{-# LANGUAGE NumericUnderscores #-}

module Test.Mem (tests) where

import Clash.Prelude
import qualified Data.List as L
import Data.Maybe (fromMaybe)
import Test.Tasty
import Test.Tasty.HUnit

import Tamal.Domain (Dom100)
import Tamal.Mem (instrRam)

{- | Pure oracle mirroring 'Clash.Prelude.blockRam' exactly: zero-init, 1-cycle
read latency, read-before-write. Produces @[out 1, out 2, ..]@ (the undefined
@out 0@ is dropped by the sampler, so the lists align). The assoc list is
most-recent-first, so 'L.lookup' returns the value from BEFORE the current
cycle's write.
-}
refRam ::
  (KnownNat n) =>
  [Unsigned n] ->
  [Maybe (Unsigned n, BitVector 32)] ->
  [BitVector 32]
refRam addrs writes = go [] (L.zip addrs writes)
 where
  go _ [] = []
  go mem ((a, w) : zs) = fromMaybe 0 (L.lookup a mem) : go (push w mem) zs
  push Nothing m = m
  push (Just (wa, wd)) m = (wa, wd) : m

{- | Sample 'instrRam' over a stimulus, dropping the undefined cycle-0 output.
'sampleN' supplies clock/reset/enable to the @HiddenClockResetEnable@ signal; the
RAM is applied directly inside it (the @Test.Uart@ idiom), and the inline
@:: Signal Dom100 _@ annotation pins the domain so @sampleN@ can solve
@KnownDomain@.
-}
simInstr :: [Unsigned 10] -> [Maybe (Unsigned 10, BitVector 32)] -> [BitVector 32]
simInstr addrs writes =
  L.drop 1
    $ sampleN
      (L.length addrs + 1)
      ( instrRam (fromList (addrs <> L.repeat 0)) (fromList (writes <> L.repeat Nothing)) ::
          Signal Dom100 (BitVector 32)
      )

tests :: TestTree
tests =
  testGroup
    "Mem"
    [ testCase "instr: write then read-back, exactly 1-cycle latency" $ do
        -- write 0xDEAD_BEEF to addr 5 at cycle 0, read addr 5 thereafter.
        -- read-before-write: the cycle-0 read of addr 5 still sees 0 (out[1]);
        -- the written value appears from out[2] onward. Both the hardware sampler
        -- and the reference oracle must agree with the concrete expectation.
        let addrs = [0, 5, 5, 5]
            writes = [Just (5, 0xDEAD_BEEF), Nothing, Nothing, Nothing]
            expected = [0, 0xDEAD_BEEF, 0xDEAD_BEEF, 0xDEAD_BEEF]
        simInstr addrs writes @?= expected
        refRam addrs writes @?= expected
    ]
