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
/// RISC-V 32-bit (RV32I) ISA**. This module is a byte-for-byte Rust mirror of the
/// HDL `Tamal.Isa`: the [`isa::Instr`] type, checked operand newtypes, and total
/// [`isa::Instr::encode`]/[`isa::Instr::decode`].
pub mod isa;

/// Control-plane messages: host → device.
///
/// Planned commands (see the architecture docs):
/// `LOAD_PROGRAM(bytecode)`, `SET_ROLE(controller|target)`,
/// `SET_IO_MODE(single|dual|quad)`, `SET_CRC(on|off)`,
/// `SET_INJECT(seed, ratio)`, `ARM`, `TRIGGER`.
pub mod control {
    // Placeholder — control command/response types land here.
}

/// Result / trace-plane events: device → host.
///
/// Planned event shape:
/// `BusEvent { timestamp, channel, cycle_type, tag, length, verdict }`.
/// The stream must tolerate dropped events (an overflow marker) so the eSPI bus
/// is never blocked by trace backpressure.
pub mod trace {
    // Placeholder — observed-transaction and verdict types land here.
}
