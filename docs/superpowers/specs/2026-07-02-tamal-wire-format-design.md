# Tamal — Wire Format (control + result framing) Design

Date: 2026-07-02
Status: Approved (design); implementation not started
Scope: The transport-agnostic **wire format** exchanged with a running rig — the
**control plane** (host → FPGA: load a program, trigger a run) and the **result
plane** (FPGA → host: the drained trace ring) — defined as a contract *and*
implemented as a pure Clash leaf core (`Tamal.Wire.Cobs` + `Tamal.Wire`),
hedgehog-tested. This is **piece 2** of the impure-shell roadmap in `hdl/PLAN.md`
and the prerequisite for the loader FSM (piece 3).

This is **HDL-only**. The Rust `crates/tamal-abi` mirror of this format and the
host `tamal-loader` are deferred until the gateware is validated in silicon; the
spec here is the contract they will implement later.

Companion to the ISA & HDL Engine design
(`docs/superpowers/specs/2026-07-01-tamal-isa-design.md`, esp. §4 — little-endian
words — and §8 — the result-ring layout), the Engine design
(`.../2026-07-02-tamal-engine-design.md`, esp. §7 — REVISION / records / HALT
terminator), the BRAM design (`.../2026-07-02-tamal-bram-design.md` — the 1024-word
instruction store and 4096-word ring), and the UART design
(`.../2026-07-01-tamal-uart-design.md`, esp. §9 — the byte-stream transport this
format rides on). Roadmap context in `hdl/PLAN.md` (piece 2).

---

## 1. Purpose & framing

The engine speaks **32-bit words** (instructions in, trace records out); the
transport speaks **bytes** (an 8N1 UART stream, §9 of the UART design). The wire
format is the glue: it defines how a host packages a compiled program into a byte
frame the FPGA can unpack, and how the FPGA packages a drained trace ring into a
byte frame the host can unpack — over a link that is *transport-agnostic* (UART
today, a future FX3 backend later; the format knows nothing of either).

Two directions, deliberately asymmetric (decision D4):

- **Control (host → FPGA):** `LOAD_PROGRAM(words…)` then `TRIGGER`. Fire-and-forget
  — the FPGA never replies on this plane.
- **Result (FPGA → host):** one `TRACE_DRAIN` frame per run, emitted when the
  engine reaches `Halted` and the loader sweeps the ring (`REVISION` word →
  record stream → `HALT` terminator).

Both directions share one framing discipline (§4): a **COBS**-delimited frame
carrying `[opcode][payload…][CRC-8]`, little-endian throughout. The pure core
models whole frames as byte *lists* (a reference model); the impure loader
(piece 3) implements the streaming, per-cycle realization that matches it.

## 2. Scope & non-goals

**In scope**

- `Tamal.Wire.Cobs`: `cobsEncode` / `cobsDecode` — the delimiter-agnostic COBS
  algorithm as a pure, hedgehog-round-tripped leaf.
- `Tamal.Wire`: frame + message layer — LE word↔bytes, the CRC-8 fold (reusing
  `Tamal.Crc`), `frameEncode`/`frameDecode`, and the `ControlMsg`/result
  encoders/decoders, plus the `WireError` taxonomy.
- The **contract**: opcode assignments, byte layouts, endianness, the delimiter,
  and the error model — documented so the future Rust `tamal-abi` and any second
  transport implement the identical bytes.
- Hedgehog + HUnit tests (`Test.Wire`) and cabal/runner wiring.

**Out of scope (deferred)**

- The **loader FSM** (piece 3): the impure UART↔BRAM↔engine sequencing — streaming
  COBS decode on RX, buffered COBS encode on TX, incremental CRC over a BRAM
  sweep, the `startIn` pulse, and the HALT-triggered drain. This spec ends at the
  pure/impure seam; the loader consumes this core.
- The Rust **`crates/tamal-abi`** mirror and the **`tamal-loader`** host transport
  (post-silicon).
- **ACK/NAK / handshaking**, an error-response frame, and **live streaming trace**
  (v1 is ring-drained-on-HALT). Opcode space is reserved for these (§8) but they
  are not built.
- **Configurable framing** knobs (COBS is fixed; CRC-8 is fixed).

## 3. Design decisions

| # | Decision | Rationale |
|---|----------|-----------|
| D1 | **Deliverable = the contract (this doc) + a pure Clash core** (`Tamal.Wire.Cobs` + `Tamal.Wire`); the Rust `tamal-abi` mirror and the loader FSM are separate/later. | HDL-only scope: validate the gateware in silicon before host tooling. Fits the repo's pure-leaf + impure-shell pattern (like `Serdes`/`Trace` → `Engine`). |
| D2 | **COBS delimiter framing** (`0x00` terminator), not a length-prefix and not SLIP. | Self-delimiting on *both* planes: it frames the variable-length result drain with no length field and dissolves the "zero-gap-before-the-terminator" ambiguity (§8.2) for free. Bounded ~0.4 % overhead (vs. SLIP's worst-case ×2). Clean pure round-trip property. `0x00` can never be an opcode → a resync anchor after any corruption. |
| D3 | **Reuse `Tamal.Crc` (CRC-8, poly `0x07`, init `0x00`, MSB-first)** over the *logical* frame, computed **pre-COBS**. | Zero new CRC code; the eSPI/SMBus-flavoured CRC already in the repo and hedgehog-tested. A corruption backstop behind COBS + the UART's own 8N1 framing-error strobe on a reliable USB link. |
| D4 | **Fire-and-forget control plane** (no ACK/NAK). | The only FPGA → host traffic is the drain; its trailing CRC and leading `REVISION` validate the whole round-trip. A control frame that fails its CRC is silently discarded (no load, no trigger) → no drain → the host times out and re-runs (deterministic, byte-reproducible). Keeps the loader's TX path drain-only — the simplest thing to bring up in silicon first. |
| D5 | **No explicit length field.** | COBS delimits the frame, so `LOAD_PROGRAM`'s word count = `payload_len / 4`. The 1024-word instruction-store cap is enforced by the *loader* (piece 3), keeping the wire layer size-agnostic. |
| D6 | **Byte-*list* reference model** (`[BitVector 8]`); the streaming, synthesizable realization is piece 3. | Consistent with `Trace.encodeRecord :: Record -> [BitVector 32]` and `ringPush`. The loader matches this model per-cycle exactly as `Engine` matches `ringPush`. |
| D7 | **Module split: `Tamal.Wire.Cobs` (algorithm) + `Tamal.Wire` (frame/message).** | Isolates the meatiest, most independently-interesting algorithm for its own round-trip tests; pedagogically cleaner for the learning goal (D9). |
| D8 | **CRC over the logical frame, then COBS wraps `opcode ++ payload ++ CRC`; the `0x00` delimiter is added/stripped by the frame layer, so COBS stays delimiter-agnostic.** | Clean separation of concerns: COBS = transport (removes zeros), CRC = semantic integrity. The layer round-trip properties compose (`cobsDecode ∘ cobsEncode`, `frameDecode ∘ frameEncode`, `decodeX ∘ encodeX`). |
| D9 | **Ping-pong TDD with a division of labour: the assistant writes the tests under `tests/`; the author writes the Clash under `src/`.** | A deliberate Clash learning exercise (mirrors the Engine design's D10). |

## 4. Layering

Three independent concerns, each with its own round-trip property (§11):

```
  ControlMsg / [BitVector 32]        semantic layer  (opcodes + words)
        │  encodeControl / encodeResult   ▲  decodeControl / decodeResult
        ▼                                 │
  logical frame  = [opcode][payload…][CRC-8]         (Tamal.Wire)
        │  frameEncode                    ▲  frameDecode
        ▼                                 │
  wire frame     = COBS(logical) ++ 0x00              (Tamal.Wire + .Cobs)
        │                                 │
        ▼                                 │
  UART byte stream                                    (piece 3 / Tamal.Uart)
```

- **CRC** is computed over `opcode ++ payload` (the logical frame *before* the
  CRC byte is appended); the CRC byte then joins the logical frame and the whole
  `opcode ++ payload ++ crc` is COBS-encoded (D8). So COBS protects the CRC byte
  too, and a receiver recovers `opcode ++ payload ++ crc` intact before checking.
- **The delimiter `0x00`** belongs to the frame layer, not to COBS: `cobsEncode`
  never emits `0x00` and `cobsDecode` never sees one (the frame layer splits the
  stream on `0x00` first). This keeps `cobsDecode (cobsEncode x) ≡ Just x` a
  clean, delimiter-free identity.

## 5. COBS — Consistent Overhead Byte Stuffing (`Tamal.Wire.Cobs`)

COBS removes every `0x00` from a byte sequence so a single `0x00` can delimit
frames. It structures the data into **groups**; each group is emitted as a
**code byte** followed by that group's non-zero data bytes.

### 5.1 The algorithm

**Encode** — scan the input, accumulating non-zero bytes into a group:

- On a `0x00` in the input: emit `code = (group length) + 1`, then the group's
  bytes; **consume** the `0x00` and start a fresh group.
- When a group reaches **254** non-zero bytes with no `0x00`: emit `code = 255`,
  then the 254 bytes; start a fresh group that does **not** consume a following
  zero (a "full" group).
- At end of input: emit `code = (group length) + 1`, then the group's bytes
  (there is always a final group — see the `[0x00]` example).

Codes are therefore always `1..255`; **`0x00` never appears** in the output. The
frame layer appends a single `0x00` delimiter afterwards.

**Decode** — repeat until the input (delimiter already stripped) is exhausted:

- Read code byte `n` (`1..255`). Copy the next `n − 1` bytes to the output.
- If `n < 255` **and** input remains, emit one `0x00` (the zero the encoder
  consumed). If `n == 255`, emit no zero (it was a full group). The **final**
  group emits no trailing zero.

Malformed input → `Nothing` (which the frame layer reports as `BadCobs`): a code
byte demanding more bytes than remain (truncated group), or a literal `0x00` in
the decode input (an interior zero is illegal in COBS data), or an empty input
list.

### 5.2 Worked examples (also the HUnit vectors)

Encoded shown **without** the trailing `0x00` delimiter:

| Input (bytes) | `cobsEncode` output | Note |
|---|---|---|
| `00` | `01 01` | two empty groups: one before the zero, one final |
| `11 22 00 33` | `03 11 22 02 33` | group `[11 22]` (zero-terminated), then `[33]` (final) |
| `11 00 00 00` | `02 11 01 01 01` | three zeros ⇒ three boundaries + a final empty group |
| `` (empty) | `01` | one empty final group |
| `01 02 … FD` (254 non-zero) | `FF 01 02 … FD` | a full 254-byte group, `code = 255`, no implicit zero |

The `254/255` boundary is the subtle case worth an explicit test: exactly 254
non-zero bytes ⇒ `code = 255` with no phantom zero; 255 non-zero bytes ⇒ a `255`
group of 254 bytes followed by a `02`-group of the last byte.

### 5.3 Signatures

```haskell
-- | COBS-encode a byte sequence. The result never contains 0x00 and does NOT
-- include the frame delimiter (the frame layer appends it).
cobsEncode :: [BitVector 8] -> [BitVector 8]

-- | COBS-decode a delimiter-stripped byte sequence back to the original bytes,
-- or Nothing if it is malformed. Cobs stays a dependency-free leaf — it knows
-- nothing of WireError; the frame layer maps Nothing to BadCobs.
cobsDecode :: [BitVector 8] -> Maybe [BitVector 8]
```

Round-trip law: `cobsDecode (cobsEncode x) ≡ Just x` for all `x` (including
`[]`, since `cobsEncode [] = [0x01]`).

## 6. CRC-8 (reuse of `Tamal.Crc`)

The per-frame integrity check reuses the engine's CRC-8 verbatim (poly `0x07`,
init `0x00`, MSB-first — eSPI/SMBus). `Tamal.Wire` exposes a thin fold:

```haskell
crc8 :: [BitVector 8] -> BitVector 8
crc8 = foldl' Crc.crc8Update 0x00
```

`frameEncode` appends `crc8 logical`; `frameDecode` recomputes over the recovered
`opcode ++ payload` and compares to the trailing byte, returning `Left BadCrc` on
mismatch. No new polynomial, no second CRC flavour.

## 7. Little-endian word packing

ISA §4 fixes little-endian on the wire, so a 32-bit word is four bytes
LSB-first:

```haskell
-- 0xAABBCCDD  <->  [0xDD, 0xCC, 0xBB, 0xAA]
wordToBytesLE :: BitVector 32 -> Vec 4 (BitVector 8)
bytesToWordLE :: Vec 4 (BitVector 8) -> BitVector 32
```

`bytesToWordLE (wordToBytesLE w) ≡ w`, and the pinned vector above nails the byte
order so a future host implementation cannot silently disagree.

## 8. Frame layouts

### 8.1 Opcodes

`0x00` is the delimiter and is therefore **never** an opcode. High bit set marks
the FPGA → host direction (cosmetic — the planes are physically separate — but a
useful sanity aid).

| Opcode | Name | Direction | v1 |
|---|---|---|---|
| `0x01` | `LOAD_PROGRAM` | host → FPGA | ✔ |
| `0x02` | `TRIGGER` | host → FPGA | ✔ |
| `0x03`–`0x7F` | reserved (control) | host → FPGA | future (SET_CONFIG-over-control, ABORT, PING…) |
| `0x81` | `TRACE_DRAIN` | FPGA → host | ✔ |
| `0x82`–`0xFF` | reserved (result) | FPGA → host | future (ACK/NAK, error, streaming trace…) |

### 8.2 Control frame (host → FPGA)

Logical frame (pre-COBS):

| Field | Bytes | Contents |
|---|---|---|
| opcode | 1 | `0x01` `LOAD_PROGRAM` or `0x02` `TRIGGER` |
| payload | 0 … 4·N | `LOAD_PROGRAM`: N instruction words, 4 bytes each **LE**. `TRIGGER`: empty |
| CRC-8 | 1 | `crc8 (opcode ++ payload)` |

Wire frame = `cobsEncode(opcode ++ payload ++ crc) ++ [0x00]`.

- **Word count is implicit** (D5): `N = payload_len / 4`. A payload length not a
  multiple of 4 ⇒ `Left BadPayloadLen`. The 1024-word cap is a *loader* check,
  not a wire-layer one.
- `TRIGGER` carries no payload; its whole logical frame is `[0x02, crc8 [0x02]]`.

### 8.3 Result frame (FPGA → host)

One frame per run, built by the loader from the drained ring (Engine design §7).
Logical frame (pre-COBS):

| Field | Bytes | Contents |
|---|---|---|
| opcode | 1 | `0x81` `TRACE_DRAIN` |
| REVISION | 4 | ring `word[0]` (`revisionWord`, e.g. `0x0001_0000`), **LE** |
| records | 4·k | ring `word[1 … ringPtr−1]` — CAPTURE/MARK words, **LE** |
| HALT term | 4 | ring `word[termAddr]` — the HALT terminator record, **LE** |
| CRC-8 | 1 | `crc8 (opcode ++ all word bytes)` |

Wire frame = `cobsEncode(opcode ++ words_LE ++ crc) ++ [0x00]`.

- **Self-delimiting** (D2): the whole drain is one COBS frame, so the host reads
  bytes to the `0x00`, `cobsDecode`s, checks the CRC, then unpacks words. The
  `HALT` record (tag `11`, ISA §8) is the logical end-of-trace.
- **No zero-gap ambiguity:** the loader emits `word[0]`, then `word[1 … ringPtr−1]`,
  then jumps to `word[termAddr]` — a *gap-free* word stream that skips the dead
  `ringPtr … termAddr−1` slots. How the loader knows `ringPtr` is a piece-3
  detail; the wire contract is simply "the words between the opcode and the CRC,
  the last of which is the `HALT` terminator." (Modelled purely, `encodeResult`
  just takes the already-assembled word list.)

## 9. The pure modules (API)

Two new modules under `hdl/src/Tamal/`, each carrying the REUSE/SPDX header
(`SPDX-License-Identifier: CERN-OHL-W-2.0`) per `AGENTS.md`, Clash ADT idioms
(`deriving stock (Generic, Show, Eq)` / `deriving anyclass NFDataX`).

### `Tamal.Wire.Cobs`

```haskell
cobsEncode :: [BitVector 8] -> [BitVector 8]
cobsDecode :: [BitVector 8] -> Maybe [BitVector 8]
```

A **dependency-free** leaf: `Cobs` imports no other `Tamal.Wire.*` module (in
particular not `WireError`), so `Tamal.Wire → Tamal.Wire.Cobs` is the only
internal edge and there is no import cycle. `cobsDecode`'s single failure mode is
"malformed" (`Nothing`); the frame layer lifts it to `BadCobs`.

### `Tamal.Wire`

```haskell
-- Semantic messages (control plane).
data ControlMsg
  = LoadProgram [BitVector 32]     -- instruction words, host → FPGA
  | Trigger                        -- start-of-run pulse
  deriving stock (Generic, Show, Eq)
  deriving anyclass NFDataX

-- Why a frame failed to decode.
data WireError
  = BadCrc                         -- CRC mismatch
  | BadCobs                        -- malformed COBS (truncated group / interior zero / empty)
  | UnknownOpcode (BitVector 8)    -- opcode not in the v1 set
  | ShortFrame                     -- fewer than [opcode, crc] bytes
  | BadPayloadLen                  -- LOAD_PROGRAM payload not a multiple of 4
  deriving stock (Generic, Show, Eq)
  deriving anyclass NFDataX

-- Primitives.
wordToBytesLE :: BitVector 32 -> Vec 4 (BitVector 8)
bytesToWordLE :: Vec 4 (BitVector 8) -> BitVector 32
crc8          :: [BitVector 8] -> BitVector 8

-- Framing layer: logical (opcode ++ payload) <-> wire (COBS + CRC + delimiter).
frameEncode :: [BitVector 8] -> [BitVector 8]
frameDecode :: [BitVector 8] -> Either WireError [BitVector 8]

-- Message layer (both directions — the reference for host and FPGA alike).
encodeControl :: ControlMsg     -> [BitVector 8]
decodeControl :: [BitVector 8]  -> Either WireError ControlMsg
encodeResult  :: [BitVector 32] -> [BitVector 8]   -- drained ring words in, wire bytes out
decodeResult  :: [BitVector 8]  -> Either WireError [BitVector 32]
```

`frameDecode` = strip the trailing `0x00` delimiter → `cobsDecode` → split into
`(opcode ++ payload)` and the trailing CRC byte → verify → return
`opcode ++ payload`. `decodeControl` then dispatches on the opcode and unpacks LE
words; `decodeResult` unpacks the `0x81` frame's words.

## 10. Relationship to the loader (piece 3)

This core is the loader's vocabulary and its reference model. The loader is the
**streaming** realization, which this spec does *not* build:

- **RX / load path.** The loader consumes `rxByte` strobes, hunts for the `0x00`
  delimiter, streams a COBS **decode** (trivially streaming: read a code byte,
  count down `n−1`, inject a zero at group ends), folds the CRC incrementally,
  and on a valid `LOAD_PROGRAM` writes each unpacked LE word into the instruction
  BRAM; on `TRIGGER` it pulses `startIn`. A bad CRC/COBS ⇒ discard (D4).
- **TX / drain path.** On `haltedOut`, the loader sweeps the ring BRAM
  (`word[0]`, `word[1…ringPtr−1]`, `word[termAddr]`), COBS-**encodes** the byte
  stream (this needs a small look-ahead buffer, ≤ 254 bytes, because a group's
  code byte precedes its data), appends the CRC, and drives `txByte` while
  `txReady`.

Both directions must reproduce this pure model byte-for-byte — the same
"pure list model, streaming impl matches it" discipline `Engine` uses for
`ringPush` (Engine design §7.1).

## 11. Testing (hedgehog + HUnit) & the TDD loop

New module `hdl/tests/Test/Wire.hs` (`tests :: TestTree`, SPDX header, added to
`hdl/tests/unittests.hs`), reusing `Test.Gen` generators.

### 11.1 Properties & vectors

- **COBS round-trip:** `cobsDecode (cobsEncode x) ≡ Just x` for random
  `x :: [BitVector 8]` (include `[]` and long, zero-dense inputs).
- **COBS output invariant:** `cobsEncode x` contains no `0x00`.
- **COBS vectors (HUnit):** the five §5.2 rows, plus the 254/255 boundary.
- **COBS malformed:** truncated group and interior-zero inputs ⇒ `Nothing`
  (and, through `frameDecode`, `Left BadCobs`).
- **LE bytes:** `bytesToWordLE (wordToBytesLE w) ≡ w`; the `0xAABBCCDD` vector.
- **Frame round-trips:** `decodeControl (encodeControl m) ≡ Right m` for random
  `ControlMsg` (random word lists for `LoadProgram`, and `Trigger`);
  `decodeResult (encodeResult ws) ≡ Right ws` for random `ws`.
- **Integrity:** flipping any single byte of an encoded frame ⇒
  `Left BadCrc` or `Left BadCobs` (never a wrong-but-`Right` decode of a
  single-byte error).
- **Error taxonomy:** unknown opcode ⇒ `UnknownOpcode`; a `LOAD_PROGRAM` payload
  whose length mod 4 ≠ 0 ⇒ `BadPayloadLen`; a frame shorter than
  `[opcode, crc]` ⇒ `ShortFrame`.
- **Delimiter:** every `frameEncode`/`encode*` output ends in exactly one `0x00`
  and contains no other `0x00`.

### 11.2 Division of labour (D9)

Ping-pong TDD, a learning exercise:

- The **assistant writes the failing test** for the next slice (red).
- The **author writes the Clash under `src/`** to pass it (green); the two
  **refactor together**, the assistant mentoring on Clash idioms.

Suggested slice order (each a red → green → refactor loop): (1) `wordToBytesLE` /
`bytesToWordLE`; (2) `cobsEncode`; (3) `cobsDecode` + the round-trip; (4) `crc8`
fold + `frameEncode` / `frameDecode`; (5) `encodeControl` / `decodeControl`;
(6) `encodeResult` / `decodeResult`; (7) the error paths. The exact task list is
the follow-up implementation plan's job.

## 12. Module decomposition & files

```
new:      hdl/src/Tamal/Wire/Cobs.hs   -- cobsEncode / cobsDecode            (+ SPDX)
          hdl/src/Tamal/Wire.hs        -- frame + message layer, WireError    (+ SPDX)
          hdl/tests/Test/Wire.hs       -- tests :: TestTree                   (+ SPDX)

modified: hdl/tamal.cabal              -- exposed-modules += Tamal.Wire.Cobs, Tamal.Wire
                                       -- test other-modules += Test.Wire
          hdl/tests/unittests.hs       -- import qualified Test.Wire; add Test.Wire.tests
          hdl/PLAN.md                  -- mark piece 2 (wire protocol) done; next = loader
```

Unchanged leaves reused: `Tamal.Crc` (the CRC-8 fold). No `topEntity` change (the
core is pure; the loader wires it in later).

## 13. Verification

From `hdl/` (cold Clash/GHC builds are slow — expected):

```
cabal build
cabal test                           # hedgehog + HUnit: Test.Wire (+ existing)
cabal run clash -- Tamal --verilog   # codegen smoke — Tamal.Wire compiles under Clash
make format-check                    # fourmolu style gate
```

The wire core is pure and not yet in `topEntity`, so the Verilog smoke exercises
library compilation under Clash, not new gateware — until the loader wires it in.

## 14. Out of scope (later specs / plans)

- **The loader FSM** (piece 3): streaming COBS on both paths, the CRC-over-sweep,
  BRAM read/write sequencing, `startIn`/`haltedOut`, and the drain trigger.
- **The Rust `crates/tamal-abi` mirror** and the **`tamal-loader` transport**
  (post-silicon; they implement this same contract).
- **ACK/NAK / handshaking** and an FPGA→host **error frame** (opcodes reserved).
- **Live streaming trace** (v1 is ring-drained-on-HALT).
- **Additional control commands** (SET_CONFIG-over-control, ABORT, PING) — opcode
  space `0x03`–`0x7F` reserved.
- **CRC-16 / configurable framing** — fixed at COBS + CRC-8 for v1.

## 15. Prior art

- **[mole](https://github.com/felipebalbi/mole)** — the sibling rig; its
  host↔device split motivates a transport-agnostic byte framing that carries the
  bytecode down and the trace back with the semantics living above the link.
- **COBS** — Cheshire & Baker, *"Consistent Overhead Byte Stuffing"* (1999): the
  standard `0x00`-delimited framing with bounded overhead, chosen here for the
  self-delimiting result drain (D2).
- **CRC-8/SMBus (poly `0x07`)** — already the engine's RX residue (`Tamal.Crc`,
  ISA §7.4); reused here so the codebase carries one CRC (D3).
