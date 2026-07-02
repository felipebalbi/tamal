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
    , testCase "RDSR sr=0 reads rxCrc into rd; sr/=0 traps (reason 3)" $ do
        let ok = [encode (Rdsr 1 0), encode (Halt 0)]
            Run s0 _ _ = runProg 40 ok
        phase s0 @?= Halted
        readReg (regs s0) 1 @?= 0
        let bad = [encode (Rdsr 1 1), encode (Halt 0)]
            Run _ ring _ = runProg 40 bad
        haltReason (L.last ring) @?= (True, 3)
    , testCase "BEQ taken skips; BNE backward loops via counter" $ do
        let prog =
              [ encode (LoadImm 1 3) -- 0: x1 = 3
              , encode (Addi 1 1 (-1)) -- 1: x1 = x1 - 1  (loop head)
              , encode (Bne 1 0 (-1)) -- 2: if x1 /= 0 goto 1
              , encode (Halt 0) -- 3
              ]
            Run s _ c = runProg 400 prog
        phase s @?= Halted
        readReg (regs s) 1 @?= 0
        assertBool "loop ran (>10 cycles)" (c > 10)
    , testCase "BEQ not taken falls through" $ do
        let prog = [encode (LoadImm 1 5), encode (Beq 1 0 5), encode (Halt 0x22)]
            Run _ ring _ = runProg 60 prog
        haltStatus (L.last ring) @?= 0x22
    , testCase "CS/RST ops latch into the pin state" $ do
        let Run s1 _ _ = runProg 30 [encode CsAssert, encode (Halt 0)]
        csN s1 @?= 0
        let Run s2 _ _ = runProg 30 [encode CsAssert, encode CsDeassert, encode (Halt 0)]
        csN s2 @?= 1
        lanes s2 @?= hiZ
        let Run s3 _ _ = runProg 30 [encode RstAssert, encode (Halt 0)]
        rstN s3 @?= 0
    , testCase "CRC_RESET runs; SET_CONFIG unsupported traps (reason 2)" $ do
        let Run s _ _ = runProg 30 [encode CrcReset, encode (Halt 0)]
        phase s @?= Halted
        rxCrc s @?= 0
        let bad = [encode (SetConfig 0b100000), encode (Halt 0)]
            Run _ ring _ = runProg 30 bad
        haltReason (L.last ring) @?= (True, 2)
    , testCase "GET_ALERT samples raw alert level into rd[0]" $ do
        let prog = [encode (GetAlert 1), encode (Halt 0)]
            rHi = drive 30 (memOf prog) (\t -> (repeat 0, 1, t == 0))
            rLo = drive 30 (memOf prog) (\t -> (repeat 0, 0, t == 0))
        readReg (regs (runState rHi)) 1 @?= 1
        readReg (regs (runState rLo)) 1 @?= 0
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

-- | Extract (trap, reason) from a HALT terminator word.
haltReason :: Ring -> (Bool, BitVector 3)
haltReason (Ring _ w) = (testBit w 9, slice d12 d10 w)

-- | Extract the status byte from a HALT terminator word.
haltStatus :: Ring -> BitVector 8
haltStatus (Ring _ w) = slice d7 d0 w

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
