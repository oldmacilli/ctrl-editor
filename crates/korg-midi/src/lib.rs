//! Platform-neutral MIDI transport boundary.
//!
//! Adapters own threads and OS callbacks.  The core only polls.

use std::collections::VecDeque;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PortId(pub String);

impl PortId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Input,
    Output,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortInfo {
    pub id: PortId,
    pub name: String,
    pub direction: Direction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MidiPacket {
    pub bytes: Vec<u8>,
    pub received_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportEvent {
    Packet(MidiPacket),
    PortsChanged,
    Disconnected { port: PortId },
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum TransportError {
    #[error("MIDI backend unavailable: {0}")]
    BackendUnavailable(String),
    #[error("MIDI port not found: {0:?}")]
    PortNotFound(PortId),
    #[error("MIDI port is blocked or unavailable — close the editor/DAW that owns it, or leave KORG Native Mode, then retry: {0:?}")]
    PortOpenFailed(PortId),
    #[error("MIDI send failed: {0}")]
    SendFailed(String),
    #[error("MIDI transport is not open")]
    NotOpen,
}

/// The MIDI backend a platform transport runs on, for the front ends: the
/// ALSA sequencer or JACK on Linux, CoreMIDI on macOS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendInfo {
    /// Stable id, saved in the preferences: `alsa`, `jack`, `coremidi`.
    pub id: String,
    /// Shown to the user: "ALSA sequencer", "JACK", "CoreMIDI".
    pub label: String,
    /// Why the backend cannot run now ("no JACK server answers …").
    pub problem: Option<String>,
}

/// Synchronous, non-blocking transport.  `poll` never blocks.
pub trait MidiTransport {
    fn list_ports(&mut self) -> Result<Vec<PortInfo>, TransportError>;
    fn open(&mut self, input: &PortId, output: &PortId) -> Result<(), TransportError>;
    fn close(&mut self);
    fn is_open(&self) -> bool;
    fn send(&mut self, message: &[u8]) -> Result<(), TransportError>;
    /// Drain buffered events into `out`.  Returns the number of events added.
    fn poll(&mut self, out: &mut Vec<TransportEvent>) -> usize;
    /// Block for at most `max_ms` until `poll` may have an event.  The
    /// default returns at once, which keeps scripted transports deterministic.
    fn wait(&mut self, max_ms: u64) {
        let _ = max_ms;
    }
}

/// In-memory transport for deterministic tests.
#[derive(Debug, Default)]
pub struct MockTransport {
    pub ports: Vec<PortInfo>,
    pub sent: Vec<Vec<u8>>,
    /// Events `poll` returns, in order: packets and port events.
    pub incoming: VecDeque<TransportEvent>,
    /// When set, `list_ports` fails with it, as a backend without a server.
    pub list_error: Option<TransportError>,
    open: bool,
}

impl MockTransport {
    pub fn new(ports: Vec<PortInfo>) -> Self {
        Self {
            ports,
            ..Default::default()
        }
    }

    pub fn push_packet(&mut self, bytes: impl Into<Vec<u8>>) {
        self.incoming.push_back(TransportEvent::Packet(MidiPacket {
            bytes: bytes.into(),
            received_at_ms: 0,
        }));
    }

    /// Queue a port event, for example a disconnect after a packet.
    pub fn push_event(&mut self, event: TransportEvent) {
        self.incoming.push_back(event);
    }
}

impl MidiTransport for MockTransport {
    fn list_ports(&mut self) -> Result<Vec<PortInfo>, TransportError> {
        match &self.list_error {
            Some(error) => Err(error.clone()),
            None => Ok(self.ports.clone()),
        }
    }

    fn open(&mut self, input: &PortId, output: &PortId) -> Result<(), TransportError> {
        let input_ok = self
            .ports
            .iter()
            .any(|port| &port.id == input && port.direction == Direction::Input);
        if !input_ok {
            return Err(TransportError::PortNotFound(input.clone()));
        }
        let output_ok = self
            .ports
            .iter()
            .any(|port| &port.id == output && port.direction == Direction::Output);
        if !output_ok {
            return Err(TransportError::PortNotFound(output.clone()));
        }
        self.open = true;
        Ok(())
    }

    fn close(&mut self) {
        self.open = false;
    }

    fn is_open(&self) -> bool {
        self.open
    }

    fn send(&mut self, message: &[u8]) -> Result<(), TransportError> {
        if !self.open {
            return Err(TransportError::NotOpen);
        }
        self.sent.push(message.to_vec());
        Ok(())
    }

    fn poll(&mut self, out: &mut Vec<TransportEvent>) -> usize {
        let mut count = 0;
        while let Some(event) = self.incoming.pop_front() {
            out.push(event);
            count += 1;
        }
        count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ports() -> Vec<PortInfo> {
        vec![
            PortInfo {
                id: PortId::new("in"),
                name: "Input".into(),
                direction: Direction::Input,
            },
            PortInfo {
                id: PortId::new("out"),
                name: "Output".into(),
                direction: Direction::Output,
            },
        ]
    }

    #[test]
    fn a_blocked_port_names_the_likely_owner() {
        let error = TransportError::PortOpenFailed(PortId::new("nanoKEY"));
        let text = error.to_string();
        assert!(text.contains("blocked"), "{text}");
        assert!(text.contains("KORG Native Mode"), "{text}");
    }

    #[test]
    fn mock_round_trips_packets() {
        let mut transport = MockTransport::new(ports());
        transport
            .open(&PortId::new("in"), &PortId::new("out"))
            .unwrap();
        transport.push_packet([0xF0, 0x42, 0xF7]);
        let mut events = Vec::new();
        assert_eq!(transport.poll(&mut events), 1);
        assert_eq!(
            events,
            vec![TransportEvent::Packet(MidiPacket {
                bytes: vec![0xF0, 0x42, 0xF7],
                received_at_ms: 0,
            })]
        );
    }

    #[test]
    fn open_rejects_unknown_port() {
        let mut transport = MockTransport::new(ports());
        assert_eq!(
            transport.open(&PortId::new("missing"), &PortId::new("out")),
            Err(TransportError::PortNotFound(PortId::new("missing")))
        );
    }

    #[test]
    fn send_requires_open() {
        let mut transport = MockTransport::new(ports());
        assert_eq!(transport.send(&[0xF0]), Err(TransportError::NotOpen));
    }
}
