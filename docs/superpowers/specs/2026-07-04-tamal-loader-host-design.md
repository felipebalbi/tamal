# Tamal — Host Loader (`tamal-loader` / `tamal-loader-cli`) Design

Date: 2026-07-04
Status: Approved (design); implementation not started
Scope: The **host-side** loader — the Rust counterpart to the on-FPGA loader FSM.
It takes the **raw bytecode** `tamal-asm` emits (a flat little-endian word stream,
no framing), packages it into the **COBS + CRC-8** wire frames the gateware
expects, ships `LOAD_PROGRAM` + `TRIGGER` over a UART transport, then reads back
the single `TRACE_DRAIN` frame, verifies it, and decodes the trace ring into a
human-readable listing with a HALT/TRAP verdict. Split across three crates:
`tamal-abi` (the transport-agnostic wire mirror), `tamal-loader` (transport +
session orchestration), and `tamal-loader-cli` (the `tamal-loader` binary).

This is the **post-silicon host tooling** the wire-format and HDL-loader specs
deferred. It implements the identical byte contract those specs defined; the
gateware is the other end of the same wire.

Companion to the wire-format design
(`docs/superpowers/specs/2026-07-02-tamal-wire-format-design.md` — the byte
contract this mirrors: §5 COBS, §6 CRC-8, §7 LE packing, §8 frame layouts), the
HDL loader design (`.../2026-07-02-tamal-loader-design.md` — the FPGA end of the
same link), the engine design (`.../2026-07-02-tamal-engine-design.md`, esp. §7 —
REVISION / record encodings / HALT terminator), and the ISA/ABI design
(`.../2026-07-03-tamal-abi-isa-design.md` — the existing `tamal-abi` mirror this
extends). The raw-bytecode producer is `tamal-asm`
(`.../2026-07-04-tamal-asm-design.md`).

---

## 1. Purpose & role

`tamal-asm` produces a **raw binary**: `Program::to_le_bytes` →
`tamal_abi::isa::program_to_le_bytes`, a flat concatenation of little-endian
32-bit instruction words with **no COBS, no CRC, no framing**. That is exactly a
`LOAD_PROGRAM` *payload* and nothing more. The host loader is the piece that turns
that payload into a link transaction and reads the result back.

Four responsibilities, mirroring the FPGA loader from the opposite side:

1. **Frame** (host): wrap the raw words into a `LOAD_PROGRAM` frame
   (`COBS(0x01 ++ words_LE ++ crc8) ++ 0x00`) and a `TRIGGER` frame
   (`COBS(0x02 ++ crc8) ++ 0x00`).
2. **Ship** (host → FPGA): send both frames over the transport (UART for v1).
3. **Drain** (FPGA → host): read bytes to the `0x00` delimiter, `frame_decode`
   (strip delimiter → COBS-decode → verify CRC), and unpack the `0x81`
   `TRACE_DRAIN` frame into little-endian words.
4. **Decode** (host): parse the drained words — `REVISION`, the record stream
   (CAPTURE / MARK), and the HALT terminator — into a typed `Trace` and print a
   listing plus the HALT/TRAP verdict.

The loader owns **no eSPI semantics** — like its FPGA twin it is a transport
bridge. Bus meaning lives in the engine (which produced the trace) and, later, the
verdict/conformance layer (Phase 4, out of scope).

## 2. Scope & non-goals

**In scope**

- `tamal-abi`: the transport-agnostic **wire mirror** — new `crc8`, `cobs`,
  `wire`, and `trace` modules implementing the byte contract of the HDL
  `Tamal.Crc` / `Tamal.Wire.Cobs` / `Tamal.Wire` and the engine's trace-record
  encodings. Golden-tested against those oracles.
- `tamal-loader`: a `Transport` trait + a `UartTransport` (`serialport`) backend;
  a `Device<T>` session (`load_program` / `trigger` / `read_trace` / `run`) with
  the timeout + auto-retry recovery loop; the `validate_program_bytes` check; a
  `thiserror` error taxonomy.
- `tamal-loader-cli`: a `clap` front-end with a single `run` subcommand that
  drives the full load → trigger → drain → decode cycle and pretty-prints the
  trace + verdict, exiting non-zero on a trap.
- Unit + property tests across all three crates, with a `MockTransport`
  "fake FPGA" so the orchestration is testable without hardware.

**Out of scope (deferred)**

- **The FX3 USB backend.** The `Transport` trait is shaped so it slots in later
  (AGENTS.md), but v1 ships only UART.
- **Additional control commands** (`SET_CONFIG`-over-control, `ABORT`, `PING`).
  v1 config is baked into the bytecode via the in-program `SET_CONFIG`
  instruction; the control plane is `LOAD_PROGRAM` + `TRIGGER` only (wire-format
  §8.1, opcodes `0x03`–`0x7F` reserved).
- **ACK/NAK / handshaking / live streaming trace.** v1 is fire-and-forget with a
  once-per-run ring drain (wire-format D4).
- **The verdict / conformance engine** (Phase 4). The loader reports the raw
  HALT/TRAP terminator; pass/fail *policy* is a later layer.
- **`JSON`/machine-readable output.** v1 prints a human listing only.
- **A `ports`-listing subcommand.** Left out of the "single `run`" surface;
  trivially addable later if wanted.

## 3. Design decisions

| # | Decision | Rationale |
|---|----------|-----------|
| D1 | **The wire mirror lives in `tamal-abi`, not `tamal-loader`.** New `crc8`, `cobs`, `wire`, `trace` modules; `tamal-loader` depends on them for the bytes. | AGENTS.md: keep `tamal-abi` transport-agnostic and put transports in `tamal-loader`. Mirrors the HDL's pure-`Tamal.Wire` / impure-loader split. `tamal-abi` already reserves `control`/`trace` module slots for exactly this. |
| D2 | **Hand-code COBS and CRC-8** (two small leaves) rather than pull the `cobs` / `crc` crates. | Byte-exactness with the HDL oracle is the whole point of a conformance rig — the HDL-parity golden tests are mandatory either way, so a crate saves ~90 lines but not the verification. Both algorithms are trivial (§5 of the wire-format spec spells them out), let us map the `BadCobs`/malformed taxonomy precisely, keep the contract crate dependency-free (`#![forbid(unsafe_code)]`), and read side-by-side with the Clash implementations for cross-checking. |
| D3 | **Four small `tamal-abi` leaves** (`crc8`, `cobs`, `wire`, `trace`) mirroring the HDL modules 1:1, not one monolithic `wire` module. | Each leaf is small, independently testable, and has a direct HDL oracle to golden-test against. Matches the repo's pure-leaf discipline. |
| D4 | **Single `run` CLI subcommand** doing the whole load → trigger → drain cycle; the `Device` library API still exposes `load_program` / `trigger` / `read_trace` separately. | `LOAD` and `TRIGGER` are always paired in fire-and-forget; a load-without-trigger CLI verb has little value. The granular library methods keep future subcommands cheap without bloating the v1 surface. |
| D5 | **Typed trace decode + pretty listing.** `decode_trace` yields `Revision`/`Record`/`Halt`; the CLI prints a listing and summarizes the verdict. | The operator needs CAPTURE/MARK/HALT semantics, not raw hex. Decoding in `tamal-abi::trace` mirrors the engine's `encodeRecord` and is reusable by any future consumer (e.g. the verdict engine). |
| D6 | **Timeout + auto-retry** on a missing or malformed drain, re-sending **both** `LOAD_PROGRAM` and `TRIGGER` each attempt. | The wire-format's recovery model (D4/D5): a dropped/CRC-failed frame → no valid drain → the host re-runs, byte-reproducibly. Re-sending LOAD too (not just TRIGGER) is required because the HDL loader writes LOAD words *through* regardless of CRC and only commits on TRIGGER, so a partial LOAD must be fully re-driven. |
| D7 | **Length + cap validation only** before sending: reject a `.bin` whose length isn't a multiple of 4 or whose word count exceeds 1024. No per-instruction decode. | These are the two limits the wire format (§8.2, multiple-of-4) and the HDL loader (§7, 1024-word instruction store) actually enforce. Per-instruction decoding would wrongly reject programs that intentionally probe illegal encodings — the engine TRAPs on those and reports it in the drain, which is the correct behavior for a conformance rig. |
| D8 | **`Transport` trait + `UartTransport` (`serialport`) backend**, with frame-reading (`read_frame`, "bytes to the next `0x00`") as a trait method. | The FX3 backend drops in as a second impl (AGENTS.md). Framing lives in the transport because different backends delimit differently (UART byte stream vs. a future FIFO packet); the `0x00`-delimited read is the UART realization. |
| D9 | **`thiserror` error taxonomy in the library; `color-eyre` reporting in the CLI.** No new dependencies in any crate. | Matches the existing crate conventions (`tamal-loader` already has `thiserror` + `serialport`; `tamal-loader-cli` already has `clap` + `color-eyre`). |

## 4. `tamal-abi` — the wire mirror

Four new modules, each carrying no transport knowledge, each a byte-exact mirror
of an HDL oracle. `lib.rs` declares them and **replaces** the stale `control`
placeholder (whose docstring lists a richer command set that predates the
finalized `LOAD`+`TRIGGER`-only wire format) with `wire`; the `trace` placeholder
is filled.

### 4.1 `tamal_abi::crc8` (mirror of `Tamal.Crc`)

```rust
/// Fold one byte into the running CRC-8 (poly 0x07, MSB-first, no reflection,
/// no final XOR) — the exact HDL `crc8Update`.
pub fn crc8_update(crc: u8, byte: u8) -> u8;

/// Fold a byte slice from the initial value 0x00.
pub fn crc8(bytes: &[u8]) -> u8;
```

Law (residue): `crc8(&[msg, crc8(msg)].concat()) == 0` for any `msg`.

### 4.2 `tamal_abi::cobs` (mirror of `Tamal.Wire.Cobs`)

```rust
/// COBS-encode. The result never contains 0x00 and does NOT include the frame
/// delimiter (the frame layer appends it). cobs_encode(&[]) == [0x01].
pub fn cobs_encode(data: &[u8]) -> Vec<u8>;

/// COBS-decode a delimiter-stripped buffer, or a classified error.
pub fn cobs_decode(data: &[u8]) -> Result<Vec<u8>, CobsError>;

pub enum CobsError { Empty, TruncatedGroup, InteriorZero }
```

Algorithm exactly as wire-format §5.1: encode accumulates non-zero bytes into a
group, emitting `code=(len+1) ++ group` on a `0x00` (consumed) or `code=255 ++
254 bytes` at the full-group cap, with a final group at end-of-input; decode reads
`code n`, copies `n−1` bytes, and injects one `0x00` at a group end when `n < 255`
and bytes remain. Law: `cobs_decode(&cobs_encode(x)) == Ok(x)` for all `x`
(including `[]`). The malformed cases (`Empty`, `TruncatedGroup`, `InteriorZero`)
are what the frame layer lifts to `WireError::BadCobs`.

### 4.3 `tamal_abi::wire` (mirror of `Tamal.Wire`)

```rust
pub const OP_LOAD_PROGRAM: u8 = 0x01;
pub const OP_TRIGGER:      u8 = 0x02;
pub const OP_TRACE_DRAIN:  u8 = 0x81;
pub const DELIMITER:       u8 = 0x00;

pub enum ControlMsg { LoadProgram(Vec<u32>), Trigger }

pub enum WireError {
    BadCrc,                                   // trailing CRC mismatch
    BadCobs,                                  // malformed COBS (lifts CobsError)
    UnknownOpcode(u8),                        // opcode not in the v1 set
    WrongOpcode { expected: u8, found: u8 },  // decode_result on a non-0x81 frame
    ShortFrame,                               // fewer than [opcode, crc] bytes
    BadPayloadLen,                            // LOAD payload / drain words not a multiple of 4
}

// LE word packing (ISA §4): 0xAABBCCDD <-> [DD, CC, BB, AA].
pub fn words_to_le_bytes(words: &[u32]) -> Vec<u8>;
pub fn le_bytes_to_words(bytes: &[u8]) -> Result<Vec<u32>, WireError>; // BadPayloadLen if len%4≠0

// Frame layer: logical (opcode ++ payload) <-> wire (COBS + CRC + delimiter).
pub fn frame_encode(logical: &[u8]) -> Vec<u8>;               // cobs(logical ++ crc8(logical)) ++ 0x00
pub fn frame_decode(wire: &[u8]) -> Result<Vec<u8>, WireError>;

// Message layer.
pub fn encode_control(msg: &ControlMsg) -> Vec<u8>;           // host → FPGA bytes
pub fn decode_result(wire: &[u8]) -> Result<Vec<u32>, WireError>; // 0x81 drain -> LE words
```

`frame_decode` strips the trailing `0x00`, `cobs_decode`s, splits the recovered
`opcode ++ payload ++ crc` into `(opcode ++ payload)` and the trailing CRC byte,
recomputes `crc8(opcode ++ payload)`, and compares. `decode_result` then asserts
the opcode is `0x81` (`WrongOpcode` otherwise) and unpacks the remaining bytes as
LE words.

### 4.4 `tamal_abi::trace` (mirror of engine §7.2)

```rust
pub struct Trace { pub revision: Revision, pub records: Vec<Record>, pub halt: Halt }

pub struct Revision { pub major: u8, pub minor: u8, pub patch: u16 } // word[0]

pub enum Record {
    Capture { nbits: u8, byte: u8 },   // [00 | 18'0 | nbits(4) | byte(8)]
    Mark    { label: u16, payload: u32 }, // [10 | 16'0 | label(14)] ++ payload(32)
}

pub struct Halt { pub trap: bool, pub reason: TrapReason, pub ovf: bool, pub status: u8 }
pub enum TrapReason { None, Decode, Config, Rdsr, Illegal } // 0..4 (engine §7.2)

pub enum TraceError { Empty, UnknownRecordTag(u8), TruncatedMark, MissingTerminator }

pub fn decode_trace(words: &[u32]) -> Result<Trace, TraceError>;
```

`decode_trace` reads `word[0]` as `Revision` (`major=w>>24`, `minor=w>>16`,
`patch=w&0xFFFF`), then walks from index 1 by the top-2-bit tag: `00` → `Capture`
(1 word: `nbits=(w>>8)&0xF`, `byte=w&0xFF`), `10` → `Mark` (2 words; a missing
second word → `TruncatedMark`), `11` → the HALT terminator
(`reason=(w>>10)&0x7`, `trap=(w>>9)&1`, `ovf=(w>>8)&1`, `status=w&0xFF`), which
ends the walk. An unterminated stream → `MissingTerminator`; any other tag →
`UnknownRecordTag`. The word stream is gap-free (wire-format §8.3): REVISION →
records → terminator.

## 5. `tamal-loader` — transport + session

### 5.1 `transport`

```rust
pub trait Transport {
    fn send(&mut self, bytes: &[u8]) -> Result<(), TransportError>;
    /// Read bytes up to and including the next 0x00 delimiter, honoring `timeout`
    /// as an overall deadline. Leading lone delimiters are skipped (resync).
    fn read_frame(&mut self, timeout: Duration) -> Result<Vec<u8>, TransportError>;
}

pub enum TransportError { Open(String), Io(std::io::Error), Timeout }

pub struct UartTransport { /* Box<dyn serialport::SerialPort> */ }
impl UartTransport { pub fn open(path: &str, baud: u32) -> Result<Self, TransportError>; }
impl Transport for UartTransport { /* read_frame loops 1-byte reads to 0x00 vs deadline */ }
```

`read_frame` accumulates bytes until it reads a `0x00`; if the accumulator is
empty at that `0x00` (a leading delimiter), it keeps reading — the `0x00` is an
unambiguous resync anchor (COBS emits no interior zeros). It tracks a wall-clock
deadline and returns `Timeout` if exceeded.

### 5.2 `device`

```rust
pub struct Device<T: Transport> { /* transport: T */ }

pub struct RunOptions { pub timeout: Duration, pub retries: u32 }

impl<T: Transport> Device<T> {
    pub fn new(transport: T) -> Self;
    pub fn load_program(&mut self, words: &[u32]) -> Result<(), Error>; // encode_control + send
    pub fn trigger(&mut self)                     -> Result<(), Error>;
    pub fn read_trace(&mut self, timeout: Duration) -> Result<Trace, Error>; // read_frame→decode_result→decode_trace
    pub fn run(&mut self, words: &[u32], opts: RunOptions) -> Result<Trace, Error>;
}
```

`run` is the recovery loop (§6). `read_trace` chains `read_frame` →
`decode_result` → `decode_trace`, mapping each failure into `Error`.

### 5.3 validation + errors

```rust
/// Read a raw .bin into LE words, enforcing the wire/loader limits.
pub fn validate_program_bytes(bytes: &[u8]) -> Result<Vec<u32>, Error>;

pub enum Error {
    Transport(TransportError),
    Wire(WireError),
    Trace(TraceError),
    BadProgramLength(usize),   // not a multiple of 4
    ProgramTooLarge(usize),    // > 1024 words
    RetriesExhausted { attempts: u32 },
}
```

`validate_program_bytes` rejects `len % 4 != 0` (`BadProgramLength`) and
`len / 4 > 1024` (`ProgramTooLarge`), else returns the words. `run` also guards
the 1024-word cap defensively.

## 6. `run` orchestration & retry (wire-format D4/D5)

```
words = validate_program_bytes(bin)?              // fail fast — never retried
for attempt in 0 ..= opts.retries {
    device.load_program(words)?                   // re-send BOTH each attempt:
    device.trigger()?                             //   LOAD is write-through on the
    match device.read_trace(opts.timeout) {       //   HDL side and only commits on
        Ok(trace)                     => return Ok(trace),      // TRIGGER, so a partial
        Err(Transport(Timeout))       => continue,              // LOAD needs a full re-run
        Err(Wire(_)) | Err(Trace(_))  => continue,   // malformed drain: deterministic re-run
        Err(Transport(Io(e)))         => return Err(...),       // hardware/port fault: propagate
        Err(other)                    => return Err(other),
    }
}
Err(RetriesExhausted { attempts: opts.retries + 1 })
```

Recoverable = a timed-out or malformed/CRC-failed drain (the fire-and-forget
re-run path). Non-recoverable = a genuine port/IO fault, propagated immediately.

## 7. `tamal-loader-cli`

```
tamal-loader run <PROGRAM.bin> --port <PORT> [--baud 2000000] [--timeout 5] [--retries 3]
```

- `--baud` default `2_000_000` matches the HDL top's `SNat @2_000_000`;
  `--timeout` is seconds (per-drain deadline); `--retries` is the extra attempts.
- Flow: read file → `validate_program_bytes` → `UartTransport::open` →
  `Device::new` → `device.run(words, opts)` → pretty-print.
- **Listing** (example):

```
REVISION 0.1.0
[0] CAPTURE  nbits=8  byte=0x5A
[1] MARK     label=0x0001  payload=0xDEADBEEF
HALT  status=0x00  (ok)
```

  On a trap the last line is e.g. `TRAP  reason=decode  ovf=false  status=0x00`.
  A `REVISION` other than `0x0001_0000` (the `tamal.cabal` 0.1.0 constant) prints
  a warning — the bitstream and CLI disagree.
- **Exit code**: `0` when `halt.trap == false` **and** `halt.ovf == false`,
  non-zero on a trap **or** a trace overflow (`ovf`), so a truncated trace is not
  reported as a clean success and scripts can gate on the verdict. Errors render
  via `color-eyre`.

## 8. Testing

### 8.1 `tamal-abi` (pure, exhaustive)

- **`crc8`**: the residue law over random messages; a pinned HDL vector.
- **`cobs`**: the five wire-format §5.2 golden vectors + the 254/255 boundary;
  round-trip proptest `cobs_decode(cobs_encode(x)) == Ok(x)`; the no-`0x00`-in-
  output invariant; malformed inputs (truncated group, interior zero, empty) →
  the matching `CobsError`.
- **`wire`**: LE round-trip with the `0xAABBCCDD` vector; frame round-trip;
  `encode_control` goldens (`Trigger` → exact bytes; a small `LoadProgram`);
  `decode_result` from a hand-built drain; a single-byte flip anywhere in an
  encoded frame → `BadCrc` or `BadCobs` (never a wrong-but-`Ok` decode); the error
  taxonomy (`UnknownOpcode`, `WrongOpcode`, `ShortFrame`, `BadPayloadLen`).
- **`trace`**: `decode_trace` goldens for CAPTURE / MARK / HALT and a trap
  terminator; the error cases (`Empty`, `MissingTerminator`, `TruncatedMark`,
  `UnknownRecordTag`).

### 8.2 `tamal-loader` (orchestration, no hardware)

A `MockTransport` "fake FPGA" implements `Transport`: it decodes the sent
`LOAD_PROGRAM` + `TRIGGER` (via `tamal-abi`) and returns a **scripted** drain from
`read_frame`. Scenarios:

- `load_program` / `trigger` emit exactly `encode_control(...)`.
- `run` happy path → the expected `Trace`.
- `run` retries on a scripted timeout, then succeeds.
- `run` retries on a scripted corrupt drain (flipped byte → `BadCrc`/`BadCobs`),
  then succeeds.
- retries exhausted → `Error::RetriesExhausted`.
- `validate_program_bytes` boundaries: non-multiple-of-4 → `BadProgramLength`;
  1024 words ok, 1025 → `ProgramTooLarge`.

### 8.3 `tamal-loader-cli`

Unit-test the `.bin`-loading and listing/verdict formatting helpers; the live
UART path stays manual (a real Arty on a real port).

## 9. Module decomposition & files

```
new:      crates/tamal-abi/src/crc8.rs          -- crc8_update / crc8
          crates/tamal-abi/src/cobs.rs          -- cobs_encode / cobs_decode / CobsError
          crates/tamal-abi/src/wire.rs          -- frame + message layer, WireError, opcodes, LE
          crates/tamal-abi/src/trace.rs         -- Trace / Record / Halt / decode_trace
          crates/tamal-loader/src/transport.rs  -- Transport trait + UartTransport + TransportError
          crates/tamal-loader/src/device.rs     -- Device<T>, RunOptions, run/load/trigger/read_trace
          crates/tamal-loader/src/error.rs      -- Error (thiserror)

modified: crates/tamal-abi/src/lib.rs           -- declare crc8/cobs/wire/trace; drop stale `control`
          crates/tamal-loader/src/lib.rs         -- modules + validate_program_bytes + re-exports
          crates/tamal-loader-cli/src/main.rs    -- clap `run` + pretty-print + verdict exit code
```

No new dependencies: `tamal-abi` stays on `thiserror` only; `tamal-loader` uses
its existing `thiserror` + `serialport`; `tamal-loader-cli` uses its existing
`clap` + `color-eyre`.

## 10. Verification

From the repo root:

```
cargo test -p tamal-abi        # crc8 / cobs / wire / trace units + proptests
cargo test -p tamal-loader     # MockTransport orchestration + validation
cargo test -p tamal-loader-cli # bin-load + formatting helpers
cargo clippy --workspace --all-targets
cargo fmt --check
```

End-to-end against real hardware (an Arty A7 flashed with the tamal bitstream over
the FTDI USB-UART) is a manual smoke test outside CI:
`tamal-loader run prog.bin --port /dev/tty.usbserial-XXXX`.

## 11. Out of scope (later specs / plans)

- The **FX3 USB transport** backend (a second `Transport` impl).
- **Additional control commands** (`SET_CONFIG`-over-control, `ABORT`, `PING`) —
  wire-format opcodes `0x03`–`0x7F` reserved.
- **ACK/NAK / handshaking**, an FPGA→host **error frame**, and **live streaming
  trace** — v1 is fire-and-forget, ring-drained-on-HALT.
- **The verdict / conformance engine** (Phase 4) — the loader reports the raw
  HALT/TRAP terminator; pass/fail policy is a later layer.
- **JSON / machine-readable output** and a **`ports`-listing** subcommand.

## 12. Prior art

- **[mole](https://github.com/felipebalbi/mole)** — the sibling rig; its host↔
  device loader/drain split motivates this transport-agnostic byte framing that
  ships bytecode down and streams the trace back with the semantics above the
  link.
- **The wire-format design** (`.../2026-07-02-tamal-wire-format-design.md`) — this
  host loader is the Rust realization of the contract that spec defined and, in
  its §2/§14, explicitly deferred to post-silicon host tooling.
- **The HDL loader design** (`.../2026-07-02-tamal-loader-design.md`) — the FPGA
  end of the identical link; this host loader must reproduce its byte contract
  exactly.
- **COBS** — Cheshire & Baker, *"Consistent Overhead Byte Stuffing"* (1999); and
  **CRC-8/SMBus (poly `0x07`)** — the eSPI PEC CRC already in `Tamal.Crc`.
