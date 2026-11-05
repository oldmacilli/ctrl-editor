//! Controller and view-model boundary for the UI.
//!
//! The UI sends `ControllerAction` values and reads `ViewModel` snapshots.
//! The controller owns the session and the transfer engine.

use crate::transfer::{
    build_plan, Clock, Millis, PortNotice, TransferDirection, TransferEngine, TransferError,
    TransferEvent, TransferKind, TransferRequest,
};
use crate::{Action as SessionAction, CoreError, EditorSession, Outcome as SessionOutcome};
use korg_devices::{jog_class_for_file_key, jog_layout, scene_jog_config};
use korg_midi::{MidiTransport, PortId, PortInfo, TransportError};
use korg_model::{
    ClassId, FileModelKey, GlobalData, JogConfig, JogError, ParameterKind, ParameterSchema,
    SceneSet, SysexDescriptor,
};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControllerAction {
    SelectScene(usize),
    SetSceneData {
        index: usize,
        data: Vec<u8>,
    },
    /// Set the parameter `ParamView::index` of `params`: the selected scene,
    /// or the global block when only global data is loaded.
    SetParameter {
        element: usize,
        index: usize,
        value: i32,
    },
    /// Set the parameter `ParamView::index` of `global_params`.
    SetGlobalParameter {
        element: usize,
        index: usize,
        value: i32,
    },
    Undo,
    Redo,
    MarkSaved,
    /// The device now holds this part (received, or written and verified).
    MarkReceived(crate::ReceivedPart),
    /// Put back what the device last held; one undo step.
    RevertToReceived,
    /// Ask the device for its data.  Needs `set_device`.
    Receive(TransferKind),
    /// Send the document's data to the device.  Needs `set_device`.
    Write(TransferKind),
    CancelTransfer,
    /// Replace the jog block in the selected scene.  `X14802` only.
    SetJog(JogConfig),
    /// Set one byte of the global block.
    SetGlobalByte {
        offset: usize,
        value: u8,
    },
    /// Write ASCII text into the global block at `offset`.
    SetGlobalText {
        offset: usize,
        text: String,
    },
    /// Set the BPM word (tenths) at its descriptor-defined offset.
    SetGlobalBpm(u16),
}

/// How often the UI should call `EditorController::tick`.
/// `docs/rewrite/10-rust-slint-architecture.md` §3.5.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TickRate {
    /// A transfer is active.
    Fast,
    /// No transfer; watch for hot-plug.
    Slow,
    /// No port open and port watch off.  Stop the timer.
    Idle,
}

impl TickRate {
    /// Timer interval in milliseconds.  `None` means stop the timer.
    pub const fn interval_ms(self) -> Option<u64> {
        match self {
            TickRate::Fast => Some(10),
            TickRate::Slow => Some(1_000),
            TickRate::Idle => None,
        }
    }
}

/// Interval between port scans.  The midir adapter reports no hot-plug
/// events, so the core compares port lists itself.
pub const PORT_SCAN_MS: u64 = 1_000;

/// The connected device as the transfer engine needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceTarget {
    pub class: ClassId,
    /// `None` when the catalog has no descriptor for this class.
    pub descriptor: Option<SysexDescriptor>,
    pub channel: u8,
    /// Catalog sizes, used to receive when no document is loaded.
    pub scene_payload_size: Option<usize>,
    pub global_payload_size: Option<usize>,
    pub timeout_ms: u64,
}

/// The controller accepts data directly for now.  File dialogs belong to the
/// UI and deliver a decoded `SceneSet` or `GlobalData`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControllerInput {
    SceneSet(SceneSet),
    Global(GlobalData),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewModel {
    pub device: Option<FileModelKey>,
    pub scene_count: usize,
    pub selected_scene: usize,
    pub dirty: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    /// `RevertToReceived` would change the document.
    pub can_revert: bool,
    pub transfer: crate::TransferState,
    /// Last known MIDI ports.
    pub ports: Vec<PortInfo>,
    pub scene_labels: Vec<String>,
    /// The scene rows, or the global rows when only global data is loaded.
    pub params: Vec<ParamView>,
    /// The global block's rows, from the global schema.
    pub global_params: Vec<ParamView>,
    pub jog: Option<JogConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamView {
    pub element: usize,
    /// Position in the schema's parameter list: keys repeat ("Value") and
    /// bit fields share a byte, so only the index names one parameter.
    pub index: usize,
    pub offset: usize,
    pub key: String,
    pub label: String,
    pub value: i32,
    pub min: i32,
    pub max: i32,
    pub kind: ParameterKind,
    pub choices: Vec<String>,
    pub display: String,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ControllerOutcome {
    pub view_changed: bool,
    pub transfer_event: Option<TransferEvent>,
    /// The port list changed (hot-plug, or the adapter said so).
    pub ports_changed: bool,
    /// Ports the adapter reported as disconnected during this tick.
    pub disconnected: Vec<PortId>,
    pub error: Option<ControllerError>,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ControllerError {
    #[error(transparent)]
    Session(#[from] CoreError),
    #[error(transparent)]
    Transfer(#[from] TransferError),
    #[error("listing MIDI ports failed: {0}")]
    Ports(TransportError),
    #[error("no device is selected for transfer")]
    NoDevice,
    #[error("no parameter schema is loaded")]
    NoSchema,
    #[error("unknown parameter: {0}")]
    UnknownParameter(String),
    #[error("parameter value does not fit the field")]
    ParameterWrite,
    #[error(transparent)]
    Jog(#[from] JogError),
    #[error("this device has no jog block")]
    NoJogLayout,
}

/// The `ParamView::key` of the wireless name row in `global_params`; it is
/// text, set with `ControllerAction::SetGlobalText` at its offset.
pub const DEVICE_NAME_KEY: &str = "DeviceName";

pub struct EditorController<T: MidiTransport, C: Clock> {
    session: EditorSession,
    transfer: TransferEngine<T, C>,
    schema: Option<ParameterSchema>,
    global_schema: Option<ParameterSchema>,
    device: Option<DeviceTarget>,
    ports: Vec<PortInfo>,
    port_watch: bool,
    last_scan: Option<Millis>,
    scan_failed: bool,
}

impl<T: MidiTransport, C: Clock> EditorController<T, C> {
    pub fn new(transport: T, clock: C) -> Self {
        Self {
            session: EditorSession::new(),
            transfer: TransferEngine::new(transport, clock),
            schema: None,
            global_schema: None,
            device: None,
            ports: Vec::new(),
            port_watch: true,
            last_scan: None,
            scan_failed: false,
        }
    }

    /// Turn the hot-plug watch on or off.  On by default.
    pub fn set_port_watch(&mut self, on: bool) {
        self.port_watch = on;
    }

    pub fn wants_tick(&self) -> TickRate {
        if self.transfer.is_active() {
            TickRate::Fast
        } else if self.port_watch || self.transfer.transport().is_open() {
            TickRate::Slow
        } else {
            TickRate::Idle
        }
    }

    /// Advance the transfer, read the transport, and watch the port list.
    /// Call from one UI timer at the rate `wants_tick` names.
    pub fn tick(&mut self) -> ControllerOutcome {
        let event = self.transfer.poll();
        let mut outcome = match &event {
            TransferEvent::Received(payload) => self.apply_received(payload.clone()),
            _ => ControllerOutcome::default(),
        };
        outcome.view_changed |= !matches!(event, TransferEvent::None);
        outcome.transfer_event = Some(event);

        for notice in self.transfer.take_port_notices() {
            match notice {
                PortNotice::Changed => self.last_scan = None,
                PortNotice::Disconnected(port) => {
                    outcome.disconnected.push(port);
                    self.last_scan = None;
                }
            }
        }
        // Port scans wait while a transfer runs; the scan is not needed for
        // it and must not delay it.
        if self.port_watch && !self.transfer.is_active() {
            let now = self.transfer.now();
            let due = self
                .last_scan
                .is_none_or(|last| now.0.saturating_sub(last.0) >= PORT_SCAN_MS);
            if due {
                self.last_scan = Some(now);
                self.scan_ports(&mut outcome);
            }
        }
        outcome.view_changed |= outcome.ports_changed || !outcome.disconnected.is_empty();
        outcome
    }

    fn scan_ports(&mut self, outcome: &mut ControllerOutcome) {
        match self.transfer.transport_mut().list_ports() {
            Ok(ports) => {
                self.scan_failed = false;
                if ports != self.ports {
                    self.ports = ports;
                    outcome.ports_changed = true;
                }
            }
            // Report a failing scan once, not on every tick.  A backend that
            // cannot list (JACK without a server) has no ports: keeping the
            // last list would show devices that cannot be opened.
            Err(error) => {
                if !self.ports.is_empty() {
                    self.ports.clear();
                    outcome.ports_changed = true;
                }
                if !self.scan_failed {
                    self.scan_failed = true;
                    outcome.error.get_or_insert(ControllerError::Ports(error));
                }
            }
        }
    }

    pub fn set_device(&mut self, device: Option<DeviceTarget>) {
        self.device = device;
    }

    /// Install a schema in its slot by its payload: the global block's own,
    /// or the scenes'.  A scene schema (or `None`) for another model drops the
    /// global schema, which belonged to the previous device.
    pub fn set_schema(&mut self, schema: Option<ParameterSchema>) {
        match schema {
            Some(schema) if schema.payload == "global" => self.global_schema = Some(schema),
            scene => {
                let model = scene.as_ref().map(|schema| schema.model.clone());
                if self.global_schema.as_ref().map(|schema| &schema.model) != model.as_ref() {
                    self.global_schema = None;
                }
                self.schema = scene;
            }
        }
    }

    /// The scene schema.
    pub fn schema(&self) -> Option<&ParameterSchema> {
        self.schema.as_ref()
    }

    pub fn global_schema(&self) -> Option<&ParameterSchema> {
        self.global_schema.as_ref()
    }

    pub fn session(&self) -> &EditorSession {
        &self.session
    }

    pub fn transfer(&self) -> &TransferEngine<T, C> {
        &self.transfer
    }

    pub fn transfer_mut(&mut self) -> &mut TransferEngine<T, C> {
        &mut self.transfer
    }

    pub fn view(&self) -> ViewModel {
        let document = self.session.document();
        ViewModel {
            device: document
                .scene_set
                .as_ref()
                .map(|set| set.model.clone())
                .or_else(|| document.global.as_ref().map(|global| global.model.clone())),
            scene_count: document
                .scene_set
                .as_ref()
                .map(|set| set.scenes.len())
                .unwrap_or(0),
            selected_scene: self.session.selected_scene(),
            dirty: self.session.is_dirty(),
            can_undo: self.session.can_undo(),
            can_revert: self.session.can_revert(),
            can_redo: self.session.can_redo(),
            transfer: self.transfer.state(),
            ports: self.ports.clone(),
            scene_labels: document
                .scene_set
                .as_ref()
                .map(|scene_set| {
                    scene_set
                        .scenes
                        .iter()
                        .enumerate()
                        .map(|(index, scene)| {
                            scene
                                .name
                                .clone()
                                .unwrap_or_else(|| format!("Scene {}", index + 1))
                        })
                        .collect()
                })
                .unwrap_or_default(),
            params: self.param_views(),
            global_params: self.global_param_views(),
            jog: self.jog_view(),
        }
    }

    fn jog_view(&self) -> Option<JogConfig> {
        let document = self.session.document();
        let set = document.scene_set.as_ref()?;
        let class = jog_class_for_file_key(set.model.as_str())?;
        let index = self.session.selected_scene();
        let scene = set.scenes.get(index)?;
        scene_jog_config(class, &scene.data).and_then(Result::ok)
    }

    fn set_global_byte(&mut self, offset: usize, value: u8) -> ControllerOutcome {
        let Some(global) = self.session.document().global.clone() else {
            return ControllerOutcome {
                error: Some(CoreError::NoGlobalData.into()),
                ..ControllerOutcome::default()
            };
        };
        let mut data = global.data;
        if offset >= data.len() {
            return ControllerOutcome {
                error: Some(CoreError::NoGlobalData.into()),
                ..ControllerOutcome::default()
            };
        }
        data[offset] = value;
        self.dispatch_session(SessionAction::SetGlobalData(data))
    }

    fn set_global_text(&mut self, offset: usize, text: &str) -> ControllerOutcome {
        let Some(global) = self.session.document().global.clone() else {
            return ControllerOutcome {
                error: Some(CoreError::NoGlobalData.into()),
                ..ControllerOutcome::default()
            };
        };
        let field = korg_devices::global_fields_for_file_key(&global.model)
            .and_then(|fields| fields.name_offset.zip(fields.name_width));
        let Some((field_offset, width)) = field else {
            return ControllerOutcome {
                error: Some(CoreError::NoGlobalData.into()),
                ..ControllerOutcome::default()
            };
        };
        let width = width as usize;
        let mut data = global.data;
        if offset != field_offset as usize
            || offset.checked_add(width).is_none_or(|end| end > data.len())
        {
            return ControllerOutcome {
                error: Some(CoreError::NoGlobalData.into()),
                ..ControllerOutcome::default()
            };
        }
        let bytes = text.as_bytes();
        // Pad as the field is padded now: the nanoKONTROL Studio pads with
        // spaces ("nanoKONTROL Studio      "), other data with zeros.
        let pad = if data[offset..offset + width].contains(&0) {
            0
        } else {
            b' '
        };
        for (index, slot) in data[offset..offset + width].iter_mut().enumerate() {
            *slot = bytes.get(index).copied().unwrap_or(pad);
        }
        self.dispatch_session(SessionAction::SetGlobalData(data))
    }

    fn set_global_bpm(&mut self, tenths: u16) -> ControllerOutcome {
        let Some(global) = self.session.document().global.clone() else {
            return ControllerOutcome {
                error: Some(CoreError::NoGlobalData.into()),
                ..ControllerOutcome::default()
            };
        };
        let offset = korg_devices::global_fields_for_file_key(&global.model)
            .and_then(|fields| fields.bpm_offset)
            .map(|offset| offset as usize);
        let mut data = global.data;
        let Some(offset) =
            offset.filter(|offset| offset.checked_add(2).is_some_and(|end| end <= data.len()))
        else {
            return ControllerOutcome {
                error: Some(CoreError::NoGlobalData.into()),
                ..ControllerOutcome::default()
            };
        };
        data[offset] = (tenths & 0xFF) as u8;
        data[offset + 1] = (tenths >> 8) as u8;
        self.dispatch_session(SessionAction::SetGlobalData(data))
    }

    fn set_jog(&mut self, config: JogConfig) -> ControllerOutcome {
        let document = self.session.document();
        let Some(set) = document.scene_set.as_ref() else {
            return ControllerOutcome {
                error: Some(ControllerError::Session(CoreError::NoSceneSet)),
                ..ControllerOutcome::default()
            };
        };
        let Some(class) = jog_class_for_file_key(set.model.as_str()) else {
            return ControllerOutcome {
                error: Some(ControllerError::NoJogLayout),
                ..ControllerOutcome::default()
            };
        };
        let Some(layout) = jog_layout(class) else {
            return ControllerOutcome {
                error: Some(ControllerError::NoJogLayout),
                ..ControllerOutcome::default()
            };
        };
        let index = self.session.selected_scene();
        let Some(scene) = set.scenes.get(index) else {
            return ControllerOutcome {
                error: Some(ControllerError::Session(CoreError::SceneIndex(index))),
                ..ControllerOutcome::default()
            };
        };
        let encoded = match config.encode() {
            Ok(bytes) => bytes,
            Err(error) => {
                return ControllerOutcome {
                    error: Some(ControllerError::Jog(error)),
                    ..ControllerOutcome::default()
                }
            }
        };
        let end = layout.offset + JogConfig::LEN;
        if scene.data.len() < end {
            return ControllerOutcome {
                error: Some(ControllerError::Jog(JogError::Short(scene.data.len()))),
                ..ControllerOutcome::default()
            };
        }
        let mut data = scene.data.clone();
        data[layout.offset..end].copy_from_slice(&encoded);
        self.dispatch_session(SessionAction::SetSceneData { index, data })
    }

    /// The rows `params` shows: the selected scene's, or the global block's
    /// when no scene set is loaded.
    fn param_views(&self) -> Vec<ParamView> {
        let document = self.session.document();
        match (&document.scene_set, &document.global) {
            (Some(scene_set), _) => self.schema.as_ref().map_or_else(Vec::new, |schema| {
                scene_set
                    .scenes
                    .get(self.session.selected_scene())
                    .map_or_else(Vec::new, |scene| Self::rows(schema, &scene.data))
            }),
            (None, Some(_)) => self.global_param_views(),
            (None, None) => Vec::new(),
        }
    }

    fn global_param_views(&self) -> Vec<ParamView> {
        let (Some(schema), Some(global)) = (&self.global_schema, &self.session.document().global)
        else {
            return Vec::new();
        };
        let mut rows = Vec::new();
        if let Some((offset, width)) = korg_devices::global_fields_for_file_key(&global.model)
            .and_then(|fields| fields.name_offset.zip(fields.name_width))
        {
            let offset = offset as usize;
            let width = width as usize;
            let name = global.data.get(offset..).unwrap_or_default();
            let end = name
                .iter()
                .take(width)
                .position(|byte| !(0x20..0x7F).contains(byte))
                .unwrap_or(name.len().min(width));
            rows.push(ParamView {
                element: 0,
                index: usize::MAX,
                offset,
                key: DEVICE_NAME_KEY.to_owned(),
                label: "Wireless Device Name".to_owned(),
                value: 0,
                min: 0,
                max: 0,
                kind: ParameterKind::Number,
                choices: Vec::new(),
                display: String::from_utf8_lossy(&name[..end]).trim_end().to_owned(),
            });
        }
        rows.extend(Self::rows(schema, &global.data));
        rows
    }

    fn rows(schema: &ParameterSchema, data: &[u8]) -> Vec<ParamView> {
        let mut rows = Vec::new();
        for element in 0..schema.elements as usize {
            let base = element * schema.stride as usize;
            let Some(slice) = data.get(base..) else {
                break;
            };
            for (index, parameter) in schema.parameters.iter().enumerate() {
                // The X14802 global table lists byte 80 twice; one row is enough.
                let repeated = schema.parameters[..index].iter().any(|earlier| {
                    (earlier.offset, earlier.width, earlier.shift, earlier.mask)
                        == (
                            parameter.offset,
                            parameter.width,
                            parameter.shift,
                            parameter.mask,
                        )
                });
                if repeated {
                    continue;
                }
                let Some(value) = parameter.decode(slice) else {
                    continue;
                };
                let label = if schema.payload == "global" {
                    parameter
                        .display_label
                        .clone()
                        .unwrap_or_else(|| parameter.key.clone())
                } else if schema.elements > 1 {
                    format!("{} {}", element + 1, parameter.key)
                } else {
                    parameter.key.clone()
                };
                let choices: Vec<String> =
                    if parameter.labels.is_empty() && schema.payload == "global" {
                        parameter.display_choices.clone()
                    } else {
                        parameter.labels.clone()
                    };
                let display = choices
                    .get((value - parameter.min) as usize)
                    .cloned()
                    .unwrap_or_else(|| value.to_string());
                rows.push(ParamView {
                    element,
                    index,
                    offset: usize::from(parameter.offset),
                    key: parameter.key.clone(),
                    label,
                    value,
                    min: parameter.min,
                    max: parameter.max,
                    kind: parameter.kind,
                    choices,
                    display,
                });
            }
        }
        rows
    }

    /// Load a decoded file.  The caller chooses the kind.
    pub fn load(&mut self, input: ControllerInput) -> ControllerOutcome {
        let action = match input {
            ControllerInput::SceneSet(scene_set) => SessionAction::LoadSceneSet(scene_set),
            ControllerInput::Global(global) => SessionAction::LoadGlobal(global),
        };
        self.dispatch_session(action)
    }

    pub fn dispatch(&mut self, action: ControllerAction) -> ControllerOutcome {
        match action {
            ControllerAction::SelectScene(index) => {
                self.dispatch_session(SessionAction::SelectScene(index))
            }
            ControllerAction::SetSceneData { index, data } => {
                self.dispatch_session(SessionAction::SetSceneData { index, data })
            }
            ControllerAction::SetParameter {
                element,
                index,
                value,
            } => self.set_parameter(element, index, value),
            ControllerAction::SetGlobalParameter {
                element,
                index,
                value,
            } => self.set_global_parameter(element, index, value),
            ControllerAction::Undo => self.dispatch_session(SessionAction::Undo),
            ControllerAction::Redo => self.dispatch_session(SessionAction::Redo),
            ControllerAction::MarkSaved => self.dispatch_session(SessionAction::MarkSaved),
            ControllerAction::MarkReceived(part) => {
                self.dispatch_session(SessionAction::MarkReceived(part))
            }
            ControllerAction::RevertToReceived => {
                self.dispatch_session(SessionAction::RevertToReceived)
            }
            ControllerAction::Receive(kind) => self.start_transfer(kind, true),
            ControllerAction::Write(kind) => self.start_transfer(kind, false),
            ControllerAction::SetJog(config) => self.set_jog(config),
            ControllerAction::SetGlobalByte { offset, value } => {
                self.set_global_byte(offset, value)
            }
            ControllerAction::SetGlobalText { offset, text } => self.set_global_text(offset, &text),
            ControllerAction::SetGlobalBpm(tenths) => self.set_global_bpm(tenths),
            ControllerAction::CancelTransfer => {
                self.transfer.cancel();
                ControllerOutcome {
                    view_changed: true,
                    ..ControllerOutcome::default()
                }
            }
        }
    }

    fn set_parameter(&mut self, element: usize, index: usize, value: i32) -> ControllerOutcome {
        if self.session.document().scene_set.is_none() && self.session.document().global.is_some() {
            return self.set_global_parameter(element, index, value);
        }
        let scene = self.session.selected_scene();
        let Some(data) = self
            .session
            .document()
            .scene_set
            .as_ref()
            .and_then(|set| set.scenes.get(scene))
            .map(|scene| scene.data.clone())
        else {
            return ControllerOutcome {
                error: Some(ControllerError::Session(CoreError::NoSceneSet)),
                ..ControllerOutcome::default()
            };
        };
        match Self::write_parameter(self.schema.as_ref(), data, element, index, value) {
            Ok(data) => self.dispatch_session(SessionAction::SetSceneData { index: scene, data }),
            Err(error) => ControllerOutcome {
                error: Some(error),
                ..ControllerOutcome::default()
            },
        }
    }

    fn set_global_parameter(
        &mut self,
        element: usize,
        index: usize,
        value: i32,
    ) -> ControllerOutcome {
        let Some(data) = self
            .session
            .document()
            .global
            .as_ref()
            .map(|global| global.data.clone())
        else {
            return ControllerOutcome {
                error: Some(ControllerError::Session(CoreError::NoGlobalData)),
                ..ControllerOutcome::default()
            };
        };
        match Self::write_parameter(self.global_schema.as_ref(), data, element, index, value) {
            Ok(data) => self.dispatch_session(SessionAction::SetGlobalData(data)),
            Err(error) => ControllerOutcome {
                error: Some(error),
                ..ControllerOutcome::default()
            },
        }
    }

    /// `data` with parameter `index` of `element` set to `value`.
    fn write_parameter(
        schema: Option<&ParameterSchema>,
        mut data: Vec<u8>,
        element: usize,
        index: usize,
        value: i32,
    ) -> Result<Vec<u8>, ControllerError> {
        let schema = schema.ok_or(ControllerError::NoSchema)?;
        let parameter = schema
            .parameters
            .get(index)
            .filter(|_| element < schema.elements as usize)
            .ok_or_else(|| {
                ControllerError::UnknownParameter(format!("{index} element {element}"))
            })?;
        let base = element * schema.stride as usize;
        let field = data
            .get_mut(base..)
            .ok_or(ControllerError::ParameterWrite)?;
        if !parameter.write(field, value) {
            return Err(ControllerError::ParameterWrite);
        }
        Ok(data)
    }

    fn start_transfer(&mut self, kind: TransferKind, receive: bool) -> ControllerOutcome {
        let fail = |error: ControllerError| ControllerOutcome {
            error: Some(error),
            ..ControllerOutcome::default()
        };
        let Some(device) = self.device.as_ref() else {
            return fail(ControllerError::NoDevice);
        };
        // A scene set over SysEx probably carries the padded per-scene
        // headers (`docs/file-format.md`, size rules).  Not proven: refuse.
        if kind == TransferKind::SceneSet {
            return fail(TransferError::LayoutUnknown(kind).into());
        }
        let document = self.session.document();
        let direction = if receive {
            let size = match kind {
                TransferKind::SceneData => document
                    .scene_set
                    .as_ref()
                    .map(|set| set.element_size as usize)
                    .or(device.scene_payload_size),
                _ => document
                    .global
                    .as_ref()
                    .map(|global| global.data.len())
                    .or(device.global_payload_size),
            };
            let Some(expected_len) = size else {
                return fail(TransferError::SizeUnknown(kind).into());
            };
            TransferDirection::Receive { expected_len }
        } else {
            let payload = match kind {
                TransferKind::SceneData => {
                    let index = self.session.selected_scene();
                    let Some(scene_set) = document.scene_set.as_ref() else {
                        return fail(CoreError::NoSceneSet.into());
                    };
                    let Some(scene) = scene_set.scenes.get(index) else {
                        return fail(CoreError::SceneIndex(index).into());
                    };
                    scene.data.clone()
                }
                _ => {
                    let Some(global) = document.global.as_ref() else {
                        return fail(CoreError::NoGlobalData.into());
                    };
                    global.data.clone()
                }
            };
            TransferDirection::Write { payload }
        };
        let request = TransferRequest {
            kind,
            direction,
            channel: device.channel,
            timeout_ms: device.timeout_ms,
        };
        let plan = match build_plan(device.class.as_str(), device.descriptor.as_ref(), &request) {
            Ok(plan) => plan,
            Err(error) => return fail(error.into()),
        };
        match self.transfer.start(plan) {
            Ok(()) => ControllerOutcome {
                view_changed: true,
                ..ControllerOutcome::default()
            },
            Err(error) => fail(error.into()),
        }
    }

    /// Put received bytes into the document when a matching part is loaded.
    /// Otherwise the UI gets them through `transfer_event`.
    fn apply_received(&mut self, payload: Vec<u8>) -> ControllerOutcome {
        let kind = self.transfer.plan().map(|plan| plan.kind());
        let document = self.session.document();
        match kind {
            Some(TransferKind::SceneData) if document.scene_set.is_some() => {
                let index = self.session.selected_scene();
                self.dispatch_session(SessionAction::SetSceneData {
                    index,
                    data: payload,
                })
            }
            Some(TransferKind::GlobalData) if document.global.is_some() => {
                self.dispatch_session(SessionAction::SetGlobalData(payload))
            }
            _ => ControllerOutcome::default(),
        }
    }

    fn dispatch_session(&mut self, action: SessionAction) -> ControllerOutcome {
        let SessionOutcome {
            document_changed,
            selection_changed,
            dirty_changed,
            error,
        } = self.session.dispatch(action);
        ControllerOutcome {
            view_changed: document_changed || selection_changed || dirty_changed,
            error: error.map(ControllerError::Session),
            ..ControllerOutcome::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transfer::ManualClock;
    use korg_midi::{Direction, MockTransport, PortId, PortInfo};
    use korg_model::{JogAcceleration, JogType, Scene, SceneSet};

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

    fn controller() -> EditorController<MockTransport, ManualClock> {
        let mut transport = MockTransport::new(ports());
        transport
            .open(&PortId::new("in"), &PortId::new("out"))
            .unwrap();
        EditorController::new(transport, ManualClock::new())
    }

    fn scene_set() -> SceneSet {
        SceneSet {
            model: FileModelKey::new("0820").unwrap(),
            element_size: 4,
            scenes: vec![Scene {
                name: None,
                data: vec![1, 2, 3, 4],
            }],
        }
    }

    #[test]
    fn jog_action_updates_the_scene_block() {
        let mut controller = controller();
        let block = [0x06, 0x05, 0x02, 0x53, 0x54, 0x56, 0x57, 0x00, 0x7F];
        let mut data = vec![0xFFu8; 560];
        data[512..521].copy_from_slice(&block);
        controller.load(ControllerInput::SceneSet(SceneSet {
            model: FileModelKey::new("1482").unwrap(),
            element_size: 560,
            scenes: vec![Scene { name: None, data }],
        }));
        assert_eq!(controller.view().jog.unwrap().kind, JogType::Encoder017F);

        let mut changed = controller.view().jog.unwrap();
        changed.kind = JogType::Continuous;
        changed.acceleration = JogAcceleration::One;
        let outcome = controller.dispatch(ControllerAction::SetJog(changed));
        assert!(outcome.error.is_none());
        assert!(controller.view().dirty);
        let data = &controller
            .session()
            .document()
            .scene_set
            .as_ref()
            .unwrap()
            .scenes[0]
            .data;
        assert_eq!(data[513], 2);
        assert_eq!(data[514], 0);
        assert_eq!(data[518], 0x57);
    }

    #[test]
    fn view_reflects_session_state() {
        let mut controller = controller();
        assert_eq!(controller.view().scene_count, 0);
        controller.load(ControllerInput::SceneSet(scene_set()));
        assert_eq!(controller.view().device.as_ref().unwrap().as_str(), "0820");
        assert_eq!(controller.view().scene_count, 1);
        controller.dispatch(ControllerAction::SetSceneData {
            index: 0,
            data: vec![9, 9, 9, 9],
        });
        assert!(controller.view().dirty);
        controller.dispatch(ControllerAction::Undo);
        assert!(!controller.view().dirty);
    }

    #[test]
    fn a_repeated_key_edits_the_row_it_names() {
        // X10820 has eight "Value" parameters; the fifth row must not edit
        // the first.
        let mut controller = controller();
        controller.set_schema(korg_devices::parameter_schema("X10820", "scene"));
        controller.load(ControllerInput::SceneSet(SceneSet {
            model: FileModelKey::new("0820").unwrap(),
            element_size: 64,
            scenes: vec![Scene {
                name: None,
                data: vec![0u8; 64],
            }],
        }));
        let row = controller.view().params[5].clone();
        assert_eq!((row.key.as_str(), row.offset), ("Value", 17));
        controller.dispatch(ControllerAction::SetParameter {
            element: row.element,
            index: row.index,
            value: 64,
        });
        let data = &controller
            .session()
            .document()
            .scene_set
            .as_ref()
            .unwrap()
            .scenes[0]
            .data;
        assert_eq!((data[8], data[10], data[17]), (0, 0, 64));
    }

    #[test]
    fn global_rows_read_and_write_the_global_block_beside_a_scene_set() {
        let mut controller = controller();
        controller.set_schema(korg_devices::parameter_schema("X14802", "scene"));
        controller.load(ControllerInput::SceneSet(SceneSet {
            model: FileModelKey::new("1482").unwrap(),
            element_size: 560,
            scenes: vec![Scene {
                name: None,
                data: vec![0x55u8; 560],
            }],
        }));
        controller.set_schema(korg_devices::parameter_schema("X14802", "global"));
        let mut global = vec![0u8; 128];
        global[1] = 3;
        controller.load(ControllerInput::Global(korg_model::GlobalData {
            model: FileModelKey::new("1482").unwrap(),
            data: global,
        }));
        let view = controller.view();
        assert!(view.params.iter().all(|row| row.label != "Controller Mode"));
        let mode = view
            .global_params
            .iter()
            .find(|row| row.label == "Controller Mode")
            .unwrap()
            .clone();
        assert_eq!(mode.display, "Live");
        controller.dispatch(ControllerAction::SetGlobalParameter {
            element: mode.element,
            index: mode.index,
            value: 6,
        });
        let document = controller.session().document();
        assert_eq!(document.global.as_ref().unwrap().data[1], 6);
        assert!(document.scene_set.as_ref().unwrap().scenes[0]
            .data
            .iter()
            .all(|byte| *byte == 0x55));
        // A scene schema of another model drops the stale global schema.
        controller.set_schema(korg_devices::parameter_schema("X10820", "scene"));
        assert!(controller.global_schema().is_none());
    }

    #[test]
    fn parameter_action_updates_scene_bytes() {
        let mut controller = controller();
        controller.set_schema(korg_devices::parameter_schema("X10820", "scene"));
        let mut data = vec![0u8; 64];
        data[9] = 1;
        controller.load(ControllerInput::SceneSet(SceneSet {
            model: FileModelKey::new("0820").unwrap(),
            element_size: 64,
            scenes: vec![Scene { name: None, data }],
        }));
        let outcome = controller.dispatch(ControllerAction::SetParameter {
            element: 0,
            index: 2, // VelocityCurve
            value: 3,
        });
        assert!(outcome.error.is_none());
        assert_eq!(
            controller
                .session()
                .document()
                .scene_set
                .as_ref()
                .unwrap()
                .scenes[0]
                .data[9],
            3
        );
        assert_eq!(
            controller
                .view()
                .params
                .iter()
                .find(|param| param.key == "VelocityCurve")
                .unwrap()
                .value,
            3
        );
        controller.dispatch(ControllerAction::Undo);
        assert_eq!(
            controller
                .session()
                .document()
                .scene_set
                .as_ref()
                .unwrap()
                .scenes[0]
                .data[9],
            1
        );
    }

    fn device(descriptor: Option<SysexDescriptor>) -> DeviceTarget {
        DeviceTarget {
            class: ClassId::new("X10830"),
            descriptor,
            channel: 0,
            scene_payload_size: Some(4),
            global_payload_size: None,
            timeout_ms: 1_000,
        }
    }

    #[test]
    fn global_bytes_and_bpm_are_editable() {
        let mut controller = controller();
        controller.load(ControllerInput::Global(korg_model::GlobalData {
            model: korg_model::FileModelKey::new("1480").unwrap(),
            data: vec![0u8; 128],
        }));
        controller.dispatch(ControllerAction::SetGlobalByte {
            offset: 0,
            value: 15,
        });
        assert_eq!(
            controller
                .session()
                .document()
                .global
                .as_ref()
                .unwrap()
                .data[0],
            15
        );
        controller.dispatch(ControllerAction::SetGlobalBpm(1200));
        let data = &controller
            .session()
            .document()
            .global
            .as_ref()
            .unwrap()
            .data;
        assert_eq!(u16::from(data[57]) | (u16::from(data[58]) << 8), 1200);
        assert!(controller.view().dirty);
    }

    #[test]
    fn unknown_global_fields_are_not_written() {
        let mut controller = controller();
        controller.set_schema(korg_devices::parameter_schema("X14802", "global"));
        controller.load(ControllerInput::Global(korg_model::GlobalData {
            model: korg_model::FileModelKey::new("1482").unwrap(),
            data: vec![0u8; 128],
        }));
        assert!(controller
            .dispatch(ControllerAction::SetGlobalBpm(1200))
            .error
            .is_some());
        assert!(controller
            .dispatch(ControllerAction::SetGlobalText {
                offset: 50,
                text: "wrong field".into(),
            })
            .error
            .is_some());
        assert!(controller
            .session()
            .document()
            .global
            .as_ref()
            .unwrap()
            .data
            .iter()
            .all(|v| *v == 0));
    }

    #[test]
    fn global_rows_use_the_editor_labels() {
        let mut controller = controller();
        controller.set_schema(korg_devices::parameter_schema("X14800", "global"));
        controller.load(ControllerInput::Global(korg_model::GlobalData {
            model: korg_model::FileModelKey::new("1480").unwrap(),
            data: vec![0u8; 128],
        }));
        let view = controller.view();
        let labels: Vec<&str> = view
            .params
            .iter()
            .map(|param| param.label.as_str())
            .collect();
        assert!(labels.contains(&"Global MIDI Channel"));
        assert!(labels.contains(&"Battery Type"));
        assert!(labels.contains(&"MIDI Clock Source"));
    }

    #[test]
    fn studio_global_rows_carry_the_measured_names() {
        let mut controller = controller();
        controller.set_schema(korg_devices::parameter_schema("X14802", "global"));
        // The device's global data received on 2026-10-21, fields only.
        let mut data = vec![0xffu8; 128];
        data[..4].copy_from_slice(&[0x00, 0x00, 0x01, 0x00]);
        data[64..68].copy_from_slice(&[0x00, 0x00, 0x02, 0x01]);
        data[72..76].copy_from_slice(&[0x02, 0x01, 0x01, 0x01]);
        data[80..82].copy_from_slice(&[0x00, 0x00]);
        controller.load(ControllerInput::Global(korg_model::GlobalData {
            model: korg_model::FileModelKey::new("1482").unwrap(),
            data,
        }));
        let view = controller.view();
        let row = |label: &str| {
            view.params
                .iter()
                .find(|param| param.label == label)
                .map(|param| param.display.as_str())
                .unwrap_or_else(|| panic!("no row {label}"))
        };
        assert_eq!(row("Global MIDI Channel"), "1");
        assert_eq!(row("Controller Mode"), "Assignable");
        assert_eq!(row("Battery Type"), "Ni-MH");
        assert_eq!(row("Use Scene Button as Scrub"), "Disable");
        assert_eq!(row("USB LED Brightness"), "3");
        assert_eq!(row("USB Auto LED Off"), "Disable");
        assert_eq!(row("Battery Auto Power Save"), "1 hour");
        assert_eq!(row("Battery LED Brightness"), "2");
        assert_eq!(row("Battery Auto LED Off"), "Enable");
    }

    #[test]
    fn global_presentation_is_taken_from_the_supplied_schema() {
        let mut schema = korg_devices::parameter_schema("X14802", "global").unwrap();
        let mode = schema
            .parameters
            .iter_mut()
            .find(|p| p.offset == 1)
            .unwrap();
        assert!(mode.display_choices_inferred);
        mode.display_label = Some("Custom mode".into());
        mode.display_choices = vec!["Custom choice".into()];
        let mut controller = controller();
        controller.set_schema(Some(schema));
        controller.load(ControllerInput::Global(korg_model::GlobalData {
            model: korg_model::FileModelKey::new("1482").unwrap(),
            data: vec![0; 128],
        }));
        let view = controller.view();
        let row = view
            .params
            .iter()
            .find(|p| p.label == "Custom mode")
            .unwrap();
        assert_eq!(row.display, "Custom choice");
    }

    #[test]
    fn transfer_needs_a_device_and_a_complete_descriptor() {
        let mut controller = controller();
        controller.load(ControllerInput::SceneSet(scene_set()));
        let outcome = controller.dispatch(ControllerAction::Write(TransferKind::SceneData));
        assert_eq!(outcome.error, Some(ControllerError::NoDevice));

        let shipped = korg_devices::sysex_descriptor("X10830");
        assert!(shipped.is_some());
        controller.set_device(Some(device(shipped)));
        let outcome = controller.dispatch(ControllerAction::Write(TransferKind::SceneData));
        assert!(matches!(
            outcome.error,
            Some(ControllerError::Transfer(
                TransferError::DescriptorIncomplete { .. }
            ))
        ));
        assert!(controller.transfer().transport().sent.is_empty());

        let outcome = controller.dispatch(ControllerAction::Receive(TransferKind::SceneSet));
        assert_eq!(
            outcome.error,
            Some(ControllerError::Transfer(TransferError::LayoutUnknown(
                TransferKind::SceneSet
            )))
        );
    }

    #[test]
    fn write_sends_the_selected_scene() {
        let mut controller = controller();
        controller.load(ControllerInput::SceneSet(scene_set()));
        controller.set_device(Some(device(Some(crate::transfer::test_descriptor()))));
        let outcome = controller.dispatch(ControllerAction::Write(TransferKind::SceneData));
        assert!(outcome.error.is_none());
        let outcome = controller.tick();
        assert_eq!(
            outcome.transfer_event,
            Some(TransferEvent::Sent(korg_sysex::Phase::ModeCheck))
        );
        assert_eq!(
            controller.view().transfer,
            crate::TransferState::Waiting(korg_sysex::Phase::ModeCheck)
        );
        controller.transfer_mut().transport_mut().push_packet(
            korg_sysex::build_ack(0x112, 0, korg_sysex::AckStatus::DataReceived).unwrap(),
        );
        let outcome = controller.tick();
        assert_eq!(
            outcome.transfer_event,
            Some(TransferEvent::Ack(korg_sysex::Phase::ModeCheck))
        );
        controller.tick();
        let sent = &controller.transfer().transport().sent[1];
        let packed = korg_sysex::pack_7in8(&[1, 2, 3, 4]);
        assert_eq!(
            &sent[sent.len() - 1 - packed.len()..sent.len() - 1],
            &packed[..]
        );
    }

    #[test]
    fn receive_replaces_the_selected_scene() {
        let mut controller = controller();
        controller.load(ControllerInput::SceneSet(scene_set()));
        controller.set_device(Some(device(Some(crate::transfer::test_descriptor()))));
        assert!(controller
            .dispatch(ControllerAction::Receive(TransferKind::SceneData))
            .error
            .is_none());
        controller.tick();
        controller.transfer_mut().transport_mut().push_packet(
            korg_sysex::build_ack(0x112, 0, korg_sysex::AckStatus::DataReceived).unwrap(),
        );
        controller.tick();
        controller.tick();
        let reply = korg_sysex::build_data_message(
            &korg_sysex::DataMessage::new(
                0x112,
                0,
                korg_sysex::DATA_COMMAND,
                korg_sysex::DATA_MARKER,
                korg_sysex::LengthStyle::ShortOrLong,
            ),
            &[7, 7, 7, 7],
        )
        .unwrap();
        controller.transfer_mut().transport_mut().push_packet(reply);
        let outcome = controller.tick();
        assert_eq!(
            outcome.transfer_event,
            Some(TransferEvent::Received(vec![7, 7, 7, 7]))
        );
        assert!(outcome.error.is_none());
        assert_eq!(
            controller
                .session()
                .document()
                .scene_set
                .as_ref()
                .unwrap()
                .scenes[0]
                .data,
            vec![7, 7, 7, 7]
        );
        assert!(controller.view().dirty);
    }

    #[test]
    fn tick_rate_follows_the_transfer() {
        let mut controller = controller();
        assert_eq!(controller.wants_tick(), TickRate::Slow);
        controller.load(ControllerInput::SceneSet(scene_set()));
        controller.set_device(Some(device(Some(crate::transfer::test_descriptor()))));
        controller.dispatch(ControllerAction::Write(TransferKind::SceneData));
        assert_eq!(controller.wants_tick(), TickRate::Fast);
        controller.dispatch(ControllerAction::CancelTransfer);
        assert_eq!(controller.wants_tick(), TickRate::Slow);

        controller.set_port_watch(false);
        controller.transfer_mut().transport_mut().close();
        assert_eq!(controller.wants_tick(), TickRate::Idle);
        assert_eq!(TickRate::Idle.interval_ms(), None);
    }

    #[test]
    fn tick_detects_hot_plug_by_port_scan() {
        let mut controller = controller();
        let outcome = controller.tick();
        assert!(outcome.ports_changed);
        assert_eq!(controller.view().ports.len(), 2);

        controller.transfer_mut().transport_mut().ports.pop();
        // Not due yet.
        assert!(!controller.tick().ports_changed);
        controller
            .transfer()
            .clock_for_tests()
            .advance(PORT_SCAN_MS);
        let outcome = controller.tick();
        assert!(outcome.ports_changed);
        assert!(outcome.view_changed);
        assert_eq!(controller.view().ports.len(), 1);
        controller
            .transfer()
            .clock_for_tests()
            .advance(PORT_SCAN_MS);
        assert!(!controller.tick().ports_changed);
    }

    #[test]
    fn adapter_port_events_reach_the_outcome_and_force_a_scan() {
        let mut controller = controller();
        controller.tick();
        controller.transfer_mut().transport_mut().push_event(
            korg_midi::TransportEvent::Disconnected {
                port: PortId::new("in"),
            },
        );
        controller.transfer_mut().transport_mut().ports.remove(0);
        // No clock advance: the notice makes the scan due at once.
        let outcome = controller.tick();
        assert_eq!(outcome.disconnected, vec![PortId::new("in")]);
        assert!(outcome.ports_changed);
    }

    #[test]
    fn a_failing_scan_empties_the_port_list_and_reports_once() {
        let mut controller = controller();
        controller.tick();
        assert_eq!(controller.view().ports.len(), 2);
        controller.transfer_mut().transport_mut().list_error = Some(
            korg_midi::TransportError::BackendUnavailable("JACK is not available".into()),
        );
        let clock = |c: &EditorController<MockTransport, ManualClock>| {
            c.transfer().clock_for_tests().advance(PORT_SCAN_MS)
        };
        clock(&controller);
        let outcome = controller.tick();
        assert!(outcome.ports_changed);
        assert!(outcome.error.is_some());
        assert!(controller.view().ports.is_empty());
        clock(&controller);
        let outcome = controller.tick();
        assert!(outcome.error.is_none(), "reported once");
        assert!(!outcome.ports_changed);
    }

    #[test]
    fn port_scan_waits_while_a_transfer_runs() {
        let mut controller = controller();
        controller.load(ControllerInput::SceneSet(scene_set()));
        controller.set_device(Some(device(Some(crate::transfer::test_descriptor()))));
        controller.dispatch(ControllerAction::Write(TransferKind::SceneData));
        let outcome = controller.tick();
        assert!(!outcome.ports_changed);
        assert!(controller.view().ports.is_empty());
    }
}
