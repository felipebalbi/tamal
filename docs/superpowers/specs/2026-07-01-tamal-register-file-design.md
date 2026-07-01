# Tamal — Register File (Engine state leaf) Design

Date: 2026-07-01
Status: Approved (design); implementation not started
Scope: The 16×32 register file as a small pure leaf — `Tamal.RegFile`: an opaque
`newtype Regs`, `initRegs`, `readReg`, `writeReg`, with `x0` hardwired to zero and
a total (aliasing, never-trapping) selector. Hedgehog-tested. The Engine, the
`State` aggregate, `Engine.step`, `RDSR`, and the BRAM/UART shell are **out of
scope** (§2, §9).

Companion to the ISA & HDL Engine design
(`docs/superpowers/specs/2026-07-01-tamal-isa-design.md`, esp. §6, §7.1, §10) and
the ALU/branch design
(`docs/superpowers/specs/2026-07-01-tamal-alu-branch-design.md`, esp. §10 Engine
integration contract). Written to be implementable from a fresh session.

---

## 1. Purpose & context

The pure **compute layer** (`Tamal.Alu`, `Tamal.Branch`) is built and tested. The
register file is the **last pure leaf** before the keystone `Engine.step`. It
holds the architectural register state the Engine reads (`rs1`/`rs2`) and writes
(`rd`), with `x0` hardwired to `0`.

Per ISA design §7.1 the register file is one component of the Engine's `State`
(alongside `PC`, the config register, the RX CRC-8 accumulator, the ring
write-pointer, and the bus-FSM state). We extract it as an independently-testable
module **now** — a small, hedgehog-tested warm-up — so that when `Engine.step`
lands, its operand-fetch/writeback path is *wiring*, not *invention*.

---

## 2. Scope & non-goals

**In scope**

- `Tamal.RegFile`: the opaque `newtype Regs`, `initRegs`, `readReg`, `writeReg`.
- `x0` semantics (read → `0`; write → discarded), owned by this module.
- Low-4-bit truncating selector (total; out-of-window selectors alias).
- Hedgehog property tests.
- Cabal / test-runner wiring.

**Out of scope (deferred)**

- **`Engine.step`** (the Mealy transition) and the `State` record that will hold
  a `Regs` field. `readReg`/`writeReg` are pure functions on a `Regs` *value*, so
  no Engine is needed to build or test them.
- **`RDSR`** and the config/CRC special registers (Engine-level state).
- **x16..x31 rejection.** ISA design §4 makes rejecting x16..x31 an
  *assembler + engine* responsibility. `Tamal.Isa.decode` does **not** currently
  reject a 5-bit register field ≥ 16, and neither does this leaf: it stays total
  and **aliases** an out-of-window selector onto its low-4-bit twin (§4, §3
  decision 4). Trapping x16..x31, if ever wanted, belongs to the Engine/assembler.
- The impure `topEntity` shell (instr-BRAM, ring-BRAM, UART, SCK/edge, `IOBUF`).

---

## 3. Design decisions

| # | Decision | Rationale |
|---|----------|-----------|
| 1 | **Separate `Tamal.RegFile` module** (vs folding into `Engine.hs`). | Makes the register file independently hedgehog-testable now, matching the existing pure-leaf layout. The Engine will simply hold a `Regs` field in its `State` and call these functions. |
| 2 | **Opaque `newtype Regs`** (constructor **not** exported) vs a bare `type Regs = Vec 16 (BitVector 32)` alias. | Encapsulation: consumers touch `Regs` only through `initRegs`/`readReg`/`writeReg`, so the internal representation can change without breaking the Engine. Zero-cost at synthesis (newtypes erase; the `Vec 16` becomes a bank of fabric flip-flops). |
| 3 | **Pure `readReg`/`writeReg` over a `Regs` value**, not a `Signal`/`blockRam` component. | Mandated by the Engine's Mealy structure: the register lives in the `mealy`/`register` wrapper, and `Engine.step` is a *pure combinational* transition with `Regs` as one field of its state. A `blockRam` regfile (1-cycle read latency, `Signal`-level) cannot be a Mealy-state field. 512 bits belongs in fabric FFs, not BRAM. Gives read-old/write-new for free (§5). |
| 4 | **Take `Reg` (`BitVector 5`), truncate to the low 4 bits**; out-of-window selectors alias rather than trap. | Call-site uniformity with `decode` (pass `rs1`/`rs2`/`rd` straight through, no conversion). Keeping the leaf **total** (aliasing) rather than partial (trapping) matches the "leaf functions are total; decode/engine handle rejection" idiom used throughout tamal. |
| 5 | **`x0` semantics live in the regfile**: `readReg` of `x0` returns `0`; `writeReg` to `x0` is discarded. | Per ALU/branch design §10, `x0`-hardwiring is the regfile's responsibility. Housing the invariant in one place keeps `Engine.step` free of `x0` special-casing. Detected on the **truncated index** (`idx == 0`), so an out-of-window x16 aliases x0 *consistently* (reads `0`, write discarded). |

---

## 4. Module layout & public interface

One new module `hdl/src/Tamal/RegFile.hs`, `[pure]`, carrying the REUSE/SPDX
header required of every `hdl/**/*.hs` file:

```haskell
-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0
```

```haskell
module Tamal.RegFile
  ( Regs          -- opaque: constructor intentionally NOT exported
  , initRegs
  , readReg
  , writeReg
  ) where

import Clash.Prelude
import Tamal.Isa (Reg)   -- Reg = BitVector 5

newtype Regs = Regs (Vec 16 (BitVector 32))
  deriving stock (Generic, Show, Eq)
  deriving anyclass NFDataX

-- | Power-up contents: all 16 registers zeroed. This IS the reset value of the
-- Engine's register bank (the top ties reset de-asserted; state relies on
-- power-up @init@ — AGENTS.md, ISA §7.1).
initRegs :: Regs
initRegs = Regs (repeat 0)

-- | Map a 5-bit selector to a physical slot: the low 4 bits. Out-of-window
-- selectors (x16..x31) alias their low-4 twin — the leaf stays total (§2, §3.4).
regIndex :: Reg -> Index 16
regIndex r = unpack (truncateB r)   -- truncateB r :: BitVector 4

-- | Read a register value; @x0@ (index 0) reads 0 regardless of slot contents.
readReg :: Regs -> Reg -> BitVector 32
readReg (Regs v) r
  | idx == 0  = 0
  | otherwise = v !! idx
  where idx = regIndex r

-- | Write a register value; writes to @x0@ (index 0) are discarded.
writeReg :: Regs -> Reg -> BitVector 32 -> Regs
writeReg regs@(Regs v) r x
  | idx == 0  = regs
  | otherwise = Regs (replace idx x v)
  where idx = regIndex r
```

**Interface rationale**

- `Regs` is exported as an **opaque type** (no data constructor). Consumers build
  it with `initRegs` and evolve it with `writeReg`; they inspect it with
  `readReg`. The Engine holds a `Regs` in its `State` and never needs the
  constructor. Derived `Eq`/`Show` are enough for tests and failure messages.
- `Reg` is imported from `Tamal.Isa` (it is `BitVector 5`), so read/write take the
  exact selector type `decode` produces.

**Clash notes**

- `truncateB r :: BitVector 4` keeps the least-significant 4 bits; `unpack ::
  BitVector 4 -> Index 16` (the `BitSize` of `Index 16` is 4). The `Index 16`
  result type drives `truncateB`'s output width.
- `(!!) :: Vec n a -> Index n -> a` and `replace :: Index n -> a -> Vec n a ->
  Vec n a` are the total Clash indexing/update primitives (no out-of-range case —
  `Index 16` is 0..15 by construction). `repeat :: KnownNat n => a -> Vec n a`.
- `idx == 0` compares `Index 16` values (`Eq`/`Num`); `0` is the `x0` slot.

---

## 5. Mealy / Engine integration contract (informative)

Not implemented here; recorded so the interface is the right shape for
`Engine.step`.

- **The register lives in the Mealy wrapper, not here.** `mealy :: NFDataX s => (s
  -> i -> (s, o)) -> s -> Signal dom i -> Signal dom o` holds the flip-flops.
  `Engine.step :: State -> BusIn -> (State, BusOut, Maybe Ring)` is a pure
  combinational transition; `Regs` is one field of `State`. Inside `step`:

  ```haskell
  let rs1v  = readReg (regs s) rs1          -- combinational read of THIS cycle
      rs2v  = readReg (regs s) rs2
      regs' = writeReg (regs s) rd result   -- pure next-state value
  in (s { regs = regs' }, …)               -- mealy clocks it at the edge
  ```

- **Read-old / write-new, hazard-free.** Because `readReg` reads the current-cycle
  value and `writeReg` produces the *next*-cycle value, a `step` that reads `rs`
  and writes `rd` in the same cycle — even when `rs == rd` — reads the **old**
  `rd` and commits the new value at the clock edge. No read-during-write hazard.
- **`initRegs` is the power-up content.** The top ties reset permanently
  de-asserted (AGENTS.md, ISA §7.1); the Mealy's initial `State` embeds
  `initRegs`, so it is the literal power-on value of the register bank.
- **No BRAM.** 512 bits (16×32) synthesizes to fabric flip-flops with a
  combinational read (16:1 32-bit mux); `blockRam` is deliberately avoided (it
  would not fit the pure Mealy state and would add read latency). The large
  instr-BRAM and ring-BRAM in ISA §10 are separate, live in the *impure*
  `topEntity` shell around `step`, and are out of scope here.
- **x16..x31.** `readReg`/`writeReg` alias out-of-window selectors (total). If the
  Engine or assembler wants to *reject* x16..x31, that trap lives there, not here.

---

## 6. Testing (hedgehog baseline)

One new test module `hdl/tests/Test/RegFile.hs`, exporting `tests :: TestTree`,
mirroring the existing tasty + tasty-hedgehog style and carrying the SPDX header.
Reuse `Test.Gen` (`genReg`, `genWord`); add two local generators:

```haskell
-- A register whose low-4 index is non-zero (i.e. not x0), in x1..x15.
genNonZeroReg :: Gen Reg
genNonZeroReg = fromIntegral <$> Gen.integral (Range.linear 1 15)

-- An arbitrary Regs built only through the public API (no constructor access).
-- NB: 'foldl'' is Data.List's — Clash.Prelude's 'foldl' is the 'Vec' one, and
-- 'ws' is a list.
genRegs :: Gen Regs
genRegs = do
  ws <- Gen.list (Range.linear 0 20) ((,) <$> genReg <*> genWord)
  pure (foldl' (\rs (r, v) -> writeReg rs r v) initRegs ws)
```

Properties / cases:

- **Read-after-write** (r ≠ x0): `readReg (writeReg rs r v) r === v`
  (`rs <- genRegs`, `r <- genNonZeroReg`, `v <- genWord`). Overwrite wins over any
  prior slot contents.
- **`x0` always reads 0**: `readReg rs 0 === 0`, and `readReg (writeReg rs 0 v) 0
  === 0` (`rs <- genRegs`).
- **Write to `x0` is a no-op**: `writeReg rs 0 v === rs` (uses derived `Eq Regs`).
- **Register independence** (distinct indices): for `regIndex r1 /= regIndex r2`,
  `readReg (writeReg rs r1 v) r2 === readReg rs r2`. Generate the pair with
  `Gen.filter` on differing low-4 indices. (`regIndex` is not exported, so the
  test recomputes the low-4 index inline: `truncateB r :: BitVector 4`.)
- **Untouched read from init**: `readReg initRegs r === 0` for any `r <- genReg`
  (all slots zero, and `x0` hardwired).
- **x16..x31 alias x0..x15** (pins the truncation): for `r <- genNonZeroReg`
  (x1..x15), `readReg (writeReg initRegs (r + 16) v) r === v`; and `writeReg rs 16
  v === writeReg rs 0 v` (both target the x0 slot → both no-ops), i.e. `=== rs`.

---

## 7. Files touched

```
new:      hdl/src/Tamal/RegFile.hs      -- Regs (opaque), initRegs, readReg, writeReg (+ SPDX)
          hdl/tests/Test/RegFile.hs     -- tests :: TestTree (+ SPDX)

modified: hdl/tamal.cabal               -- library exposed-modules += Tamal.RegFile
                                        --   test other-modules   += Test.RegFile
          hdl/tests/unittests.hs        -- import qualified Test.RegFile; add Test.RegFile.tests

no change: hdl/tests/Test/Gen.hs        -- reuse genReg, genWord; genNonZeroReg/genRegs are
                                        --   local to Test.RegFile
```

---

## 8. Verification

From `hdl/` (cold Clash/GHC builds are slow — expected):

```
stack build
stack test                           # hedgehog: Test.RegFile (+ existing)
stack run clash -- Tamal --verilog   # codegen smoke — confirms Tamal.RegFile is Clash-clean
make format-check                    # fourmolu style gate
```

`Tamal.RegFile` is pure and not yet referenced by `topEntity`, so the Verilog
smoke exercises library compilation under Clash, not new gateware.

---

## 9. Out of scope / follow-ups (roadmap)

Ordered as in `hdl/PLAN.md`:

1. **This spec** — the register file (pure, hedgehog). ← implement next
2. **`Engine.step`** — the Mealy transition; introduces the `State` record (with a
   `Regs` field), composes decode + regfile + `alu`/`dataResult` + `branchTaken` +
   `Serdes` + `Crc` + `Trace`, and adds `RDSR`, PC/branch-offset math, and the
   bus-op FSM.
3. **`topEntity` shell** — instr-BRAM, ring-BRAM, UART load/drain FSM, SCK/edge
   gen, `IOBUF` tri-state wiring.

Explicitly deferred here: `Engine.step`, the `State` aggregate, `RDSR`, x16..x31
trapping, and all BUS-op execution.

---

## 10. Prior art

- **[Lion](https://github.com/standardsemiconductor/lion)** — RV32I in Clash; its
  register file is likewise a plain `Vec`-backed value read/written by pure
  functions inside the CPU's Mealy state, with `x0` hardwired.
- **[mole](https://github.com/felipebalbi/mole)** — the sibling I2C/I3C rig whose
  Layer-0/Layer-1 split keeps this leaf protocol-ignorant.
