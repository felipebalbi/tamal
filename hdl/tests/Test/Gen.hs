module Test.Gen
  ( genBit
  , genByte
  , genReg
  , genIndex8
  , genBusInstr
  , genCtrlInstr
  , genDataInstr
  , genInstr
  ) where

import Clash.Prelude
import Clash.Hedgehog.Sized.BitVector (genDefinedBitVector)
import Hedgehog (Gen)
import qualified Hedgehog.Gen as Gen

import Tamal.Isa

genBit :: Gen Bit
genBit = Gen.element [0, 1]

genByte :: Gen (BitVector 8)
genByte = genDefinedBitVector

genReg :: Gen (BitVector 5)
genReg = genDefinedBitVector

-- | Count minus one (0..7 -> 1..8 bits) for PUT_BITS/GET_BITS.
genIndex8 :: Gen (Index 8)
genIndex8 = unpack <$> (genDefinedBitVector :: Gen (BitVector 3))

genBusInstr :: Gen Instr
genBusInstr = Gen.choice
  [ pure CsAssert
  , pure CsDeassert
  , PutByteImm <$> genByte
  , PutByteReg <$> genReg
  , GetByte    <$> genReg
  , PutBitsImm <$> genIndex8 <*> genByte
  , PutBitsReg <$> genReg <*> genIndex8
  , GetBits    <$> genReg <*> genIndex8
  , TarImm     <$> (genDefinedBitVector :: Gen (BitVector 4))
  , TarReg     <$> genReg
  , pure RstAssert
  , pure RstDeassert
  , GetAlert   <$> genReg
  ]

genCtrlInstr :: Gen Instr
genCtrlInstr = Gen.choice
  [ Halt      <$> genByte
  , Beq       <$> genReg <*> genReg <*> genOff
  , Bne       <$> genReg <*> genReg <*> genOff
  , Bltu      <$> genReg <*> genReg <*> genOff
  , Bgeu      <$> genReg <*> genReg <*> genOff
  , WaitOn    <$> genReg
              <*> (genDefinedBitVector :: Gen (BitVector 2))
              <*> (genDefinedBitVector :: Gen (BitVector 9))
  , SetConfig <$> (genDefinedBitVector :: Gen (BitVector 6))
  , Mark      <$> genOff <*> genReg
  , pure CrcReset
  ]
  where genOff = genDefinedBitVector :: Gen (BitVector 11)

genDataInstr :: Gen Instr
genDataInstr = Gen.choice
  [ LoadImm <$> genReg <*> genI
  , Lui     <$> genReg <*> (genDefinedBitVector :: Gen (BitVector 20))
  , Mov     <$> genReg <*> genReg
  , Add     <$> genReg <*> genReg <*> genReg
  , Addi    <$> genReg <*> genReg <*> genI
  , Sub     <$> genReg <*> genReg <*> genReg
  , And_    <$> genReg <*> genReg <*> genReg
  , Andi    <$> genReg <*> genReg <*> genI
  , Or_     <$> genReg <*> genReg <*> genReg
  , Ori     <$> genReg <*> genReg <*> genI
  , Xor_    <$> genReg <*> genReg <*> genReg
  , Xori    <$> genReg <*> genReg <*> genI
  , Shift   <$> genReg <*> genReg <*> genShOp <*> (genDefinedBitVector :: Gen (BitVector 5))
  , Rdsr    <$> genReg <*> (genDefinedBitVector :: Gen (BitVector 5))
  ]
  where
    genI    = genDefinedBitVector :: Gen (BitVector 11)
    genShOp = Gen.element [0b00, 0b01, 0b10] :: Gen (BitVector 2)

genInstr :: Gen Instr
genInstr = Gen.choice [genBusInstr, genCtrlInstr, genDataInstr]
