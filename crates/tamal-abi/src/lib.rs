//! The tamal ABI: everything shared by the host tooling and the FPGA gateware.
//!
//! This crate is the project ABI. It defines two things:
//!
//! 1. The **bytecode / ISA encoding** ([`isa`]) — the 32-bit instruction words
//!    the [`tamal-asm`](../tamal_asm/index.html) assembler emits and the FPGA
//!    engine executes.
//! 2. The **wire format** exchanged with a running rig: the **control plane**
//!    (host → FPGA) and the **result/trace plane** (FPGA → host).
//!
//! It is deliberately **transport-agnostic** — it knows nothing about UART,
//! JTAG, or a future FX3 USB link. Transports live in `tamal-loader`.
//!
//! Nothing here is implemented yet; the modules below sketch the intended shape.

#![forbid(unsafe_code)]

/// The tamal instruction encoding.
///
/// The tamal engine's ISA is **inspired by — but not 100% compatible with — the
/// RISC-V 32-bit (RV32I) ISA**. Borrowed: 32-bit fixed-width instruction words;
/// a 32-entry register file `x0`..`x31` with `x0` hardwired to zero; the
/// R/I/S/B/U/J format shapes. Diverged: tamal repurposes/extends the opcode
/// space with eSPI bus operations (drive/sample cycles, per-channel ops,
/// deterministic timing, compile-time error injection, capture/verdict), so
/// tamal bytecode is **not** interchangeable with a stock RISC-V toolchain.
pub mod isa {
    //! Placeholder — instruction word layout, opcode/format tables, and the
    //! encode/decode helpers land here.
}

/// Control-plane messages: host → device.
///
/// Planned commands (see the architecture docs):
/// `LOAD_PROGRAM(bytecode)`, `SET_ROLE(controller|target)`,
/// `SET_IO_MODE(single|dual|quad)`, `SET_CRC(on|off)`,
/// `SET_INJECT(seed, ratio)`, `ARM`, `TRIGGER`.
pub mod control {
    //! Placeholder — control command/response types land here.
}

/// Result / trace-plane events: device → host.
///
/// Planned event shape:
/// `BusEvent { timestamp, channel, cycle_type, tag, length, verdict }`.
/// The stream must tolerate dropped events (an overflow marker) so the eSPI bus
/// is never blocked by trace backpressure.
pub mod trace {
    //! Placeholder — observed-transaction and verdict types land here.
}
