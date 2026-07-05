//! Host-side connection to a tamal rig.
//!
//! This crate owns the live link to the FPGA. It drives the **control plane**
//! (`LOAD_PROGRAM` a compiled program, then `TRIGGER` a run) and ingests the
//! **result plane** (the drained trace ring), speaking the [`tamal_abi`] wire
//! format over a pluggable [`transport`]. Run configuration (role, I/O mode,
//! CRC, error injection) is baked into the bytecode via the engine's
//! `SET_CONFIG` instruction, not carried as separate control commands.
//!
//! The [`Device`] session ties it together: [`Device::load_program`],
//! [`Device::trigger`], [`Device::read_trace`], and [`Device::run`] — the
//! fire-and-forget load → trigger → drain cycle with a timeout + auto-retry
//! recovery loop. [`validate_program_bytes`] enforces the multiple-of-4 /
//! 1024-word limits before anything is sent.
//!
//! v1 targets the Arty A7's **USB-UART** ([`transport::UartTransport`]); the
//! [`transport::Transport`] trait is shaped so a future FX3 USB backend slots
//! in without touching [`tamal_abi`]. A pass/fail verdict layer over the
//! decoded trace is a later phase.

#![forbid(unsafe_code)]

/// Pluggable link layers between host and device (UART today; FX3 later).
pub mod transport;

/// Loader error taxonomy and program-input validation.
pub mod error;

/// The [`Device`] session: frame + ship control messages, read + decode drains.
pub mod device;

pub use device::{Device, RunOptions};
pub use error::{Error, MAX_PROGRAM_WORDS, validate_program_bytes};
