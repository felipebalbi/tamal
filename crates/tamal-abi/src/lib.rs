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

/// The `SET_CONFIG` payload codec (`Role`/`IoMode`/`Sck`/`AlertSource`).
///
/// A Rust mirror of the HDL `Tamal.Config`, plus the host-only [`config::Config::pack`]
/// direction the gateware never needs.
pub mod config;

/// CRC-8 (eSPI/SMBus PEC) — a Rust mirror of the HDL `Tamal.Crc`.
pub mod crc8;

/// COBS framing (spec §5) — a Rust mirror of the HDL `Tamal.Wire.Cobs`.
pub mod cobs;

/// The frame + message wire layer (spec §8) — a Rust mirror of the HDL `Tamal.Wire`.
///
/// Control plane (host → FPGA): `LOAD_PROGRAM` + `TRIGGER`. Result plane
/// (FPGA → host): the `TRACE_DRAIN` frame. Frame = `COBS(opcode ++ payload ++
/// crc8) ++ 0x00`, little-endian throughout.
pub mod wire;

/// Typed decode of the drained trace ring (engine §7.2 record encodings):
/// `REVISION` word, the CAPTURE/MARK record stream, and the HALT terminator.
pub mod trace;
