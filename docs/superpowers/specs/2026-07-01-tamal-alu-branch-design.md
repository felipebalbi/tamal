# Tamal — ALU & Branch Comparator (DATA/CTRL compute layer) Design

Date: 2026-07-01
Status: Approved (design); implementation not started
Scope: The pure, combinational **compute layer** of the tamal engine — the
DATA-group ALU and the CTRL-group branch comparator — as two independent,
hedgehog-tested Clash modules (`Tamal.Alu`, `Tamal.Branch`). Plus one decode
tightening in `Tamal.Isa`. The register file, `Engine.step`, and the `RDSR`
special-register read are **out of scope** (see §10, §14).

This spec is a companion to the ISA & HDL Engine design
(`docs/superpowers/specs/2026-07-01-tamal-isa-design.md`, esp. §6.1, §6.2, §10,
§11). It is written to be implementable from a fresh session with no other
context.

---

## 1. Purpose & context

Every **pure leaf core** from the §10 module decomposition of the ISA design is
built and property-tested — `Tamal.Isa` (encode/decode), `Tamal.Crc`,
`Tamal.Bus.Serdes`, `Tamal.Config`, `Tamal.Trace` — **except the compute
layer**. The keystone `Engine.step :: State -> BusIn -> (State, BusOut, Maybe
Ring)` cannot execute the DATA group without an **ALU**, nor resolve CTRL
branches without a **branch comparator**.

These two are the only remaining pure, combinational, single-cycle, independently
testable leaf cores, and both are prerequisites for the Engine. Building them now
turns `Engine.step` from a design problem into an assembly job (it will *call*
these in a single cycle while it does the hard, stateful work of sequencing the
BUS ops).

**Why these are leaves and the BUS ops are not.** `alu`/`branchTaken` are
single-cycle functions of operand *values* (`result = f(op, a, b)`). A BUS op
(`PUT_BYTE`, `GET_BYTE`, `TAR`, …) is multi-cycle and externally timed (SCK
phases, tri-state, turnaround, sampling against clock edges); its pure kernel
already lives in `Tamal.Bus.Serdes`, and its *sequencing* belongs to
`Engine.step` (bus FSM) plus the impure `topEntity` shell (SCK gen, `IOBUF`).
BUS-op execution is therefore explicitly **not** part of this spec.

---

## 2. Scope & non-goals

**In scope**

- `Tamal.Alu`: the `AluOp` enum, the thin `alu` core, and the pure `dataResult`
  wrapper that gives complete DATA-group value semantics over register *values*.
- `Tamal.Branch`: the `BranchOp` enum and the `branchTaken` comparator.
- One guard tightening in `Tamal.Isa.decodeData` so the reserved `SHIFT` op
  (`0b11`) traps at decode (keeping the ALU total).
- Hedgehog property tests for both modules, plus one decode-trap assertion.
- Cabal / test-runner wiring for the new modules.
- A consequential documentation tweak in the ISA design doc: `li = LUI + ADDI`
  (was `LUI + ORI`; see §7).

**Out of scope (deferred)**

- **Register file** (16×32, `x0` = 0). `alu`/`dataResult`/`branchTaken` take
  register *values*, so no register file is needed to build or test them. It
  lands with the Engine.
- **`Engine.step`** (the Mealy transition, bus FSM, PC/CRC/trace).
- **`RDSR`** (DATA sub `0xD`). It reads engine state (the RX CRC-8 accumulator
  for `sr=0`; other `sr#` reserved → TRAP), not a function of `rs1`/`rs2`. It
  becomes an Engine-level special-register mux, specced with the Engine.
- The impure `topEntity` shell, SCK/edge timing, `IOBUF` tri-states, BRAM, UART.

---

## 3. Design decisions

| # | Decision | Rationale |
|---|----------|-----------|
| 1 | **Two modules: `Tamal.Alu` + `Tamal.Branch`.** | One concern per module, matching the existing `Isa`/`Crc`/`Config`/`Serdes`/`Trace` layout. DATA compute and CTRL comparison are distinct concerns; branch being small doesn't justify folding it into "ALU". |
| 2 | **Layered ALU: thin `alu` core + pure `dataResult` wrapper.** | `alu` is the Lion-style `Op`-dispatched core (§10). `dataResult` owns immediate/mux/`LUI`/`MOV` glue and delivers *complete, testable* DATA-group semantics now, without waiting for the Engine. Wrapper takes register *values*, so no register file dependency. |
| 3 | **`RDSR` deferred to the Engine.** | It reads engine state, not operands; it doesn't fit a pure `op a b` core. Keeps `alu`/`dataResult` clean and total. |
| 4 | **Reserved `SHIFT` op (`0b11`) trapped at decode; ALU stays total.** | Reserved encodings are a decode-layer concern (matching every other reserved-field trap). The core `AluOp` then has no reserved case, and `alu` is total — the Lion "`decode → Either Exception`, `alu` total" idiom. |
| 5 | **Uniform sign-extension for all I-form immediates; `li = LUI + ADDI`.** | Simple, RISC-V-consistent rule. Switching `li` to `LUI + ADDI` removes the only reason to zero-extend `ORI` (composition), so all immediates can sign-extend uniformly. Byte masks are unaffected (bit 10 clear). See §7. |
| 6 | **`branchTaken` returns only "taken?"; unsigned compares via `BitVector` `Ord`.** | PC math and offset handling belong to the Engine. `BitVector`'s `Ord` is unsigned = exactly `BLTU`/`BGEU`. No signed branches in v1. |

---

## 4. Module layout & public interfaces

Two new modules under `hdl/src/Tamal/`, both `[pure]` (no clock, no state, fully
combinational), both carrying the REUSE/SPDX header required of every
`hdl/**/*.hs` file:

```haskell
-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0
```

### `Tamal.Alu` (DATA group)

```haskell
module Tamal.Alu
  ( AluOp (..)
  , alu
  , dataResult
  ) where

data AluOp = Add | Sub | And | Or | Xor | Sll | Srl | Sra
  deriving stock (Generic, Show, Eq, Enum, Bounded)
  deriving anyclass NFDataX

-- Thin, Op-dispatched core.
alu :: AluOp -> BitVector 32 -> BitVector 32 -> BitVector 32

-- Complete DATA-group value semantics over register VALUES (rs1val, rs2val).
-- Total over Instr; non-DATA-compute constructors hit a documented default.
dataResult :: Instr -> BitVector 32 -> BitVector 32 -> BitVector 32
```

### `Tamal.Branch` (CTRL group)

```haskell
module Tamal.Branch
  ( BranchOp (..)
  , branchTaken
  ) where

data BranchOp = Beq | Bne | Bltu | Bgeu
  deriving stock (Generic, Show, Eq, Enum, Bounded)
  deriving anyclass NFDataX

branchTaken :: BranchOp -> BitVector 32 -> BitVector 32 -> Bool
```

**Interface rationale**

- `alu` collapses the reg/imm opcode pairs (`ADD`≡`ADDI`, `AND`≡`ANDI`, …):
  `dataResult` resolves operand B (register value or sign-extended immediate)
  *before* calling `alu`, so the core has 8 ops, not 14.
- `dataResult` is the **only** place aware of immediates and `Instr` shape.
- `Tamal.Branch` has **no** `Instr` wrapper: branches need no immediate/mux
  resolution (both operands are registers; the offset is the Engine's PC math).
  Mapping the four branch `Instr` constructors to `BranchOp` is a trivial remap
  the Engine does inline (YAGNI — no `branchResult`-on-`Instr`).
- `Enum`/`Bounded` are derived so tests can enumerate every op via
  `[minBound .. maxBound]`.

---

## 5. `Tamal.Alu` — the `alu` core

`alu` is a single total `case`. Operands are 32-bit register values; the shift
amount is the **low 5 bits of operand B** (RISC-V-style masking, so a shift by
≥ 32 is well-defined, never undefined).

```haskell
alu :: AluOp -> BitVector 32 -> BitVector 32 -> BitVector 32
alu op a b = case op of
  Add -> a + b
  Sub -> a - b
  And -> a .&. b
  Or  -> a .|. b
  Xor -> a `xor` b
  Sll -> a `shiftL` sh
  Srl -> a `shiftR` sh                              -- logical: zero fill
  Sra -> pack (shiftR (unpack a :: Signed 32) sh)   -- arithmetic: sign fill
  where
    -- shift amount = low 5 bits of b, as a 0..31 Int
    sh :: Int
    sh = fromIntegral (unpack (truncateB b) :: Unsigned 5)
```

| `AluOp` | Result | Notes |
|---|---|---|
| `Add` | `a + b` | `BitVector` `Num`, wraps mod 2³² |
| `Sub` | `a - b` | two's-complement wrap |
| `And` | `a .&. b` | |
| `Or`  | `a .|. b` | |
| `Xor` | `a xor b` | |
| `Sll` | `a << sh` | `sh = b[4:0]` |
| `Srl` | `a >> sh` (logical) | `BitVector` `shiftR` is zero-fill |
| `Sra` | `a >> sh` (arithmetic) | via `unpack a :: Signed 32`, then `pack` back |

Clash notes:
- `BitVector n` has a `Num` instance (`+`, `-` wrap mod 2ⁿ) and `Bits`
  (`.&.`, `.|.`, `xor`, `shiftL`, `shiftR`, `complement`).
- `BitVector`'s `shiftR` is **logical** (zero-fill); arithmetic shift must go
  through `Signed 32`.
- `truncateB b :: BitVector 5` keeps the **least-significant** 5 bits; `unpack`
  to `Unsigned 5` then `fromIntegral` yields the `Int` shift count. (`Unsigned`
  has `Integral`; `BitVector` does not — do not `fromIntegral` a `BitVector`.)

---

## 6. `Tamal.Alu` — the `dataResult` wrapper

`dataResult` resolves operand B, applies the extension rules (§7), and dispatches
to `alu`. `LUI`/`MOV`/`LOAD_IMM` bypass `alu` (constant / pass-through), keeping
the core arithmetic-only. Constructor field types come from `Tamal.Isa`:
`imm :: BitVector 11`, `imm21 :: BitVector 21`, `shOp :: BitVector 2`,
`amt :: BitVector 5`.

```haskell
dataResult :: Instr -> BitVector 32 -> BitVector 32 -> BitVector 32
dataResult instr rs1v rs2v = case instr of
  LoadImm _ imm      -> signExtend imm
  Lui     _ imm21    -> (zeroExtend imm21 :: BitVector 32) `shiftL` 11
  Mov     _ _        -> rs1v
  Add     _ _ _      -> alu Add rs1v rs2v
  Addi    _ _ imm    -> alu Add rs1v (signExtend imm)
  Sub     _ _ _      -> alu Sub rs1v rs2v
  And_    _ _ _      -> alu And rs1v rs2v
  Andi    _ _ imm    -> alu And rs1v (signExtend imm)
  Or_     _ _ _      -> alu Or  rs1v rs2v
  Ori     _ _ imm    -> alu Or  rs1v (signExtend imm)
  Xor_    _ _ _      -> alu Xor rs1v rs2v
  Xori    _ _ imm    -> alu Xor rs1v (signExtend imm)
  Shift   _ _ shOp amt -> alu (toAluShift shOp) rs1v (zeroExtend amt)
  _                  -> 0   -- BUS / CTRL / RDSR: never reached on the DATA path
  where
    toAluShift :: BitVector 2 -> AluOp
    toAluShift = \case
      0b00 -> Sll
      0b01 -> Srl
      _    -> Sra   -- 0b10; 0b11 is unreachable (decode traps it, §9)
```

| DATA `Instr` | `dataResult` returns | Extension |
|---|---|---|
| `LoadImm _ imm` | `signExtend imm` (21-bit) | sign |
| `Lui _ imm21` | `zeroExtend imm21 << 11` → bits `[31:11]`, low 11 = 0 | — |
| `Mov _ _` | `rs1v` | — |
| `Add` / `Addi` | `alu Add rs1v rs2v` / `alu Add rs1v (signExtend imm)` | sign |
| `Sub` | `alu Sub rs1v rs2v` | — |
| `And_` / `Andi` | `alu And …` (reg / `signExtend imm`) | sign |
| `Or_` / `Ori` | `alu Or …` (reg / `signExtend imm`) | sign |
| `Xor_` / `Xori` | `alu Xor …` (reg / `signExtend imm`) | sign |
| `Shift _ _ shOp amt` | `alu (toAluShift shOp) rs1v (zeroExtend amt)` | zero (count) |
| *any other* | `0` (documented-unreachable default) | — |

Key points:

- **Totality & the defensive default.** `dataResult` matches on the whole
  `Instr` ADT, so Clash requires a total function. The `_ -> 0` arm covers BUS,
  CTRL, and `RDSR` constructors, which the Engine never routes through
  `dataResult` (it dispatches on the decoded group and handles `RDSR`
  separately). The default is *unreachable in the Engine*; `0` is a safe,
  deterministic placeholder. (A `Maybe`-returning variant was considered and
  rejected as needless — every real DATA-compute input yields a value.)
- **Uniform sign-extension** for `ADDI`/`ANDI`/`ORI`/`XORI`/`LOAD_IMM` (§7). The
  shift amount is the one exception: it is a *count*, so `zeroExtend amt`.
- **`LUI`** places `imm21` at bits `[31:11]` with the low 11 zero — a pure lane
  placement, not an arithmetic op.
- **`x0` hardwiring is NOT here.** `dataResult` takes raw register values; the
  Engine/register file is responsible for reading `x0` as 0 and for discarding
  writes to `x0`.

---

## 7. Immediate extension & `li` composition (rationale)

The extension rule and the `li` expansion are entangled; this section records the
reasoning so it isn't relitigated.

### 7.1 The rule

All I-form immediates (`ADDI`, `ANDI`, `ORI`, `XORI`, `LOAD_IMM`) **sign-extend**
from bit 10 of the 11-bit `imm` field. This makes `addi rd,rd,-1` subtract 1
(`imm = 0x7FF` → `0xFFFF_FFFF`) and lets `li rd,-1` be a single `LOAD_IMM`. The
shift *count* is zero-extended (it is not a signed value).

### 7.2 Why `li = LUI + ADDI`, not `LUI + ORI`

The *only* reason zero-extension was ever considered for `ORI` was so
`li = LUI + ORI` could compose without the low half's sign bits corrupting the
high half. Expanding `li` as `LUI + ADDI` instead — with the standard `%hi`/`%lo`
+1 carry adjustment the assembler already performs — removes that reason
entirely, so every immediate can sign-extend uniformly. `ORI` remains in the ISA
as a general logical-immediate op; it is simply no longer `li`'s low half.

Byte masks are unaffected: realistic masks (`0xFF`, `0x0F`, `0x7F`, …) have bit
10 clear, so sign- and zero-extension are identical for them.

### 7.3 No reachability gap: the 21-bit `LUI`/`LOAD_IMM` immediate

An earlier revision paired a **20-bit** `LUI` (`imm20 << 12`) with the 11-bit
`ADDI`. Because tamal's immediate field is one bit narrower than RISC-V's 12-bit
I-imm, `LUI`'s bit-12 boundary did **not** meet `ADDI`'s sign-extended reach, so
the band `K mod 4096 ∈ [1024, 3071]` needed a third instruction — worst case
`LUI + 3 ADDI` = **4 words**. RISC-V avoids this only because its 12-bit immediate
(`[-2048, 2047]`) exactly meets `LUI`'s bit-12 boundary.

The fix lives in the **encoding**, not the tiling. `LUI` and `LOAD_IMM` each carry
a **21-bit** immediate spread across the entire `rs1 ++ rs2 ++ imm` operand space
(`5 + 5 + 11 == 21` — there is no `rd`-free bit left to reserve). `LUI` now shifts
by **11**, placing its 21 bits at `[31:11]`, exactly where `ADDI`'s sign-extended
low half `[10:0]` begins. The boundaries meet, so the gap is *gone*:

- every 32-bit constant is reachable in **≤ 2** instructions, with uniform
  sign-extension preserved (`li = LUI + ADDI`, still no `ORI` needed);
- `LOAD_IMM` alone now reaches the full signed-21 range `[-2^20, 2^20-1]` in **1**
  instruction (was `[-1024, 1023]`);
- no dead `LUI rd, 0` and no dead trailing `ADDI rd, rd, 0`.

This was a **human catch during code inspection**: the old `LUI` reserved bit 20
even though the field already had room for it, and the "gap" the earlier draft
called *inherent to the 11-bit immediate* was really an artifact of pinning
`LUI`'s shift to 12 while `ADDI` covered 11. Widening to 21 and realigning the
shift both recovers the wasted bit and closes the gap. The ALU spec fixes only the
*primitive* semantics (`LUI = imm21 << 11`, `ADDI = a + sext(imm11)`); the exact
`li` tiling lives in `tamal-asm` (§5.4 of the asm design).

### 7.4 Doc consequence

The ISA design doc is updated in the same commit as this spec: §4 and §6.3 now
say `li = LUI + ADDI` (was `LUI + ORI`), and `LOAD_IMM`'s `ext(imm)` is written
`sext(imm)` for precision. See `2026-07-01-tamal-isa-design.md`.

---

## 8. `Tamal.Branch` — the comparator

```haskell
branchTaken :: BranchOp -> BitVector 32 -> BitVector 32 -> Bool
branchTaken op a b = case op of
  Beq  -> a == b
  Bne  -> a /= b
  Bltu -> a <  b   -- unsigned: BitVector Ord
  Bgeu -> a >= b   -- unsigned
```

| `BranchOp` | Result |
|---|---|
| `Beq`  | `a == b` |
| `Bne`  | `a /= b` |
| `Bltu` | `a <  b` (unsigned) |
| `Bgeu` | `a >= b` (unsigned) |

- Returns **only "taken?"**. The Engine computes `PC` (`PC += signExtend off`
  when taken; else next instruction) — see §10.
- `BitVector`'s `Ord` is **unsigned**, which is exactly `BLTU`/`BGEU`. There are
  no signed branches in v1 (`BLT`/`BGE` reserved for a later phase).
- `Beq`/`Bne` are exact complements, as are `Bltu`/`Bgeu` — free test
  invariants (§11).

---

## 9. Decode tightening (`Tamal.Isa`)

Today `decodeData` accepts a `SHIFT` whose 2-bit op field is the reserved value
`0b11` (`hdl/src/Tamal/Isa.hs:227` checks only `shMid == 0`). Per ISA design
§6.2 that encoding is reserved and must trap. Tighten the guard so the reserved
op decodes to `Left`, keeping the ALU total (§3 decision 4):

```haskell
-- hdl/src/Tamal/Isa.hs, decodeData, sub 0xc (line 227):
-- before:
--   0xc -> only (z rs2 && shMid == 0)                (Shift rd rs1 shOp shAmt)
-- after:
    0xc -> only (z rs2 && shMid == 0 && shOp /= 0b11) (Shift rd rs1 shOp shAmt)
```

`shOp` is already bound in the `where` clause (`hdl/src/Tamal/Isa.hs:234`). A
reserved shift op now yields `Left ReservedFieldNonZero`.

**Error variant:** reuse the existing `ReservedFieldNonZero` — no new
`DecodeError` constructor. It already means "this field holds a value that isn't
allowed here," and reusing it avoids churn in the `DecodeError` ADT, its `Show`,
and downstream matches. (A dedicated `ReservedShiftOp` was considered and
rejected as not worth the ripple.)

**Generator already agrees.** `Test.Gen.genDataInstr` already restricts
`genShOp = Gen.element [0b00, 0b01, 0b10]` (`hdl/tests/Test/Gen.hs:93`), so it
never emits `0b11`. The round-trip properties therefore remain green with **no
generator change**; the tightening simply aligns the decoder with an assumption
the generator already bakes in.

---

## 10. Engine integration contract (informative)

This section is *not* implemented here; it records how `Engine.step` will consume
this layer, so the interfaces above are the right shape.

- **Operand fetch (Engine/regfile).** The Engine reads `rs1v = reg[rs1]`,
  `rs2v = reg[rs2]`, where `reg[x0] = 0` (hardwired). It passes *values* to this
  layer; `dataResult`/`branchTaken` never see the register file.
- **DATA-compute writeback.** For a decoded DATA-compute `Instr`:
  `rd' = dataResult instr rs1v rs2v`; write `rd'` to `reg[rd]` unless `rd == x0`
  (writes to `x0` are discarded — Engine/regfile responsibility, not
  `dataResult`'s).
- **`RDSR` (separate path).** The Engine handles `RDSR` itself: `sr = 0` →
  read the RX CRC-8 accumulator into `rd`; other `sr#` → TRAP. It does **not**
  call `dataResult` for `RDSR` (which is why `RDSR` falls into the `_ -> 0`
  default there).
- **Branches.** The Engine maps the four branch constructors (`Beq`/`Bne`/
  `Bltu`/`Bgeu`) to `BranchOp`, then: `taken = branchTaken op rs1v rs2v`; if
  taken, `PC += signExtend off` (word-offset semantics, ISA design §6.1); else
  advance to the next instruction.

---

## 11. Testing (hedgehog baseline)

Two new test modules, mirroring the existing tasty + tasty-hedgehog + hedgehog +
`clash-prelude-hedgehog` style (`tests/Test/*.hs`), each exporting
`tests :: TestTree`. Each carries the SPDX header. Reuse `Test.Gen`
(`genReg`, `genWord`, `genDataInstr`); add a 32-bit operand generator
(`genReg32 = genDefinedBitVector :: Gen (BitVector 32)`) — put it in `Test.Gen`
or locally.

### 11.1 `Test.Alu`

**`alu` core** — cross-check every op against a Haskell reference model over
random operand pairs (`op <- forAll (Gen.element [minBound..maxBound])`):

- `Add`/`Sub`/`And`/`Or`/`Xor` match the reference.
- `alu Sub a b === alu Add a (complement b + 1)` (two's complement).
- Shift-amount masking: `alu Sll a b === alu Sll a (b .&. 0x1F)` (and same for
  `Srl`/`Sra`); shift-by-0 is identity for all three.
- `Srl` zero-fills (top bits become 0 for any `a`); `Sra` sign-fills
  (`msb` of `a` replicates). Concrete vectors: `alu Sra 0x8000_0000 1 ===
  0xC000_0000`; `alu Srl 0x8000_0000 1 === 0x4000_0000`.

**`dataResult` wrapper** — over `genDataInstr` filtered to DATA-compute
constructors, plus random `rs1v`/`rs2v`:

- `Mov` returns `rs1v`; `LoadImm imm` returns `signExtend imm` (21-bit); `Lui
  imm21` returns `zeroExtend imm21 << 11` with low 11 bits zero.
- Immediate arithmetic/logical forms use `signExtend` and agree with `alu`
  (`dataResult (Addi rd rs imm) x _ === alu Add x (signExtend imm)`, etc.).
- Reg-reg forms agree with `alu` (`dataResult (Add rd a b) x y === alu Add x y`,
  etc.).
- `Shift` uses `zeroExtend amt` and agrees with `alu (toAluShift shOp) x
  (zeroExtend amt)`.

### 11.2 `Test.Branch`

- `branchTaken` matches a reference over all four ops and random pairs.
- Complementarity: `branchTaken Beq a b == not (branchTaken Bne a b)`;
  `branchTaken Bltu a b == not (branchTaken Bgeu a b)`.
- Unsigned boundary (locks out an accidental signed compare):
  `branchTaken Bltu 0x7FFF_FFFF 0x8000_0000 === True`;
  `branchTaken Bgeu 0xFFFF_FFFF 0x0000_0000 === True`;
  `branchTaken Bltu 0x8000_0000 0x7FFF_FFFF === False`.
- Reflexive equality: `branchTaken Beq a a === True`.

### 11.3 Decode tightening (`Test.Isa`)

- Add a `testCase`: a `SHIFT` word with `shOp = 0b11` decodes to
  `Left ReservedFieldNonZero`. Build the word via the existing field-join helper
  (group `0b10`, sub `0xc`, `imm[10:9] = 0b11`, `imm[8:5] = 0`, `amt` arbitrary,
  `rs2 = 0`).
- The existing "any 32-bit word decodes canonical or traps" property continues
  to hold (such words now take the trap branch).

---

## 12. Files touched

```
new:      hdl/src/Tamal/Alu.hs              -- AluOp, alu, dataResult (+ SPDX header)
          hdl/src/Tamal/Branch.hs           -- BranchOp, branchTaken   (+ SPDX header)
          hdl/tests/Test/Alu.hs             -- tests :: TestTree       (+ SPDX header)
          hdl/tests/Test/Branch.hs          -- tests :: TestTree       (+ SPDX header)

modified: hdl/src/Tamal/Isa.hs              -- decodeData sub 0xc: add `&& shOp /= 0b11` (line 227)
          hdl/tests/Test/Isa.hs             -- assert shOp=0b11 -> Left ReservedFieldNonZero
          hdl/tamal.cabal                   -- library exposed-modules += Tamal.Alu, Tamal.Branch
                                            --   test other-modules   += Test.Alu, Test.Branch
          hdl/tests/unittests.hs            -- import qualified Test.Alu, Test.Branch;
                                            --   add Test.Alu.tests, Test.Branch.tests to the tree
          docs/superpowers/specs/2026-07-01-tamal-isa-design.md
                                            -- §4/§6.3: li = LUI + ADDI; §6.2: LOAD_IMM sext(imm)

no change: hdl/tests/Test/Gen.hs            -- genShOp already excludes 0b11 (line 93)
```

`tamal.cabal` insertion points: add the two modules to the `library`
`exposed-modules` list (currently `Tamal.Domain … Tamal.Trace`) and the two test
modules to the `test-suite test-library` `other-modules` list (currently
`Test.Gen … Test.Trace`). In `tests/unittests.hs`, add
`import qualified Test.Alu` / `import qualified Test.Branch` and append
`Test.Alu.tests`, `Test.Branch.tests` to the top-level `testGroup "tamal"` list.

---

## 13. Verification

From `hdl/` (cold Clash/GHC builds are slow — expected; caching is load-bearing
in CI):

```
cabal build
cabal test                       # hedgehog: Test.Alu, Test.Branch, Test.Isa (+ existing)
cabal run clash -- Tamal --verilog   # codegen smoke — confirms the new modules are
                                     # Clash-clean even though they aren't in topEntity yet
```

The two new modules are pure and not yet referenced by `topEntity`, so the
Verilog smoke exercises library compilation under Clash, not new gateware.

---

## 14. Out of scope / follow-ups (roadmap)

Ordered as in `hdl/PLAN.md`:

1. **This spec** — ALU + branch comparator (pure, hedgehog). ← implement next
2. **Register file** (16×32, `x0` = 0) — trivial, lands with the Engine.
3. **`Engine.step`** — the Mealy transition composing decode + regfile + `alu`/
   `dataResult` + `branchTaken` + `Serdes` + `Crc` + `Trace` + the bus FSM;
   includes `RDSR` (CRC special-register read) and PC/branch-offset math.
4. **`topEntity` shell** — instr-BRAM, ring-BRAM, UART load/drain FSM, SCK/edge
   gen, `IOBUF` tri-state wiring (the impure, timing-critical part).

Explicitly deferred here: the register file, `Engine.step`, `RDSR`, all BUS-op
*execution/sequencing*, signed branches (`BLT`/`BGE`), and the assembler-side
`li` constant-tiling (§7.3 / the asm design §5.4).

---

## 15. Prior art

- **[Lion](https://github.com/standardsemiconductor/lion)** — RV32I in Clash.
  This layer borrows its idioms directly: a small total `Op`-dispatched `alu`, a
  total `decode → Either Exception`, a pure `branch`-style comparator, and
  synthesizable ADTs with `Generic`/`NFDataX`. tamal diverges from RV32I (own
  opcode groups, 11-bit immediates, unsigned-only branches in v1), so no
  `riscv-formal`/RVFI — hedgehog property tests are the verification baseline.
- **[mole](https://github.com/felipebalbi/mole)** — the sibling I2C/I3C rig;
  source of the bus-agnostic Layer-0 engine / protocol-aware Layer-1 host split
  that keeps this compute layer eSPI-ignorant.
