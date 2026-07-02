# Tamal — Loader (UART load/drain FSM) Design

Date: 2026-07-02
Status: Approved (design); implementation not started
Scope: The impure **loader**: the on-FPGA finite-state machine that bridges the
UART (`Tamal.Uart`) ↔ the two block RAMs (`Tamal.Mem`) ↔ the engine's control
ports (`startIn`/`haltedOut`). It is the **streaming realization** of the pure
`Tamal.Wire` reference model — parsing control frames into instruction-BRAM
writes + a `startIn` pulse (the *load* path) and, on HALT, sweeping the trace
ring into one `TRACE_DRAIN` byte frame over the UART (the *drain* path). This is
**piece 3** of the impure-shell roadmap in `hdl/PLAN.md` (build order: BRAM →
wire protocol → **loader** → IOBUF → topEntity).

This is **HDL-only**. The Rust `crates/tamal-abi` mirror of the wire format and
the host `tamal-loader` transport are deferred until the gateware is validated in
silicon; they implement the same contract later.

Companion to the wire-format design
(`docs/superpowers/specs/2026-07-02-tamal-wire-format-design.md`, esp. §8 —
frame layouts — and §10 — the loader relationship), the Engine design
(`.../2026-07-02-tamal-engine-design.md`, esp. §7 — REVISION / records / HALT
terminator — and §8 — start/soft-init), the BRAM design
(`.../2026-07-02-tamal-bram-design.md`, esp. §6 — the latency & port-ownership
contract), and the UART design (`.../2026-07-01-tamal-uart-design.md`, esp. §9 —
the byte-stream transport). Roadmap context in `hdl/PLAN.md` (piece 3).

---

## 1. Purpose & role

The pure `Tamal.Wire` core defined the *contract* — control and result frames as
byte **lists** (`encodeControl`/`decodeControl`, `encodeResult`/`decodeResult`),
a reference model. The loader is the **impure, streaming** half: it consumes and
produces those exact bytes one UART strobe at a time, wired to real memories and
the running engine. It reproduces the pure model **byte for byte** — the same
"pure list model, streaming impl matches it" discipline `Engine` uses for
`ringPush` (Engine design §7.1).

Three responsibilities:

1. **Load** (host → FPGA): stream-decode `LOAD_PROGRAM(words…)` frames and write
   each little-endian word into the instruction BRAM.
2. **Trigger** (host → FPGA): on a valid `TRIGGER` frame, pulse the engine's
   `startIn` for one cycle.
3. **Drain** (FPGA → host): when the engine reaches `Halted`, sweep the trace
   ring (`word[0]` REVISION → `word[1..ringPtr−1]` records → `word[termAddr]`
   HALT terminator) and emit it as one `TRACE_DRAIN` frame over the UART.

The loader owns **no eSPI semantics** — it is a transport bridge. All bus meaning
lives in the engine (upstream) and the host (downstream).

## 2. Scope & non-goals

**In scope**

- `Tamal.Loader`: the lifecycle FSM (`RxControl` → `Run` → `Drain`) plus the
  wiring of UART, both BRAM ports it owns, and the engine's `startIn`/`haltedOut`.
- `Tamal.Loader.Cobs`: the **streaming** COBS decode + encode blocks (the
  isolated leaf), each property-tested against the pure `Tamal.Wire.Cobs` oracle.
- The frame/message streaming layer (in the FSM): incremental CRC-8, the
  one-byte holdback that separates the trailing CRC from payload, LE
  word (dis)assembly, opcode dispatch, the drain generator.
- One **engine change**: expose `ringPtr` as `BusOut.ringPtrOut` (a pure
  projection — §4).
- `Test.Loader` (hedgehog + HUnit, Signal-level) and cabal/runner wiring.

**Out of scope (deferred)**

- **IOBUF** (piece 4) and **`topEntity`** (piece 5): the loader is *instantiated*
  by the top but does not wire the eSPI pins itself.
- The Rust **`crates/tamal-abi`** mirror and the **`tamal-loader`** host transport
  (post-silicon).
- **ACK/NAK / handshaking / retransmit protocol** on the FPGA side — v1 is
  fire-and-forget (wire-format D4); the loader silently discards bad frames.
- **UART flow control** — the Arty's FTDI has no RTS/CTS wired (§10); none is
  needed.
- **Live streaming trace** — v1 drains once, on HALT.
- Choosing the **baud rate** and instantiating the UART — that is the top's job
  (`SNat @2_000_000`); the loader is baud-agnostic.

## 3. Design decisions

| # | Decision | Rationale |
|---|----------|-----------|
| D1 | **Expose `ringPtr` from the engine** as a new `BusOut.ringPtrOut :: Unsigned 12` (pure projection; §4). The loader reads it to bound the drain. | The drain must stream exactly `word[0..ringPtr−1]` + the terminator, and the loader cannot recover `ringPtr` by scanning: the ring BRAM is **not** cleared on soft-init (stale records from a prior run survive in `[ringPtr..termAddr−1]`) and legitimately-zero record words exist (`captureWord 0 0 = 0x0`), so there is no reliable end marker. Exposing existing authoritative state is non-behavioral (`step` and its 92 properties unchanged) and `Test.Engine` uses only accessors, so it keeps compiling. Also the value future live-trace-streaming needs. |
| D2 | **Lifecycle FSM (`Tamal.Loader`) + isolated streaming-COBS codec (`Tamal.Loader.Cobs`).** One FSM, not an RX/TX split, because the load and drain paths are **temporally disjoint** (load precedes the run; drain follows HALT) and share no live state. | Isolates the one genuinely hard algorithm (streaming COBS) for its own strong round-trip property against the pure oracle — mirroring the piece-2 `Cobs`/`Wire` split (wire-format D7). A single 3-state FSM is simpler than a sequencer arbitrating two sub-FSMs that never run at once. |
| D3 | **Buffered COBS encoder** — a ≤254-byte group look-ahead buffer inside the codec — **not** a two-pass BRAM re-read. | Keeps the codec a clean, isolated `byte-in → byte-out` stream transformer (property-testable in isolation). A two-pass re-read would entangle the encoder with the ring-BRAM source and its addressing. The code byte inherently precedes its ≤254 data bytes, so *some* group buffer is unavoidable; 254 bytes is trivial on Artix-7. |
| D4 | **Write-through load + one-byte holdback for CRC; the commit is the separate `TRIGGER`.** Words stream into the instr BRAM as received; a failed frame is simply not re-loaded. | Buffering up to 1024 words to commit atomically is impractical (4 KB — a second BRAM). `LOAD_PROGRAM` and `TRIGGER` are **separate frames** (wire-format §8.2), so a bad `LOAD` is overwritten by the host's retry before its `TRIGGER`; garbage never runs in a well-behaved fire-and-forget flow (D4 of the wire format). The holdback cleanly separates the trailing CRC byte from payload in a stream. |
| D5 | **No UART flow control** (§10). Rely on fire-and-forget + whole-frame CRC + COBS resync + host timeout/re-run to tolerate byte loss on the unpaced link. | **The Arty's FTDI USB-UART has no RTS/CTS wired** (a board fact). The FPGA side cannot overrun (per-byte RX work is O(1) cycles ≪ the ~500-cycle byte period at 2 Mbaud/100 MHz; the drain self-paces off `txReady`). Host-side RX overrun on the drain is absorbed by FTDI/OS buffering and, ultimately, the CRC+re-run backstop. Baud is a top-level `SNat` knob if it ever bites — no protocol change. |
| D6 | **Ping-pong TDD**: the assistant writes the failing tests under `tests/`; the author writes the Clash under `src/`; refactor together. | Continues the deliberate Clash learning exercise (wire-format D9, engine D10). |

## 4. The engine change (`ringPtr` exposure)

The **only** change to the tested engine is a pure output projection — no edit to
`step`, no state change, no behavioral difference:

```haskell
data BusOut = BusOut
  { pcOut      :: Unsigned AW
  , csOut      :: Bit
  , sckOut     :: Bit
  , rstOut     :: Bit
  , lanesOut   :: Lanes
  , haltedOut  :: Bool
  , ringPtrOut :: Unsigned 12   -- NEW: next-record slot; the drain's upper bound
  }

busOut s = BusOut
  { …
  , ringPtrOut = ringPtr s      -- NEW: one line
  }
```

`ringPtr` is frozen once the engine is `Halted` (no `Halted` transition touches
it), so the loader can read `ringPtrOut` at drain time without latching it on the
`halted` edge. `Test.Engine` accesses `BusOut` only through field accessors
(`pcOut bo`, `sckOut bo`, …) and never constructs or exhaustively matches a
`BusOut` literal, so adding a field leaves every existing engine test compiling
and passing unchanged.

## 5. Interface

The loader is one impure Signal block. Inputs/outputs are grouped into records
(house style, mirroring `BusIn`/`BusOut`); field names are chosen to avoid
record-field clashes with `BusIn`/`BusOut`/`Ring` when the topEntity imports them
together.

```haskell
data LoaderIn = LoaderIn
  { rxByte    :: Maybe (BitVector 8)   -- from UART Rx (one-cycle strobe)
  , txReady   :: Bool                  -- from UART Tx (high when idle/ready)
  , halted    :: Bool                  -- engine BusOut.haltedOut
  , ringPtrIn :: Unsigned 12           -- engine BusOut.ringPtrOut (§4)
  , ringData  :: BitVector 32          -- ring-BRAM read data
  }

data LoaderOut = LoaderOut
  { txByte   :: Maybe (BitVector 8)               -- to UART Tx
  , instrWr  :: Maybe (Unsigned 10, BitVector 32) -- instr-BRAM write port
  , ringAddr :: Unsigned 12                       -- ring-BRAM read address
  , startOut :: Bool                              -- to engine BusIn.startIn
  }

loader :: (HiddenClockResetEnable dom) => Signal dom LoaderIn -> Signal dom LoaderOut
```

**Port ownership** (BRAM design §6 — collision-free by construction):

| Port | Driven by | Read by |
|---|---|---|
| instr-BRAM write (`instrWr`) | **loader** | — |
| instr-BRAM read addr (`pcOut`) | engine | instr BRAM |
| ring-BRAM write (`Maybe Ring`) | engine | ring BRAM |
| ring-BRAM read addr (`ringAddr`) | **loader** | ring BRAM |

The loader never drives the ring **write** port and never reads the instr
**read** port; load-time writes and drain-time reads never overlap the engine's
run-time accesses (the loader writes the instr BRAM only in `RxControl`, reads
the ring BRAM only in `Drain`). RX framing errors (`rxErr` from the UART) are
**ignored** — a corrupted byte surfaces downstream as a COBS or CRC failure and
is discarded (D4/D5).

## 6. Lifecycle FSM

Three states (a `mealyS` State-monad transition — the idiom `hdl/PLAN.md` flagged
for exactly this long sequential FSM):

```
        valid TRIGGER frame            halted (level)     drain complete
  ┌───────────────┐  startIn pulse   ┌──────┐          ┌────────┐
  │   RxControl   │ ───────────────► │ Run  │ ───────► │ Drain  │ ──┐
  │ (listen for   │                  └──────┘          └────────┘   │
  │  control      │ ◄─────────────────────────────────────────────┘
  │  frames)      │   valid LOAD_PROGRAM frame → words in BRAM, stay in RxControl
  └───────────────┘
```

- **`RxControl`** — the only state that consumes RX. Stream-decode each incoming
  frame (§7). A valid `LOAD_PROGRAM` leaves its words in the instr BRAM and stays
  here. A valid `TRIGGER` pulses `startOut` for one cycle and advances to `Run`.
  Malformed / bad-CRC / unknown frames have no effect.
- **`Run`** — the engine executes. The loader drives `instrWr = Nothing`,
  `txByte = Nothing`, `startOut = False`, and ignores RX. It watches `halted` as
  a **level** (the `Halted` phase is stable). On `halted` → `Drain`.
- **`Drain`** — sweep the ring and emit one `TRACE_DRAIN` frame (§8). On
  completion → `RxControl` (the rig is re-runnable: a later `TRIGGER` re-runs the
  same program; a later `LOAD_PROGRAM` reloads it).

`startOut` is a **one-cycle pulse**: the engine samples `startIn` in `Idle`/
`Halted` and soft-inits on the first high cycle, so a single pulse suffices
(Engine design §8, `stepIdle`/`stepHalted`).

## 7. RX / load path (`RxControl`)

Streams `frameDecode` ∘ `decodeControl` (wire-format §8.2, §9). Data flow:

```
rxByte ─► [delimiter watch] ─► [streaming COBS decode] ─► [frame/message layer] ─► instrWr / startOut
             0x00 = boundary      Tamal.Loader.Cobs        CRC-8 + holdback + opcode dispatch
```

- **Delimiter.** The `0x00` delimiter belongs to the frame layer, not COBS
  (wire-format §4). The FSM treats a received `0x00` as a frame boundary — an
  unambiguous **resync anchor**, since COBS guarantees no interior zeros (D2 of
  the wire format). Non-zero bytes feed the decode codec.
- **Streaming COBS decode** (§9): read a code byte `n` (1..255), copy the next
  `n−1` bytes to the logical output, and inject one `0x00` at a group end when
  `n < 255` **and** bytes remain before the delimiter. A code byte that
  overshoots the delimiter, or a truncated group, marks the frame **malformed**.
- **Frame/message layer** (in the FSM):
  - **One-byte holdback** separates the trailing CRC from payload while streaming.
    The decoded logical stream is `opcode ++ payload ++ crc`. Hold the most-recent
    decoded byte; when the *next* decoded byte arrives, the held byte is
    *confirmed* — fold it into the running CRC-8 and route it (first confirmed
    byte = opcode; the rest = payload). At the delimiter, the still-held byte is
    the **CRC candidate**, compared against the running CRC.
  - **Opcode dispatch** on the first confirmed byte:
    - `0x01 LOAD_PROGRAM` — reset the write-address counter to 0; assemble
      confirmed payload bytes four-at-a-time (little-endian, `bytesToWordLE`) into
      words; write each completed word to `instrWr = Just (addr, word)` and
      increment `addr`.
    - `0x02 TRIGGER` — expect an empty payload.
    - anything else — ignore the frame.
  - **At the delimiter:**
    - CRC-good + `TRIGGER` + empty payload → **pulse `startOut`**, go to `Run`.
    - CRC-good + `LOAD_PROGRAM` → words already written (write-through); stay.
    - CRC-bad / malformed / unknown opcode / `LOAD` payload length not a multiple
      of 4 / `TRIGGER` with payload → **discard** (no `startOut`; any
      write-through bytes are harmless — the next good `LOAD` overwrites from
      addr 0). This is exactly D4's fire-and-forget.
- **Overflow (> 1024 words).** The write-address counter (`Unsigned 10`)
  **saturates at 1023** rather than wrapping; further words are dropped. The
  1024-word instruction-store cap is a loader check (wire-format D5); a program
  that large is a host error.

Because the per-byte work above is a fixed handful of cycles and the UART byte
period is ~500 cycles (§10), the RX path can never fall behind the incoming
stream.

## 8. TX / drain path (`Drain`)

Streams `encodeResult` ∘ `frameEncode` (wire-format §8.3, §9). Data flow:

```
[drain generator] ─► [streaming COBS encode] ─► [delimiter] ─► txByte (paced by txReady)
 opcode+LE words+CRC     Tamal.Loader.Cobs         append 0x00
```

- **Drain generator** produces the *logical* byte stream and the CRC:
  1. emit opcode `0x81`;
  2. emit the four little-endian bytes (`wordToBytesLE`) of `ring[0]`
     (REVISION), then `ring[1 .. ringPtrIn−1]` (records), then `ring[termAddr]`
     (HALT terminator, `termAddr = maxBound = 4095`);
  3. emit the final **CRC-8 byte** (folded over the opcode + all word bytes),
     flagged `last`.
  Ring reads carry the BRAM's 1-cycle latency: drive `ringAddr`, latch `ringData`
  the next cycle, split into four LE bytes. `ringPtrIn == 1` (no records) yields
  the minimal drain — `ring[0]` + `ring[termAddr]` only (the empty `[1..0]` range
  is skipped).
- **Streaming COBS encode** (§9) buffers each group of ≤254 non-zero bytes
  (because the code byte precedes its data, D3), emitting `code ++ group` on a
  zero input or at the 254-byte cap, and flushing the final group on `last`.
- **Delimiter.** After the encoder's final group, the FSM appends a single
  `0x00`.
- **UART pacing.** Each encoder output byte is offered as `txByte = Just b` when
  `txReady` is high (the `uartTx` handshake, UART design §7). The path is
  two-rate: the generator/encoder fill is BRAM-paced (fast), the emit is
  UART-paced (slow); with ~500 cycles per byte there is vast headroom, so
  correctness — not throughput — governs. On the final delimiter's completion →
  `RxControl`.

## 9. The streaming COBS codec (`Tamal.Loader.Cobs`)

The isolated leaf: **pure COBS only** — no CRC, no delimiter (both live in the
FSM, exactly as `Tamal.Wire` sits above `Tamal.Wire.Cobs`). Two small `mealy`
blocks whose **sampled output equals the pure `Tamal.Wire.Cobs` oracle**. Exact
port shapes (valid/last/ready strobes) are an implementation detail for the plan;
the contract is the equivalence:

- **decode block** — COBS input bytes (plus a frame-end pulse marking the
  delimiter) → a valid-strobed logical-byte stream, plus a `malformed` flag.
  Property: `extractValid (decodeStream (cobsEncode x)) ≡ x` for all `x`.
- **encode block** — logical input bytes (the last one flagged) → a
  valid-strobed COBS-byte stream (no delimiter). Property:
  `extractValid (encodeStream x) ≡ cobsEncode x` for all `x`.

`extractValid` collects the bytes on cycles where the output valid strobe is
high. The delimiter and the CRC are added/checked by the FSM, so these two
properties compose with the pure `frameEncode`/`frameDecode` laws to give the
whole loader byte-for-byte fidelity to `Tamal.Wire`.

## 10. No UART flow control (hardware reality)

Digilent did not wire the FTDI USB-UART's RTS/CTS to the FPGA on the Arty A7, so
there is **no hardware flow control in either direction**. The design tolerates
this rather than fighting it (D5):

- **FPGA-side cannot overrun.** RX per-byte work (streaming COBS + CRC fold +
  word assembly + one BRAM write) is a fixed handful of cycles; the byte period
  is `100 MHz / 2 Mbaud × 10 bits ≈ 500 cycles`. The loader always drains each
  `rxByte` strobe long before the next. The drain self-paces off `txReady`.
- **Host-side RX overrun** (the host not emptying its FTDI/OS buffer fast enough
  during a drain) is the only real exposure. It is absorbed by FTDI + OS
  buffering at 2 Mbaud and, ultimately, by the **CRC + re-run backstop**: a
  dropped or garbled byte → a failed COBS/CRC → the host discards the drain, sees
  no valid result, times out, and re-runs — byte-reproducibly (wire-format D4).
- **Escape hatch:** the baud rate is a top-level `SNat` (`Tamal.Uart` is
  parametric); lowering it needs no loader or protocol change.

The loader therefore contains **zero** flow-control logic.

## 11. Testing (hedgehog + HUnit, Signal-level) & the TDD loop

New module `hdl/tests/Test/Loader.hs` (`tests :: TestTree`, SPDX header, added to
`hdl/tests/unittests.hs`), in the tasty + tasty-hedgehog style, reusing
`Test.Gen` and the pure `Tamal.Wire`/`Tamal.Wire.Cobs` encoders as oracles. The
Signal-level idiom follows `Test.Mem`/`Test.Uart`: `sampleN` with an inline
`:: Signal Dom100 _` annotation, `fromList` stimulus, drop the undefined cycle-0
output, compare against a pure model.

The loader is tested **in isolation** with its surroundings *modeled*: assert
directly on the `instrWr` output stream (no real instr BRAM needed); drive
`ringData` from a pure "ring lookup with 1-cycle latency" oracle keyed on the
loader's own `ringAddr`; drive `halted`/`ringPtrIn` directly.

### 11.1 Properties & vectors

1. **Codec (`Tamal.Loader.Cobs`)** vs the pure oracle:
   - `extractValid (decodeStream (cobsEncode x)) === x` for random `x` (incl.
     `[]`, zero-dense inputs, and the 254/255-byte boundary).
   - `extractValid (encodeStream x) === cobsEncode x` for random `x`.
   - decode ∘ encode round-trip.
   - malformed inputs (truncated group, code overshoot) → `malformed` asserted.
2. **RX / load:**
   - feed the exact bytes of `encodeControl (LoadProgram ws)` with UART-spaced
     idle gaps between `rxByte` strobes; assert the `instrWr` stream
     `=== [(0,w0),(1,w1),…]` (property over random `ws`).
   - `encodeControl Trigger` → exactly one `startOut` pulse, and **only after**
     the delimiter (never before the CRC is checked).
   - single-byte flip anywhere in an encoded frame → **no** `startOut`, no
     effect (discarded).
   - unknown opcode / non-multiple-of-4 `LOAD` payload / short frame → discarded.
   - `> 1024` words → `instrWr` addresses saturate at 1023.
   - back-to-back `LOAD` then `TRIGGER` → words written, then one pulse.
3. **TX / drain:**
   - model a ring (`REVISION ++ records ++ terminator`) at a chosen `ringPtrIn`;
     run `Drain`; collect the `txByte` stream; assert `=== encodeResult ws`
     exactly (COBS + CRC + trailing `0x00`) — property over random record lists.
   - `ringPtrIn == 1` minimal drain (`REVISION` + terminator only).
   - intermittently hold `txReady` low → the emitted stream is unchanged (no
     drop, no duplication): the self-pacing proof for the no-flow-control link.
4. **End-to-end lifecycle:** `LOAD` → `TRIGGER` → drive `halted`/`ringPtrIn`/ring
   model → `Drain`; assert `instrWr`, the single `startOut`, and the drain bytes
   all match the pure model; then a second `LOAD`/`TRIGGER` proves re-runnability
   and instruction overwrite.

### 11.2 Division of labour (D6)

Ping-pong TDD: the assistant writes the failing test for each slice (red); the
author writes the Clash under `src/` to pass it (green); the two refactor
together, the assistant mentoring on Clash idioms.

Suggested slice order (each a red → green → refactor loop): (1) the `ringPtrOut`
engine projection; (2) the streaming COBS **decode** block + its property;
(3) the streaming COBS **encode** block + its property; (4) the RX frame/message
layer (holdback, CRC, opcode dispatch, `LOAD` word writes); (5) the `TRIGGER`
pulse + `RxControl`→`Run` transition; (6) the drain generator + `Run`→`Drain`→
`RxControl`; (7) the overflow, corruption, and end-to-end cases. The exact task
list is the follow-up implementation plan's job.

## 12. Module decomposition & files

```
new:      hdl/src/Tamal/Loader.hs        -- lifecycle FSM + BRAM/UART/engine wiring   (+SPDX)
          hdl/src/Tamal/Loader/Cobs.hs   -- streaming COBS decode + encode (isolated) (+SPDX)
          hdl/tests/Test/Loader.hs       -- tests :: TestTree                          (+SPDX)

modified: hdl/src/Tamal/Engine.hs        -- BusOut += ringPtrOut; busOut += one line (§4)
          hdl/tamal.cabal                -- exposed-modules += Tamal.Loader, Tamal.Loader.Cobs
                                         -- test other-modules += Test.Loader
          hdl/tests/unittests.hs         -- import qualified Test.Loader; add Test.Loader.tests
          hdl/PLAN.md                    -- mark piece 3 (loader) done; next = IOBUF
```

Unchanged leaves reused: `Tamal.Wire` / `Tamal.Wire.Cobs` (the pure oracle and
the frame/opcode/LE/CRC vocabulary), `Tamal.Crc` (the CRC-8 fold), `Tamal.Uart`
(the transport), `Tamal.Mem` (the two BRAMs). No `topEntity` change here — the
loader is wired to pins in piece 5.

## 13. Verification

From `hdl/` (cold Clash/GHC builds are slow — expected):

```
stack build
stack test                           # hedgehog + HUnit: Test.Loader (+ existing)
stack run clash -- Tamal --verilog   # codegen smoke — Tamal.Loader compiles under Clash
make format-check                    # fourmolu style gate
```

The loader is synthesizable but not yet in `topEntity` (still the placeholder
heartbeat until piece 5), so the Verilog smoke exercises Clash compilation of the
loader, not new gateware on the pins — that lands with the top.

## 14. Out of scope (later specs / plans)

- **IOBUF** (piece 4) and **`topEntity`** integration (piece 5).
- The Rust **`crates/tamal-abi`** mirror and the **`tamal-loader`** host transport
  (post-silicon; they implement the same wire contract).
- **ACK/NAK / retransmit**, an FPGA→host **error frame** (opcodes reserved by the
  wire format), and **live streaming trace** (v1 drains once, on HALT).
- **Additional control commands** (SET_CONFIG-over-control, ABORT, PING) — opcode
  space `0x03`–`0x7F` reserved by the wire format.
- **UART flow control / adaptive baud** — fixed no-flow-control link (§10); baud
  is a top-level knob.

## 15. Prior art

- **[mole](https://github.com/felipebalbi/mole)** — the sibling rig; its host↔
  device loader/drain split motivates this transport bridge that ships bytecode
  down and streams the trace back with the semantics living above the link.
- **COBS** — Cheshire & Baker, *"Consistent Overhead Byte Stuffing"* (1999): the
  streaming decode (read code, count down, inject a zero) and buffered encode
  (≤254-byte group look-ahead) realized here match the pure `Tamal.Wire.Cobs`.
- **The wire-format design** (`.../2026-07-02-tamal-wire-format-design.md`, §10)
  — the loader is the streaming realization it named and deferred.
