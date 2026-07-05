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
//! Nothing here is implemented yet — these are placeholders for the structure.

#![forbid(unsafe_code)]

/// Pluggable link layers between host and device (UART today; FX3 later).
pub mod transport;

/// A connected tamal rig — control + result streams.
///
/// Placeholder type; the real connection lifecycle, program-load API, and
/// result subscription land in a later plan.
#[derive(Debug, Default)]
pub struct Device {
    _private: (),
}

impl Device {
    /// Create a placeholder handle. Real construction will take a transport.
    pub fn new() -> Self {
        Self::default()
    }
}
