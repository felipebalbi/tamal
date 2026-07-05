# tamal-abi

The **tamal ABI**: the transport-agnostic contract shared by the host tooling and
the FPGA gateware. Every type here is a **byte-exact Rust counterpart** to an HDL
module, so the two sides of the link cannot silently disagree.

Part of the [tamal](../../README.md) eSPI compliance rig. **MIT-licensed** (host
tooling); the gateware it mirrors lives under [`hdl/`](../../hdl) and is
CERN-OHL-P-2.0.

## What it provides

| Module | Purpose | Mirrors (HDL) |
|--------|---------|---------------|
| [`isa`] | The `Instr` type + total `encode`/`decode` for the 32-bit tamal instruction word (RV32I-*inspired*, not compatible). Little-endian `program_to_le_bytes`. | `Tamal.Isa` |
| [`config`] | The `SET_CONFIG` payload codec (`Role`/`IoMode`/`Sck`/`AlertSource`), plus the host-only `pack` direction. | `Tamal.Config` |
| [`crc8`] | CRC-8 fold (poly `0x07`, init `0x00`, MSB-first — eSPI/SMBus PEC). | `Tamal.Crc` |
| [`cobs`] | COBS framing (`cobs_encode`/`cobs_decode`); no `0x00` in output, delimiter added by the frame layer. | `Tamal.Wire.Cobs` |
| [`wire`] | The frame + message layer: `LOAD_PROGRAM`/`TRIGGER`/`TRACE_DRAIN` opcodes, `frame_encode`/`frame_decode`, `encode_control`, `decode_result`, and the `WireError` taxonomy. Frame = `COBS(opcode ++ payload ++ crc8) ++ 0x00`, little-endian. | `Tamal.Wire` |
| [`trace`] | Typed decode of the drained ring into `Trace { revision, records, halt }` (CAPTURE / MARK / HALT terminator). | engine `encodeRecord` (§7.2) |

## Where it sits

```
tamal-asm    ──► tamal-abi   (isa: encode instructions to bytecode)
tamal-loader ──► tamal-abi   (wire + trace: frame control, decode drains)
```

It depends only on `thiserror`. It has **no transport** dependency by design — no
`serialport`, no `std::io` in the wire types; transports live in
[`tamal-loader`](../tamal-loader).

## Example

```rust
use tamal_abi::isa::{Instr, program_to_le_bytes};
use tamal_abi::wire::{encode_control, ControlMsg, decode_result};

// Assemble-then-frame: raw LE bytecode -> a LOAD_PROGRAM wire frame.
let prog = [Instr::CsAssert, Instr::Halt(0)];
let bytes = program_to_le_bytes(&prog);
let words: Vec<u32> = bytes.chunks_exact(4)
    .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
    .collect();
let frame = encode_control(&ControlMsg::LoadProgram(words));   // COBS+CRC+delimiter

// Result plane: unpack a TRACE_DRAIN frame back into ring words.
let _ = decode_result(&frame); // (Err here: `frame` is a control frame, not a drain)
```

## Status

Complete and property-tested (round-trips, golden vectors, and single-byte-flip
corruption). The COBS/CRC/frame/trace codecs are cross-checked against the HDL by
parallel implementation; the ISA mirror is the same encode/decode the assembler and
engine use.

## See also

- Wire contract: [`docs/superpowers/specs/2026-07-02-tamal-wire-format-design.md`](../../docs/superpowers/specs/2026-07-02-tamal-wire-format-design.md)
- ISA/ABI design: [`docs/superpowers/specs/2026-07-03-tamal-abi-isa-design.md`](../../docs/superpowers/specs/2026-07-03-tamal-abi-isa-design.md)

[`isa`]: https://docs.rs/tamal-abi/latest/tamal_abi/isa/
[`config`]: https://docs.rs/tamal-abi/latest/tamal_abi/config/
[`crc8`]: https://docs.rs/tamal-abi/latest/tamal_abi/crc8/
[`cobs`]: https://docs.rs/tamal-abi/latest/tamal_abi/cobs/
[`wire`]: https://docs.rs/tamal-abi/latest/tamal_abi/wire/
[`trace`]: https://docs.rs/tamal-abi/latest/tamal_abi/trace/
