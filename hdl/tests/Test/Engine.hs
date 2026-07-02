-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0

module Test.Engine (tests) where

import Clash.Prelude
import qualified Data.List as L
import qualified Hedgehog as H
import qualified Hedgehog.Gen as Gen
import qualified Hedgehog.Range as Range
import Test.Tasty
import Test.Tasty.HUnit
import Test.Tasty.Hedgehog (testProperty)

import Tamal.Alu (dataResult)
import Tamal.Bus.Serdes (hiZ)
import Tamal.Engine
import Tamal.Isa (Instr (..), Reg, encode)
import Tamal.RegFile (Regs, initRegs, readReg, writeReg)

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
    , testProperty "DATA program: engine regs == refRegs" $ H.property $ do
        prog <- H.forAll (Gen.list (Range.linear 1 12) genDataOnly)
        let img = fmap encode prog <> [encode (Halt 0)]
            Run s _ _ = runProg 400 img
        H.assert (phase s == Halted)
        let regsE = fmap (readRegE s) [0 .. 15]
            regsR = fmap (readReg (refRegs prog)) [0 .. 15]
        regsE H.=== regsR
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

-- | Read a register out of the engine's final state.
readRegE :: State -> Reg -> BitVector 32
readRegE s = readReg (regs s)

-- | Instantaneous reference for straight-line DATA-compute programs.
refRegs :: [Instr] -> Regs
refRegs = L.foldl' apply initRegs
 where
  apply rs i = case i of
    LoadImm rd _ -> wb rs i rd
    Lui rd _ -> wb rs i rd
    Mov rd _ -> wb rs i rd
    Add rd _ _ -> wb rs i rd
    Addi rd _ _ -> wb rs i rd
    Sub rd _ _ -> wb rs i rd
    And_ rd _ _ -> wb rs i rd
    Andi rd _ _ -> wb rs i rd
    Or_ rd _ _ -> wb rs i rd
    Ori rd _ _ -> wb rs i rd
    Xor_ rd _ _ -> wb rs i rd
    Xori rd _ _ -> wb rs i rd
    Shift rd _ _ _ -> wb rs i rd
    _ -> rs

-- | DATA writeback: rd := dataResult i rs1v rs2v (mirrors the engine).
wb :: Regs -> Instr -> Reg -> Regs
wb rs i rd = writeReg rs rd (dataResult i (readReg rs (src1 i)) (readReg rs (src2 i)))

src1 :: Instr -> Reg
src1 = \case
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
  _ -> 0

src2 :: Instr -> Reg
src2 = \case
  Add _ _ b -> b
  Sub _ _ b -> b
  And_ _ _ b -> b
  Or_ _ _ b -> b
  Xor_ _ _ b -> b
  _ -> 0

-- | Generate a target register in x1..x4 (keeps programs small + observable).
genRd :: H.Gen Reg
genRd = Gen.element [1, 2, 3, 4]

-- | A single straight-line DATA-compute instruction.
genDataOnly :: H.Gen Instr
genDataOnly =
  Gen.choice
    [ LoadImm <$> genRd <*> genImm11
    , Mov <$> genRd <*> genRd
    , Add <$> genRd <*> genRd <*> genRd
    , Addi <$> genRd <*> genRd <*> genImm11
    , Sub <$> genRd <*> genRd <*> genRd
    , Xor_ <$> genRd <*> genRd <*> genRd
    ]
 where
  genImm11 = fromIntegral <$> Gen.int (Range.linearFrom 0 (-1024) 1023)
