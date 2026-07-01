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
  -- CTRL / DATA groups encode in Tasks 4–5; the reserved word keeps the ADT
  -- total until then (decode for these lands later too).
  _               -> encodeRest

encodeRest :: BitVector 32
encodeRest = joinW (0b11, 0xF, 0, 0, 0, 0)

-- Decode dispatches on the group, then the per-group decoder rebuilds the
-- instruction and checks reserved fields.
decode :: BitVector 32 -> Either DecodeError Instr
decode w =
  case grp of
    0b00 -> decodeBus sub' rd rs1 rs2 imm
    0b01 -> Left OpcodeUnimplemented   -- Task 4 (CTRL)
    0b10 -> Left OpcodeUnimplemented   -- Task 5 (DATA)
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

