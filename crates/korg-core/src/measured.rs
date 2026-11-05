//! Measured receive exchange with a KORG device.
//!
//! The sequence and frames are measured from the official editor.  See
//! `docs/midi-sysex.md` and
//! `docs/specs/2026-10-20-measured-midi-and-jog-config.md`.

use crate::transfer::{Clock, Millis};
use korg_midi::{MidiTransport, PortId, TransportError, TransportEvent};
use korg_sysex::{
    ack_status, build_commit, build_request, build_write_frame, parse_data_reply, parse_identity,
    reply_model, AckStatus, Assembler, IdentityReply, Request, SysexError, IDENTITY_REQUEST,
};
use std::collections::VecDeque;
use thiserror::Error;

/// Per-model sizes for a receive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReceiveLayout {
    pub scene_count: u8,
    pub scene_len: usize,
    pub global_len: usize,
}

#[derive(Debug, Error)]
pub enum MeasuredError {
    #[error(transparent)]
    Transport(#[from] TransportError),
    #[error(transparent)]
    Sysex(#[from] SysexError),
    #[error("no reply within {0} ms")]
    Timeout(u64),
    #[error("the device disconnected")]
    Disconnected,
    #[error("reply model {0:#06x} does not match the identity reply")]
    UnexpectedModel(u32),
    #[error("mode check did not run")]
    NoIdentity,
    #[error("calibration is not available for model {0:#06x}")]
    CalibrationUnavailable(u32),
    #[error("scene index {0} is outside the layout")]
    SceneIndex(u8),
    #[error("unexpected acknowledgement {0:?}")]
    UnexpectedStatus(AckStatus),
}

/// Synchronous measured session.  It owns the transport and polls it.
pub struct MeasuredSession<T: MidiTransport, C: Clock> {
    transport: T,
    input: PortId,
    output: PortId,
    clock: C,
    timeout_ms: u64,
    device_id: Option<u8>,
    model_id: Option<u32>,
    assembler: Assembler,
    inbox: VecDeque<Vec<u8>>,
    /// Send the identity request before every exchange, as the official
    /// editor does.  Off, it is sent only until the device has answered once.
    check_every_exchange: bool,
}

impl<T: MidiTransport, C: Clock> MeasuredSession<T, C> {
    pub fn new(transport: T, input: PortId, output: PortId, clock: C, timeout_ms: u64) -> Self {
        Self {
            transport,
            input,
            output,
            clock,
            timeout_ms,
            device_id: None,
            model_id: None,
            assembler: Assembler::default(),
            inbox: VecDeque::new(),
            check_every_exchange: true,
        }
    }

    /// Skip the identity request once the device has answered.  Each one is a
    /// round trip, about 45 ms over Bluetooth.  Writes keep their explicit
    /// mode checks; this only changes `exchange`.
    pub fn check_identity_once(&mut self) {
        self.check_every_exchange = false;
    }

    pub fn open(&mut self) -> Result<(), MeasuredError> {
        self.transport.open(&self.input, &self.output)?;
        Ok(())
    }

    pub fn device_id(&self) -> Option<u8> {
        self.device_id
    }

    pub fn model_id(&self) -> Option<u32> {
        self.model_id
    }

    pub fn transport(&self) -> &T {
        &self.transport
    }

    pub fn transport_mut(&mut self) -> &mut T {
        &mut self.transport
    }

    /// Send the identity request and store the result.
    pub fn mode_check(&mut self) -> Result<IdentityReply, MeasuredError> {
        self.transport.send(&IDENTITY_REQUEST)?;
        let deadline = self.deadline();
        loop {
            let frame = self.next_frame(deadline)?;
            if let Ok(reply) = parse_identity(&frame) {
                self.device_id = Some(reply.device_id);
                self.model_id = Some(reply.model_id);
                return Ok(reply);
            }
        }
    }

    pub fn receive_scene_set(
        &mut self,
        layout: ReceiveLayout,
    ) -> Result<Vec<Vec<u8>>, MeasuredError> {
        self.exchange(Request::SceneSetInfo)?;
        let mut scenes = Vec::with_capacity(usize::from(layout.scene_count));
        for index in 0..layout.scene_count {
            scenes.push(self.receive_scene(index, layout.scene_len)?);
        }
        Ok(scenes)
    }

    /// Make scene `index` the device's active scene (`0x14`).  The device
    /// acknowledges with `0x23`.
    pub fn select_scene(&mut self, index: u8) -> Result<(), MeasuredError> {
        self.exchange(Request::SelectScene(index))?;
        Ok(())
    }

    pub fn receive_scene(&mut self, index: u8, scene_len: usize) -> Result<Vec<u8>, MeasuredError> {
        self.exchange(Request::SelectScene(index))?;
        let frame = self.exchange(Request::ReadBlock)?;
        Ok(parse_data_reply(&frame, scene_len)?)
    }

    pub fn receive_global(&mut self, global_len: usize) -> Result<Vec<u8>, MeasuredError> {
        self.exchange(Request::SceneSetInfo)?;
        let frame = self.exchange(Request::GlobalData)?;
        Ok(parse_data_reply(&frame, global_len)?)
    }

    /// Write every scene in order.  One commit per scene.
    pub fn write_scene_set(
        &mut self,
        scenes: &[Vec<u8>],
        header: [u8; 4],
    ) -> Result<(), MeasuredError> {
        if scenes.len() > usize::from(u8::MAX) {
            return Err(MeasuredError::SceneIndex(u8::MAX));
        }
        for (index, scene) in scenes.iter().enumerate() {
            self.write_scene(index as u8, scene, header)?;
        }
        Ok(())
    }

    /// Run one touch-pad calibration step (`0x1D` command).  The device
    /// answers with `0x23`; the sequence is 01, 03, 02, 04.
    pub fn calibrate(&mut self, command: u8) -> Result<(), MeasuredError> {
        if self.model_id.is_none() {
            self.mode_check()?;
        }
        self.check_calibration_model()?;
        self.exchange(Request::SceneSetInfo)?;
        self.mode_check()?;
        self.check_calibration_model()?;
        let device_id = self.device_id.ok_or(MeasuredError::NoIdentity)?;
        let model_id = self.model_id.ok_or(MeasuredError::NoIdentity)?;
        let data = korg_sysex::build_calibration_frame(device_id, model_id, command)?;
        self.transport.send(&data)?;
        let ack = self.next_frame(self.deadline())?;
        let status = ack_status(&ack)?;
        if status != AckStatus::DataReceived {
            return Err(MeasuredError::UnexpectedStatus(status));
        }
        Ok(())
    }

    fn check_calibration_model(&self) -> Result<(), MeasuredError> {
        let model = self.model_id.ok_or(MeasuredError::NoIdentity)?;
        if !korg_devices::legacy_calibration_matches_model(model) {
            return Err(MeasuredError::CalibrationUnavailable(model));
        }
        Ok(())
    }

    /// Write global data.  `header` is the measured four-byte data header.
    pub fn write_global(&mut self, payload: &[u8], header: [u8; 4]) -> Result<(), MeasuredError> {
        self.exchange(Request::SceneSetInfo)?;
        self.mode_check()?;
        let device_id = self.device_id.ok_or(MeasuredError::NoIdentity)?;
        let model_id = self.model_id.ok_or(MeasuredError::NoIdentity)?;
        let data = build_write_frame(device_id, model_id, header, payload)?;
        self.transport.send(&data)?;
        let ack = self.next_frame(self.deadline())?;
        let status = ack_status(&ack)?;
        if status != AckStatus::DataReceived {
            return Err(MeasuredError::UnexpectedStatus(status));
        }
        Ok(())
    }

    /// Write one scene.  `header` is the measured four-byte data header.
    pub fn write_scene(
        &mut self,
        index: u8,
        payload: &[u8],
        header: [u8; 4],
    ) -> Result<(), MeasuredError> {
        self.exchange(Request::SceneSetInfo)?;
        self.mode_check()?;
        let device_id = self.device_id.ok_or(MeasuredError::NoIdentity)?;
        let model_id = self.model_id.ok_or(MeasuredError::NoIdentity)?;

        let data = build_write_frame(device_id, model_id, header, payload)?;
        self.transport.send(&data)?;
        let ack = self.next_frame(self.deadline())?;
        let status = ack_status(&ack)?;
        if status != AckStatus::DataReceived {
            return Err(MeasuredError::UnexpectedStatus(status));
        }

        self.mode_check()?;
        let commit = build_commit(device_id, model_id, index)?;
        self.transport.send(&commit)?;
        let ack = self.next_frame(self.deadline())?;
        let status = ack_status(&ack)?;
        if status != AckStatus::WriteComplete {
            return Err(MeasuredError::UnexpectedStatus(status));
        }
        Ok(())
    }

    fn exchange(&mut self, request: Request) -> Result<Vec<u8>, MeasuredError> {
        if self.check_every_exchange || self.device_id.is_none() {
            self.mode_check()?;
        }
        let device_id = self.device_id.ok_or(MeasuredError::NoIdentity)?;
        let model_id = self.model_id.ok_or(MeasuredError::NoIdentity)?;
        let message = build_request(device_id, model_id, request)?;
        self.transport.send(&message)?;
        let frame = self.next_frame(self.deadline())?;
        let reply = reply_model(&frame)?;
        if reply != model_id {
            return Err(MeasuredError::UnexpectedModel(reply));
        }
        Ok(frame)
    }

    fn deadline(&self) -> Millis {
        Millis(self.clock.now().0 + self.timeout_ms)
    }

    fn next_frame(&mut self, deadline: Millis) -> Result<Vec<u8>, MeasuredError> {
        loop {
            if let Some(frame) = self.inbox.pop_front() {
                return Ok(frame);
            }
            let mut events = Vec::new();
            self.transport.poll(&mut events);
            for event in events {
                match event {
                    TransportEvent::Packet(packet) => {
                        for frame in self.assembler.push(&packet.bytes) {
                            self.inbox.push_back(frame);
                        }
                    }
                    TransportEvent::Disconnected { .. } => return Err(MeasuredError::Disconnected),
                    TransportEvent::PortsChanged => {}
                }
            }
            if let Some(frame) = self.inbox.pop_front() {
                return Ok(frame);
            }
            let now = self.clock.now();
            if now >= deadline {
                return Err(MeasuredError::Timeout(self.timeout_ms));
            }
            self.transport.wait(deadline.0 - now.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use korg_midi::{Direction, MockTransport, PortInfo};
    use korg_sysex::{pack_7in8, SYSEX_END, SYSEX_START};
    use std::cell::Cell;

    const MODEL: u32 = 0x136;
    const DEVICE: u8 = 0x0F;

    struct StepClock(Cell<u64>);

    impl Clock for StepClock {
        fn now(&self) -> Millis {
            let value = self.0.get();
            self.0.set(value + 10);
            Millis(value)
        }
    }

    fn ports() -> Vec<PortInfo> {
        vec![
            PortInfo {
                id: PortId::new("in"),
                name: "in".into(),
                direction: Direction::Input,
            },
            PortInfo {
                id: PortId::new("out"),
                name: "out".into(),
                direction: Direction::Output,
            },
        ]
    }

    fn session() -> MeasuredSession<MockTransport, StepClock> {
        MeasuredSession::new(
            MockTransport::new(ports()),
            PortId::new("in"),
            PortId::new("out"),
            StepClock(Cell::new(0)),
            100,
        )
    }

    fn identity() -> Vec<u8> {
        vec![
            SYSEX_START,
            0x7E,
            DEVICE,
            0x06,
            0x02,
            0x42,
            0x36,
            0x01,
            0x00,
            0x00,
            0x08,
            0x00,
            0x01,
            0x00,
            SYSEX_END,
        ]
    }

    #[test]
    fn calibration_never_sends_the_command_to_another_model() {
        let mut session = session();
        session.open().unwrap();
        let mut wrong_identity = identity();
        wrong_identity[6] = 0x37;
        session.transport_mut().push_packet(wrong_identity);
        assert!(matches!(
            session.calibrate(3),
            Err(MeasuredError::CalibrationUnavailable(0x137))
        ));
        assert_eq!(session.transport().sent, [IDENTITY_REQUEST.to_vec()]);
    }

    fn frame(body: &[u8], payload: &[u8]) -> Vec<u8> {
        let packed = pack_7in8(payload);
        let mut header = body.to_vec();
        // Data frames carry their packed length in the first three header
        // bytes; ack and info frames already carry theirs.
        if header.len() == 4 && matches!(header[3], 0x40 | 0x51) {
            let length = packed.len() + 1;
            header[0] = length as u8;
            header[1] = (length >> 7) as u8;
            header[2] = (length >> 14) as u8;
        }
        let mut out = vec![SYSEX_START, 0x42, 0x40 | DEVICE, 0x00, 0x01, 0x36];
        out.extend_from_slice(&header);
        out.extend_from_slice(&packed);
        out.push(SYSEX_END);
        out
    }

    #[test]
    fn reads_a_scene_set_from_scripted_replies() {
        let mut session = session();
        session.open().unwrap();
        let scenes = [vec![1u8; 10], vec![2u8; 10]];
        session.transport_mut().push_packet(identity());
        session
            .transport_mut()
            .push_packet(frame(&[0x02, 0x00, 0x00, 0x42, 0x00], &[]));
        for scene in &scenes {
            session.transport_mut().push_packet(identity());
            session
                .transport_mut()
                .push_packet(frame(&[0x01, 0x00, 0x00, 0x23], &[]));
            session.transport_mut().push_packet(identity());
            session
                .transport_mut()
                .push_packet(frame(&[0x01, 0x05, 0x00, 0x40], scene));
        }

        let out = session
            .receive_scene_set(ReceiveLayout {
                scene_count: 2,
                scene_len: 10,
                global_len: 10,
            })
            .unwrap();
        assert_eq!(out, scenes);
        assert_eq!(session.device_id(), Some(DEVICE));
        assert_eq!(session.model_id(), Some(MODEL));
    }

    #[test]
    fn reads_global_data() {
        let mut session = session();
        session.open().unwrap();
        session.transport_mut().push_packet(identity());
        session
            .transport_mut()
            .push_packet(frame(&[0x02, 0x00, 0x00, 0x42, 0x00], &[]));
        session.transport_mut().push_packet(identity());
        session
            .transport_mut()
            .push_packet(frame(&[0x14, 0x01, 0x00, 0x51], &[7u8; 10]));

        let out = session.receive_global(10).unwrap();
        assert_eq!(out, vec![7u8; 10]);
    }

    #[test]
    fn times_out_without_a_reply() {
        let mut session = session();
        session.open().unwrap();
        assert!(matches!(
            session.mode_check(),
            Err(MeasuredError::Timeout(100))
        ));
    }
    #[test]
    fn writes_a_scene_with_the_measured_sequence() {
        let mut session = session();
        session.open().unwrap();
        session.transport_mut().push_packet(identity());
        session
            .transport_mut()
            .push_packet(frame(&[0x02, 0x00, 0x00, 0x42, 0x00], &[]));
        session.transport_mut().push_packet(identity());
        session
            .transport_mut()
            .push_packet(frame(&[0x01, 0x00, 0x00, 0x23], &[]));
        session.transport_mut().push_packet(identity());
        session
            .transport_mut()
            .push_packet(frame(&[0x01, 0x00, 0x00, 0x21], &[]));

        let payload = [1u8, 2, 3];
        session
            .write_scene(1, &payload, korg_sysex::X14802_SCENE_WRITE_HEADER)
            .unwrap();
        let sent = &session.transport().sent;
        assert_eq!(sent.len(), 6);
        assert_eq!(
            sent[1],
            korg_sysex::build_request(DEVICE, MODEL, Request::SceneSetInfo).unwrap()
        );
        assert!(sent[3].starts_with(&[0xF0, 0x42, 0x4F, 0x00, 0x01, 0x36, 0x01, 0x05]));
        assert_eq!(sent[5], korg_sysex::build_commit(DEVICE, MODEL, 1).unwrap());
    }
}
