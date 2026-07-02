# Tamal HDL — build plan / next steps

Living roadmap for the Clash gateware. Tracks what pure cores exist, what's
missing, and the order to build them. Anchored to the ISA & HDL Engine design
(`docs/superpowers/specs/2026-07-01-tamal-isa-design.md`, esp. §6, §10, §11), the
ALU/branch design (`docs/superpowers/specs/2026-07-01-tamal-alu-branch-design.md`),
the register-file design (`.../2026-07-01-tamal-register-file-design.md`), and the
UART design (`.../2026-07-01-tamal-uart-design.md`).

## Where things stand

Every **pure leaf core** from the §10 module decomposition is built and
hedgehog-tested — the compute layer (ALU + branch), the **register file**, and
the **UART transport** included. What remains is the keystone **Engine** and the
impure `topEntity` shell that wires everything to the pins.

| Module | Purpose | Status |
|---|---|---|
| `Tamal.Isa` | `Instr` ADT, total `encode`/`decode`, reserved-field trap | done, tested |
| `Tamal.Crc` | `crc8Update` (poly 0x07, init 0x00, MSB-first) | done, tested |
| `Tamal.Bus.Serdes` | x1 serialize/deserialize + `tarBeat` | done, tested |
| `Tamal.Config` | `SET_CONFIG` decode | done, tested |
| `Tamal.Trace` | ring record encode + `ringPush` overflow | done, tested |
| `Tamal.Alu` | `alu` core + `dataResult` wrapper (DATA group) | done, tested |
| `Tamal.Branch` | `branchTaken` comparator (CTRL group, unsigned) | done, tested |
| `Tamal.RegFile` | 16×32 register file, `x0` hardwired 0 | done, tested |
| `Tamal.Uart.*` | 8N1 UART (NCO tick, RX, TX, umbrella) — host transport | done, tested |
| `Tamal.Domain` | `Dom100` clock domain | done |
| `Tamal` (top) | synthesis entry point | **placeholder heartbeat** (LED blink) |

Confirmed absent: no `Engine.hs`, and the real `topEntity` shell.

## The gap

Every ingredient the engine needs now exists as a tested leaf: decode
(`Isa`), operand compute (`Alu`, `Branch`), architectural state (`RegFile`), the
bus kernel (`Bus.Serdes`), CRC, trace-ring encoding, and — off to the side — the
host transport (`Uart`). Two things remain:

1. **`Engine.step` :: `State -> BusIn -> (State, BusOut, Maybe Ring)`** — the first
   full *stateful* piece (a Mealy transition). It introduces the `State` record
   (PC, `Regs`, config, RX CRC-8, ring pointer, bus-FSM state) and composes decode
   + regfile + `alu`/`dataResult` + `branchTaken` + `Serdes` + `Crc` + `Trace`,
   plus PC/branch-offset math, `RDSR` (RX CRC-8 special-register read), and the
   multi-cycle bus FSM that sequences BUS ops against externally-timed SCK.
2. **The real `topEntity` shell** — instr-BRAM, ring-BRAM, the **UART load/drain
   FSM** (UART ↔ memory), SCK/edge gen, and `IOBUF` tri-state wiring. This is
   where the UART and the Engine finally meet the pins.

## Next step: `Engine.step`

The compute cores, register file, and transport are done; the Engine is now an
*assembly* problem, not an *invention* problem — every function it calls is
built and tested. It is, however, the difficulty spike: the first Mealy machine
of real size, and the first to sequence the externally-timed bus. It warrants its
own design doc (like the leaves), and may split into sub-specs — e.g. the
decode→regfile→ALU→writeback / branch path vs. the bus-op sequencing FSM.

Concrete scope (§6.1 / §6.2 / §7 / §10):

- **`State`** — PC, `Regs`, config register, RX CRC-8 accumulator, ring
  write-pointer + sticky overflow, bus-FSM/SCK-phase state.
- **DATA/CTRL** — operand fetch (`readReg`), `dataResult` writeback (discarding
  `x0`), `branchTaken` + `PC += signExtend off`.
- **`RDSR`** — special-register read (sr=0 → RX CRC-8; else TRAP).
- **BUS ops** — the FSM that drives/samples `Bus.Serdes` beats with turnaround.
- **Tests** — hedgehog over instruction sequences; the engine executes a small
  program and its `State`/ring output matches a reference (test-plan item 5).

## Ordering

1. **ALU + branch comparator** (pure, hedgehog) — done
2. **Register file** (16×32, `x0` = 0) — done
3. **UART transport** (8N1 RX/TX, NCO 16× oversample, loopback-tested) — done
4. **`Engine.hs` — `step` Mealy** ← do this next — composes decode + regfile +
   `alu`/`dataResult` + `branchTaken` + serdes + CRC + trace; includes `RDSR`,
   PC/branch-offset math, and the bus-op FSM (test-plan item 5)
5. **Real `topEntity` shell** — instr-BRAM, ring-BRAM, UART load/drain FSM, SCK
   gen, IOBUF wiring (the impure, timing-critical part)

Short version: the pure leaves are all in place — the Engine is the next build,
and it turns from design problem into wiring because decode, the register file,
the ALU/branch cores, serdes, CRC, and trace are already solid.
