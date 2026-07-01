# Tamal — ISA & HDL Engine (Framework + Phase 1) Design

Date: 2026-07-01
Status: Approved (design); implementation not started
Scope: The tamal instruction set (framework + Phase-1 opcode set) and the Clash
HDL engine that decodes and executes it, with a hedgehog property-test baseline
and CI. Later-phase opcodes get reserved slots + placeholders only.

## 1. Purpose & the key reframing

Tamal's on-FPGA engine is a **programmable SPI shift engine that knows the eSPI
pins, not the eSPI protocol**. The host (`tamal-asm` + `tamal-loader`, Rust)
constructs *every* byte on the wire — CMD opcode, header, data, and CRC — and
hands the engine a flat byte stream. The engine's sole job is to shift those
bytes onto the bus at the right times, guarantee a host-specified turnaround
window (legal *or* deliberately illegal), and shift response bits back into
bytes.

The eSPI "Command → TAR → Response" structure (spec §4.1), the CMD/HDR/DATA/CRC
decomposition, the four channels, and every message type live entirely in host
software. In the engine, a transaction is nothing more than:

```
PUT_BYTE×k  ;  TAR n  ;  GET_BYTE×m
```

The engine understands: chip-select framing, SCK generation, MSB-first
shift-out/shift-in over a configurable IO width (x1/x2/x4), a host-parameterized
turnaround, the two sideband pins **ALERT#** and **RESET#**, a reception CRC-8
accumulator (the single eSPI-flavored concession — see §7), and just enough
compute/control to run reactive loops (WAIT_STATE polling, alert waiting).

This mirrors [`mole`](https://github.com/felipebalbi/mole): a bus-agnostic
Layer-0 engine driven by a protocol-aware Layer-1 host.

## 2. Layering & the three planes

- **Layer 0 — the SPI shift engine** (this HDL, this ISA). Knows SPI + the eSPI
  pins/electrical behaviour. **Zero eSPI protocol knowledge.**
- **Layer 1 — the host** (`tamal-asm` + `tamal-loader`). Owns all eSPI
  semantics: builds every byte incl. CRC, chooses packet structure, requests
  legal/illegal TAR widths, injects errors at compile time, and turns the
  captured transcript into a pass/fail verdict.

Three planes (per `AGENTS.md`):

- **Control** (host → FPGA): load bytecode into instruction BRAM, trigger.
  This is the `tamal-abi` control wire format — *not* the ISA.
- **Bus** (FPGA ↔ DUT): `CS#`, `CLK`, `IO[3:0]`, `ALERT#`, `RESET#` — all live
  in v1.
- **Trace** (FPGA → host): the BRAM result-ring, drained on HALT (§8). Never
  block the bus on trace backpressure — drop with an overflow marker.

## 3. Design decisions (from brainstorming)

| # | Decision | Rationale |
|---|----------|-----------|
| 1 | **RISC-V assembly surface, mole-style binary core.** Keep x0–x31 names, 32-bit words, ABI names, directives, pseudo-ops, numeric labels; implement a lean purpose-built core (bus/control/data opcode groups), not a full RV32I ALU. | Honors README/AGENTS "inspired by, not compatible with"; matches mole's lean bus engine. |
| 2 | **Scope = framework + Phase 1.** Controller role, single I/O, host-owned CRC on TX. Later phases → reserved opcode slots + placeholders. | One implementable spec; avoids over-designing unbuilt hardware. |
| 3 | **Byte-oriented bus ops + a bit-level escape.** Engine owns clock gen, lane mapping, MSB-first serialization, TAR. **Host owns CRC** (see 4). | Compact programs; hard timing lives in tested hardware; bit escape preserves malformed-cycle authoring. |
| 4 | **TX CRC host-owned; RX CRC engine-verified.** Host supplies the TX CRC byte (enables compliant-packet-with-bad-CRC tests). Engine keeps an RX CRC-8 accumulator so programs can "branch if wrong CRC." | A compliant packet with a deliberately-bad CRC is a first-class link-layer test; reactive RX verification needs a hardware residue. |
| 5 | **Host-side verdict; reserve inline-verdict flag bits.** Reactive compare+branch (needed for WAIT_STATE) via ordinary DATA/CTRL ops; pass/fail from the transcript on the host. Reserve encoding bits for a future inline expect/mask verdict (Phase 4). | Lean now, no re-encode later. |
| 6 | **20 MHz SCK only in v1** (100 MHz / 5). `sck` is a config enum (20/33/50/66) with only 20 live; others → `TRAP`. | Simplest clean divider; higher rates drop in later without ISA change. |
| 7 | **16 physical registers** (x0–x15, x0=0); encoding reserves RISC-V 5-bit register fields. | Comfortable headroom for host codegen, tiny in fabric, grows to 32 with no encoding change. |
| 8 | **BRAM result-ring, drained on HALT**, sticky overflow marker. | Deterministic and simple for short Phase-1 conformance programs. |
| 9 | **CI covers HDL + Rust.** Hedgehog is the verification baseline; no Vivado in CI. | Covers the whole repo's untested code. |

## 4. Instruction word — 32-bit fixed width, little-endian on the wire

```
 31  30 29     26 25   21 20   16 15   11 10                     0
┌──────┬─────────┬───────┬───────┬───────┬───────────────────────┐
│group │   sub   │  rd   │  rs1  │  rs2  │     imm / payload      │
│ (2)  │   (4)   │  (5)  │  (5)  │  (5)  │         (11)           │
└──────┴─────────┴───────┴───────┴───────┴───────────────────────┘
```

- **group** `[31:30]`: `00`=BUS, `01`=CTRL, `10`=DATA, `11`=reserved (execute →
  `TRAP`).
- **sub** `[29:26]`: 16 sub-opcodes per group.
- **rd / rs1 / rs2** `[25:11]`: RISC-V-standard **5-bit** register selectors
  (encode x0–x31; **x0–x15 live in v1**, x0 hardwired 0; x16–x31 rejected by
  assembler and engine).
- **imm / payload** `[10:0]`: immediate, branch offset, TAR count, bit-count, or
  opcode-specific. Big 32-bit constants (rare — the host builds packet bytes)
  come from a `li` pseudo-op expanding to `LUI` + `ADDI` (standard `%hi`/`%lo`
  carry adjustment; the 11-bit immediate leaves a residual reachability gap that
  `tamal-asm` covers with a 3-instruction sequence — see the ALU/branch design).

Fields unused by an opcode are **reserved-must-be-zero**; a non-zero reserved
field → `TRAP`. This gives forward room and catches malformed bytecode.

**Register / branch model:** 16 registers x0..x15 (32-bit), x0 = 0. Branches
compare two registers directly (RISC-V style, no condition-flag register).

## 5. BUS group (`group = 00`) — SPI signalling + sidebands

The only opcodes that touch pins. The engine reads the current `io_mode`/`sck`
config and owns SCK generation, MSB-first shifting, lane mapping, and tri-state.

### 5.1 SPI timing model (mode 0)
eSPI is **SPI mode 0**: SCK idles low; the controller **changes data on the
falling edge** and the slave **samples on the rising edge** (and vice-versa for
GET). v1 SCK = 20 MHz = 100 MHz **/5**; a phase counter times the edges (a clean
~50% duty may use an MMCM-derived internal clock — an implementation detail
outside the ISA). Between BUS ops (CS# still asserted) SCK holds idle-low, so
programs keep transfer ops contiguous within a transaction.

### 5.2 Per-lane electrical model (the pure/impure seam)
Each lane `l ∈ {0,1,2,3}` is `(o[l] : Bit, oe[l] : Bit)` — output value +
output-enable. The pure engine computes `(o, oe)` per SCK phase and consumes
sampled inputs; the top entity wires `oe → IOBUF.T`, `o → IOBUF.I`,
`IOBUF.O → sampler`. `CS#`, `CLK`, `RESET#` are plain controller outputs;
`ALERT#` is a synchronized input.

### 5.3 BUS sub-opcodes (v1)

| sub | Mnemonic | Operands | Effect |
|----|----------|----------|--------|
| `0x0` | `CS_ASSERT` | — | Drive `CS#` low; SCK idle-low. Begins a frame. |
| `0x1` | `CS_DEASSERT` | — | Tri-state IO drivers; drive `CS#` high. Ends a frame. |
| `0x2` | `PUT_BYTE` (imm) | `imm[7:0]` | Shift 8 bits **MSB-first** over the active IO width. |
| `0x3` | `PUT_BYTE` (reg) | `rs1` | Shift `rs1[7:0]` MSB-first. |
| `0x4` | `GET_BYTE` | `rd` | Sample 8 bits MSB-first → `rd` **and** a CAPTURE ring record; **auto-updates RX CRC-8**. |
| `0x5` | `PUT_BITS` (imm) | `imm[10:8]=n−1`, `imm[7:0]=bits` | Shift `n∈1..8` bits (left-justified). Sub-byte / malformed-field escape. |
| `0x6` | `PUT_BITS` (reg) | `rs1`, `imm[10:8]=n−1` | Shift `n` bits from `rs1`. |
| `0x7` | `GET_BITS` | `rd`, `imm[10:8]=n−1` | Sample `n` bits → `rd` + ring. **CRC-neutral** (does not update RX CRC). |
| `0x8` | `TAR` (imm) | `imm[3:0]=n` | Turnaround **n** clocks (§5.5). Host picks `n`. |
| `0x9` | `TAR` (reg) | `rs1` | Turnaround `rs1` clocks. |
| `0xA` | `RST_ASSERT` | — | Drive `RESET#` asserted (low), async. |
| `0xB` | `RST_DEASSERT` | — | Drive `RESET#` deasserted (high). |
| `0xC` | `GET_ALERT` | `rd` | Sample synchronized `ALERT#` (or `IO[1]` per `alert_source`) → `rd[0]`. |
| `0xD`–`0xF` | reserved | — | `TRAP`. |

The reserved low `imm` bits on `GET_*`/`PUT_*` are the placeholder for the future
inline expect/mask/verdict flag (decision 5).

### 5.4 PUT / GET mechanics (v1, x1 single I/O)
- **PUT_BYTE/BITS**: drive the data bit on **IO[0]** while SCK low → rising edge
  (slave samples) → falling edge (next bit). `IO[1..3]` tri-stated. 8 clocks/byte
  in x1.
- **GET_BYTE/BITS**: engine tri-states **all** its IO drivers; each clock, the
  rising edge samples **IO[1]** (MISO), accumulate MSB-first. 8 clocks/byte in x1.
- `io_mode = x2/x4` makes PUT/GET width-agnostic (4 / 2 clocks per byte) using the
  standard multi-lane bit→lane map; concrete x2/x4 maps land with Phase 3.
  **v1 is x1 only**; x2/x4 config values → `TRAP`.

### 5.5 TAR mechanics (host-controlled, legal *or* illegal)
`TAR n` clocks `n` SCK cycles:
- **cycle 0**: drive **all** active IO lanes to logic `1` with `oe=1` (eSPI's
  "drive high for the first TAR clock");
- **cycles 1..n−1**: `oe=0` on all lanes (tri-stated; weak pull-ups hold high),
  handing the bus to the slave.

`n=2` is the legal eSPI TAR. The host may request `n≥3` (over-long) or `n=1`/`n=0`
(too-short) to deliberately violate turnaround — a first-class compliance test,
since the host owns `n`.

## 6. CTRL group (`group = 01`) and DATA group (`group = 10`)

Enough compute/control for reactive loops while keeping the RISC-V surface.

### 6.1 CTRL group

| sub | Mnemonic | Operands | Effect |
|----|----------|----------|--------|
| `0x0` | `HALT` | `imm[7:0]`=status | End program, write HALT terminator (status) to ring, trigger drain. |
| `0x1` | `BEQ` | `rs1,rs2,off` | PC += off if `rs1==rs2`. |
| `0x2` | `BNE` | `rs1,rs2,off` | PC += off if `rs1!=rs2`. |
| `0x3` | `BLTU` | `rs1,rs2,off` | Unsigned `<`. |
| `0x4` | `BGEU` | `rs1,rs2,off` | Unsigned `>=`. |
| `0x5` | `WAIT_ON` | `rd,cond,imm`=timeout | Block until `cond` (ALERT# asserted) or timeout; `rd ← 1` met / `0` timed out. |
| `0x6` | `SET_CONFIG` | packed | Set `role / io_mode / sck / alert_source` (§7); unimplemented value → `TRAP`. |
| `0x7` | `MARK` | `imm`=label, `rs1`=payload | Write a MARK record (label + reg payload) to the ring for host↔trace correlation. |
| `0x8` | `CRC_RESET` | — | Reset RX CRC-8 accumulator to `0x00`. |
| `0x9`–`0xF` | reserved | — | `TRAP`. |

`off` = signed `imm[10:0]` (word-aligned, ±1024 instr; extension bits reserved).
`j off` is a pseudo-op for `beq x0,x0,off`. `call`/`ret` (JAL/JALR) reserved for
later — v1 programs are branch-structured.

### 6.2 DATA group

| sub | Mnemonic | Form | Effect |
|----|----------|------|--------|
| `0x0` | `LOAD_IMM` | `rd, imm` | `rd ← sext(imm[10:0])`. |
| `0x1` | `LUI` | `rd, imm20` | `rd ← imm << 12` (pairs with `ADDI` for 32-bit consts). |
| `0x2` | `MOV` | `rd, rs1` | `rd ← rs1`. |
| `0x3`/`0x4` | `ADD`/`ADDI` | rr / ri | add. |
| `0x5` | `SUB` | rr | subtract (`DEC` = `ADDI rd,rd,-1`). |
| `0x6`/`0x7` | `AND`/`ANDI` | rr / ri | mask sampled bytes, etc. |
| `0x8`/`0x9` | `OR`/`ORI` | rr / ri | |
| `0xA`/`0xB` | `XOR`/`XORI` | rr / ri | |
| `0xC` | `SHIFT` | `rd,rs1,{dir,arith,amt}` | `SLL`/`SRL`/`SRA`. |
| `0xD` | `RDSR` | `rd, sr#` | Read special register → `rd`. **`sr=0` = RX CRC-8**; other sr# reserved. |
| `0xE`–`0xF` | reserved | — | `TRAP`. |

### 6.3 RISC-V assembly surface (`tamal-asm`, host-side)
The lean core is what fabric decodes; the assembler exposes familiar sugar:

- **Pseudo-ops**: `nop`=`addi x0,x0,0`, `mv`=`addi rd,rs,0`, `li`=`lui`+`addi`,
  `j off`=`beq x0,x0,off`, `beqz/bnez rs`=`beq/bne rs,x0`. `call/ret` reserved.
- **Directives / labels**: `.text .data .word .globl .equ .align .macro .option`,
  numeric locals `1f`/`1b` — all assembler-side; the HDL only sees 32-bit words.
- **ABI names**: `zero`(x0), `ra`(x1), `sp`(x2)… onto x0–x15; x16–x31 reject in v1.

## 7. Engine state, config, special registers, and CRC

### 7.1 Engine state (power-up `init`, no reset port)
`PC` · register file x0..x15 (x0=0) · config register · RX CRC-8 accumulator ·
ring write-pointer + sticky overflow flag · SCK phase counter / bus-FSM state ·
`ALERT#` synchronizer (inbound `RESET#` synchronizer reserved for target role).

Per `AGENTS.md`, the top entity ties reset permanently de-asserted; all state
relies on power-up `init`.

### 7.2 `SET_CONFIG` fields (slow state; set while CS# deasserted, applies to later frames)

| Field | Bits | v1 live | Reserved (→`TRAP`) |
|---|---|---|---|
| `role` | 1 | `0`=controller | `1`=target |
| `io_mode` | 2 | `0`=x1 | `1`=x2, `2`=x4 |
| `sck` | 2 | `0`=20 MHz | `1`=33, `2`=50, `3`=66 |
| `alert_source` | 1 | `0`=ALERT# pin, `1`=IO[1] | — |

Power-up default = controller / x1 / 20 MHz / ALERT# pin.

### 7.3 Special registers (via `RDSR`)
`sr=0` RX CRC-8 accumulator (8 bits, zero-extended). `sr=1..` reserved (cycle
counter, engine status, revision).

### 7.4 CRC contract (asymmetric)
- **TX: host-owned.** The host supplies the CRC byte via `PUT_BYTE`; there is no
  TX CRC engine. Deliberately-wrong TX CRC is trivial to author.
- **RX: engine-verified.** A fixed **CRC-8** accumulator (poly `0x07`, init
  `0x00`, MSB-first — eSPI/SMBus) is auto-updated by every `GET_BYTE`. This is the
  engine's *only* eSPI-flavored primitive; `GET_BITS` (the escape) is CRC-neutral.
- **WAIT_STATE exclusion.** Since the engine cannot know what a WAIT_STATE byte
  is, the host excludes them with a reset-at-top-of-poll idiom (§9).
- **Residue verification.** For CRC-8/`0x07`, init `0x00`, no reflection/xorout,
  feeding the message *and* its trailing CRC byte yields `0x00` exactly when
  correct. So "branch if wrong CRC" is: feed the received CRC byte via a final
  `GET_BYTE`, `rdsr` the residue, then ordinary `bne rd, x0, fail`.

## 8. Result-ring / trace format

Fixed-size BRAM ring of 32-bit words, drained little-endian over the transport on
HALT.

```
word[0]                    REVISION   [31:24]=major [23:16]=minor [15:0]=patch
word[1 .. limit-1]         record stream
word[limit] (last, fixed)  HALT terminator (overflow-proof reserved slot)
```

| Record | Tag `[31:30]` | Words | Payload |
|---|---|---|---|
| CAPTURE | `00` | 1 | `[11:8]`=nbits(1–8), `[7:0]`=sampled byte (from `GET_BYTE`/`GET_BITS`) |
| MARK | `10` | 2 | w0 `[13:0]`=label; w1 = `rs1` payload |
| HALT | `11` | 1 | `[8]`=overflow, `[7:0]`=status |

If the ring fills: set sticky **overflow**, drop further records, **never stall
the bus**. The HALT slot is written at a fixed address so an overflowing record
stream cannot clobber the terminator. `REVISION` is written at program start and
lets the host confirm the bitstream matches the CLI it was built against.

## 9. Worked example — reactive Get-Status-shaped transaction

Every byte is host-generated; the engine sees only CS/PUT/TAR/GET/branch.

```asm
    set_config CONTROLLER, X1, SCK20, ALERT_PIN  ; controller, single-IO, 20 MHz
    cs_assert
    put_byte 0x25                            ; host-built CMD byte (GET_STATUS-shaped)
    put_byte 0x7A                            ; host-built (possibly wrong) TX CRC
    tar 2                                     ; legal turnaround; `tar 3` = illegal
poll:
    crc_reset                                ; drop prior WAIT_STATE byte from CRC
    get_byte x5                              ; sample + auto-update RX CRC
    li x6, 0x0F                              ; WAIT_STATE response code
    beq x5, x6, poll                         ; WAIT_STATE -> keep polling
    ; loop exit: RX CRC == crc8(RSP opcode); WAIT_STATE bytes discarded by resets
    get_byte x5                              ; STATUS lo   (auto-CRC)
    get_byte x5                              ; STATUS hi   (auto-CRC)
    get_byte x5                              ; received CRC byte -> residue
    rdsr x7, CRC                             ; read RX CRC-8 residue
    bne  x7, x0, bad_crc                     ; "branch if wrong CRC"
    cs_deassert
    halt 0x00                                ; OK
bad_crc:
    cs_deassert
    halt 0x11                                ; host-defined "CRC mismatch" verdict code
```

`0x25`, `0x0F`, the CRC bytes, and the packet shape are all host knowledge; the
engine remains eSPI-ignorant.

## 10. HDL module decomposition (Clash)

Pure cores (property-tested) with a thin impure shell.

```
src/Tamal/
  Isa.hs         Instr ADT + decode/encode + reserved-field detection   [pure]
  Crc.hs         crc8Update / crc8  (poly 0x07, init 0x00, MSB-first)    [pure]
  Bus/Serdes.hs  byte<->lane serialize/deserialize per io_mode+dir; TAR  [pure]
  Config.hs      config register type + decode                          [pure]
  Engine.hs      step :: State -> BusIn -> (State, BusOut, Maybe Ring)   [pure Mealy]
  Trace.hs       ring-record encode + overflow logic                    [pure]
  Domain.hs      Dom100 (exists)
Tamal.hs         topEntity: instr-BRAM, ring-BRAM, UART load/drain FSM,
                 SCK/edge gen, IOBUF tri-state wiring around Engine.step [impure shell]
```

`Engine.step` is a pure transition; `topEntity` lifts it with `mealy`/`register`.
The only non-property-tested logic is SCK edge timing, the `IOBUF` tri-states,
and the UART FSM.

**Clash idioms (adopted from Lion, see §12):** model the ISA as synthesizable
ADTs (`data Instr = … deriving stock (Generic, Show, Eq) deriving anyclass
NFDataX`); `decode :: BitVector 32 -> Either Trap Instr` is total; `branch ::
Branch -> BitVector 32 -> BitVector 32 -> Bool` (with `sign = unpack :: BitVector
32 -> Signed 32`) and the `Op`-dispatched ALU are small, total, independently
testable pure functions; named-port record idiom (`"clk" ::: …`) at the top.

## 11. Testing (hedgehog baseline) & CI

### 11.1 Hedgehog property-test plan
`tests/` uses tasty + tasty-hedgehog + hedgehog + clash-prelude-hedgehog
(`clash-prelude-hedgehog` is already pinned in `hdl/stack.yaml`; add `hedgehog`
and `tasty-hedgehog` to the `tamal.cabal` test-suite).

1. **ISA round-trip**: `decode (encode i) ≡ Right i`; random 32-bit words decode
   to a valid `Instr` or a `Trap`; reserved-nonzero fields ⇒ trap.
2. **CRC-8**: matches a reference impl; residue law `crc8 (msg ++ [crc8 msg]) ≡
   0`; known eSPI/SMBus vectors.
3. **Serdes round-trip**: `deserialize m (serialize m b) ≡ b`; x1 emits 8
   lane-vectors MSB-first on IO[0]; lane/bit ordering correct.
4. **TAR**: `tar n` ⇒ cycle 0 drives active lanes high (`oe=1,o=1`), cycles
   `1..n-1` `oe=0`, total length `n`.
5. **Engine step**: PUT_BYTE = 8 SCK cycles driving the byte MSB-first; GET_BYTE
   samples MSB-first and `crc' ≡ crc8Update crc b`; CS/SCK idle behaviour;
   `CRC_RESET`→0; branches update PC; `x0` stays 0; reserved opcode → `TRAP`;
   HALT writes the terminator.
6. **Ring**: record↔word round-trip; overflow sets the flag, stops writes, and
   preserves the HALT slot.

### 11.2 CI (`.github/workflows/ci.yml`, ubuntu, no Vivado)
- **hdl job** (`working-directory: hdl`): setup stack/GHC → cache `~/.stack` +
  `hdl/.stack-work` (keyed on `stack.yaml.lock`) → `stack build` → `stack test`
  (hedgehog) → `stack run clash -- Tamal --verilog` (codegen smoke). Cold
  GHC/Clash build is slow, so caching is load-bearing.
- **rust job**: `dtolnay/rust-toolchain` (stable + clippy + rustfmt) →
  `Swatinem/rust-cache` → `cargo build --workspace` → `cargo test --workspace` →
  `cargo clippy --workspace --all-targets -- -D warnings` →
  `cargo fmt --all --check`.

## 12. Prior art & inspiration

- **[mole](https://github.com/felipebalbi/mole)** — the sibling I2C/I3C rig.
  Tamal borrows its architecture (bus-agnostic Layer-0 engine + protocol-aware
  Layer-1 host), its opcode-group ISA shape, its per-op capture flags, the
  result-ring-drained-on-halt trace model, and the **compile-time-only
  error-injection contract** (the engine has zero runtime randomness; injected
  errors are host encoder choices; `ratio = 0` is byte-identical to no
  injection).
- **[Lion](https://github.com/standardsemiconductor/lion)** — a formally-verified
  RV32I in Clash. Tamal borrows its Clash idioms: synthesizable instruction ADTs
  with `Generic`/`NFDataX`, a total `decode → Either Exception`, pure
  `branch`/`alu` functions, and the named-port record style. It does **not**
  use `riscv-formal`/RVFI (tamal diverges from RV32I, so RV conformance checking
  cannot be pointed at it) — hedgehog property tests are tamal's v1 verification
  baseline. Lion does, however, demonstrate the *technique* recorded as future
  work below.

## 13. Out of scope (later plans)

- **Target role** (external clock; setup/hold/CDC against an externally-driven
  SCK), inbound `RESET#` observation.
- **Dual/quad I/O** concrete lane maps (x2/x4 config values `TRAP` in v1).
- **Higher SCK rates** (33/50/66 MHz).
- **Inline hardware verdict** (expect/mask flag bits are reserved but unused).
- **Richer timing knobs** (configurable setup/hold/TAR-width/inter-byte gap for
  deliberate timing violations).
- **Subroutine linkage** (`JAL`/`JALR`, `call`/`ret`).
- **Live streaming trace** (v1 is ring-drained-on-HALT).
- **Formal verification** — an optional post-v1 SymbiYosys/BMC harness on the
  Clash-generated Verilog, driven by *tamal-specific* properties (e.g.
  "`PUT_BYTE` emits exactly 8 SCK edges," "reserved fields ⇒ `TRAP`"), layered on
  top of hedgehog (Lion-style technique, not `riscv-formal`).
- The `tamal-abi` control/result wire format, the real `tamal-asm`
  lexer/parser/encoder, and the `tamal-loader` transport (separate specs).
