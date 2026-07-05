//! The `Device<T>` session: frame + ship control messages, read + decode drains.

use tamal_abi::wire::{ControlMsg, encode_control};

use crate::error::Error;
use crate::transport::Transport;

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
    #[allow(dead_code)] // exercised by the drain tests Tasks 8–9 add to this module.
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
}
