-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0

module Test.Alu (tests) where

-- 'And'/'Xor' hidden: Clash.Prelude re-exports the Data.Bits newtypes of those
-- names, which would clash with the 'AluOp' constructors imported below.

import Clash.Hedgehog.Sized.BitVector (genDefinedBitVector)
import Clash.Prelude hiding (And, Xor)
import Hedgehog (Gen, forAll, property, (===))
import qualified Hedgehog.Gen as Gen
import Test.Tasty
import Test.Tasty.HUnit
import Test.Tasty.Hedgehog (testProperty)

import Tamal.Alu
import qualified Tamal.Isa as Isa
import Test.Gen (genReg, genWord)

genImm :: Gen (BitVector 11)
genImm = genDefinedBitVector

genImm21 :: Gen (BitVector 21)
genImm21 = genDefinedBitVector

genAmt :: Gen (BitVector 5)
genAmt = genDefinedBitVector

-- Valid (non-reserved) shift ops, mirroring dataResult's toAluShift.
genShOp :: Gen (BitVector 2)
genShOp = Gen.element [0b00, 0b01, 0b10]

shiftOpToAlu :: BitVector 2 -> AluOp
shiftOpToAlu 0b00 = Sll
shiftOpToAlu 0b01 = Srl
shiftOpToAlu _ = Sra

tests :: TestTree
tests =
  testGroup
    "Alu"
    [ testGroup
        "alu core"
        [ testProperty "Add" $ property $ do
            a <- forAll genWord
            b <- forAll genWord
            alu Add a b === a + b
        , testProperty "Sub" $ property $ do
            a <- forAll genWord
            b <- forAll genWord
            alu Sub a b === a - b
        , testProperty "And" $ property $ do
            a <- forAll genWord
            b <- forAll genWord
            alu And a b === a .&. b
        , testProperty "Or" $ property $ do
            a <- forAll genWord
            b <- forAll genWord
            alu Or a b === a .|. b
        , testProperty "Xor" $ property $ do
            a <- forAll genWord
            b <- forAll genWord
            alu Xor a b === a `xor` b
        , testProperty "Sub == Add of two's complement" $ property $ do
            a <- forAll genWord
            b <- forAll genWord
            alu Sub a b === alu Add a (complement b + 1)
        , testProperty "Sll masks shift amount to low 5 bits" $ property $ do
            a <- forAll genWord
            b <- forAll genWord
            alu Sll a b === alu Sll a (b .&. 0x1F)
        , testProperty "Srl masks shift amount to low 5 bits" $ property $ do
            a <- forAll genWord
            b <- forAll genWord
            alu Srl a b === alu Srl a (b .&. 0x1F)
        , testProperty "Sra masks shift amount to low 5 bits" $ property $ do
            a <- forAll genWord
            b <- forAll genWord
            alu Sra a b === alu Sra a (b .&. 0x1F)
        , testProperty "shift by 0 is identity (Sll/Srl/Sra)" $ property $ do
            a <- forAll genWord
            alu Sll a 0 === a
            alu Srl a 0 === a
            alu Sra a 0 === a
        , testProperty "Sra preserves the sign bit" $ property $ do
            a <- forAll genWord
            b <- forAll genWord
            slice d31 d31 (alu Sra a b) === slice d31 d31 a
        , testCase "Sra 0x80000000 by 1 = 0xC0000000 (sign-fill)"
            $ alu Sra 0x80000000 1
            @?= 0xC0000000
        , testCase "Srl 0x80000000 by 1 = 0x40000000 (zero-fill)"
            $ alu Srl 0x80000000 1
            @?= 0x40000000
        ]
    , testGroup
        "dataResult wrapper"
        [ testProperty "Mov returns rs1v" $ property $ do
            rd <- forAll genReg
            rs <- forAll genReg
            x <- forAll genWord
            y <- forAll genWord
            dataResult (Isa.Mov rd rs) x y === x
        , testProperty "LoadImm sign-extends imm" $ property $ do
            rd <- forAll genReg
            imm <- forAll genImm21
            x <- forAll genWord
            y <- forAll genWord
            dataResult (Isa.LoadImm rd imm) x y === signExtend imm
        , testProperty "Lui places imm21 at [31:11], low 11 zero" $ property $ do
            rd <- forAll genReg
            i21 <- forAll genImm21
            x <- forAll genWord
            y <- forAll genWord
            let r = dataResult (Isa.Lui rd i21) x y
            r === (zeroExtend i21 :: BitVector 32) `shiftL` 11
            (r .&. 0x7FF) === 0
        , testProperty "Addi = alu Add rs1v (signExtend imm)" $ property $ do
            rd <- forAll genReg
            rs <- forAll genReg
            imm <- forAll genImm
            x <- forAll genWord
            y <- forAll genWord
            dataResult (Isa.Addi rd rs imm) x y === alu Add x (signExtend imm)
        , testProperty "Andi = alu And rs1v (signExtend imm)" $ property $ do
            rd <- forAll genReg
            rs <- forAll genReg
            imm <- forAll genImm
            x <- forAll genWord
            y <- forAll genWord
            dataResult (Isa.Andi rd rs imm) x y === alu And x (signExtend imm)
        , testProperty "Ori = alu Or rs1v (signExtend imm)" $ property $ do
            rd <- forAll genReg
            rs <- forAll genReg
            imm <- forAll genImm
            x <- forAll genWord
            y <- forAll genWord
            dataResult (Isa.Ori rd rs imm) x y === alu Or x (signExtend imm)
        , testProperty "Xori = alu Xor rs1v (signExtend imm)" $ property $ do
            rd <- forAll genReg
            rs <- forAll genReg
            imm <- forAll genImm
            x <- forAll genWord
            y <- forAll genWord
            dataResult (Isa.Xori rd rs imm) x y === alu Xor x (signExtend imm)
        , testProperty "Add = alu Add rs1v rs2v" $ property $ do
            rd <- forAll genReg
            a <- forAll genReg
            b <- forAll genReg
            x <- forAll genWord
            y <- forAll genWord
            dataResult (Isa.Add rd a b) x y === alu Add x y
        , testProperty "Sub = alu Sub rs1v rs2v" $ property $ do
            rd <- forAll genReg
            a <- forAll genReg
            b <- forAll genReg
            x <- forAll genWord
            y <- forAll genWord
            dataResult (Isa.Sub rd a b) x y === alu Sub x y
        , testProperty "And_ = alu And rs1v rs2v" $ property $ do
            rd <- forAll genReg
            a <- forAll genReg
            b <- forAll genReg
            x <- forAll genWord
            y <- forAll genWord
            dataResult (Isa.And_ rd a b) x y === alu And x y
        , testProperty "Or_ = alu Or rs1v rs2v" $ property $ do
            rd <- forAll genReg
            a <- forAll genReg
            b <- forAll genReg
            x <- forAll genWord
            y <- forAll genWord
            dataResult (Isa.Or_ rd a b) x y === alu Or x y
        , testProperty "Xor_ = alu Xor rs1v rs2v" $ property $ do
            rd <- forAll genReg
            a <- forAll genReg
            b <- forAll genReg
            x <- forAll genWord
            y <- forAll genWord
            dataResult (Isa.Xor_ rd a b) x y === alu Xor x y
        , testProperty "Shift = alu (toAluShift shOp) rs1v (zeroExtend amt)" $ property $ do
            rd <- forAll genReg
            rs <- forAll genReg
            shOp <- forAll genShOp
            amt <- forAll genAmt
            x <- forAll genWord
            y <- forAll genWord
            dataResult (Isa.Shift rd rs shOp amt) x y
              === alu (shiftOpToAlu shOp) x (zeroExtend amt)
        ]
    ]
