-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0

module Test.Engine (tests) where

import Clash.Prelude
import qualified Data.List as L
import Test.Tasty
import Test.Tasty.HUnit

import Tamal.Bus.Serdes (hiZ)
import Tamal.Engine

tests :: TestTree
tests =
  testGroup
    "Engine"
    [ testCase "initState: Idle, pins safe" $ do
        phase initState @?= Idle
        csN initState @?= 1
        sck initState @?= 0
        rstN initState @?= 1
        lanes initState @?= hiZ
    , testCase "Idle without start stays Idle"
        $ let (s', _, r) = step initState (BusIn 0 (repeat 0) 0 False)
           in (phase s', r) @?= (Idle, Nothing)
    , testCase "start soft-inits into Preamble"
        $ let (s', _, _) = step initState (BusIn 0 (repeat 0) 0 True)
           in phase s' @?= Preamble
    , testCase "Preamble writes REVISION at ring[0]"
        $ let Run _ ring _ = drive 5 (const 0) (\t -> (repeat 0, 0, t == 0))
           in L.take 1 ring @?= [Ring 0 revisionWord]
    ]

-- | A run result: final state + ring writes (in emission order) + cycles used.
data Run = Run
  { runState :: State
  , runRing :: [Ring]
  , runCycles :: Int
  }

-- | Program memory backed by a plain list (test-only; 0-filled past the end).
memOf :: [BitVector 32] -> Unsigned 10 -> BitVector 32
memOf prog a = (prog L.++ L.repeat 0) L.!! fromIntegral a

{- | Fold 'step' for up to @budget@ cycles or until Halted. Models the 1-cycle
instruction-BRAM latency: @instrWord(t) = mem[pcOut(t-1)]@. @inputs@ supplies
@(ioIn, alert, start)@ per cycle.
-}
drive ::
  Int ->
  (Unsigned 10 -> BitVector 32) ->
  (Int -> (Vec 4 Bit, Bit, Bool)) ->
  Run
drive budget mem inputs = go 0 initState 0 []
 where
  go !t !s !prevPc !ring
    | phase s == Halted && t > 0 = Run s (L.reverse ring) t
    | t >= budget = Run s (L.reverse ring) t
    | otherwise =
        let (io, alert, start) = inputs t
            inp = BusIn (mem prevPc) io alert start
            (s', bo, mr) = step s inp
            ring' = maybe ring (: ring) mr
         in go (t + 1) s' (pcOut bo) ring'

-- | Run a fixed program: all-zero io, start pulsed at cycle 0.
runProg :: Int -> [BitVector 32] -> Run
runProg budget prog = drive budget (memOf prog) inputs
 where
  inputs 0 = (repeat 0, 0, True)
  inputs _ = (repeat 0, 0, False)
