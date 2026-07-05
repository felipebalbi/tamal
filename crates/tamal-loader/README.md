# tamal-loader

The **host-side loader**: it takes the raw bytecode [`tamal-asm`](../tamal-asm)
emits, frames it for the rig (COBS + CRC-8 via [`tamal-abi`](../tamal-abi)), ships
`LOAD_PROGRAM` + `TRIGGER` over a transport, reads back the `TRACE_DRAIN`, and
decodes it into a typed trace.

Part of the [tamal](../../README.md) eSPI compliance rig. **MIT-licensed.** The
command-line front-end is [`tamal-loader-cli`](../tamal-loader-cli).

## What it provides

- [`Transport`] — a byte-pipe trait (`send` + `read_frame`). [`UartTransport`]
  is the v1 backend (over [`serialport`](https://crates.io/crates/serialport));
  the trait is shaped so a future FX3 USB backend drops in without touching
  [`tamal-abi`].
- [`Device`]`<T: Transport>` — the session:
  - [`load_program`](Device::load_program) / [`trigger`](Device::trigger) — send the two control frames.
  - [`read_trace`](Device::read_trace) — read a frame → `decode_result` → `decode_trace` → a typed [`Trace`](tamal_abi::trace::Trace).
  - [`run`](Device::run) — the whole **fire-and-forget** cycle (load → trigger → drain) with a timeout + auto-retry loop ([`RunOptions`]). A lost or CRC-failed drain re-runs deterministically; a genuine I/O fault propagates.
- [`validate_program_bytes`] — enforces the wire/loader limits (length a multiple
  of 4; ≤ [`MAX_PROGRAM_WORDS`] = 1024) before anything is sent.
- [`Error`] — a `thiserror` taxonomy over transport / wire / trace failures plus
  the program-size and retries-exhausted cases.

Run configuration (role, I/O mode, CRC, error injection) is **baked into the
bytecode** via the engine's `SET_CONFIG` instruction — it is not a separate
control command. A pass/fail verdict layer over the decoded trace is a later phase.

## Example

```rust
use std::time::Duration;
use tamal_loader::{Device, RunOptions, validate_program_bytes};
use tamal_loader::transport::UartTransport;

let bytes = std::fs::read("prog.bin")?;
let words = validate_program_bytes(&bytes)?;                     // length/cap check

let transport = UartTransport::open("/dev/tty.usbserial-XXXX", 2_000_000)?;
let mut device = Device::new(transport);

let trace = device.run(&words, RunOptions { timeout: Duration::from_secs(5), retries: 3 })?;
println!("revision {:?}, {} records, halt {:?}",
         trace.revision, trace.records.len(), trace.halt);
# Ok::<(), tamal_loader::Error>(())
```

## Where it sits

```
tamal-loader ──► tamal-abi   (wire: frame control, decode drains + trace)
             └──► serialport (UART backend)
```

## Status

The framing/orchestration is complete and tested against a mock "fake FPGA"
[`Transport`] (happy path, retry-on-timeout, retry-on-corrupt-drain,
retries-exhausted, program-size limits). The live [`UartTransport`] byte path is
exercised on real hardware (an Arty A7 over USB-UART), not in CI — a serial
loopback/hardware smoke test is the recommended next coverage step.

## See also

- Host-loader design: [`docs/superpowers/specs/2026-07-04-tamal-loader-host-design.md`](../../docs/superpowers/specs/2026-07-04-tamal-loader-host-design.md)
- The gateware's matching load/drain FSM: [`docs/superpowers/specs/2026-07-02-tamal-loader-design.md`](../../docs/superpowers/specs/2026-07-02-tamal-loader-design.md)

[`Transport`]: https://docs.rs/tamal-loader/latest/tamal_loader/transport/trait.Transport.html
[`UartTransport`]: https://docs.rs/tamal-loader/latest/tamal_loader/transport/struct.UartTransport.html
[`Device`]: https://docs.rs/tamal-loader/latest/tamal_loader/struct.Device.html
[`RunOptions`]: https://docs.rs/tamal-loader/latest/tamal_loader/struct.RunOptions.html
[`validate_program_bytes`]: https://docs.rs/tamal-loader/latest/tamal_loader/fn.validate_program_bytes.html
[`MAX_PROGRAM_WORDS`]: https://docs.rs/tamal-loader/latest/tamal_loader/constant.MAX_PROGRAM_WORDS.html
[`Error`]: https://docs.rs/tamal-loader/latest/tamal_loader/enum.Error.html
