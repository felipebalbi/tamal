//! The `Device<T>` session: frame + ship control messages, read + decode drains.

use std::time::Duration;

use tamal_abi::trace::{Trace, decode_trace};
use tamal_abi::wire::{ControlMsg, decode_result, encode_control};

use crate::error::Error;
use crate::transport::{Transport, TransportError};

/// Knobs for [`Device::run`].
#[derive(Debug, Clone, Copy)]
pub struct RunOptions {
    /// Per-drain read deadline.
    pub timeout: Duration,
    /// Extra attempts after the first (total attempts = `retries + 1`).
    pub retries: u32,
}

/// A connected tamal rig over a transport `T`.
pub struct Device<T: Transport> {
    transport: T,
}

impl<T: Transport> Device<T> {
    /// Wrap a transport in a session.
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    /// Frame and send `LOAD_PROGRAM(words)`.
    pub fn load_program(&mut self, words: &[u32]) -> Result<(), Error> {
        let frame = encode_control(&ControlMsg::LoadProgram(words.to_vec()));
        self.transport.send(&frame)?;
        Ok(())
    }

    /// Frame and send `TRIGGER`.
    pub fn trigger(&mut self) -> Result<(), Error> {
        let frame = encode_control(&ControlMsg::Trigger);
        self.transport.send(&frame)?;
        Ok(())
    }

    /// Read the next frame, decode it as a `TRACE_DRAIN`, and parse the trace ring.
    pub fn read_trace(&mut self, timeout: Duration) -> Result<Trace, Error> {
        let wire = self.transport.read_frame(timeout)?;
        let words = decode_result(&wire)?;
        Ok(decode_trace(&words)?)
    }

    /// Load, trigger, and read the drain, re-running on a timed-out or malformed
    /// drain up to `opts.retries` extra times. Re-sends BOTH `LOAD_PROGRAM` and
    /// `TRIGGER` each attempt (a partial LOAD is committed only by TRIGGER).
    /// Genuine transport I/O faults propagate immediately.
    pub fn run(&mut self, words: &[u32], opts: RunOptions) -> Result<Trace, Error> {
        let attempts = opts.retries.saturating_add(1);
        for _ in 0..attempts {
            self.load_program(words)?;
            self.trigger()?;
            match self.read_trace(opts.timeout) {
                Ok(trace) => return Ok(trace),
                // Recoverable: a lost or malformed drain -> deterministic re-run.
                Err(Error::Transport(TransportError::Timeout))
                | Err(Error::Wire(_))
                | Err(Error::Trace(_)) => continue,
                // Non-recoverable (port/IO fault, etc.).
                Err(e) => return Err(e),
            }
        }
        Err(Error::RetriesExhausted { attempts })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::time::Duration;

    use tamal_abi::wire::{OP_TRACE_DRAIN, frame_encode, words_to_le_bytes};

    use crate::transport::TransportError;

    /// A scriptable in-memory "fake FPGA": records every `send`, and replays a
    /// queue of scripted `read_frame` results. Reused by Tasks 8 and 9.
    pub(super) struct MockTransport {
        pub sent: Vec<Vec<u8>>,
        pub responses: VecDeque<Result<Vec<u8>, TransportError>>,
    }

    impl MockTransport {
        pub fn new(responses: Vec<Result<Vec<u8>, TransportError>>) -> Self {
            Self {
                sent: Vec::new(),
                responses: responses.into(),
            }
        }
    }

    impl Transport for MockTransport {
        fn send(&mut self, bytes: &[u8]) -> Result<(), TransportError> {
            self.sent.push(bytes.to_vec());
            Ok(())
        }
        fn read_frame(&mut self, _timeout: Duration) -> Result<Vec<u8>, TransportError> {
            self.responses
                .pop_front()
                .unwrap_or(Err(TransportError::Timeout))
        }
    }

    /// Build a valid `TRACE_DRAIN` wire frame from ring words. Reused by Tasks 8–9.
    pub(super) fn drain_frame(words: &[u32]) -> Vec<u8> {
        let mut logical = vec![OP_TRACE_DRAIN];
        logical.extend_from_slice(&words_to_le_bytes(words));
        frame_encode(&logical)
    }

    #[test]
    fn load_and_trigger_emit_exact_wire_bytes() {
        let mut dev = Device::new(MockTransport::new(vec![]));
        let words = vec![0x0800_0064u32];
        dev.load_program(&words).unwrap();
        dev.trigger().unwrap();
        let sent = &dev.transport.sent;
        assert_eq!(sent[0], encode_control(&ControlMsg::LoadProgram(words)));
        assert_eq!(sent[1], vec![0x03, 0x02, 0x0E, 0x00]); // pinned TRIGGER frame
    }

    #[test]
    fn read_trace_decodes_a_good_drain() {
        // REVISION 0.1.0, CAPTURE nbits=8 byte=0x5A, HALT status=0.
        let words = vec![0x0001_0000u32, 0x0000_085A, 0xC000_0000];
        let mut dev = Device::new(MockTransport::new(vec![Ok(drain_frame(&words))]));
        let trace = dev.read_trace(Duration::from_secs(1)).unwrap();
        assert_eq!(trace.revision, tamal_abi::trace::Revision::EXPECTED);
        assert_eq!(trace.records.len(), 1);
        assert_eq!(trace.halt.status, 0);
        assert!(!trace.halt.trap);
    }

    #[test]
    fn read_trace_surfaces_a_corrupt_drain_as_wire_error() {
        let words = vec![0x0001_0000u32, 0xC000_0000];
        let mut frame = drain_frame(&words);
        frame[2] ^= 0x01; // flip a payload byte -> CRC/COBS failure
        let mut dev = Device::new(MockTransport::new(vec![Ok(frame)]));
        assert!(matches!(
            dev.read_trace(Duration::from_secs(1)),
            Err(Error::Wire(_))
        ));
    }

    #[test]
    fn run_happy_path_returns_trace() {
        let words = vec![0x0001_0000u32, 0xC000_0000];
        let mut dev = Device::new(MockTransport::new(vec![Ok(drain_frame(&words))]));
        let opts = RunOptions {
            timeout: Duration::from_secs(1),
            retries: 3,
        };
        let trace = dev.run(&[0x0800_0064], opts).unwrap();
        assert_eq!(trace.halt.status, 0);
        // Two frames sent per attempt (LOAD + TRIGGER); one attempt here.
        assert_eq!(dev.transport.sent.len(), 2);
    }

    #[test]
    fn run_retries_on_timeout_then_succeeds() {
        let words = vec![0x0001_0000u32, 0xC000_0000];
        let mut dev = Device::new(MockTransport::new(vec![
            Err(TransportError::Timeout),
            Ok(drain_frame(&words)),
        ]));
        let opts = RunOptions {
            timeout: Duration::from_secs(1),
            retries: 3,
        };
        assert!(dev.run(&[0x0800_0064], opts).is_ok());
        // Re-sent LOAD + TRIGGER on the retry: 4 frames total.
        assert_eq!(dev.transport.sent.len(), 4);
    }

    #[test]
    fn run_retries_on_corrupt_drain_then_succeeds() {
        let words = vec![0x0001_0000u32, 0xC000_0000];
        let mut bad = drain_frame(&words);
        bad[2] ^= 0x01;
        let mut dev = Device::new(MockTransport::new(vec![Ok(bad), Ok(drain_frame(&words))]));
        let opts = RunOptions {
            timeout: Duration::from_secs(1),
            retries: 3,
        };
        assert!(dev.run(&[0x0800_0064], opts).is_ok());
    }

    #[test]
    fn run_exhausts_retries() {
        let mut dev = Device::new(MockTransport::new(vec![
            Err(TransportError::Timeout),
            Err(TransportError::Timeout),
        ]));
        let opts = RunOptions {
            timeout: Duration::from_secs(1),
            retries: 1,
        };
        assert!(matches!(
            dev.run(&[0x0800_0064], opts),
            Err(Error::RetriesExhausted { attempts: 2 })
        ));
    }
}
