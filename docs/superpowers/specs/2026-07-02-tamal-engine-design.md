# Tamal — Engine (`Engine.step`) Design

Date: 2026-07-02
Status: Approved (design); implementation not started
Scope: The keystone `Tamal.Engine` — the pure Mealy transition
`step :: State -> BusIn -> (State, BusOut, Maybe Ring)` that composes every
existing pure leaf (decode, register file, ALU/branch, serdes, CRC, trace) into
a running engine: instruction fetch/decode/execute, the DATA/CTRL datapath with
PC/branch math, `RDSR`, the multi-cycle SCK-timed bus FSM, trace-ring emission,
and the HALT/TRAP lifecycle. Also the small `Tamal.Trace` change that extends the
HALT record with a trap flag. The impure `topEntity` shell (instruction/ring
BRAM, UART load/drain FSM, `IOBUF` tri-states, pin wiring) is a **separate,
later spec** — this document ends at the pure/impure seam.

Companion to the ISA & HDL Engine design
(`docs/superpowers/specs/2026-07-01-tamal-isa-design.md`, esp. §4–§8, §10, §11),
the ALU/branch design (`.../2026-07-01-tamal-alu-branch-design.md`, esp. §10),
the register-file design (`.../2026-07-01-tamal-register-file-design.md`, esp.
§5), and the UART design (`.../2026-07-01-tamal-uart-design.md`). Roadmap context
in `hdl/PLAN.md`.

## 1. Purpose & framing

Every pure leaf the engine needs is already built and hedgehog-tested: `Isa`
(decode/encode), `Alu` (`alu`/`dataResult`), `Branch` (`branchTaken`), `RegFile`,
`Bus.Serdes` (`serializeX1`/`deserializeX1`/`tarBeat`), `Crc` (`crc8Update`),
`Trace` (`encodeRecord`/`ringPush`), and `Config` (`decodeConfig`). The engine is
therefore an **assembly problem, not an invention problem** — but it is the first
stateful piece of real size and the first to sequence the externally-observed,
SCK-timed bus.

Per the ISA reframing (§1 of that spec): the engine is a **programmable SPI shift
engine that knows the eSPI pins, not the eSPI protocol**. It shifts host-built
bytes onto the wire at the right times, guarantees a host-specified turnaround,
shifts response bits back, keeps an RX CRC-8 residue, and runs just enough
compute/control for reactive loops. All eSPI semantics live in the host.

`step` is a single pure transition; the `topEntity` lifts it with `mealy` /
`register`. Keeping `step` pure makes essentially all of the bus timing testable
under hedgehog (decision D2).

## 2. Design decisions

| # | Decision | Rationale |
|---|----------|-----------|
| D1 | **One unified Engine spec** (State + datapath + bus FSM together), implemented in ordered TDD tasks. | `step` is one Mealy function whose parts share the `State` record; splitting into sub-specs would fragment a single cohesive transition. |
| D2 | **Execution model = "fat pure step": `step` runs every 100 MHz fabric cycle and owns the SCK phase counter; SCK/CS#/RESET#/lanes are pure outputs.** The top is thin wiring (BRAM, IOBUF, ALERT# sync, UART). | Puts the entire bus-timing state machine under hedgehog (test item 5, "PUT_BYTE = 8 SCK cycles"). Keeps `step` the single Mealy the ISA spec names. Costs a small wording amendment to ISA §10 ("SCK edge timing" is now pure; only `IOBUF` tri-states + UART remain impure). |
| D3 | **Program memory = synchronous BRAM with an explicit `Fetch` bubble; PC is a word index, width `aw = 10`.** | Author chose BRAM over async/distributed RAM for fabric economy. Branch offsets are word-aligned instruction counts (ISA §6.1), so a word-index PC does `PC += off` with no byte scaling; `aw = 10` gives a 1024-word store, matching the ±1024-instruction branch range. The 1-cycle fetch bubble is negligible against ~40-cycle bus ops. |
| D4 | **TRAP → drive pins safe + write the HALT terminator with a `trap` flag + 3-bit `reason`.** Extends the `Tamal.Trace` HALT record into its 21 reserved bits. | Keeps the host-owned status byte intact while flagging engine traps distinctly; no new record type. The reserved bits already exist in the encoding. |
| D5 | **Trace ring stays drop-on-full with a sticky overflow marker (no wrap-around).** | Matches ISA §8, AGENTS.md ("drop with an overflow marker"), and the already-tested `ringPush`. A compliance transcript wants the ordered *prefix* + an explicit overflow flag, not a silently-overwritten tail. |
| D6 | **`GET_ALERT` is a single-cycle `Exec` op** (samples the synchronized alert into `rd[0]`), not a bus beat. | It toggles no SCK; electrically it is a sample, not a clocked transfer. |
| D7 | **All pin drives are held in `State` (registered / Moore-style outputs)**; `BusOut` reflects them. | Glitch-free bus pins; `BusOut` is a pure projection of `State`. |
| D8 | **SCK = fabric/5; `busPhase :: Index 5`; SCK low {0,1,2}, high {3,4}; PUT drives @ phase 0, GET samples @ phase 3.** | Simplest clean /5 divider (ISA §6). ≈30 ns setup before the rising edge; sample just after it. Asymmetric duty is fine at 20 MHz (a clean 50 % would need an MMCM — ISA §5.1, out of scope). The three constants are tunable knobs to be validated in silicon. |
| D9 | **No reset port; `start` performs a synchronous soft-init of the run state** (`pc`, `ringPtr`, `ovf`, `regs`, `rxCrc`, `cfg`←power-up default). | Preserves the power-up-`init`, no-reset design (AGENTS.md, ISA §7.1) while giving byte-reproducible re-runs from `Idle` **or** `Halted`. |
| D10 | **Execution follows TDD with a division of labour: the author writes the Clash under `src/`; the assistant writes the tests under `tests/` and mentors.** | Ping-pong TDD (assistant red → author green → refactor together) is a strong learning vehicle for Clash and keeps the engine honest against the reference model. |

## 3. Module boundary: signature, `BusIn`, `BusOut`, `Ring`, `State`

`Tamal.Engine` exports the `step` transition plus the types below. `step` is a
pure, total function of the current state and this cycle's inputs.

```haskell
step :: State -> BusIn -> (State, BusOut, Maybe Ring)
```

### 3.1 `BusIn` — what the top feeds the engine each fabric cycle

```haskell
data BusIn = BusIn
  { instrWord :: BitVector 32   -- word from instruction BRAM at the registered PC
                                --   (valid the cycle after Fetch; 1-cycle latency)
  , ioIn      :: Vec 4 Bit      -- sampled IO lanes (IO[1] = MISO for GET / in-band alert)
  , alertIn   :: Bit            -- synchronized ALERT# pin
  , startIn   :: Bool           -- control-plane trigger (top holds it until loaded)
  }
```

### 3.2 `BusOut` — pins + memory control (a pure projection of `State`)

```haskell
data BusOut = BusOut
  { pcOut  :: Unsigned 10       -- fetch address to instruction BRAM (aw = 10)
  , csN    :: Bit
  , sck    :: Bit
  , rstN   :: Bit
  , lanes  :: Lanes             -- Vec 4 (Bit, Bit) = (o, oe) per IO lane (Serdes.Lanes)
  , halted :: Bool              -- asserted in Halted; drives the top's ring-drain FSM
  }
```

### 3.3 `Ring` — the `Maybe Ring` third element (≤ 1 ring word per cycle)

```haskell
data Ring = Ring
  { rAddr :: Unsigned 12        -- ring-BRAM word address
  , rData :: BitVector 32       -- the record word
  }
```

`Just` = one ring-BRAM write this cycle. Multi-word records (MARK = 2 words) span
two cycles via the `TraceEmit` phase; CAPTURE, HALT, and REVISION are one word.

### 3.4 `State` — the Mealy state (power-up `init`, no reset port)

```haskell
data Phase
  = Idle | Preamble | Fetch | Exec
  | BusBeat | TraceEmit | WaitAlert | Halted
  deriving (Generic, NFDataX, Show, Eq)

data Pending
  = PendNone
  | PendGet Reg (Unsigned 4) Bool  -- rd, nbits, updateCrc?  (completion of a GET)
  | PendMark (BitVector 32)        -- payload word emitted by TraceEmit (a MARK)
  deriving (Generic, NFDataX, Show, Eq)

data State = State
  { phase    :: Phase
  -- architectural
  , pc       :: Unsigned 10
  , regs     :: Regs
  , cfg      :: Config
  , rxCrc    :: BitVector 8
  -- trace
  , ringPtr  :: Unsigned 12       -- next record slot (starts at 1)
  , ovf      :: Bool              -- sticky overflow
  -- pin latches (registered outputs, D7)
  , csN      :: Bit
  , sck      :: Bit
  , rstN     :: Bit
  , lanes    :: Lanes
  -- bus micro-FSM scratch
  , busPhase :: Index 5           -- SCK phase 0..4 (D8)
  , beatIx   :: Unsigned 4        -- current bit / TAR clock (0..)
  , beatTot  :: Unsigned 4        -- total beats for this op (bits, or TAR n)
  , shifter  :: BitVector 8       -- PUT: bits to drive; GET: MSB-first accumulator
  -- deferred work
  , pending  :: Pending
  , waitTimer :: BitVector 9      -- WAIT_ON timeout countdown
  } deriving (Generic, NFDataX, Show, Eq)

initState :: State   -- phase = Idle, pins safe (csN=1, sck=0, rstN=1, lanes=hiZ),
                     -- pc=0, regs=initRegs, cfg=powerUpDefault, rxCrc=0,
                     -- ringPtr=1, ovf=False, everything else zero.
```

`powerUpDefault = Config Controller X1 Sck20 AlertPin` (ISA §7.2).

## 4. Lifecycle FSM

Fabric-granular: each node is one or more 100 MHz cycles. `Fetch` is the 1-cycle
BRAM latency bubble; `Exec` is the decode/dispatch hub. HALT and TRAP are handled
**inline in the `Exec` transition** (each writes exactly one terminator word and
jumps to `Halted`), so they need no dedicated phase.

```
                        instr done: PC := PC+1 | PC+off
            ┌──────────────────────────────────────────────┐
            │                                               │
   start    v   REVISION@0; PC:=0        instr valid        │      HALT: terminator@limit
  Idle ──► Preamble ───────────────► Fetch ───────────► Exec ─────────────────────────► Halted
   ▲                                   ▲                 │ │ decode/config/rdsr err        ▲
   │ start (soft-init, D9)             │  op / emit done │ └──(inline: safe pins + trap)───┘
   └───────────────────────────────── (any) ◄───────────┤
                                          BusBeat  TraceEmit  WaitAlert
                                       (PUT/GET/TAR) (MARK w1) (WAIT_ON)
```

`Exec` dispatch (per decoded instruction class):

| Instr class | Work in `Exec` | Extra phase(s) | Next |
|---|---|---|---|
| DATA compute (LOAD_IMM/LUI/MOV/ADD…/SHIFT) | `regs := writeReg regs rd (dataResult …)` | — | Fetch |
| RDSR | `sr=0` → `writeReg regs rd (zeroExtend rxCrc)`; else trap | — | Fetch / Halted(trap) |
| Branch (BEQ/BNE/BLTU/BGEU) | `pc := taken ? pc+sext(off) : pc+1` | — | Fetch |
| SET_CONFIG | `cfg := decodeConfig p` (or trap) | — | Fetch / Halted(trap) |
| CRC_RESET | `rxCrc := 0` | — | Fetch |
| MARK | emit word0 (label) if 2 slots free | TraceEmit (word1 = payload) | Fetch |
| GET_ALERT | `rd := zeroExtend (selected alert bit)` | — | Fetch |
| WAIT_ON | arm `waitTimer` | WaitAlert (poll/tick) | Fetch |
| CS_/RST_ ASSERT/DEASSERT | set the pin latch | — | Fetch |
| PUT_BYTE / PUT_BITS | load `shifter`, set `beatTot` | BusBeat | Fetch |
| GET_BYTE / GET_BITS | set `pending = PendGet …` | BusBeat | Fetch |
| TAR | set `beatTot = n` | BusBeat | Fetch |
| HALT | write terminator@limit (status) | — | Halted |
| decode error / reserved group | — (inline trap) | — | Halted |

## 5. Bus micro-FSM & SCK timing

### 5.1 SCK generation (pure, D8)

SCK = fabric/5. Within a 5-phase beat (`busPhase :: Index 5`):

```
phase:   0    1    2    3    4     0    1    2    3    4
SCK :  ______________       ‾‾‾‾‾‾______________       ‾‾‾‾‾‾   (low {0,1,2}, high {3,4})
IO0 : <     bit7 (MSB)  drive@0    ><      bit6   drive@0    >   (PUT)
IO1 :                    o sample@3               o sample@3     (GET)
                              ↑ rising edge (2→3): slave latches PUT / we latch GET
```

`busPhase` increments each fabric cycle, wrapping 4→0. `sck := if busPhase >= 3
then 1 else 0`. Outside a bus op SCK holds idle-low. The rising edge falls at the
2→3 boundary; the falling edge at the 4→0 wrap.

### 5.2 Beat sequencing (reuses the tested `Serdes` leaf)

- **PUT_BYTE / PUT_BITS:** at phase 0 of each beat drive `serializeX1 byte !!
  beatIx` (MSB-first, IO[0] only; `IO[1..3]` tri-stated). `beatTot = 8` for a
  byte, `n` for `PUT_BITS`. `PUT_BITS` drives the **top `n` bits** (left-justified,
  since `serializeX1 byte !! 0` is the MSB); the immediate form's `imm[7:0]` is
  already left-justified (ISA §5.3), and the register form loads `shifter` from
  `rs1[7:0]`. Advance `beatIx` at the 4→0 wrap.
- **GET_BYTE / GET_BITS:** engine tri-states all its drivers (`lanes := hiZ`); at
  phase 3 accumulate `shifter := (shifter << 1) .|. ioIn!!1`. After `beatTot`
  beats the result is right-justified (first-sampled bit is the MSB of the field).
- **TAR n:** each of the `n` clocks drives `lanes := tarBeat beatIx` — clock 0 =
  `driveHigh` (all active lanes `oe=1, o=1`), clocks 1..n−1 = `hiZ`. SCK toggles on
  every clock. `n = 0` ⇒ zero clocks (host's deliberate too-short turnaround);
  `n = 2` is the legal eSPI TAR.

Durations: PUT/GET_BYTE = 8 × 5 = 40 cycles; PUT/GET_BITS `n` = `n` × 5; TAR `n` =
`n` × 5.

### 5.3 Completion (last beat, phase 4 → wrap)

- **PUT / TAR:** `pc := pc + 1` → `Fetch`.
- **GET (from `pending = PendGet rd nbits crc?`):** `regs := writeReg regs rd
  (zeroExtend shifter)`; emit a **CAPTURE** record `(nbits, shifter)`; if `crc?`
  (i.e. GET_BYTE, not GET_BITS) `rxCrc := crc8Update rxCrc shifter`; `pc := pc+1`
  → `Fetch`. Trace overflow drops only the CAPTURE — the register write, CRC
  update, and PC advance always happen (never stall the bus).

## 6. Datapath, config, pins, WAIT_ON

### 6.1 Operand fetch & DATA/RDSR writeback
`rs1v = readReg regs rs1`, `rs2v = readReg regs rs2` (RegFile hardwires
`x0 = 0`). DATA-compute writeback is `writeReg regs rd (dataResult instr rs1v
rs2v)` (writes to `x0` discarded by `writeReg`). `RDSR` is engine-handled: `sr =
0` reads `rxCrc` (zero-extended); any other `sr#` traps (reason 3).

### 6.2 Branch / PC math
`taken = branchTaken op rs1v rs2v`. `pc' = taken ? pc + signExt(off) : pc + 1`,
where `off :: BitVector 11` is interpreted as `Signed 11`, sign-extended to `aw`,
and added mod 2^`aw`. The offset is **relative to the branch instruction's own
PC** (the assembler's convention; ISA §6.1). `j off` = `beq x0,x0,off`.

### 6.3 Config / CRC control
`SET_CONFIG p` → `decodeConfig p`: `Right cfg'` sets `cfg`; `Left _` traps (reason
2). `CRC_RESET` sets `rxCrc := 0`. Both advance PC. Applied immediately; the host
issues them while CS# is deasserted (ISA §7.2).

### 6.4 Pin ops (single cycle, latch into `State`)
`CS_ASSERT` → `csN := 0`; `CS_DEASSERT` → `lanes := hiZ, csN := 1`; `RST_ASSERT` →
`rstN := 0`; `RST_DEASSERT` → `rstN := 1`; `GET_ALERT` → `regs := writeReg regs rd
(zeroExtend b)` where `b = if cfgAlertSource == AlertPin then alertIn else ioIn!!1`.
All advance PC.

### 6.5 WAIT_ON
`WAIT_ON rd cond timeout` arms `waitTimer := timeout` and enters `WaitAlert`. Each
`WaitAlert` cycle: if the alert is **asserted** (ALERT# / IO[1] driven **low**,
selected by `cfgAlertSource`) → `writeReg regs rd 1`, `pc+1`, `Fetch`; else if
`waitTimer == 0` → `writeReg regs rd 0`, `pc+1`, `Fetch`; else decrement
`waitTimer`. (`cond` selects the wait predicate; v1 recognises "ALERT# asserted",
other encodings reserved → trap.) `GET_ALERT` by contrast stores the **raw**
synchronized level (the host knows ALERT# is active-low).

## 7. Trace ring layout, REVISION, and the extended HALT record

### 7.1 Ring layout (D5)
Ring BRAM of `D` words (address `Unsigned 12`):

```
word[0]                 REVISION      [major8 | minor8 | patch16]  (written in Preamble)
word[1 .. termAddr-1]   record stream (ringPtr starts at 1, advances)
word[termAddr]          HALT terminator (fixed, overflow-proof)
```

`termAddr = D − 1`. The `limit` used by the fits/overflow check is `termAddr − 1`
(the last usable record slot), matching `ringPush`'s contract: a `count`-word
record starting at `ringPtr` fits iff `ringPtr + count − 1 ≤ limit`. On a miss:
`ovf := True`, drop the record (atomically, both MARK words), never write past
`limit`, never stall.

`REVISION` is a compile-time constant `revisionWord :: BitVector 32` =
`0x0001_0000` (major 0, minor 1, patch 0 — tracks `tamal.cabal` 0.1.0), letting
the host confirm the bitstream matches the CLI (ISA §8).

The engine does **not** call `ringPush` directly (it returns a `[BitVector 32]`,
not synthesizable per-cycle); it emits ≤ 1 word/cycle using the identical
fits/overflow logic. `ringPush`/`encodeRecord` remain the **reference model** for
the tests (§9).

### 7.2 Record encodings (must match `Tamal.Trace.encodeRecord`)
- **CAPTURE** (1 word): `[00 | 18'b0 | nbits(4) | byte(8)]`.
- **MARK** (2 words): `[10 | 16'b0 | label(14)]`, then `payload(32)`.
- **HALT** (1 word, **extended** — the D4 change): `[11 | 17'b0 | reason(3) |
  trap(1) | ovf(1) | status(8)]`.

`reason ∈ {0 = none, 1 = decode, 2 = config, 3 = rdsr, 4 = illegal/reserved-group}`;
`trap = reason ≠ 0`.

### 7.3 `Tamal.Trace` change required
Extend the HALT record so the engine can flag traps:

```haskell
-- before: Halt Bool (BitVector 8)                 -- overflow, status
-- after:  Halt Bool (BitVector 3) Bool (BitVector 8)  -- trap, reason, overflow, status
--         encodeRecord (Halt trap reason ovf st) =
--           [bitCoerce (0b11 :: BitVector 2, 0 :: BitVector 17,
--                       reason, pack trap, pack ovf, st)]
```

The old 21 reserved bits were zero, so this is a compatible refinement; its
hedgehog tests (`Test.Trace`) grow to cover `trap`/`reason`.

### 7.4 HALT / TRAP transitions
- **HALT s:** write terminator to `word[termAddr]` with `trap=0, reason=0,
  ovf=ovf, status=s`; `phase := Halted`.
- **TRAP:** drive pins safe (`csN:=1, lanes:=hiZ, sck:=0, rstN:=1`), write
  terminator to `word[termAddr]` with `trap=1, reason=code, ovf=ovf`; `phase :=
  Halted`.
- **Halted:** hold; assert `halted` so the top drains the ring. A `start` pulse
  re-inits (D9).

## 8. Start / lifecycle (D9)
`Idle` and `Halted` both respond to `startIn` by transitioning to `Preamble` with
a synchronous soft-init: `pc:=0, regs:=initRegs, cfg:=powerUpDefault, rxCrc:=0,
ringPtr:=1, ovf:=False`, pins safe, scratch cleared. `Preamble` writes
`revisionWord` to `word[0]` and enters `Fetch`. This yields byte-reproducible
re-runs with no reset port.

## 9. Testing (hedgehog baseline; ISA test-plan items 5 & 6)

New module `hdl/tests/Test/Engine.hs` (`tests :: TestTree`, SPDX header, added to
`unittests.hs`), reusing `Test.Gen`.

### 9.1 Harness — a pure-fold cosimulation driver
Because `step` is pure, no clocked simulation is needed. A driver folds `step`
over cycles:
- **Program memory:** a list of `encode`d instructions; `instrWord =
  mem[pcOut]`, modelling the 1-cycle BRAM latency the `Fetch` phase expects.
- **Scripted slave:** a per-cycle `ioIn!!1` source that presents a chosen byte at
  the sample phases (keyed off the engine's SCK schedule); also scripts `alertIn`
  and `startIn`.
- **Watchdog:** fail if `Halted` is not reached within N cycles (guards against
  non-terminating generated programs).

### 9.2 Reference interpreter
`refRun :: [Instr] -> RefResult` with instantaneous semantics (no timing),
producing expected `regs`, `rxCrc`, ordered ring records, and PC trace — built
from the already-tested leaves (`dataResult`, `branchTaken`, `readReg`/`writeReg`,
`crc8Update`, `encodeRecord`, `ringPush`). Engine tests assert the engine's
committed results equal `refRun`. **This checks composition / sequencing / timing
/ PC / lifecycle, not the leaves** (which are tested elsewhere).

### 9.3 Properties
1. **Datapath vs. reference** — random terminating DATA/CTRL programs → final
   `regs ≡ refRun`; `x0` stays 0.
2. **PC / branch math** — taken/not-taken advance; forward + backward offsets; a
   `j`-based loop terminates via a counter + HALT.
3. **PUT_BYTE timing** — `cs_assert; put_byte b; cs_deassert` emits exactly 8 SCK
   rising edges, IO0 = bits of `b` MSB-first at the drive phases, correct CS
   framing, SCK idle-low outside the op, 40 cycles/byte.
4. **GET_BYTE / GET_BITS** — `rd == b`; `rxCrc' ≡ crc8Update rxCrc b`; CAPTURE
   `(nbits, byte)` emitted; **GET_BITS leaves `rxCrc` unchanged** (CRC-neutral).
5. **CRC residue end-to-end** — GET a message + its correct CRC byte ⇒ `rdsr`
   residue `== 0`; corrupt ⇒ `≠ 0` (ISA §7.4).
6. **TAR** — clock 0 drives lanes high, clocks 1..n−1 hi-Z, `n` SCK cycles total;
   `n = 0` ⇒ no clocks.
7. **Trace / ring** — REVISION at `word[0]`; records ordered; overflow ⇒ sticky
   flag, writes stop, terminator preserved at `termAddr`; MARK's two words emit
   (and drop) atomically.
8. **TRAP** — reserved group / reserved-field word ⇒ terminator `trap=1` with the
   right `reason` (decode 1 / config 2 / rdsr 3 / illegal 4); safe pins asserted.
9. **HALT & lifecycle** — terminator carries `status`; `halted` asserted; Idle
   pins safe before `start`; **re-run determinism** — run → HALT → `start` → run
   yields byte-identical ring + regs.

### 9.4 Generator strategy
The hard part is generating *terminating* programs. Constrain to well-formed
sequences (balanced `cs_assert`/`cs_deassert`, bounded/forward-biased branches,
always HALT-terminated); the driver watchdog is the backstop.

## 10. Implementation approach (TDD, D10)
Ping-pong TDD with a division of labour:
- The **assistant writes the failing test** for the next slice (red), starting
  with the `Trace` HALT-record extension and the datapath, then bus timing, trace,
  and lifecycle — roughly the order of §9.3.
- The **author writes the Clash under `src/`** to pass it (green); the two
  **refactor together**. The assistant mentors on Clash idioms as needed.

Suggested slice order (each a red→green→refactor loop): (1) `Trace` HALT
extension; (2) `State`/`BusIn`/`BusOut`/`Ring` types + `initState` + `Idle`/`start`
soft-init + `Preamble` REVISION; (3) Fetch/Exec + DATA/RDSR datapath vs. `refRun`;
(4) branch/PC math; (5) pin ops + `SET_CONFIG`/`CRC_RESET`; (6) the SCK bus FSM
(PUT, then GET + CRC + CAPTURE, then TAR); (7) MARK/`TraceEmit` + overflow;
(8) `WAIT_ON`/`GET_ALERT`; (9) HALT + TRAP terminators; (10) re-run determinism.
The exact task list is the job of the follow-up implementation plan.

## 11. Module decomposition & files
- **New:** `hdl/src/Tamal/Engine.hs` (`step`, `State`, `Phase`, `Pending`,
  `BusIn`, `BusOut`, `Ring`, `initState`, `revisionWord`) — SPDX header, Clash
  ADT idioms (`Generic`/`NFDataX`).
- **Changed:** `hdl/src/Tamal/Trace.hs` (extend the HALT record, §7.3) and
  `hdl/tests/Test/Trace.hs`.
- **New:** `hdl/tests/Test/Engine.hs`; wire into `hdl/tests/unittests.hs`.
- **Unchanged leaves:** `Isa`, `Alu`, `Branch`, `RegFile`, `Bus.Serdes`, `Crc`,
  `Config`.

## 12. Out of scope (later specs)
- **The impure `topEntity` shell:** instruction BRAM + registered-PC read, ring
  BRAM, the UART load/drain FSM (control-plane load, HALT-triggered drain), the
  ALERT#/RESET# synchronizers, `IOBUF` tri-state instantiation, and pin/XDC
  wiring. This is the next document.
- **Target role, dual/quad I/O concrete lane maps, higher SCK rates, inline
  hardware verdict, subroutine linkage (`JAL`/`JALR`), live streaming trace** —
  all reserved by the ISA, unchanged here.
- The `tamal-abi` control/result wire format and the `tamal-asm`/`tamal-loader`
  host paths (separate specs).
