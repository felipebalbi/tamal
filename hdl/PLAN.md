# Tamal HDL — build plan / next steps

Living roadmap for the Clash gateware. Tracks what pure cores exist, what's
missing, and the order to build them. Anchored to the ISA & HDL Engine design
(`docs/superpowers/specs/2026-07-01-tamal-isa-design.md`, esp. §6, §10, §11) and
the ALU/branch design (`docs/superpowers/specs/2026-07-01-tamal-alu-branch-design.md`).

## Where things stand

Every **pure leaf core** from the §10 module decomposition is built and
hedgehog-tested — *the compute layer (ALU + branch comparator) included*. What
remains is the **register file** and the keystone **Engine**.

| Module | Purpose | Status |
|---|---|---|
| `Tamal.Isa` | `Instr` ADT, total `encode`/`decode`, reserved-field trap | done, tested |
| `Tamal.Crc` | `crc8Update` (poly 0x07, init 0x00, MSB-first) | done, tested |
| `Tamal.Bus.Serdes` | x1 serialize/deserialize + `tarBeat` | done, tested |
| `Tamal.Config` | `SET_CONFIG` decode | done, tested |
| `Tamal.Trace` | ring record encode + `ringPush` overflow | done, tested |
| `Tamal.Alu` | `alu` core + `dataResult` wrapper (DATA group) | done, tested |
| `Tamal.Branch` | `branchTaken` comparator (CTRL group, unsigned) | done, tested |
| `Tamal.Domain` | `Dom100` clock domain | done |
| `Tamal` (top) | synthesis entry point | **placeholder heartbeat** (LED blink) |

Confirmed absent: no `Engine.hs`, no register file.

## The gap

The keystone `Engine.step :: State -> BusIn -> (State, BusOut, Maybe Ring)`
(§10) now has both compute cores it needs — `dataResult`/`alu` for the DATA
group, `branchTaken` for CTRL branches. Two things still stand between the cores
and a running engine:

1. **The register file** (16×32, `x0` = 0) — the last pure-ish leaf. It holds the
   architectural state the Engine reads (`rs1`/`rs2`) and writes (`rd`), with
   `x0` hardwired to zero. Small, but its read/write interface is what
   `Engine.step` is built on.
2. **`Engine.step` itself** — the first *stateful* piece (a Mealy transition) and
   the composition point for decode + regfile + `alu`/`dataResult` + `branchTaken`
   + `Serdes` + `Crc` + `Trace`, plus PC/branch-offset math, `RDSR`
   (RX CRC-8 special-register read), and the multi-cycle bus FSM that sequences
   BUS ops.

## Next step: the register file

Not the whole Engine yet. Build the 16×32 register file next, because:

1. **It continues the established rhythm.** It is the last thing buildable and
   testable close to isolation before the difficulty spike into the stateful
   `Engine.step` Mealy and the impure `topEntity` shell (BRAM, UART, SCK/edge
   gen, IOBUF tri-states) — the parts AGENTS.md flags as the genuinely hard
   timing/tri-state logic.
2. **It pins the Engine's operand interface.** `Engine.step` reads two source
   registers and writes one destination each cycle; nailing the regfile
   read/write semantics (and `x0`) first turns that part of the Engine into
   wiring.

Concrete scope (§6 / §10 / §14):

- **State** — `Vec 16 (BitVector 32)`, initialised to all-zero.
- **Read** — combinational read of a `Reg` selector, with `x0` reading `0`
  regardless of contents.
- **Write** — write `rd` unless `rd == x0` (writes to `x0` are discarded).
- **v1 register window** — x0..x15 live; x16..x31 are rejected upstream
  (assembler + engine), so the file indexes 16 entries.
- **Tests** — mirror the existing hedgehog style: read-after-write, `x0` always
  reads zero, writes to `x0` are no-ops, reads of untouched registers, and
  independence of distinct registers.

## Ordering

1. **ALU + branch comparator** (pure, hedgehog) — done
2. **Register file** (16×32, `x0` = 0) ← do this next
3. **`Engine.hs` — `step` Mealy** — composes decode + regfile + `alu`/`dataResult`
   + `branchTaken` + serdes + CRC + trace; includes `RDSR` and PC/branch-offset
   math and the bus-op FSM (test-plan item 5)
4. **Real `topEntity` shell** — instr-BRAM, ring-BRAM, UART load/drain FSM, SCK
   gen, IOBUF wiring (the impure, timing-critical part)

Short version: the register file is the right next step — it's the last leaf,
keeps the hedgehog-first discipline, and fixes the operand interface so the
subsequent `Engine.step` is assembly, not invention.
