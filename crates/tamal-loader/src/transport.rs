//! Pluggable transports. v1 ships a UART (`serialport`) backend; the FX3 USB
//! backend slots in later as another `Transport` impl.

use std::io::{Read, Write};
use std::time::Duration;
use std::time::Instant;

use tamal_abi::wire::DELIMITER;

/// Why a transport operation failed.
#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    /// The port could not be opened.
    #[error("failed to open transport: {0}")]
    Open(String),
    /// An underlying I/O error.
    #[error("transport I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// No complete frame arrived within the deadline.
    #[error("timed out waiting for a frame")]
    Timeout,
}

/// A byte pipe to the rig. Backends frame their own reads (UART delimits on 0x00).
pub trait Transport {
    /// Send all bytes to the device.
    fn send(&mut self, bytes: &[u8]) -> Result<(), TransportError>;

    /// Read one frame — bytes up to and including the next `0x00` delimiter —
    /// or `Timeout` if none arrives within `timeout`.
    fn read_frame(&mut self, timeout: Duration) -> Result<Vec<u8>, TransportError>;
}

/// Feed one received byte into a frame accumulator. Returns the complete frame
/// (including the trailing delimiter) once a `0x00` closes a non-empty frame;
/// leading lone delimiters are skipped as resync anchors.
pub(crate) fn push_frame_byte(acc: &mut Vec<u8>, byte: u8) -> Option<Vec<u8>> {
    if byte == DELIMITER {
        if acc.is_empty() {
            None
        } else {
            acc.push(byte);
            Some(std::mem::take(acc))
        }
    } else {
        acc.push(byte);
        None
    }
}

/// A UART transport over a `serialport` link.
pub struct UartTransport {
    port: Box<dyn serialport::SerialPort>,
}

impl UartTransport {
    /// Open `path` at `baud` (8N1). A short per-read timeout lets `read_frame`
    /// poll its overall deadline.
    pub fn open(path: &str, baud: u32) -> Result<Self, TransportError> {
        let port = serialport::new(path, baud)
            .timeout(Duration::from_millis(50))
            .open()
            .map_err(|e| TransportError::Open(e.to_string()))?;
        Ok(Self { port })
    }
}

impl Transport for UartTransport {
    fn send(&mut self, bytes: &[u8]) -> Result<(), TransportError> {
        self.port.write_all(bytes)?;
        self.port.flush()?;
        Ok(())
    }

    fn read_frame(&mut self, timeout: Duration) -> Result<Vec<u8>, TransportError> {
        let deadline = Instant::now() + timeout;
        let mut acc = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            if Instant::now() >= deadline {
                return Err(TransportError::Timeout);
            }
            match self.port.read(&mut byte) {
                Ok(0) => continue,
                Ok(_) => {
                    if let Some(frame) = push_frame_byte(&mut acc, byte[0]) {
                        return Ok(frame);
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => continue,
                Err(e) => return Err(TransportError::Io(e)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accumulates_until_delimiter() {
        let mut acc = Vec::new();
        assert_eq!(push_frame_byte(&mut acc, 0x03), None);
        assert_eq!(push_frame_byte(&mut acc, 0x02), None);
        assert_eq!(push_frame_byte(&mut acc, 0x0E), None);
        assert_eq!(
            push_frame_byte(&mut acc, 0x00),
            Some(vec![0x03, 0x02, 0x0E, 0x00])
        );
        assert!(acc.is_empty(), "accumulator resets after a frame");
    }

    #[test]
    fn skips_leading_delimiters() {
        let mut acc = Vec::new();
        assert_eq!(
            push_frame_byte(&mut acc, 0x00),
            None,
            "leading delimiter is a resync no-op"
        );
        assert_eq!(push_frame_byte(&mut acc, 0x00), None);
        assert_eq!(push_frame_byte(&mut acc, 0x01), None);
        assert_eq!(push_frame_byte(&mut acc, 0x00), Some(vec![0x01, 0x00]));
    }
}
