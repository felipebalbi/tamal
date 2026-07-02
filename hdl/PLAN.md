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
the **UART transport** included — and now the keystone **`Engine.step`** Mealy is
built and hedgehog-tested too. What remains is the impure `topEntity` shell that
wires everything to the pins.

| Module | Purpose | Status |
|---|---|---|
| `Tamal.Isa` | `Instr` ADT, total `encode`/`decode`, reserved-field trap | done, tested |
| `Tamal.Crc` | `crc8Update` (poly 0x07, init 0x00, MSB-first) | done, tested |
| `Tamal.Bus.Serdes` | x1 serialize/deserialize + `tarBeat` | done, tested |
| `Tamal.Config` | `SET_CONFIG` decode | done, tested |
| `Tamal.Trace` | ring record encode (+ trap flag/reason) + `ringPush` overflow | done, tested |
| `Tamal.Alu` | `alu` core + `dataResult` wrapper (DATA group) | done, tested |
| `Tamal.Branch` | `branchTaken` comparator (CTRL group, unsigned) | done, tested |
| `Tamal.RegFile` | 16×32 register file, `x0` hardwired 0 | done, tested |
| `Tamal.Uart.*` | 8N1 UART (NCO tick, RX, TX, umbrella) — host transport | done, tested |
| `Tamal.Engine` | `step` Mealy: fetch/decode/datapath + SCK bus FSM + trace + HALT/TRAP | done, tested |
| `Tamal.Domain` | `Dom100` clock domain | done |
| `Tamal` (top) | synthesis entry point | **placeholder heartbeat** (LED blink) |

Confirmed absent: only the real `topEntity` shell (the Engine wired to the pins).

## The gap

Every ingredient the engine needs exists as a tested leaf: decode
(`Isa`), operand compute (`Alu`, `Branch`), architectural state (`RegFile`), the
bus kernel (`Bus.Serdes`), CRC, trace-ring encoding, and — off to the side — the
host transport (`Uart`). The keystone that composed them is now done, so one
thing remains:

1. **`Engine.step` :: `State -> BusIn -> (State, BusOut, Maybe Ring)`** — *done,
   hedgehog-tested.* The first full *stateful* piece (a Mealy transition). It owns
   the `State` record (PC, `Regs`, config, RX CRC-8, ring pointer, bus-FSM state)
   and composes decode + regfile + `alu`/`dataResult` + `branchTaken` + `Serdes` +
   `Crc` + trace, plus PC/branch-offset math, `RDSR`, the SCK `/5` bus FSM, and the
   HALT/TRAP lifecycle. See `docs/superpowers/specs/2026-07-02-tamal-engine-design.md`.
2. **The real `topEntity` shell** — instr-BRAM, ring-BRAM, the **UART load/drain
   FSM** (UART ↔ memory), SCK/edge gen, and `IOBUF` tri-state wiring. This is
   where the UART and the Engine finally meet the pins. ← **do this next**


## Next step: the `topEntity` shell

`Engine.step` is done and hedgehog-tested (92 engine+leaf properties; the bus
timing, PC/branch math, CRC residue, trace ring, and HALT/TRAP lifecycle all
under test via a pure-fold cosimulation driver + reference interpreter). What
remains is the impure shell that lifts `step` onto the board:

- **instruction BRAM** — synchronous, feeding `BusIn.instrWord` at the registered
  `pcOut` (the `Fetch` phase models the 1-cycle latency).
- **ring BRAM** — driven by the `Maybe Ring` writes; drained on `halted`.
- **UART load/drain FSM** — control-plane load into instr-BRAM; on HALT, stream
  the ring out over the `Uart` transport.
- **pin wiring** — `IOBUF` tri-states from `lanesOut` `(o, oe)`, plus `CS#`/`SCK`/
  `RESET#` outputs and the `ALERT#` synchronizer, bound to the XDC.

This is the impure, timing-critical part; it warrants its own design doc.

## Ordering

1. **ALU + branch comparator** (pure, hedgehog) — done
2. **Register file** (16×32, `x0` = 0) — done
3. **UART transport** (8N1 RX/TX, NCO 16× oversample, loopback-tested) — done
4. **`Engine.hs` — `step` Mealy** — done, hedgehog-tested (composes decode +
   regfile + `alu`/`dataResult` + `branchTaken` + serdes + CRC + trace; includes
   `RDSR`, PC/branch-offset math, the SCK `/5` bus FSM, and HALT/TRAP)
5. **Real `topEntity` shell** ← do this next — instr-BRAM, ring-BRAM, UART
   load/drain FSM, SCK gen, IOBUF wiring (the impure, timing-critical part)

Short version: the pure leaves and the Engine keystone are all in place and
tested — what remains is wiring `step` to the pins in the impure `topEntity`
shell.
