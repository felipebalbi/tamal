-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-W-2.0

{- |
DATA-group compute layer (ALU/branch design §5–7). Two layers:

  * 'alu' — a thin, total, @Op@-dispatched arithmetic/logic/shift core over
    register *values*. Shift amount is the low 5 bits of operand B.
  * 'dataResult' — the wrapper that resolves operand B (register value or
    sign-extended immediate), places 'Tamal.Isa.Lui'/'Tamal.Isa.Mov'/
    'Tamal.Isa.LoadImm' constants, and dispatches to 'alu'. It takes register
    VALUES, so it needs no register file; x0-hardwiring and writeback masking
    are the Engine's job.

The ISA is imported qualified as @Isa@ because 'AluOp'\'s @Add@/@Sub@ would
otherwise clash with 'Tamal.Isa.Instr'\'s @Add@/@Sub@ constructors.
-}
module Tamal.Alu
  ( AluOp (..)
  , alu
  , dataResult
  ) where

-- 'And'/'Xor' are hidden because Clash.Prelude re-exports the Data.Bits newtype
-- wrappers of those names, which would clash with 'AluOp'\'s constructors. We
-- use the '.&.'/'xor' operators, not those wrappers. ('Or' is not re-exported.)
import Clash.Prelude hiding (And, Xor)
import Tamal.Isa (Instr)
import qualified Tamal.Isa as Isa

{- | The arithmetic/logic/shift operations of the DATA group. Register/immediate
opcode pairs (e.g. @ADD@/@ADDI@) collapse to one 'AluOp'; 'dataResult' resolves
the immediate before calling 'alu'. There is no reserved case, so 'alu' is
total (the reserved shift op is trapped at decode).
-}
data AluOp = Add | Sub | And | Or | Xor | Sll | Srl | Sra
  deriving stock (Generic, Show, Eq, Enum, Bounded)
  deriving anyclass (NFDataX)

{- | The thin, total, op-dispatched core: @op@ applied to two 32-bit register
values. The shift amount is the low 5 bits of operand B (RISC-V-style masking,
so a shift by ≥ 32 is well-defined). @Srl@ is logical (zero-fill); @Sra@ is
arithmetic, reached by reinterpreting the operand as 'Signed' before shifting.
-}
alu :: AluOp -> BitVector 32 -> BitVector 32 -> BitVector 32
alu op r1 r2 = case op of
  Add -> r1 + r2
  Sub -> r1 - r2
  And -> r1 .&. r2
  Or -> r1 .|. r2
  Xor -> r1 `xor` r2
  Sll -> r1 `shiftL` sh
  Srl -> r1 `shiftR` sh
  Sra -> pack (shiftR (unpack r1 :: Signed 32) sh)
 where
  sh :: Int
  sh = fromIntegral (unpack (truncateB r2) :: Unsigned 5)

{- | Complete DATA-group value semantics over register /values/ @rs1v@/@rs2v@.
Resolves operand B (a register value or a sign-extended immediate), places
'Isa.Lui'/'Isa.Mov'/'Isa.LoadImm' constants directly, and dispatches everything
else to 'alu'. Total over 'Instr'; non-DATA-compute constructors (BUS, CTRL,
@RDSR@) hit the @0@ default, which the Engine never routes here.
-}
dataResult :: Instr -> BitVector 32 -> BitVector 32 -> BitVector 32
dataResult instr rs1v rs2v = case instr of
  Isa.LoadImm _ imm -> signExtend imm
  Isa.Lui _ imm21 -> (zeroExtend imm21 :: BitVector 32) `shiftL` 11
  Isa.Mov _ _ -> rs1v
  Isa.Add _ _ _ -> alu Add rs1v rs2v
  Isa.Addi _ _ imm -> alu Add rs1v (signExtend imm)
  Isa.Sub _ _ _ -> alu Sub rs1v rs2v
  Isa.And_ _ _ _ -> alu And rs1v rs2v
  Isa.Andi _ _ imm -> alu And rs1v (signExtend imm)
  Isa.Or_ _ _ _ -> alu Or rs1v rs2v
  Isa.Ori _ _ imm -> alu Or rs1v (signExtend imm)
  Isa.Xor_ _ _ _ -> alu Xor rs1v rs2v
  Isa.Xori _ _ imm -> alu Xor rs1v (signExtend imm)
  Isa.Shift _ _ shOp amt -> alu (toAluShift shOp) rs1v (zeroExtend amt)
  _ -> 0 -- BUS / CTRL / RDSR: never routed here by the Engine
 where
  toAluShift :: BitVector 2 -> AluOp
  toAluShift = \case
    0b00 -> Sll
    0b01 -> Srl
    _ -> Sra -- 0b10; 0b11 is unreachable (decode traps it, Task 1)
