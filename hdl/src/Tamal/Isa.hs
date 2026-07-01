-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0

{- |
Tamal instruction encoding (spec §4–6). 32-bit fixed-width words:

@
 31 30 | 29 .. 26 | 25 .. 21 | 20 .. 16 | 15 .. 11 | 10 .. 0
 group |   sub    |    rd    |   rs1    |   rs2    |  imm
@

This module owns the @Instr@ ADT and total @encode@ / @decode@. Fields
unused by an opcode are reserved-must-be-zero; a non-zero reserved field
decodes to @Left ReservedFieldNonZero@.
-}
module Tamal.Isa
  ( Instr (..)
  , DecodeError (..)
  , Reg
  , encode
  , decode
  ) where

import Clash.Prelude

type Reg = BitVector 5

data Instr
  -- BUS group (group 00)
  = CsAssert
  | CsDeassert
  | PutByteImm (BitVector 8)
  | PutByteReg Reg
  | GetByte    Reg
  | PutBitsImm (Index 8) (BitVector 8)   -- n-1, bits (n = count in 1..8)
  | PutBitsReg Reg (Index 8)
  | GetBits    Reg (Index 8)
  | TarImm     (BitVector 4)
  | TarReg     Reg
  | RstAssert
  | RstDeassert
  | GetAlert   Reg
  -- CTRL group (group 01) — decoding lands in Task 4
  | Halt    (BitVector 8)
  | Beq  Reg Reg (BitVector 11)
  | Bne  Reg Reg (BitVector 11)
  | Bltu Reg Reg (BitVector 11)
  | Bgeu Reg Reg (BitVector 11)
  | WaitOn Reg (BitVector 2) (BitVector 9)   -- rd, cond, timeout
  | SetConfig (BitVector 6)
  | Mark (BitVector 11) Reg                  -- label, payload reg
  | CrcReset
  -- DATA group (group 10) — decoding lands in Task 5
  | LoadImm Reg (BitVector 11)
  | Lui     Reg (BitVector 20)
  | Mov     Reg Reg
  | Add  Reg Reg Reg
  | Addi Reg Reg (BitVector 11)
  | Sub  Reg Reg Reg
  | And_ Reg Reg Reg
  | Andi Reg Reg (BitVector 11)
  | Or_  Reg Reg Reg
  | Ori  Reg Reg (BitVector 11)
  | Xor_ Reg Reg Reg
  | Xori Reg Reg (BitVector 11)
  | Shift Reg Reg (BitVector 2) (BitVector 5)  -- rd, rs1, op(SLL/SRL/SRA/rsv), amt
  | Rdsr  Reg (BitVector 5)                    -- rd, sr#
  deriving stock (Generic, Show, Eq)
  deriving anyclass NFDataX

data DecodeError
  = ReservedFieldNonZero
  | OpcodeUnimplemented
  | IllegalOpcode
  deriving stock (Generic, Show, Eq)
  deriving anyclass NFDataX

-- Field split/join. group=2, sub=4, rd=5, rs1=5, rs2=5, imm=11 (sum 32).
type Fields = (BitVector 2, BitVector 4, BitVector 5, BitVector 5, BitVector 5, BitVector 11)

splitWord :: BitVector 32 -> Fields
splitWord = bitCoerce

joinW :: Fields -> BitVector 32
joinW = bitCoerce

-- Sub-field helpers for the 11-bit imm.
-- PUT_BITS/GET_BITS: imm[10:8] = n-1, imm[7:0] = bits.
bitsField :: BitVector 11 -> (BitVector 3, BitVector 8)
bitsField = bitCoerce

mkBitsImm :: Index 8 -> BitVector 8 -> BitVector 11
mkBitsImm n b = bitCoerce (pack n, b)

-- LUI imm20 occupies rs1 ++ rs2 ++ imm low 20 bits; bit 20 reserved 0.
splitImm20 :: BitVector 20 -> (BitVector 5, BitVector 5, BitVector 11)
splitImm20 i20 = bitCoerce ((0 :: BitVector 1) ++# i20)

joinImm20 :: BitVector 5 -> BitVector 5 -> BitVector 11 -> (BitVector 1, BitVector 20)
joinImm20 rs1 rs2 imm = bitCoerce (rs1 ++# rs2 ++# imm)

encode :: Instr -> BitVector 32
encode = \case
  -- BUS group (00)
  CsAssert        -> joinW (0b00, 0x0, 0, 0, 0, 0)
  CsDeassert      -> joinW (0b00, 0x1, 0, 0, 0, 0)
  PutByteImm b    -> joinW (0b00, 0x2, 0, 0, 0, zeroExtend b)
  PutByteReg rs   -> joinW (0b00, 0x3, 0, rs, 0, 0)
  GetByte rd      -> joinW (0b00, 0x4, rd, 0, 0, 0)
  PutBitsImm n b  -> joinW (0b00, 0x5, 0, 0, 0, mkBitsImm n b)
  PutBitsReg rs n -> joinW (0b00, 0x6, 0, rs, 0, mkBitsImm n 0)
  GetBits rd n    -> joinW (0b00, 0x7, rd, 0, 0, mkBitsImm n 0)
  TarImm n        -> joinW (0b00, 0x8, 0, 0, 0, zeroExtend n)
  TarReg rs       -> joinW (0b00, 0x9, 0, rs, 0, 0)
  RstAssert       -> joinW (0b00, 0xA, 0, 0, 0, 0)
  RstDeassert     -> joinW (0b00, 0xB, 0, 0, 0, 0)
  GetAlert rd     -> joinW (0b00, 0xC, rd, 0, 0, 0)
  -- CTRL group (01)
  Halt s          -> joinW (0b01, 0x0, 0, 0, 0, zeroExtend s)
  Beq  a b off    -> joinW (0b01, 0x1, 0, a, b, off)
  Bne  a b off    -> joinW (0b01, 0x2, 0, a, b, off)
  Bltu a b off    -> joinW (0b01, 0x3, 0, a, b, off)
  Bgeu a b off    -> joinW (0b01, 0x4, 0, a, b, off)
  WaitOn rd c t   -> joinW (0b01, 0x5, rd, 0, 0, bitCoerce (c, t))
  SetConfig p     -> joinW (0b01, 0x6, 0, 0, 0, zeroExtend p)
  Mark lbl rs     -> joinW (0b01, 0x7, 0, rs, 0, lbl)
  CrcReset        -> joinW (0b01, 0x8, 0, 0, 0, 0)
  -- DATA group (10)
  LoadImm rd i    -> joinW (0b10, 0x0, rd, 0, 0, i)
  Lui rd i20      -> let (rs1', rs2', imm') = splitImm20 i20
                     in joinW (0b10, 0x1, rd, rs1', rs2', imm')
  Mov rd rs       -> joinW (0b10, 0x2, rd, rs, 0, 0)
  Add rd a b      -> joinW (0b10, 0x3, rd, a, b, 0)
  Addi rd a i     -> joinW (0b10, 0x4, rd, a, 0, i)
  Sub rd a b      -> joinW (0b10, 0x5, rd, a, b, 0)
  And_ rd a b     -> joinW (0b10, 0x6, rd, a, b, 0)
  Andi rd a i     -> joinW (0b10, 0x7, rd, a, 0, i)
  Or_ rd a b      -> joinW (0b10, 0x8, rd, a, b, 0)
  Ori rd a i      -> joinW (0b10, 0x9, rd, a, 0, i)
  Xor_ rd a b     -> joinW (0b10, 0xA, rd, a, b, 0)
  Xori rd a i     -> joinW (0b10, 0xB, rd, a, 0, i)
  Shift rd a op a5 -> joinW (0b10, 0xC, rd, a, 0, bitCoerce (op, 0 :: BitVector 4, a5))
  Rdsr rd sr      -> joinW (0b10, 0xD, rd, 0, 0, zeroExtend sr)

-- Decode dispatches on the group, then the per-group decoder rebuilds the
-- instruction and checks reserved fields.
decode :: BitVector 32 -> Either DecodeError Instr
decode w =
  case grp of
    0b00 -> decodeBus sub' rd rs1 rs2 imm
    0b01 -> decodeCtrl sub' rd rs1 rs2 imm
    0b10 -> decodeData sub' rd rs1 rs2 imm
    _    -> Left IllegalOpcode         -- group 11 reserved
  where
    (grp, sub', rd, rs1, rs2, imm) = splitWord w

-- Accept an instruction only when its reserved fields are all zero.
only :: Bool -> Instr -> Either DecodeError Instr
only ok r = if ok then Right r else Left ReservedFieldNonZero

decodeBus
  :: BitVector 4 -> BitVector 5 -> BitVector 5 -> BitVector 5 -> BitVector 11
  -> Either DecodeError Instr
decodeBus sub' rd rs1 rs2 imm =
  case sub' of
    0x0 -> only (z rd && z rs1 && z rs2 && z imm)        CsAssert            -- no operands: everything reserved
    0x1 -> only (z rd && z rs1 && z rs2 && z imm)        CsDeassert          -- no operands: everything reserved
    0x2 -> only (z rd && z rs1 && z rs2 && immHi8 == 0)  (PutByteImm (truncateB imm))  -- imm[7:0]=byte, imm[10:8] reserved
    0x3 -> only (z rd && z rs2 && z imm)                 (PutByteReg rs1)    -- rs1 used, reset reserved
    0x4 -> only (z rs1 && z rs2 && z imm)                (GetByte rd)        -- rd used, rest reserved
    0x5 -> only (z rd && z rs1 && z rs2)                 (PutBitsImm nBits bBits)  -- imm carries n+bits
    0x6 -> only (z rd && z rs2 && bBits == 0)            (PutBitsReg rs1 nBits)  -- imm carries n+bits
    0x7 -> only (z rs1 && z rs2 && bBits == 0)           (GetBits rd nBits)
    0x8 -> only (z rd && z rs1 && z rs2 && immHi4 == 0)  (TarImm (truncateB imm))
    0x9 -> only (z rd && z rs2 && z imm)                 (TarReg rs1)
    0xa -> only (z rd && z rs1 && z rs2 && z imm)        RstAssert
    0xb -> only (z rd && z rs1 && z rs2 && z imm)        RstDeassert
    0xc -> only (z rs1 && z rs2 && z imm)                (GetAlert rd)
    _   -> Left IllegalOpcode
  where
    z :: KnownNat n => BitVector n -> Bool
    z = (== 0)
    (n3, bBits) = bitsField imm
    nBits  = unpack n3 :: Index 8
    immHi8 = slice d10 d8 imm    -- imm[10:8]  (BitVector 3) reserved for PUT_BYTE
    immHi4 = slice d10 d4 imm    -- imm[10:4]  (BitVector 7) reserved for TAR

decodeCtrl
  :: BitVector 4 -> BitVector 5 -> BitVector 5 -> BitVector 5 -> BitVector 11
  -> Either DecodeError Instr
decodeCtrl sub' rd rs1 rs2 imm =
  case sub' of
    0x0 -> only (z rd && z rs1 && z rs2 && immHi8 == 0)  (Halt (truncateB imm))  -- imm[7:0]=status, imm[10:8] reserved
    0x1 -> only (z rd)                                   (Beq  rs1 rs2 imm)  -- rs1,rs2,off=imm; rd reserved
    0x2 -> only (z rd)                                   (Bne  rs1 rs2 imm)
    0x3 -> only (z rd)                                   (Bltu rs1 rs2 imm)
    0x4 -> only (z rd)                                   (Bgeu rs1 rs2 imm)
    0x5 -> only (z rs1 && z rs2)                         (WaitOn rd cond timeout)  -- rd used, imm=cond++timeout
    0x6 -> only (z rd && z rs1 && z rs2 && immHi6 == 0)  (SetConfig (truncateB imm))  -- imm[5:0]=payload, imm[10:6] reserved
    0x7 -> only (z rd && z rs2)                          (Mark imm rs1)  -- label=imm, rs1=payload
    0x8 -> only (z rd && z rs1 && z rs2 && z imm)        CrcReset
    _   -> Left IllegalOpcode
  where
    z :: KnownNat n => BitVector n -> Bool
    z = (== 0)
    (cond, timeout) = bitCoerce imm :: (BitVector 2, BitVector 9)
    immHi8 = slice d10 d8 imm    -- HALT: imm[10:8]  (BitVector 3) reserved
    immHi6 = slice d10 d6 imm    -- SET_CONFIG: imm[10:6] (BitVector 5) reserved

decodeData
  :: BitVector 4 -> BitVector 5 -> BitVector 5 -> BitVector 5 -> BitVector 11
  -> Either DecodeError Instr
decodeData sub' rd rs1 rs2 imm =
  case sub' of
    0x0 -> only (z rs1 && z rs2)                  (LoadImm rd imm)     -- imm=i11; rs1,rs2 reserved
    0x1 -> only (hi == 0)                         (Lui rd i20)         -- i20 packed in rs1++rs2++imm; bit20 reserved
    0x2 -> only (z rs2 && z imm)                  (Mov rd rs1)         -- rd,rs1 used; rs2,imm reserved
    0x3 -> only (z imm)                           (Add rd rs1 rs2)     -- reg-reg: imm reserved
    0x4 -> only (z rs2)                           (Addi rd rs1 imm)    -- imm op: rs2 reserved
    0x5 -> only (z imm)                           (Sub rd rs1 rs2)
    0x6 -> only (z imm)                           (And_ rd rs1 rs2)
    0x7 -> only (z rs2)                           (Andi rd rs1 imm)
    0x8 -> only (z imm)                           (Or_ rd rs1 rs2)
    0x9 -> only (z rs2)                           (Ori rd rs1 imm)
    0xa -> only (z imm)                           (Xor_ rd rs1 rs2)
    0xb -> only (z rs2)                           (Xori rd rs1 imm)
    0xc -> only (z rs2 && shMid == 0 && shOp /= 0b11) (Shift rd rs1 shOp shAmt)  -- imm=op[10:9]++rsv[8:5]++amt[4:0]; op 0b11 reserved
    0xd -> only (z rs1 && z rs2 && immHi5 == 0)   (Rdsr rd (truncateB imm))  -- imm[4:0]=sr#, imm[10:5] reserved
    _   -> Left IllegalOpcode
  where
    z :: KnownNat n => BitVector n -> Bool
    z = (== 0)
    (hi, i20)            = joinImm20 rs1 rs2 imm
    (shOp, shMid, shAmt) = bitCoerce imm :: (BitVector 2, BitVector 4, BitVector 5)
    immHi5 = slice d10 d5 imm    -- RDSR imm[10:5] (BitVector 6) reserved

