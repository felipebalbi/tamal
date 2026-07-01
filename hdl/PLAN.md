# Tamal HDL — build plan / next steps

Living roadmap for the Clash gateware. Tracks what pure cores exist, what's
missing, and the order to build them. Anchored to the ISA & HDL Engine design
(`docs/superpowers/specs/2026-07-01-tamal-isa-design.md`, esp. §6, §10, §11).

## Where things stand

Every **pure leaf core** from the §10 module decomposition is built and
hedgehog-tested — *except the compute layer*.

| Module | Purpose | Status |
|---|---|---|
| `Tamal.Isa` | `Instr` ADT, total `encode`/`decode`, reserved-field trap | done, tested |
| `Tamal.Crc` | `crc8Update` (poly 0x07, init 0x00, MSB-first) | done, tested |
| `Tamal.Bus.Serdes` | x1 serialize/deserialize + `tarBeat` | done, tested |
| `Tamal.Config` | `SET_CONFIG` decode | done, tested |
| `Tamal.Trace` | ring record encode + `ringPush` overflow | done, tested |
| `Tamal.Domain` | `Dom100` clock domain | done |
| `Tamal` (top) | synthesis entry point | **placeholder heartbeat** (LED blink) |

Confirmed absent: no `Engine.hs`, no ALU, no branch unit, no register file.

## The gap

The keystone `Engine.step :: State -> BusIn -> (State, BusOut, Maybe Ring)`
(§10) cannot execute the DATA group without an **ALU**, nor resolve CTRL
branches without a **branch comparator**. These are the only two remaining pure,
combinational, independently-testable leaf cores — and both are prerequisites
for the Engine.

## Next step: the pure compute pair (ALU + branch comparator)

Not the whole Engine yet. Build the ALU (DATA group) and the branch comparator
(CTRL group) next, because:

1. **It continues the established rhythm.** Everything so far is a small, total,
   pure function property-tested in isolation before assembly. §10 explicitly
   names the `Op`-dispatched ALU and `branch :: Branch -> BitVector 32 ->
   BitVector 32 -> Bool` as exactly this kind of function. This is the last work
   that fits the pattern.
2. **It unblocks the keystone.** With ALU + branch done, `Engine.step` becomes
   *wiring*, not *invention*.
3. **It's the last thing buildable in complete isolation** before the difficulty
   spike into stateful (`Engine.step` Mealy) and impure (BRAM, UART, SCK/edge
   gen, IOBUF tri-states) territory — the parts AGENTS.md flags as the genuinely
   hard timing/tri-state logic. Nail the pure compute while it's cheap.

Concrete scope (straight off §6.1 / §6.2):

- **ALU** — `LOAD_IMM`, `LUI`, `MOV`, `ADD`/`ADDI`, `SUB`, `AND`/`ANDI`,
  `OR`/`ORI`, `XOR`/`XORI`, `SHIFT` (SLL/SRL/SRA). Signedness seam: `SRA` needs
  `unpack :: BitVector 32 -> Signed 32`.
- **Branch** — `BEQ`/`BNE` plus **unsigned** `BLTU`/`BGEU` (unsigned compare is
  the easy thing to get wrong).
- **Tests** — mirror the existing hedgehog style: cross-check against a
  reference model, `x0`-stays-zero, shift-amount edges, signed/unsigned
  boundaries (test-plan item 5 groundwork).

## Ordering after the compute pair

1. **ALU + branch comparator** (pure, hedgehog) ← do this next
2. **Register file** (16×32, `x0` = 0) — trivial, lands with the Engine
3. **`Engine.hs` — `step` Mealy** — composes decode + regfile + ALU + branch +
   serdes + CRC + trace (test-plan item 5)
4. **Real `topEntity` shell** — instr-BRAM, ring-BRAM, UART load/drain FSM, SCK
   gen, IOBUF wiring (the impure, timing-critical part)

Short version: the ALU (paired with the branch comparator) is the right next
step — it's the last pure core, keeps the hedgehog-first discipline, and turns
the subsequent `Engine.step` from a design problem into an assembly job.
