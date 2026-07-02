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
import Tamal.Crc (crc8Update)
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
    , testCase "PUT_BYTE emits exactly 8 SCK rising edges"
        $ risingEdges
          (driveTrace 200 [encode CsAssert, encode (PutByteImm 0xA5), encode CsDeassert, encode (Halt 0)])
        @?= 8
    , testCase "PUT_BYTE drives IO0 MSB-first (sampled at each SCK rising edge)" $ do
        let prog = [encode CsAssert, encode (PutByteImm 0xA5), encode CsDeassert, encode (Halt 0)]
            bos = driveTrace 200 prog
            atRising =
              [ fst (lanesOut b !! 0)
              | (a, b) <- L.zip bos (L.drop 1 bos)
              , sckOut a == 0
              , sckOut b == 1
              ]
        atRising @?= toList (unpack 0xA5 :: Vec 8 Bit)
    , testCase "GET_BYTE: rd=byte, rxCrc updated, CAPTURE emitted" $ do
        let prog = [encode CsAssert, encode (GetByte 1), encode CsDeassert, encode (Halt 0)]
            Run s ring _ = runGet 0xC3 prog
        readReg (regs s) 1 @?= 0xC3
        rxCrc s @?= crc8Update 0 0xC3
        assertBool
          "CAPTURE present (tag 00, byte 0xC3)"
          (L.any (\(Ring _ w) -> slice d31 d30 w == 0 && slice d7 d0 w == 0xC3) ring)
    , testCase "GET_BITS is CRC-neutral" $ do
        let prog = [encode CsAssert, encode (GetBits 1 7), encode CsDeassert, encode (Halt 0)]
            Run s _ _ = runGet 0xFF prog
        rxCrc s @?= 0
    , testCase "TAR n=2 clocks SCK twice"
        $ risingEdges
          (driveTrace 200 [encode CsAssert, encode (TarImm 2), encode CsDeassert, encode (Halt 0)])
        @?= 2
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

-- | Record the per-cycle 'BusOut' stream (for bus-timing assertions).
driveTrace :: Int -> [BitVector 32] -> [BusOut]
driveTrace budget prog = go 0 initState 0 []
 where
  mem = memOf prog
  go !t !s !prevPc !acc
    | phase s == Halted && t > 0 = L.reverse acc
    | t >= budget = L.reverse acc
    | otherwise =
        let inp = BusIn (mem prevPc) (repeat 0) 0 (t == 0)
            (s', bo, _) = step s inp
         in go (t + 1) s' (pcOut bo) (bo : acc)

-- | Count low→high transitions of 'sckOut' across a 'BusOut' stream.
risingEdges :: [BusOut] -> Int
risingEdges bos = L.length (L.filter id (L.zipWith rise scks (L.drop 1 scks)))
 where
  scks = L.map sckOut bos
  rise a b = a == 0 && b == 1

{- | Drive a GET: a slave presents @b@ MSB-first on IO[1], advancing one bit per
SCK rising edge (keyed off the engine's own 'sckOut'). The engine samples at the
rising edge, before the slave advances, so the pre-increment bit is the one read.
-}
runGet :: BitVector 8 -> [BitVector 32] -> Run
runGet b prog = go 0 initState 0 [] (0 :: Int) (0 :: Bit)
 where
  mem = memOf prog
  bitsV = unpack b :: Vec 8 Bit
  go !t !s !prevPc !ring !bitIx !prevSck
    | phase s == Halted && t > 0 = Run s (L.reverse ring) t
    | t >= 400 = Run s (L.reverse ring) t
    | otherwise =
        let curBit = if bitIx < 8 then bitsV !! bitIx else 0
            io = replace (1 :: Index 4) curBit (repeat 0)
            inp = BusIn (mem prevPc) io 0 (t == 0)
            (s', bo, mr) = step s inp
            bitIx' = if prevSck == 0 && sckOut bo == 1 then bitIx + 1 else bitIx
         in go (t + 1) s' (pcOut bo) (maybe ring (: ring) mr) bitIx' (sckOut bo)

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
