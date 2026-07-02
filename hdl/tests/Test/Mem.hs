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
import Tamal.Mem (instrRam, ringRam)

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

{- | Sample 'ringRam' over a stimulus, dropping the undefined cycle-0 output.
Same shape as 'simInstr' at the ring's 'Unsigned 12' width.
-}
simRing :: [Unsigned 12] -> [Maybe (Unsigned 12, BitVector 32)] -> [BitVector 32]
simRing addrs writes =
  L.drop 1
    $ sampleN
      (L.length addrs + 1)
      ( ringRam (fromList (addrs <> L.repeat 0)) (fromList (writes <> L.repeat Nothing)) ::
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
    , testCase "instr: read-before-write collision returns old then new"
        $
        -- at cycle 1, write 0x2222 to addr 3 WHILE reading addr 3: the read still
        -- yields the old 0x1111 (out[2]); 0x2222 appears from out[3].
        simInstr
          [0, 3, 3, 3]
          [Just (3, 0x1111), Just (3, 0x2222), Nothing, Nothing]
        @?= [0, 0x1111, 0x2222, 0x2222]
    , testCase "instr: address 0 is a normal slot (no x0 hardwiring here)"
        $ simInstr
          [0, 0, 0]
          [Just (0, 0xCAFE_F00D), Nothing, Nothing]
        @?= [0, 0xCAFE_F00D, 0xCAFE_F00D]
    , testCase "instr: top address (maxBound = 1023) reads back"
        $ simInstr
          [0, maxBound, maxBound]
          [Just (maxBound, 0x0BAD_C0DE), Nothing, Nothing]
        @?= [0, 0x0BAD_C0DE, 0x0BAD_C0DE]
    , testCase "ring: write then read-back at a mid address"
        $ simRing
          [0, 42, 42, 42]
          [Just (42, 0x1234_5678), Nothing, Nothing, Nothing]
        @?= [0, 0x1234_5678, 0x1234_5678, 0x1234_5678]
    , testCase "ring: drain sweep streams the written block in order"
        $
        -- write 4 words to addrs 100..103 (one per cycle), then sweep-read
        -- 100..103. sweep = take 4 (drop 4 ..): reads issued at cycles 4..7
        -- surface post-latency, by which point all 4 writes have landed.
        let blk = [0xA0, 0xA1, 0xA2, 0xA3] :: [BitVector 32]
            base = 100 :: Unsigned 12
            writes = [Just (base + i, v) | (i, v) <- L.zip [0 .. 3] blk] <> L.replicate 4 Nothing
            addrs = L.replicate 4 0 <> [base + i | i <- [0 .. 3]]
         in L.take 4 (L.drop 4 (simRing addrs writes)) @?= blk
    ]
