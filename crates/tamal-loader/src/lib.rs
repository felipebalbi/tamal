//! Host-side connection to a tamal rig.
//!
//! This crate owns the live link to the FPGA: it drives the **control plane**
//! (load a compiled program, set role/IO-mode/CRC/injection, arm/trigger) and
//! ingests the **result plane** (the observed-transaction + verdict stream),
//! speaking the [`tamal_abi`] wire format over a pluggable transport.
//!
//! v1 targets the Arty A7's **USB-UART**; the [`transport`] module is shaped so
//! a future FX3 USB backend slots in without touching [`tamal_abi`].
//!
//! It implements the wire transport and the [`Device`] session over
//! [`tamal_abi`]; the result-plane drain and verdict handling land as the
//! engine grows.

#![forbid(unsafe_code)]

/// Pluggable link layers between host and device (UART today; FX3 later).
pub mod transport;

/// Loader error taxonomy and program-input validation.
pub mod error;

/// The [`Device`] session: frame + ship control messages, read + decode drains.
pub mod device;

pub use device::Device;
pub use error::{Error, MAX_PROGRAM_WORDS, validate_program_bytes};
