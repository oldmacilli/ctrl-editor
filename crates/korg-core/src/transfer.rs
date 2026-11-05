//! SysEx transfer engine.
//!
//! The engine is synchronous and deterministic.  A `Clock` supplies time and a
//! `MidiTransport` supplies bytes.  The caller drives it with `poll`.
//!
//! Outgoing bytes are built here, from a `SysexDescriptor`, never by the UI.
//! A descriptor with an unknown value refuses the transfer
//! (`TransferError::DescriptorIncomplete`).  See
//! `docs/rewrite/10-rust-slint-architecture.md` §1.4.

use korg_midi::{MidiTransport, PortId, TransportError, TransportEvent};
use korg_model::SysexDescriptor;
use korg_sysex::{
    build_data_message, classify, unpack_7in8, AckStatus, Assembler, DataMessage, LengthStyle,
    Phase, PhaseFailure, Reply, SysexError, DATA_MARKER,
};
use std::cell::Cell;
use std::collections::VecDeque;
use std::time::Instant;
use thiserror::Error;

/// Monotonic milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Millis(pub u64);

pub trait Clock {
    fn now(&self) -> Millis;
}

#[derive(Debug, Clone)]
pub struct SystemClock {
    origin: Instant,
}

impl Default for SystemClock {
    fn default() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}

impl SystemClock {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Millis {
        Millis(self.origin.elapsed().as_millis() as u64)
    }
}

/// Clock for tests.  Time moves only through `advance`.
#[derive(Debug, Default)]
pub struct ManualClock {
    now: Cell<u64>,
}

impl ManualClock {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn advance(&self, millis: u64) {
        self.now.set(self.now.get() + millis);
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Millis {
        Millis(self.now.get())
    }
}

/// Which data a transfer moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferKind {
    SceneData,
    SceneSet,
    GlobalData,
}

impl TransferKind {
    /// Key of the data command in `SysexDescriptor::commands`.
    pub const fn key(self) -> &'static str {
        match self {
            TransferKind::SceneData => "scene_data",
            TransferKind::SceneSet => "scene_set",
            TransferKind::GlobalData => "global_data",
        }
    }

    /// Key of the commit command that ends a write.
    pub const fn write_key(self) -> &'static str {
        match self {
            TransferKind::SceneData => "scene_data_write",
            TransferKind::SceneSet => "scene_set_write",
            TransferKind::GlobalData => "global_data_write",
        }
    }

    /// Key of the command that asks the device for its data.
    pub const fn request_key(self) -> &'static str {
        match self {
            TransferKind::SceneData => "scene_data_request",
            TransferKind::SceneSet => "scene_set_request",
            TransferKind::GlobalData => "global_data_request",
        }
    }
}

/// Key of the mode-check command in `SysexDescriptor::commands`.
pub const MODE_CHECK_KEY: &str = "mode_check";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferDirection {
    /// Ask the device for `expected_len` unpacked bytes.
    Receive { expected_len: usize },
    /// Send `payload` and make the device store it.
    Write { payload: Vec<u8> },
}

/// What the caller asks for.  `build_plan` turns it into bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferRequest {
    pub kind: TransferKind,
    pub direction: TransferDirection,
    pub channel: u8,
    pub timeout_ms: u64,
}

/// What a step waits for after its message is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expect {
    /// An acknowledgement with the status that `Phase::expected` names.
    Ack,
    /// A data reply that unpacks to `len` bytes.
    Data { len: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub phase: Phase,
    pub message: Vec<u8>,
    pub expect: Expect,
}

/// The outgoing messages of one transfer.  Only `build_plan` creates one, so
/// every plan comes from a complete descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferPlan {
    kind: TransferKind,
    model_id: u32,
    length_style: LengthStyle,
    steps: Vec<Step>,
    timeout_ms: u64,
}

impl TransferPlan {
    pub fn kind(&self) -> TransferKind {
        self.kind
    }

    pub fn model_id(&self) -> u32 {
        self.model_id
    }

    pub fn steps(&self) -> &[Step] {
        &self.steps
    }

    pub fn is_receive(&self) -> bool {
        self.steps
            .iter()
            .any(|step| matches!(step.expect, Expect::Data { .. }))
    }
}

/// Build the messages for `request` from the device descriptor.
///
/// Step order is the engine's current reading of `docs/midi-sysex.md`:
/// write = mode check, data, commit; receive = mode check, request.  That
/// order is inference (review 12, issue 4).  The mode-check, commit and
/// request messages carry an empty payload; their payload is not recovered
/// either.  Both stay safe only because every command byte must come from the
/// descriptor, and no shipped descriptor is complete yet.
pub fn build_plan(
    class: &str,
    descriptor: Option<&SysexDescriptor>,
    request: &TransferRequest,
) -> Result<TransferPlan, TransferError> {
    let missing = |field: &str| TransferError::DescriptorIncomplete {
        model: class.to_owned(),
        field: field.to_owned(),
    };
    let descriptor = descriptor.ok_or_else(|| missing("descriptor"))?;
    let model_id = descriptor.model_id.ok_or_else(|| missing("model_id"))?.0;
    let length_style = match descriptor.length_style.as_deref() {
        Some("short_or_long") => LengthStyle::ShortOrLong,
        Some("fixed3") => LengthStyle::Fixed3,
        _ => return Err(missing("length_style")),
    };
    let command = |key: &str| {
        descriptor
            .commands
            .get(key)
            .copied()
            .flatten()
            .ok_or_else(|| missing(key))
    };
    let message = |command: u8, payload: &[u8]| {
        build_data_message(
            &DataMessage::new(
                model_id,
                request.channel,
                command,
                DATA_MARKER,
                length_style,
            ),
            payload,
        )
        .map_err(TransferError::Encode)
    };

    let mode_check = Step {
        phase: Phase::ModeCheck,
        message: message(command(MODE_CHECK_KEY)?, &[])?,
        expect: Expect::Ack,
    };
    let steps = match &request.direction {
        TransferDirection::Write { payload } => {
            if payload.is_empty() {
                return Err(TransferError::EmptyPayload);
            }
            let data = command(request.kind.key())?;
            let commit = command(request.kind.write_key())?;
            vec![
                mode_check,
                Step {
                    phase: Phase::Data,
                    message: message(data, payload)?,
                    expect: Expect::Ack,
                },
                Step {
                    phase: Phase::Write,
                    message: message(commit, &[])?,
                    expect: Expect::Ack,
                },
            ]
        }
        TransferDirection::Receive { expected_len } => {
            if *expected_len == 0 {
                return Err(TransferError::EmptyPayload);
            }
            let ask = command(request.kind.request_key())?;
            vec![
                mode_check,
                Step {
                    phase: Phase::Data,
                    message: message(ask, &[])?,
                    expect: Expect::Data { len: *expected_len },
                },
            ]
        }
    };
    Ok(TransferPlan {
        kind: request.kind,
        model_id,
        length_style,
        steps,
        timeout_ms: request.timeout_ms,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferState {
    Idle,
    Sending(Phase),
    Waiting(Phase),
    Done,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferEvent {
    None,
    Sent(Phase),
    Ack(Phase),
    /// A write finished.
    Done,
    /// A receive finished with the unpacked payload.
    Received(Vec<u8>),
    Failed(TransferError),
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum TransferError {
    #[error("a transfer is already active")]
    Busy,
    #[error("{model}: SysEx descriptor has no value for {field}")]
    DescriptorIncomplete { model: String, field: String },
    #[error("the payload layout for {0:?} over SysEx is not known")]
    LayoutUnknown(TransferKind),
    #[error("the payload size for {0:?} is not known")]
    SizeUnknown(TransferKind),
    #[error("a transfer needs a non-empty payload")]
    EmptyPayload,
    #[error("message could not be encoded: {0}")]
    Encode(SysexError),
    #[error("the device did not reply before the timeout")]
    NoReply,
    #[error("the device is not supported by this mode check")]
    UnsupportedDevice,
    #[error("the device rejected the data")]
    Nak,
    #[error("the device reported a write error")]
    WriteError,
    #[error("reply model id {actual:#06x} does not match {expected:#06x}")]
    UnexpectedModel { expected: u32, actual: u32 },
    #[error("unexpected reply")]
    UnexpectedReply,
    #[error("transport failed: {0}")]
    Transport(TransportError),
    #[error("reply is not valid SysEx: {0}")]
    BadReply(SysexError),
}

impl From<PhaseFailure> for TransferError {
    fn from(failure: PhaseFailure) -> Self {
        match failure {
            PhaseFailure::UnsupportedDevice => TransferError::UnsupportedDevice,
            PhaseFailure::Nak => TransferError::Nak,
            PhaseFailure::WriteError => TransferError::WriteError,
        }
    }
}

/// A port event the engine read from the transport.  The engine reads the
/// transport on every poll, so it keeps these for the controller instead of
/// dropping them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortNotice {
    Changed,
    Disconnected(PortId),
}

pub struct TransferEngine<T: MidiTransport, C: Clock> {
    transport: T,
    clock: C,
    plan: Option<TransferPlan>,
    step: usize,
    state: TransferState,
    deadline: Option<Millis>,
    assembler: Assembler,
    /// Complete messages not yet handled.  One transport poll can deliver
    /// several; they wait here so none is dropped.
    inbox: VecDeque<Vec<u8>>,
    notices: Vec<PortNotice>,
    /// A port disconnected during the current transfer.  Kept across polls,
    /// because the poll that reads it may return an earlier ack first.
    port_lost: bool,
}

impl<T: MidiTransport, C: Clock> TransferEngine<T, C> {
    pub fn new(transport: T, clock: C) -> Self {
        Self {
            transport,
            clock,
            plan: None,
            step: 0,
            state: TransferState::Idle,
            deadline: None,
            assembler: Assembler::default(),
            inbox: VecDeque::new(),
            notices: Vec::new(),
            port_lost: false,
        }
    }

    pub fn now(&self) -> Millis {
        self.clock.now()
    }

    #[cfg(test)]
    pub(crate) fn clock_for_tests(&self) -> &C {
        &self.clock
    }

    pub fn is_active(&self) -> bool {
        matches!(
            self.state,
            TransferState::Sending(_) | TransferState::Waiting(_)
        )
    }

    /// Port events read since the last call.
    pub fn take_port_notices(&mut self) -> Vec<PortNotice> {
        std::mem::take(&mut self.notices)
    }

    pub fn state(&self) -> TransferState {
        self.state
    }

    /// The plan of the current or last transfer.
    pub fn plan(&self) -> Option<&TransferPlan> {
        self.plan.as_ref()
    }

    pub fn transport(&self) -> &T {
        &self.transport
    }

    pub fn transport_mut(&mut self) -> &mut T {
        &mut self.transport
    }

    pub fn start(&mut self, plan: TransferPlan) -> Result<(), TransferError> {
        if self.is_active() {
            return Err(TransferError::Busy);
        }
        let Some(first) = plan.steps.first() else {
            return Err(TransferError::EmptyPayload);
        };
        self.state = TransferState::Sending(first.phase);
        self.plan = Some(plan);
        self.step = 0;
        self.assembler.clear();
        self.inbox.clear();
        self.deadline = None;
        self.port_lost = false;
        Ok(())
    }

    pub fn cancel(&mut self) {
        self.plan = None;
        self.step = 0;
        self.deadline = None;
        self.assembler.clear();
        self.inbox.clear();
        self.state = TransferState::Idle;
    }

    /// Advance the engine.  Call from a UI timer, also when no transfer runs:
    /// every call reads the transport, so port events are never lost and
    /// stray bytes do not pile up in the adapter.
    pub fn poll(&mut self) -> TransferEvent {
        self.pump();
        let disconnected = self.port_lost;
        match self.state {
            TransferState::Idle | TransferState::Done | TransferState::Failed => {
                // Nobody waits for these bytes.
                self.inbox.clear();
                TransferEvent::None
            }
            TransferState::Sending(phase) => {
                if disconnected {
                    return self.fail_disconnected();
                }
                let Some(step) = self
                    .plan
                    .as_ref()
                    .and_then(|plan| plan.steps.get(self.step))
                else {
                    return self.fail(TransferError::UnexpectedReply);
                };
                let message = step.message.clone();
                let timeout = self.plan.as_ref().map_or(0, |plan| plan.timeout_ms);
                // A reply to this message cannot exist yet; anything queued
                // belongs to the previous step.
                self.inbox.clear();
                match self.transport.send(&message) {
                    Ok(()) => {
                        self.state = TransferState::Waiting(phase);
                        self.deadline = Some(Millis(self.clock.now().0.saturating_add(timeout)));
                        TransferEvent::Sent(phase)
                    }
                    Err(error) => self.fail(TransferError::Transport(error)),
                }
            }
            TransferState::Waiting(phase) => {
                // Replies that arrived before a disconnect still count.
                while let Some(message) = self.inbox.pop_front() {
                    if let Some(event) = self.handle_message(phase, &message) {
                        return event;
                    }
                }
                if disconnected {
                    return self.fail_disconnected();
                }
                let Some(deadline) = self.deadline else {
                    return TransferEvent::None;
                };
                if self.clock.now() >= deadline {
                    return self.fail(TransferError::NoReply);
                }
                TransferEvent::None
            }
        }
    }

    /// Read every transport event.  Complete messages go to `inbox`, port
    /// events to `notices`.  A disconnect sets `port_lost`.
    fn pump(&mut self) {
        let mut events = Vec::new();
        self.transport.poll(&mut events);
        for event in events {
            match event {
                TransportEvent::Packet(packet) => {
                    self.inbox.extend(self.assembler.push(&packet.bytes));
                }
                TransportEvent::Disconnected { port } => {
                    self.port_lost = true;
                    self.notices.push(PortNotice::Disconnected(port));
                }
                TransportEvent::PortsChanged => self.notices.push(PortNotice::Changed),
            }
        }
    }

    fn fail_disconnected(&mut self) -> TransferEvent {
        self.fail(TransferError::Transport(
            TransportError::BackendUnavailable("device disconnected".to_owned()),
        ))
    }

    /// `None` means the message does not concern this step.
    fn handle_message(&mut self, phase: Phase, bytes: &[u8]) -> Option<TransferEvent> {
        let plan = self.plan.as_ref()?;
        let expected_model = plan.model_id;
        let expect = plan.steps.get(self.step)?.expect;
        let reply = match korg_sysex::parse_reply_with_style(bytes, Some(plan.length_style)) {
            Ok(reply) => reply,
            Err(error) => return Some(self.fail(TransferError::BadReply(error))),
        };
        match reply {
            Reply::Ack {
                model_id, status, ..
            } => {
                if model_id != expected_model {
                    return Some(self.fail(TransferError::UnexpectedModel {
                        expected: expected_model,
                        actual: model_id,
                    }));
                }
                match expect {
                    Expect::Ack => Some(match classify(phase, status) {
                        Ok(()) => self.advance(phase),
                        Err(failure) => self.fail(failure.into()),
                    }),
                    // While data is due, only a refusal ends the step.
                    Expect::Data { .. } if status == AckStatus::Nak => {
                        Some(self.fail(TransferError::Nak))
                    }
                    Expect::Data { .. } => None,
                }
            }
            Reply::Data {
                model_id,
                marker,
                packed,
                ..
            } => {
                let Expect::Data { len } = expect else {
                    return None;
                };
                if model_id != expected_model {
                    return Some(self.fail(TransferError::UnexpectedModel {
                        expected: expected_model,
                        actual: model_id,
                    }));
                }
                // `0x40` announces another transfer (`docs/midi-sysex.md`);
                // only `0x51` carries the data.
                if marker != DATA_MARKER {
                    return None;
                }
                Some(match unpack_7in8(&packed, len) {
                    Ok(payload) => {
                        self.state = TransferState::Done;
                        TransferEvent::Received(payload)
                    }
                    Err(error) => self.fail(TransferError::BadReply(error)),
                })
            }
            _ => None,
        }
    }

    fn advance(&mut self, phase: Phase) -> TransferEvent {
        let next = self
            .plan
            .as_ref()
            .and_then(|plan| plan.steps.get(self.step + 1))
            .map(|step| step.phase);
        match next {
            Some(next) => {
                self.step += 1;
                self.state = TransferState::Sending(next);
                TransferEvent::Ack(phase)
            }
            None => {
                self.state = TransferState::Done;
                TransferEvent::Done
            }
        }
    }

    fn fail(&mut self, error: TransferError) -> TransferEvent {
        self.state = TransferState::Failed;
        TransferEvent::Failed(error)
    }
}

/// Synthetic descriptor for tests.  The command bytes are made up and are
/// not device facts.
#[cfg(test)]
pub(crate) fn test_descriptor() -> SysexDescriptor {
    let commands = [
        (MODE_CHECK_KEY, 0x10),
        ("scene_data", 0x11),
        ("scene_data_write", 0x12),
        ("scene_data_request", 0x13),
        ("global_data", 0x14),
        ("global_data_write", 0x15),
        ("global_data_request", 0x16),
    ]
    .into_iter()
    .map(|(key, byte)| (key.to_owned(), Some(byte)))
    .collect();
    SysexDescriptor {
        model_id: Some(korg_model::SysexModelId(0x112)),
        length_style: Some("short_or_long".to_owned()),
        commands,
        source: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use korg_midi::{Direction, MockTransport, PortId, PortInfo};
    use korg_sysex::{build_ack, AckStatus, DATA_COMMAND};

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

    fn write_request() -> TransferRequest {
        TransferRequest {
            kind: TransferKind::SceneData,
            direction: TransferDirection::Write {
                payload: vec![1, 2, 3, 0x80],
            },
            channel: 0,
            timeout_ms: 1_000,
        }
    }

    fn receive_request(len: usize) -> TransferRequest {
        TransferRequest {
            kind: TransferKind::SceneData,
            direction: TransferDirection::Receive { expected_len: len },
            channel: 0,
            timeout_ms: 1_000,
        }
    }

    fn plan() -> TransferPlan {
        build_plan("X10830", Some(&test_descriptor()), &write_request()).unwrap()
    }

    fn engine() -> TransferEngine<MockTransport, ManualClock> {
        let mut transport = MockTransport::new(ports());
        transport
            .open(&PortId::new("in"), &PortId::new("out"))
            .unwrap();
        TransferEngine::new(transport, ManualClock::new())
    }

    fn ack(status: AckStatus) -> Vec<u8> {
        build_ack(0x112, 0, status).unwrap()
    }

    /// A KORG data reply as the device would send it.
    fn device_data(model_id: u32, payload: &[u8]) -> Vec<u8> {
        build_data_message(
            &DataMessage::new(
                model_id,
                0,
                DATA_COMMAND,
                DATA_MARKER,
                LengthStyle::ShortOrLong,
            ),
            payload,
        )
        .unwrap()
    }

    fn data_message() -> Vec<u8> {
        device_data(0x112, &[1, 2, 3])
    }

    fn incomplete_field(descriptor: Option<&SysexDescriptor>, request: &TransferRequest) -> String {
        match build_plan("X10830", descriptor, request) {
            Err(TransferError::DescriptorIncomplete { field, .. }) => field,
            other => panic!("expected DescriptorIncomplete, got {other:?}"),
        }
    }

    #[test]
    fn plan_refuses_each_missing_descriptor_value() {
        assert_eq!(incomplete_field(None, &write_request()), "descriptor");

        let mut no_model = test_descriptor();
        no_model.model_id = None;
        assert_eq!(
            incomplete_field(Some(&no_model), &write_request()),
            "model_id"
        );

        let mut no_style = test_descriptor();
        no_style.length_style = None;
        assert_eq!(
            incomplete_field(Some(&no_style), &write_request()),
            "length_style"
        );

        for key in [MODE_CHECK_KEY, "scene_data", "scene_data_write"] {
            let mut unknown = test_descriptor();
            unknown.commands.insert(key.to_owned(), None);
            assert_eq!(incomplete_field(Some(&unknown), &write_request()), key);
            let mut absent = test_descriptor();
            absent.commands.remove(key);
            assert_eq!(incomplete_field(Some(&absent), &write_request()), key);
        }

        let mut no_request = test_descriptor();
        no_request.commands.remove("scene_data_request");
        assert_eq!(
            incomplete_field(Some(&no_request), &receive_request(4)),
            "scene_data_request"
        );
    }

    #[test]
    fn shipped_descriptors_build_only_when_complete() {
        let catalog = korg_devices::sysex_catalog().unwrap();
        assert!(!catalog.devices.is_empty());
        for (class, descriptor) in &catalog.devices {
            for (request, keys) in [
                (
                    write_request(),
                    [MODE_CHECK_KEY, "scene_data", "scene_data_write"].as_slice(),
                ),
                (
                    receive_request(4),
                    [MODE_CHECK_KEY, "scene_data_request"].as_slice(),
                ),
            ] {
                let ready = keys
                    .iter()
                    .all(|key| descriptor.commands.get(*key).copied().flatten().is_some());
                let result = build_plan(class, Some(descriptor), &request);
                if ready {
                    assert!(
                        result.is_ok(),
                        "{class} has every required command and must build a plan"
                    );
                } else {
                    assert!(
                        matches!(result, Err(TransferError::DescriptorIncomplete { .. })),
                        "{class} must refuse when a required command is missing"
                    );
                }
            }
        }
    }

    #[test]
    fn plan_builds_framed_messages_from_the_descriptor() {
        let plan = plan();
        assert_eq!(plan.model_id(), 0x112);
        let commands: Vec<u8> = plan.steps().iter().map(|step| step.message[6]).collect();
        assert_eq!(commands, vec![0x10, 0x11, 0x12]);
        for step in plan.steps() {
            assert_eq!(&step.message[..6], &[0xF0, 0x42, 0x40, 0x00, 0x01, 0x12]);
            assert_eq!(*step.message.last().unwrap(), 0xF7);
            assert!(step.message[1..step.message.len() - 1]
                .iter()
                .all(|byte| *byte < 0x80));
        }
        assert!(!plan.is_receive());
    }

    #[test]
    fn plan_refuses_an_empty_payload() {
        let mut request = write_request();
        request.direction = TransferDirection::Write { payload: vec![] };
        assert_eq!(
            build_plan("X10830", Some(&test_descriptor()), &request),
            Err(TransferError::EmptyPayload)
        );
    }

    #[test]
    fn successful_transfer_uses_three_phases() {
        let mut engine = engine();
        engine.start(plan()).unwrap();

        assert_eq!(engine.poll(), TransferEvent::Sent(Phase::ModeCheck));
        engine
            .transport_mut()
            .push_packet(ack(AckStatus::DataReceived));
        assert_eq!(engine.poll(), TransferEvent::Ack(Phase::ModeCheck));

        assert_eq!(engine.poll(), TransferEvent::Sent(Phase::Data));
        engine
            .transport_mut()
            .push_packet(ack(AckStatus::DataReceived));
        assert_eq!(engine.poll(), TransferEvent::Ack(Phase::Data));

        assert_eq!(engine.poll(), TransferEvent::Sent(Phase::Write));
        engine
            .transport_mut()
            .push_packet(ack(AckStatus::WriteComplete));
        assert_eq!(engine.poll(), TransferEvent::Done);
        assert_eq!(engine.state(), TransferState::Done);
        assert_eq!(engine.transport().sent.len(), 3);
    }

    #[test]
    fn receive_returns_the_unpacked_payload() {
        let mut engine = engine();
        let plan = build_plan("X10830", Some(&test_descriptor()), &receive_request(9)).unwrap();
        assert!(plan.is_receive());
        engine.start(plan).unwrap();

        assert_eq!(engine.poll(), TransferEvent::Sent(Phase::ModeCheck));
        engine
            .transport_mut()
            .push_packet(ack(AckStatus::DataReceived));
        assert_eq!(engine.poll(), TransferEvent::Ack(Phase::ModeCheck));
        assert_eq!(engine.poll(), TransferEvent::Sent(Phase::Data));
        assert_eq!(engine.transport().sent[1][6], 0x13);

        let payload = vec![0x00, 0x7F, 0x80, 0xFF, 1, 2, 3, 4, 5];
        // An ack while data is due is not the answer; the data is.
        let mut bytes = ack(AckStatus::DataReceived);
        bytes.extend(device_data(0x112, &payload));
        engine.transport_mut().push_packet(bytes);
        assert_eq!(engine.poll(), TransferEvent::Received(payload));
        assert_eq!(engine.state(), TransferState::Done);
    }

    #[test]
    fn receive_fails_on_nak_short_data_and_wrong_model() {
        let start = |engine: &mut TransferEngine<MockTransport, ManualClock>| {
            let plan = build_plan("X10830", Some(&test_descriptor()), &receive_request(9)).unwrap();
            engine.start(plan).unwrap();
            engine.poll();
            engine
                .transport_mut()
                .push_packet(ack(AckStatus::DataReceived));
            engine.poll();
            engine.poll();
        };

        let mut nak = engine();
        start(&mut nak);
        nak.transport_mut().push_packet(ack(AckStatus::Nak));
        assert_eq!(nak.poll(), TransferEvent::Failed(TransferError::Nak));

        let mut short = engine();
        start(&mut short);
        short.transport_mut().push_packet(data_message());
        assert_eq!(
            short.poll(),
            TransferEvent::Failed(TransferError::BadReply(SysexError::Truncated))
        );

        let mut other = engine();
        start(&mut other);
        other
            .transport_mut()
            .push_packet(device_data(0x113, &[0; 9]));
        assert_eq!(
            other.poll(),
            TransferEvent::Failed(TransferError::UnexpectedModel {
                expected: 0x112,
                actual: 0x113
            })
        );
    }

    #[test]
    fn nak_fails_the_data_phase() {
        let mut engine = engine();
        engine.start(plan()).unwrap();
        engine.poll();
        engine
            .transport_mut()
            .push_packet(ack(AckStatus::DataReceived));
        engine.poll();
        engine.poll();
        engine.transport_mut().push_packet(ack(AckStatus::Nak));
        assert_eq!(engine.poll(), TransferEvent::Failed(TransferError::Nak));
        assert_eq!(engine.state(), TransferState::Failed);
    }

    #[test]
    fn wrong_mode_status_is_unsupported() {
        let mut engine = engine();
        engine.start(plan()).unwrap();
        engine.poll();
        engine
            .transport_mut()
            .push_packet(ack(AckStatus::WriteComplete));
        assert_eq!(
            engine.poll(),
            TransferEvent::Failed(TransferError::UnsupportedDevice)
        );
    }

    #[test]
    fn timeout_fails_without_a_reply() {
        let mut engine = engine();
        engine.start(plan()).unwrap();
        engine.poll();
        assert_eq!(engine.poll(), TransferEvent::None);
        engine.clock.advance(1_001);
        assert_eq!(engine.poll(), TransferEvent::Failed(TransferError::NoReply));
    }

    #[test]
    fn fragmented_reply_is_assembled() {
        let mut engine = engine();
        engine.start(plan()).unwrap();
        engine.poll();
        let reply = ack(AckStatus::DataReceived);
        engine.transport_mut().push_packet(&reply[..3]);
        assert_eq!(engine.poll(), TransferEvent::None);
        engine.transport_mut().push_packet(&reply[3..5]);
        assert_eq!(engine.poll(), TransferEvent::None);
        engine.transport_mut().push_packet(&reply[5..]);
        assert_eq!(engine.poll(), TransferEvent::Ack(Phase::ModeCheck));
    }

    #[test]
    fn ack_after_ignored_message_in_same_packet_is_handled() {
        let mut engine = engine();
        engine.start(plan()).unwrap();
        engine.poll();
        let mut bytes = data_message();
        bytes.extend(ack(AckStatus::DataReceived));
        engine.transport_mut().push_packet(bytes);
        assert_eq!(engine.poll(), TransferEvent::Ack(Phase::ModeCheck));
    }

    #[test]
    fn ack_in_second_packet_of_one_poll_is_handled() {
        let mut engine = engine();
        engine.start(plan()).unwrap();
        engine.poll();
        engine.transport_mut().push_packet(data_message());
        engine
            .transport_mut()
            .push_packet(ack(AckStatus::DataReceived));
        assert_eq!(engine.poll(), TransferEvent::Ack(Phase::ModeCheck));
    }

    #[test]
    fn stale_reply_does_not_leak_into_next_phase() {
        let mut engine = engine();
        engine.start(plan()).unwrap();
        engine.poll();
        let mut bytes = ack(AckStatus::DataReceived);
        bytes.extend(ack(AckStatus::DataReceived));
        engine.transport_mut().push_packet(bytes);
        assert_eq!(engine.poll(), TransferEvent::Ack(Phase::ModeCheck));
        assert_eq!(engine.poll(), TransferEvent::Sent(Phase::Data));
        assert_eq!(engine.poll(), TransferEvent::None);
    }

    #[test]
    fn idle_poll_keeps_port_events_and_drops_stray_bytes() {
        let mut engine = engine();
        engine
            .transport_mut()
            .push_packet(ack(AckStatus::DataReceived));
        engine
            .transport_mut()
            .push_event(TransportEvent::PortsChanged);
        assert_eq!(engine.poll(), TransferEvent::None);
        assert!(engine.transport().incoming.is_empty());
        assert_eq!(engine.take_port_notices(), vec![PortNotice::Changed]);
        assert!(engine.take_port_notices().is_empty());

        // A stray ack read while idle must not complete the next transfer.
        engine.start(plan()).unwrap();
        assert_eq!(engine.poll(), TransferEvent::Sent(Phase::ModeCheck));
        assert_eq!(engine.poll(), TransferEvent::None);
    }

    #[test]
    fn port_events_during_a_transfer_are_kept() {
        let mut engine = engine();
        engine.start(plan()).unwrap();
        engine.poll();
        engine
            .transport_mut()
            .push_event(TransportEvent::PortsChanged);
        engine
            .transport_mut()
            .push_packet(ack(AckStatus::DataReceived));
        assert_eq!(engine.poll(), TransferEvent::Ack(Phase::ModeCheck));
        assert_eq!(engine.take_port_notices(), vec![PortNotice::Changed]);
    }

    #[test]
    fn ack_before_disconnect_still_counts() {
        let mut engine = engine();
        engine.start(plan()).unwrap();
        engine.poll();
        engine
            .transport_mut()
            .push_packet(ack(AckStatus::DataReceived));
        engine
            .transport_mut()
            .push_event(TransportEvent::Disconnected {
                port: PortId::new("in"),
            });
        assert_eq!(engine.poll(), TransferEvent::Ack(Phase::ModeCheck));
        assert_eq!(
            engine.take_port_notices(),
            vec![PortNotice::Disconnected(PortId::new("in"))]
        );
        // The next step must not send into a port that is gone.
        assert!(matches!(
            engine.poll(),
            TransferEvent::Failed(TransferError::Transport(_))
        ));
        assert_eq!(engine.transport().sent.len(), 1);
    }

    #[test]
    fn wrong_model_is_rejected() {
        let mut engine = engine();
        engine.start(plan()).unwrap();
        engine.poll();
        let reply = build_ack(0x113, 0, AckStatus::DataReceived).unwrap();
        engine.transport_mut().push_packet(reply);
        assert_eq!(
            engine.poll(),
            TransferEvent::Failed(TransferError::UnexpectedModel {
                expected: 0x112,
                actual: 0x113
            })
        );
    }
}
