//! TUI state and key handling.  No terminal I/O: `main` feeds keys and job
//! messages in, `ui` draws the state.

use korg_app::assign::FieldKind;
use korg_app::detect::Detected;
use korg_app::monitor::SceneMonitor;
use korg_app::panel::{Binding, Panel, TreeLine, TreeRow};
use korg_app::{DeviceJob, DeviceLink, JobOutput, JobProgress, LinkKind, MidiBackendSwitch};
use korg_core::{
    Clock, ControllerAction, ControllerInput, ControllerOutcome, EditorController, ParamView,
    ReceivedPart, ViewModel,
};
use korg_devices::Catalog;
use korg_midi::MidiTransport;
use korg_model::{ParameterKind, SceneSet};
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::{Position, Rect};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// One row of a device page (`korg_app::pages`): an editable byte of the
/// selected control's scene record, labelled as the original labels it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlField {
    /// Absolute scene offset.
    pub offset: usize,
    pub label: &'static str,
    /// The page heading above this field ("X-axis"), if any.
    pub section: Option<&'static str>,
    pub kind: FieldKind,
    /// The value as the original shows it.
    pub text: String,
    /// The shown number or choice index; `None` for text and empty slots.
    pub shown: Option<i32>,
    /// False when another field makes this one meaningless.
    pub enabled: bool,
    pub field: korg_app::pages::PageField,
}

impl ControlField {
    /// The value as the official editor labels it; "—" when unused.
    pub fn display(&self) -> String {
        if self.enabled {
            self.text.clone()
        } else {
            "—".to_owned()
        }
    }

    /// The table's Range column.
    pub fn range(&self) -> String {
        self.kind.range_hint()
    }
}

/// The channel field a schema row holds, when it is a `MidiCh` byte.
fn param_channel(param: &ParamView) -> Option<FieldKind> {
    korg_app::assign::channel_field(&param.key, param.kind, param.max)
}

/// The value of a schema row as the editor shows it: channels 1-based.
pub(crate) fn param_display(param: &ParamView) -> String {
    match param_channel(param) {
        Some(kind) => kind
            .labels()
            .get(usize::try_from(param.value).unwrap_or(usize::MAX))
            .cloned()
            .unwrap_or_else(|| param.value.to_string()),
        None => param.display.clone(),
    }
}

/// The Range column of a schema row.
pub(crate) fn param_range(param: &ParamView) -> String {
    if param.key == korg_core::DEVICE_NAME_KEY || param.kind == ParameterKind::Text {
        return "text".to_owned();
    }
    param_channel(param)
        .map(FieldKind::range_hint)
        .unwrap_or_else(|| {
            if param.choices.is_empty() {
                format!("{}..{}", param.min, param.max)
            } else {
                format!("{} choices", param.choices.len())
            }
        })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Devices,
    Editor,
    /// The files of the current device under the product folder.
    Browse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    /// The device schematic: arrows move between controls.
    Panel,
    /// The control list: an accordion by group.
    List,
    Params,
}

/// A menu command. The menus follow the original editor's File, Edit and
/// Send menus (`docs/rewrite/72-feature-parity.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    SelectDevice,
    RescanDevices,
    DetectDevices,
    New,
    Open,
    Save,
    SaveAs,
    SaveSysex,
    LoadSceneFile,
    SaveSceneFile,
    Quit,
    Undo,
    Redo,
    RevertReceived,
    Cut,
    Copy,
    Paste,
    Clear,
    Port,
    Link,
    MidiBackend,
    AutoReceive,
    WarnBeforeWrite,
    Browse,
    ShowValues,
    MidiMonitor,
    ClearMonitor,
    GlobalPage,
    ReceiveSet,
    ReceiveGlobal,
    WriteSet,
    ReceiveScene,
    WriteScene,
    WriteGlobal,
    CancelTransfer,
    Calibrate,
    Keys,
    About,
}

pub struct MenuItem {
    pub label: &'static str,
    pub keys: &'static str,
    pub command: Command,
}

const fn item(label: &'static str, keys: &'static str, command: Command) -> MenuItem {
    MenuItem {
        label,
        keys,
        command,
    }
}

pub const MENUS: [(&str, &[MenuItem]); 4] = [
    (
        "File",
        &[
            item("Select Device", "Esc", Command::SelectDevice),
            item("Rescan Devices", "", Command::RescanDevices),
            item("Detect Devices", "", Command::DetectDevices),
            item("New", "", Command::New),
            item("Open…", "o", Command::Open),
            item("Browse Device Files…", "b", Command::Browse),
            item("Save", "", Command::Save),
            item("Save As…", "s", Command::SaveAs),
            item("Save SysEx Dump…", "", Command::SaveSysex),
            item("Load Scene Data…", "", Command::LoadSceneFile),
            item("Save Scene Data…", "", Command::SaveSceneFile),
            item("Quit", "q", Command::Quit),
        ],
    ),
    (
        "Edit",
        &[
            item("Undo", "u", Command::Undo),
            item("Redo", "U", Command::Redo),
            item("Revert to Last Received", "", Command::RevertReceived),
            item("Cut", "", Command::Cut),
            item("Copy", "", Command::Copy),
            item("Paste", "", Command::Paste),
            item("Clear", "", Command::Clear),
            item("MIDI Port…", "p", Command::Port),
            item("Switch Link (USB/Bluetooth)", "", Command::Link),
            item("Switch MIDI Backend", "M", Command::MidiBackend),
            item("Auto Receive", "", Command::AutoReceive),
            item("Warn Before Write", "", Command::WarnBeforeWrite),
            item("Show Control Bindings", "v", Command::ShowValues),
            item("MIDI Monitor", "i", Command::MidiMonitor),
            item("Clear MIDI Monitor", "", Command::ClearMonitor),
            item("Global Page", "e", Command::GlobalPage),
        ],
    ),
    (
        "Send",
        &[
            item("Load Scene Set", "r", Command::ReceiveSet),
            item("Write Scene Set", "W", Command::WriteSet),
            item("Load Scene Data", "", Command::ReceiveScene),
            item("Write Scene Data", "w", Command::WriteScene),
            item("Load Global Data", "", Command::ReceiveGlobal),
            item("Write Global Data", "", Command::WriteGlobal),
            item("Cancel Transfer", "x", Command::CancelTransfer),
            item("Calibrate Touch Pad", "", Command::Calibrate),
        ],
    ),
    (
        "Help",
        &[
            item("Keys and Mouse", "?", Command::Keys),
            item("About", "", Command::About),
        ],
    ),
];

/// Screen areas from the last frame, for mouse hit-testing. `ui::draw`
/// fills it; `main` stores it in `App::hits`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Hits {
    pub menu_titles: Vec<Rect>,
    pub menu_items: Vec<Rect>,
    pub confirm_yes: Option<Rect>,
    pub confirm_no: Option<Rect>,
    pub tabs: Vec<Rect>,
    /// Device rows and the list's scroll offset.
    pub devices: Option<(Rect, usize)>,
    /// Browse rows and the list's scroll offset.
    pub browse: Option<(Rect, usize)>,
    /// Parameter rows (below the header), scroll offset, value column.
    pub params: Option<(Rect, usize, (u16, u16))>,
    /// The schematic's drawing area.
    pub panel: Option<Rect>,
    /// Control-list rows: area, scroll offset and line width in characters.
    pub tree: Option<(Rect, usize, usize)>,
    /// Chip cells and the control each one selects.
    pub chips: Vec<(Rect, usize)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptKind {
    OpenFile,
    SaveFile,
    SaveSceneFile,
    SaveSysex,
    Value,
    Port,
    Search,
}

impl PromptKind {
    pub fn title(self) -> &'static str {
        match self {
            PromptKind::OpenFile => "Open file",
            PromptKind::SaveFile => "Save as",
            PromptKind::SaveSceneFile => "Save scene data as",
            PromptKind::SaveSysex => "Save SysEx dump as",
            PromptKind::Value => "Set value",
            PromptKind::Port => "MIDI port (part of the name; empty = automatic)",
            PromptKind::Search => "Search devices (empty = all)",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Modal {
    None,
    Help,
    Prompt {
        kind: PromptKind,
        input: String,
    },
    /// An open menu: index into `MENUS` and the highlighted item.
    Menu {
        menu: usize,
        item: usize,
    },
    /// A device job waiting for `y`.  The text says what it will do.
    Confirm {
        input: String,
        output: String,
        job: DeviceJob,
        text: String,
    },
}

/// A job the main loop should start on a worker thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobRequest {
    pub input: String,
    pub output: String,
    /// The scene active on the device when the job went out, for
    /// `run_job_restoring`.
    pub active: Option<u8>,
    pub job: DeviceJob,
}

/// What the worker thread reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobMessage {
    Progress(JobProgress),
    Done(Result<JobOutput, String>),
    /// The result of Detect Devices.
    Detected(Result<Vec<Detected>, String>),
}

#[derive(Debug, Clone)]
pub struct Busy {
    pub label: String,
    pub step: String,
    pub started: Instant,
}

pub struct App<T: MidiTransport, C: Clock> {
    pub controller: EditorController<T, C>,
    /// Follows the scene the device announces on its ports.
    pub monitor: SceneMonitor<T>,
    pub catalog: Catalog,
    pub products: PathBuf,
    pub class: Option<String>,
    pub screen: Screen,
    pub focus: Focus,
    pub modal: Modal,
    pub status: String,
    pub device_cursor: usize,
    pub connected_only: bool,
    /// Text that filters the device list, shared rules with the GUI.
    pub search: String,
    /// The Browse screen: highlighted file of `browse_files`.
    pub browse_cursor: usize,
    /// Draw the binding of each control of the open group under the schematic.
    pub show_bindings: bool,
    /// The parameter table shows the global block instead of the scene.
    pub global_page: bool,
    pub param_cursor: usize,
    pub port_override: Option<String>,
    /// USB or Bluetooth when the device is connected on both; shared with the
    /// GUI.  `l` cycles it.
    pub link: Option<LinkKind>,
    /// Tests turn this off: key handling must not write the shared
    /// preferences file.
    pub write_preferences: bool,
    pub path: Option<PathBuf>,
    /// One scene copied with Edit -> Copy, shared by Cut and Paste.
    pub scene_clipboard: Option<Vec<u8>>,
    /// Receive the scene set after opening a connected device.
    pub auto_receive: bool,
    pub calibration_step: usize,
    pub warn_before_write: bool,
    pub busy: Option<Busy>,
    /// The user dropped the running job and its result must not apply.
    pub cancel_requested: bool,
    pub quit: bool,
    /// The selected control on the schematic, an index into `panel().controls`.
    pub control: Option<usize>,
    /// The control-list group that is open; at most one.
    pub open_group: Option<String>,
    /// The highlighted row of the control list.
    pub list_cursor: usize,
    pub hits: Hits,
    pending: Option<JobRequest>,
    /// The job on the worker, for `korg_app::written_part` when it ends.
    running: Option<DeviceJob>,
    /// Detect Devices asked for a scan the main loop has not started.
    detect_pending: bool,
    pub detecting: bool,
    /// Show the MIDI monitor under the parameters.
    pub monitor_shown: bool,
    /// Controls that sent a message recently, and when.
    lit: Vec<(usize, Instant)>,
    /// The scene a running job leaves the device on, applied on success.
    restore: Option<u8>,
    quit_armed: bool,
    last_click: Option<(Instant, Position)>,
    /// The MIDI backend (ALSA sequencer or JACK on Linux); `M` switches.
    /// `main` installs the platform's; tests keep a fixed one.
    pub midi: MidiBackendSwitch,
}

impl<T: MidiTransport, C: Clock> App<T, C> {
    pub fn new(
        controller: EditorController<T, C>,
        monitor: SceneMonitor<T>,
        catalog: Catalog,
        products: PathBuf,
    ) -> Self {
        Self {
            controller,
            monitor,
            catalog,
            products,
            class: None,
            screen: Screen::Devices,
            focus: Focus::Params,
            modal: Modal::None,
            status: "Select a device, or press o to open a file. ? for help.".into(),
            device_cursor: 0,
            connected_only: false,
            search: String::new(),
            browse_cursor: 0,
            show_bindings: false,
            global_page: false,
            param_cursor: 0,
            port_override: None,
            link: korg_app::preferences::load().link,
            write_preferences: true,
            path: None,
            scene_clipboard: None,
            auto_receive: korg_app::preferences::load().auto_receive,
            calibration_step: 0,
            warn_before_write: korg_app::preferences::load().warn_before_write,
            busy: None,
            cancel_requested: false,
            quit: false,
            control: None,
            open_group: None,
            list_cursor: 0,
            hits: Hits::default(),
            pending: None,
            running: None,
            detect_pending: false,
            detecting: false,
            monitor_shown: false,
            lit: Vec::new(),
            restore: None,
            quit_armed: false,
            last_click: None,
            midi: MidiBackendSwitch::fixed(korg_midi::BackendInfo {
                id: "mock".into(),
                label: "mock".into(),
                problem: None,
            }),
        }
    }

    /// The schematic of the open device, if it has one.
    pub fn panel(&self) -> Option<Panel> {
        self.class.as_deref().and_then(korg_app::panel::panel)
    }

    /// The binding of the selected control.
    pub fn selected_binding(&self) -> Binding {
        self.panel()
            .zip(self.control)
            .and_then(|(panel, index)| panel.controls.get(index).map(|c| c.binding))
            .unwrap_or(Binding::None)
    }

    /// The device's links, USB first.
    pub fn links(&self, class: &str) -> Vec<DeviceLink> {
        self.catalog
            .find_by_class(class)
            .map(|device| korg_app::device_links(&self.view().ports, device))
            .unwrap_or_default()
    }

    /// Switch the selected device to its next connected link and save the
    /// choice for the GUI.
    fn cycle_link(&mut self) {
        // The editor's device, or the one under the cursor on the device list.
        let device = match self.screen {
            Screen::Editor => self
                .class
                .as_deref()
                .and_then(|class| self.catalog.find_by_class(class)),
            _ => self
                .device_rows()
                .get(self.device_cursor)
                .map(|&index| &self.catalog.devices[index]),
        };
        let Some(device) = device else {
            self.status = "No device is selected.".into();
            return;
        };
        let links = korg_app::device_links(&self.view().ports, device);
        if links.is_empty() {
            self.status = format!("{} is not connected.", device.device);
            return;
        }
        // Cycle from the link in use, not from the saved preference: the
        // preference is `None` until `l` or the GUI sets it, and that means
        // USB.
        let current = korg_app::choose_link(&links, self.link).map(|link| link.kind);
        let Some(kind) = korg_app::next_link(&links, current) else {
            self.status = "Only one link connected".into();
            return;
        };
        self.link = Some(kind);
        let mut preferences = korg_app::preferences::load();
        preferences.link = self.link;
        self.save_preferences(&preferences);
        self.follow_monitor();
        self.status = format!("Link: {}", kind.label());
    }

    /// Switch to the next MIDI backend, save the choice for the GUI, and let
    /// the monitor reopen the device on the new backend.
    pub fn switch_midi_backend(&mut self) {
        if self.busy.is_some() {
            self.status = "A device job is running. Switch the MIDI backend after it.".into();
            return;
        }
        let info = match self.midi.switch() {
            Ok(info) => info.clone(),
            Err(message) => {
                self.status = message.into();
                return;
            }
        };
        let mut preferences = korg_app::preferences::load();
        preferences.midi_backend = info.id.clone();
        self.save_preferences(&preferences);
        // The monitor's connection belongs to the old backend; `tick` opens
        // the device again once the next port scan lists it.
        self.monitor.disconnect();
        self.status = korg_app::midi_backend::status(&info);
    }

    fn save_preferences(&self, preferences: &korg_app::preferences::Preferences) {
        if self.write_preferences {
            korg_app::preferences::save(preferences);
        }
    }

    fn has_document(&self) -> bool {
        let view = self.view();
        view.scene_count > 0 || self.controller.session().document().global.is_some()
    }

    /// Why `command` cannot run now, or `None` when it can.
    pub fn unavailable(&self, command: Command) -> Option<&'static str> {
        let needs_document = matches!(
            command,
            Command::Save
                | Command::SaveAs
                | Command::Undo
                | Command::Redo
                | Command::WriteSet
                | Command::WriteScene
                | Command::WriteGlobal
        );
        if needs_document && !self.has_document() {
            return Some("Open a device or a file first.");
        }
        match command {
            Command::Cut | Command::Copy | Command::Paste | Command::Clear
                if self.controller.session().document().scene_set.is_none() =>
            {
                Some("No scene set is loaded.")
            }
            Command::ReceiveScene | Command::ReceiveGlobal if self.class.is_none() => {
                Some("Select a device first.")
            }
            Command::Calibrate if self.class.as_deref() != Some("X14800") => {
                Some("Touch-pad calibration is only for the nanoKEY Studio.")
            }
            Command::WriteGlobal if self.controller.session().document().global.is_none() => {
                Some("No global data is loaded. Open a global file first.")
            }
            Command::Undo if !self.view().can_undo => Some("Nothing to undo."),
            Command::RevertReceived if !self.view().can_revert => {
                Some("Nothing to revert: the document matches what the device last sent.")
            }
            Command::DetectDevices if self.busy.is_some() || self.detecting => {
                Some("Wait for the running device job to finish.")
            }
            Command::Redo if !self.view().can_redo => Some("Nothing to redo."),
            Command::ReceiveSet
                if self.screen == Screen::Devices && self.device_rows().is_empty() =>
            {
                Some("No device is listed.")
            }
            _ => None,
        }
    }

    pub fn run(&mut self, command: Command) {
        if let Some(reason) = self.unavailable(command) {
            self.status = reason.into();
            return;
        }
        let key = |c: char| KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);
        match command {
            Command::SelectDevice => self.screen = Screen::Devices,
            Command::RescanDevices => {
                self.tick();
                self.device_cursor = 0;
                self.status = format!("{} device(s) after rescan.", self.device_rows().len());
            }
            Command::Open => self.on_key(key('o')),
            Command::Save => match self.path.clone() {
                Some(path) => self.save_to(path),
                None => self.save_as(),
            },
            Command::SaveAs => self.save_as(),
            Command::Quit => self.on_key(key('q')),
            Command::Undo => self.dispatch(ControllerAction::Undo),
            Command::Redo => self.dispatch(ControllerAction::Redo),
            Command::RevertReceived => {
                self.dispatch(ControllerAction::RevertToReceived);
                self.status =
                    "Reverted to the data last received from the device; u undoes it.".into();
            }
            Command::DetectDevices => {
                self.detect_pending = true;
                self.detecting = true;
                self.status = "Asking every MIDI port which KORG device answers…".into();
            }
            Command::MidiMonitor => {
                self.monitor_shown = !self.monitor_shown;
                self.status = if self.monitor_shown {
                    "MIDI monitor shown: play the device; i hides it.".into()
                } else {
                    "MIDI monitor hidden.".into()
                };
            }
            Command::ClearMonitor => {
                self.monitor.clear_log();
                self.status = "MIDI monitor cleared.".into();
            }
            Command::Port => self.on_key(key('p')),
            Command::ReceiveSet => match self.screen {
                Screen::Devices => self.on_key(key('r')),
                Screen::Editor | Screen::Browse => self.receive(self.class.clone()),
            },
            Command::ReceiveGlobal => self.receive_global(),
            Command::SaveSysex => self.save_sysex_as(),
            Command::WriteSet => self.confirm_write(true),
            Command::WriteScene => self.confirm_write(false),
            Command::WriteGlobal => self.confirm_global_write(),
            Command::CancelTransfer => self.cancel_job(),
            Command::Browse => self.open_browse(),
            Command::ShowValues => {
                self.show_bindings = !self.show_bindings;
                self.status = format!(
                    "Control bindings {}.",
                    if self.show_bindings {
                        "shown"
                    } else {
                        "hidden"
                    }
                );
            }
            Command::GlobalPage => self.toggle_global_page(),
            Command::Calibrate => self.start_calibration(),
            Command::Keys => self.modal = Modal::Help,
            Command::About => {
                self.status = format!(
                    "korg-tui {}: terminal front end of the KORG KONTROL EDITOR rewrite.",
                    env!("CARGO_PKG_VERSION")
                )
            }
            Command::Link => self.cycle_link(),
            Command::MidiBackend => self.switch_midi_backend(),
            Command::AutoReceive => {
                self.auto_receive = !self.auto_receive;
                let mut preferences = korg_app::preferences::load();
                preferences.auto_receive = self.auto_receive;
                self.save_preferences(&preferences);
                self.status = format!(
                    "Automatic receive {}.",
                    if self.auto_receive {
                        "enabled"
                    } else {
                        "disabled"
                    }
                );
            }
            Command::WarnBeforeWrite => {
                self.warn_before_write = !self.warn_before_write;
                let mut preferences = korg_app::preferences::load();
                preferences.warn_before_write = self.warn_before_write;
                self.save_preferences(&preferences);
                self.status = format!(
                    "Warn before write {}.",
                    if self.warn_before_write {
                        "enabled"
                    } else {
                        "disabled"
                    }
                );
            }
            Command::New => self.new_document(),
            Command::LoadSceneFile => self.on_key(key('o')),
            Command::SaveSceneFile => self.save_scene_as(),
            Command::Copy => {
                self.copy_scene();
            }
            Command::Cut => {
                if self.copy_scene() {
                    self.clear_scene();
                }
            }
            Command::Paste => self.paste_scene(),
            Command::Clear => self.clear_scene(),
            Command::ReceiveScene => self.receive_scene(),
        }
    }

    /// Select control `index` on the schematic and show what it edits.
    pub fn select_control(&mut self, index: usize) {
        let Some(panel) = self.panel() else {
            return;
        };
        let Some(control) = panel.controls.get(index) else {
            return;
        };
        self.control = Some(index);
        self.open_group = Some(control.group.to_owned());
        if self.control_fields().is_some() {
            self.param_cursor = 0;
        }
        match control.binding {
            Binding::Element(element) => {
                if let Some(row) = self.view().params.iter().position(|p| p.element == element) {
                    // Keep the cursor when it is already on this element.
                    let current = self.view().params.get(self.param_cursor).map(|p| p.element);
                    if current != Some(element) {
                        self.param_cursor = row;
                    }
                }
                self.status = format!("{}: {}", control.label, control.group);
            }
            Binding::Jog => self.status = "Jog Wheel: Tab or click to edit.".into(),
            // The nanoKONTROL Studio's fields come from its record, not a
            // schema binding (`control_fields`).
            Binding::None if self.control_fields().is_some_and(|f| !f.is_empty()) => {
                self.status = format!("{}: {}", control.label, control.group)
            }
            Binding::None => {
                self.status = format!(
                    "{}: parameters for this control are not decoded yet.",
                    control.label
                )
            }
        }
    }

    /// Follow the parameter cursor on the schematic.
    fn sync_control(&mut self) {
        if self.control_fields().is_some() {
            return;
        }
        let Some(panel) = self.panel() else {
            return;
        };
        let Some(element) = self.view().params.get(self.param_cursor).map(|p| p.element) else {
            return;
        };
        if self.selected_binding() != Binding::Element(element) {
            self.control = panel.find(Binding::Element(element)).or(self.control);
            self.follow_control();
        }
    }

    /// The control-list lines of the open panel: headers, single controls
    /// and the chips of the open group, wrapped to `line_chars`.
    pub fn tree_lines(&self, line_chars: usize) -> Vec<TreeLine> {
        self.panel()
            .map(|panel| panel.tree_lines(self.open_group.as_deref(), self.control, line_chars))
            .unwrap_or_default()
    }

    /// Lines at the width of the last drawn frame.
    fn lines(&self) -> Vec<TreeLine> {
        // The list box is 44 columns wide before the terminal is squeezed.
        self.tree_lines(self.hits.tree.map_or(44, |(_, _, chars)| chars))
    }

    /// Open the group of the selected control, so the list follows it.
    fn follow_control(&mut self) {
        self.open_group = self.control.and_then(|index| {
            let panel = self.panel()?;
            panel
                .controls
                .get(index)
                .map(|control| control.group.to_owned())
        });
    }

    fn new_document(&mut self) {
        let Some(class) = self.class.clone() else {
            self.status = "Open a device first.".into();
            return;
        };
        let Some(device) = self.catalog.find_by_class(&class).cloned() else {
            self.status = "Unknown device.".into();
            return;
        };
        match korg_app::open_device(&mut self.controller, &device, &self.products) {
            Ok(status) => {
                self.path = None;
                self.screen = Screen::Editor;
                self.param_cursor = 0;
                self.status = status;
            }
            Err(error) => self.status = error,
        }
    }

    /// The selected control's page from `korg_app::pages`, the same rows the
    /// GUI shows; `None` for devices without decoded pages and on the
    /// Global page.  With no control selected, the device's default page.
    pub fn control_fields(&self) -> Option<Vec<ControlField>> {
        if self.global_page {
            return None;
        }
        let class = self.class.as_deref()?;
        let panel = self.panel()?;
        let control = self.control.and_then(|index| panel.controls.get(index));
        let scene = self.scene_bytes();
        if scene.is_empty() {
            return None;
        }
        let mut section = None;
        let mut fields = Vec::new();
        for row in korg_app::pages::fields(class, &panel, control, scene)? {
            match row {
                korg_app::pages::PageRow::Heading(heading) => section = Some(heading),
                korg_app::pages::PageRow::Field(field) => fields.push(ControlField {
                    offset: field.offset,
                    label: field.label,
                    section,
                    kind: field.kind,
                    text: field.display(scene),
                    shown: field.shown(scene),
                    enabled: field.enabled,
                    field,
                }),
            }
        }
        Some(fields)
    }

    /// Rows the parameter table shows.
    pub fn param_rows(&self) -> usize {
        self.control_fields()
            .map_or(self.view().params.len(), |fields| fields.len())
    }

    /// The selected scene's data; empty when no scene set is loaded.
    pub fn scene_bytes(&self) -> &[u8] {
        self.controller
            .session()
            .document()
            .scene_set
            .as_ref()
            .and_then(|set| set.scenes.get(self.view().selected_scene))
            .map_or(&[], |scene| scene.data.as_slice())
    }

    /// One line for the status bar under the schematic: what the selected
    /// control sends in the selected scene.
    pub fn control_details(&self) -> Option<String> {
        let panel = self.panel()?;
        let control = panel.controls.get(self.control?)?;
        Some(korg_app::assign::details(
            &panel.class,
            control,
            self.scene_bytes(),
        ))
    }

    fn scene_data(&self) -> Option<(usize, Vec<u8>)> {
        let view = self.view();
        let scene_set = self.controller.session().document().scene_set.as_ref()?;
        let scene = scene_set.scenes.get(view.selected_scene)?;
        Some((view.selected_scene, scene.data.clone()))
    }

    fn copy_scene(&mut self) -> bool {
        let Some((index, data)) = self.scene_data() else {
            self.status = "No scene is selected.".into();
            return false;
        };
        self.scene_clipboard = Some(data);
        self.status = format!("Copied scene {}.", index + 1);
        true
    }

    fn paste_scene(&mut self) {
        let Some(data) = self.scene_clipboard.clone() else {
            self.status = "The scene clipboard is empty.".into();
            return;
        };
        let index = self.view().selected_scene;
        self.dispatch(ControllerAction::SetSceneData { index, data });
        self.status = format!("Pasted into scene {}.", index + 1);
    }

    fn clear_scene(&mut self) {
        let Some((index, data)) = self.scene_data() else {
            self.status = "No scene is selected.".into();
            return;
        };
        self.dispatch(ControllerAction::SetSceneData {
            index,
            data: vec![0u8; data.len()],
        });
        self.status = format!("Cleared scene {}.", index + 1);
    }

    fn save_scene_as(&mut self) {
        let suggestion = self
            .path
            .clone()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| {
                korg_app::export_scene_chunk(
                    &self.controller,
                    &self.catalog,
                    self.class.as_deref(),
                    self.view().selected_scene,
                )
                .map(|(_, extension)| format!("korg-scene.{extension}"))
                .unwrap_or_default()
            });
        self.prompt(PromptKind::SaveSceneFile, suggestion);
    }

    fn save_scene_to(&mut self, path: PathBuf) {
        let index = self.view().selected_scene;
        match korg_app::save_scene_path(
            &mut self.controller,
            &self.catalog,
            self.class.as_deref(),
            index,
            &path,
        ) {
            Ok(status) => self.status = status,
            Err(error) => self.status = error,
        }
    }

    fn start_calibration(&mut self) {
        let Some(class) = self.class.clone() else {
            self.status = "Select a device first.".into();
            return;
        };
        if class != "X14800" {
            self.status = "Touch-pad calibration is only for the nanoKEY Studio.".into();
            return;
        }
        if self.calibration_step >= 4 {
            self.calibration_step = 0;
        }
        let command = [0x01, 0x03, 0x02, 0x04][self.calibration_step];
        if self.calibration_step == 0 {
            self.status = "Starting touch-pad calibration...".into();
        }
        let (input, output) = match self.ports_for(Some(&class)) {
            Ok(ports) => ports,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        if self.busy.is_some() {
            self.status = "A device job is already running.".into();
            return;
        }
        self.queue(input, output, DeviceJob::Calibrate { command }, "Calibrate");
    }

    fn receive_global(&mut self) {
        let Some(class) = self.class.clone() else {
            self.status = "No device is selected.".into();
            return;
        };
        let (input, output) = match self.ports_for(Some(&class)) {
            Ok(ports) => ports,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        if self.busy.is_some() {
            self.status = "A device job is already running.".into();
            return;
        }
        self.queue(input, output, DeviceJob::ReceiveGlobal, "Receive global");
    }

    fn receive_scene(&mut self) {
        let Some(class) = self.class.clone() else {
            self.status = "No device is selected.".into();
            return;
        };
        let (input, output) = match self.ports_for(Some(&class)) {
            Ok(ports) => ports,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        if self.busy.is_some() {
            self.status = "A device job is already running.".into();
            return;
        }
        self.queue(
            input,
            output,
            DeviceJob::ReceiveScene {
                index: self.view().selected_scene as u8,
            },
            "Receive scene",
        );
    }

    fn save_sysex_as(&mut self) {
        let suggestion = self
            .path
            .clone()
            .map(|path| path.with_extension("syx").display().to_string())
            .unwrap_or_else(|| "korg-dump.syx".to_owned());
        self.prompt(PromptKind::SaveSysex, suggestion);
    }

    fn save_sysex_to(&mut self, path: PathBuf) {
        match korg_app::save_sysex_path(&self.controller, self.class.as_deref(), &path) {
            Ok(status) => self.status = status,
            Err(error) => self.status = error,
        }
    }

    fn save_as(&mut self) {
        let suggestion = self
            .path
            .clone()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| {
                korg_app::export_chunk(&self.controller, &self.catalog, self.class.as_deref())
                    .map(|(_, extension)| format!("korg-export.{extension}"))
                    .unwrap_or_default()
            });
        self.prompt(PromptKind::SaveFile, suggestion);
    }

    fn save_to(&mut self, path: PathBuf) {
        match korg_app::save_path(
            &mut self.controller,
            &self.catalog,
            self.class.as_deref(),
            &path,
        ) {
            Ok(status) => {
                self.path = Some(path);
                self.status = status;
            }
            Err(error) => self.status = error,
        }
    }

    /// The controller's view; on the Global page `params` holds the global
    /// rows, so the table, the cursor and edits all follow the page.
    pub fn view(&self) -> ViewModel {
        let mut view = self.controller.view();
        if self.global_page {
            view.params = std::mem::take(&mut view.global_params);
        } else if let Binding::Element(element) = self.selected_binding() {
            // The selected control's rows only, as the GUI shows them.
            view.params.retain(|param| param.element == element);
        }
        view
    }

    fn toggle_global_page(&mut self) {
        self.global_page = !self.global_page;
        self.param_cursor = 0;
        self.status = if !self.global_page {
            "Scene page.".into()
        } else if self.controller.session().document().global.is_none() {
            "Global page: no global data yet. Send → Load Global Data receives it.".into()
        } else {
            "Global page: e returns to the scene.".into()
        };
    }

    /// Set row `index` of the table: the scene's or, on the Global page, the
    /// global block's.
    fn set_param(&mut self, element: usize, index: usize, value: i32) {
        self.dispatch(if self.global_page {
            ControllerAction::SetGlobalParameter {
                element,
                index,
                value,
            }
        } else {
            ControllerAction::SetParameter {
                element,
                index,
                value,
            }
        });
    }

    /// Classes with a matching MIDI port, from the last port scan.
    pub fn connected(&self) -> Vec<String> {
        korg_app::connected_classes(&self.view().ports, &self.catalog)
    }

    /// Indexes into `catalog.devices` shown on the device screen.
    pub fn device_rows(&self) -> Vec<usize> {
        let connected = self.connected();
        let ports = self.view().ports;
        (0..self.catalog.devices.len())
            .filter(|&index| {
                korg_app::device_passes_filter(
                    &self.catalog.devices[index],
                    &connected,
                    self.connected_only,
                    &ports,
                    &self.search,
                )
            })
            .collect()
    }

    /// The files of the current device under the product folder.
    pub fn browse_files(&self) -> Vec<(String, PathBuf)> {
        self.class
            .as_deref()
            .and_then(|class| self.catalog.find_by_class(class))
            .map(|device| korg_app::device_files(device, &self.products))
            .unwrap_or_default()
    }

    pub fn device_name(&self) -> String {
        self.class
            .as_deref()
            .and_then(|class| self.catalog.find_by_class(class))
            .map(|device| device.device.clone())
            .or_else(|| self.view().device.map(|model| model.to_string()))
            .unwrap_or_else(|| "No device".into())
    }

    /// The job the main loop should start, once.
    pub fn take_job(&mut self) -> Option<JobRequest> {
        let request = self.pending.take();
        if let Some(request) = &request {
            self.running = Some(request.job.clone());
        }
        request
    }

    /// True once after Detect Devices: the main loop starts the scan.
    pub fn take_detect(&mut self) -> bool {
        std::mem::take(&mut self.detect_pending)
    }

    pub fn tick(&mut self) {
        let outcome = self.controller.tick();
        self.show(&outcome);
        self.follow_monitor();
        if self.monitor.poll() {
            if let Some(scene) = self.monitor.active {
                self.status = format!("The device switched to scene {}.", scene + 1);
            }
        }
        self.light_controls();
    }

    /// How long a control stays lit after it sent a message.
    const LIT_FOR: Duration = Duration::from_millis(400);

    /// Light the controls that sent the monitor's new messages.
    fn light_controls(&mut self) {
        let now = Instant::now();
        self.lit
            .retain(|(_, at)| now.duration_since(*at) < Self::LIT_FOR);
        let events = self.monitor.take_live();
        let (Some(class), Some(panel)) = (self.class.clone(), self.panel()) else {
            return;
        };
        let global = korg_app::live::global_channel(
            &class,
            self.controller
                .session()
                .document()
                .global
                .as_ref()
                .map(|global| global.data.as_slice()),
        );
        let scene = self.scene_bytes().to_vec();
        for event in events {
            for index in
                korg_app::live::lit_controls(&class, &panel, &scene, &event.message, global)
            {
                self.lit.retain(|(lit, _)| *lit != index);
                self.lit.push((index, now));
            }
        }
    }

    /// Controls lit by recent messages, indices into `panel().controls`.
    pub fn lit_controls(&self) -> Vec<usize> {
        let now = Instant::now();
        self.lit
            .iter()
            .filter(|(_, at)| now.duration_since(*at) < Self::LIT_FOR)
            .map(|(index, _)| *index)
            .collect()
    }

    /// Keep the scene monitor on the ports of the current link, or off.
    fn follow_monitor(&mut self) {
        let override_port = self.port_override.clone();
        let ports = match (self.class.as_deref(), override_port) {
            (Some(_), Some(port)) => Some((port.clone(), port)),
            (Some(class), None) => {
                let links = self
                    .catalog
                    .find_by_class(class)
                    .map(|device| korg_app::device_links(&self.view().ports, device))
                    .unwrap_or_default();
                korg_app::choose_link(&links, self.link)
                    .map(|link| (link.input.clone(), link.output.clone()))
            }
            _ => None,
        };
        match ports {
            Some((input, output)) => {
                if self.monitor.connect(&input, &output).is_err() {
                    self.monitor.disconnect();
                }
            }
            None => self.monitor.disconnect(),
        }
    }

    fn show(&mut self, outcome: &ControllerOutcome) {
        if let Some(error) = &outcome.error {
            self.status = error.to_string();
        }
    }

    fn dispatch(&mut self, action: ControllerAction) {
        let outcome = self.controller.dispatch(action);
        self.show(&outcome);
    }

    pub fn on_job(&mut self, message: JobMessage) {
        match message {
            JobMessage::Detected(result) => {
                self.detecting = false;
                self.status = match result {
                    Err(error) => format!("Detect Devices failed: {error}"),
                    Ok(found) => {
                        korg_app::detect::remember(&found);
                        let names: Vec<String> =
                            found.iter().map(|d| d.summary(&self.catalog)).collect();
                        match names.len() {
                            0 => "No KORG device answered.".to_owned(),
                            n => format!("Found {n}: {}", names.join("; ")),
                        }
                    }
                };
                self.follow_monitor();
            }
            JobMessage::Progress(progress) => {
                if let Some(busy) = self.busy.as_mut() {
                    busy.step = match progress {
                        JobProgress::Opened { input, .. } => format!("opened {input}"),
                        JobProgress::Identified { model, key } => {
                            format!("device model {model:#06x} (key {key})")
                        }
                        JobProgress::Receiving => "receiving".into(),
                        JobProgress::WritingGlobal => "writing global".into(),
                        JobProgress::VerifyingGlobal => "reading back global".into(),
                        JobProgress::Writing { scene, of } => {
                            format!("writing scene {} of {of}", scene + 1)
                        }
                        JobProgress::Verifying { scene, of } => {
                            format!("reading back scene {} of {of}", scene + 1)
                        }
                    };
                }
            }
            JobMessage::Done(result) => {
                if self.cancel_requested {
                    self.cancel_requested = false;
                    self.busy = None;
                    self.running = None;
                    self.status = "Cancelled. The job result was dropped.".into();
                    return;
                }
                let label = self.busy.take().map(|busy| busy.label).unwrap_or_default();
                let ok = result.is_ok();
                let job = self.running.take();
                self.status = match result {
                    Err(error) => format!("{label} failed: {error}"),
                    Ok(JobOutput::SceneSet { key, scenes }) => {
                        let count = scenes.len();
                        let element_size = scenes.first().map(Vec::len).unwrap_or(0) as u16;
                        let class = self
                            .catalog
                            .find_by_file_key(&key)
                            .into_iter()
                            .find(|device| device.scene_payload_size == Some(element_size))
                            .map(|device| device.class.clone());
                        if let Some(device) =
                            class.as_deref().and_then(|c| self.catalog.find_by_class(c))
                        {
                            self.controller
                                .set_schema(korg_devices::parameter_schema(&device.class, "scene"));
                            self.controller
                                .set_device(Some(korg_app::device_target(device)));
                        }
                        self.class = class;
                        let scenes = korg_app::named_scenes(&key, scenes);
                        self.controller.load(ControllerInput::SceneSet(SceneSet {
                            model: key,
                            element_size,
                            scenes,
                        }));
                        // The document now equals the device.
                        self.dispatch(ControllerAction::MarkSaved);
                        self.dispatch(ControllerAction::MarkReceived(ReceivedPart::SceneSet));
                        self.path = None;
                        self.screen = Screen::Editor;
                        self.param_cursor = 0;
                        self.control = None;
                        self.open_group = None;
                        format!("Received {count} scenes from the device.")
                    }
                    Ok(JobOutput::Global { key, data }) => {
                        if let Some(class) = self.class.as_deref() {
                            self.controller
                                .set_schema(korg_devices::parameter_schema(class, "global"));
                        }
                        self.controller
                            .load(ControllerInput::Global(korg_model::GlobalData {
                                model: key,
                                data,
                            }));
                        self.dispatch(ControllerAction::MarkSaved);
                        self.dispatch(ControllerAction::MarkReceived(ReceivedPart::Global));
                        self.screen = Screen::Editor;
                        self.global_page = true;
                        self.param_cursor = 0;
                        let len = self
                            .controller
                            .session()
                            .document()
                            .global
                            .as_ref()
                            .map_or(0, |global| global.data.len());
                        format!("Received {len} bytes of global data; e returns to the scene.")
                    }
                    Ok(JobOutput::Scene { index, data }) => {
                        self.controller.dispatch(ControllerAction::SetSceneData {
                            index: index as usize,
                            data,
                        });
                        self.dispatch(ControllerAction::MarkReceived(ReceivedPart::Scene(
                            index as usize,
                        )));
                        format!("Received scene {}.", index + 1)
                    }
                    Ok(JobOutput::Written { scenes }) => {
                        format!("Wrote {scenes} scene(s); read back identical.")
                    }
                    Ok(JobOutput::GlobalWritten) => "Wrote global; read back identical.".into(),
                    Ok(JobOutput::CalibrationDone) => {
                        self.calibration_step += 1;
                        match self.calibration_step {
                            1 => "Touch the top-left corner, then choose Calibrate again.".into(),
                            2 => {
                                "Touch the bottom-right corner, then choose Calibrate again.".into()
                            }
                            3 => "Finishing calibration...".into(),
                            _ => "Calibration complete. Reboot the nanoKEY Studio.".into(),
                        }
                    }
                };
                if let Some(scene) = self.restore.take() {
                    if ok {
                        self.monitor.set_active(scene);
                    }
                }
                // A write read back identical: the device holds it now.
                if let (true, Some(job)) = (ok, job) {
                    if let Some(part) =
                        korg_app::written_part(&job, self.controller.session().document())
                    {
                        self.dispatch(ControllerAction::MarkReceived(part));
                    }
                }
            }
        }
    }

    pub fn on_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && key.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        match std::mem::replace(&mut self.modal, Modal::None) {
            Modal::None => {}
            Modal::Help => return,
            Modal::Menu { menu, item } => {
                let items = MENUS[menu].1.len();
                match key.code {
                    KeyCode::Esc | KeyCode::F(10) => {}
                    KeyCode::Left | KeyCode::Char('h') => {
                        self.modal = Modal::Menu {
                            menu: (menu + MENUS.len() - 1) % MENUS.len(),
                            item: 0,
                        }
                    }
                    KeyCode::Right | KeyCode::Char('l') => {
                        self.modal = Modal::Menu {
                            menu: (menu + 1) % MENUS.len(),
                            item: 0,
                        }
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        self.modal = Modal::Menu {
                            menu,
                            item: (item + items - 1) % items,
                        }
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        self.modal = Modal::Menu {
                            menu,
                            item: (item + 1) % items,
                        }
                    }
                    KeyCode::Enter => self.run(MENUS[menu].1[item].command),
                    _ => self.modal = Modal::Menu { menu, item },
                }
                return;
            }
            Modal::Prompt { kind, mut input } => {
                match key.code {
                    KeyCode::Esc => {
                        self.modal = Modal::None;
                        self.status = "Cancelled.".into();
                    }
                    KeyCode::Enter => self.submit(kind, input.trim().to_owned()),
                    KeyCode::Backspace => {
                        input.pop();
                        self.modal = Modal::Prompt { kind, input };
                    }
                    // Alt and Ctrl chords are not text.
                    KeyCode::Char(c)
                        if !key
                            .modifiers
                            .intersects(KeyModifiers::ALT | KeyModifiers::CONTROL) =>
                    {
                        input.push(c);
                        self.modal = Modal::Prompt { kind, input };
                    }
                    _ => self.modal = Modal::Prompt { kind, input },
                }
                return;
            }
            Modal::Confirm {
                input,
                output,
                job,
                text,
            } => {
                match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') => {
                        self.queue(input, output, job, "Write")
                    }
                    KeyCode::Char('n') | KeyCode::Esc => {
                        self.status = "Write cancelled. Nothing was sent.".into()
                    }
                    _ => {
                        self.modal = Modal::Confirm {
                            input,
                            output,
                            job,
                            text,
                        }
                    }
                }
                return;
            }
        }

        let quit_armed = std::mem::take(&mut self.quit_armed);
        match key.code {
            KeyCode::Char('q') => {
                if self.view().dirty && !quit_armed {
                    self.quit_armed = true;
                    self.status = "Unsaved changes. Press q again to quit, s to save.".into();
                } else if self.busy.is_some() && !quit_armed {
                    self.quit_armed = true;
                    self.status = "A device job is running. Press q again to quit anyway.".into();
                } else {
                    self.quit = true;
                }
                return;
            }
            KeyCode::Char('?') => {
                self.modal = Modal::Help;
                return;
            }
            KeyCode::F(10) | KeyCode::Char('m') => {
                self.modal = Modal::Menu { menu: 0, item: 0 };
                return;
            }
            KeyCode::Char('o') => {
                self.prompt(PromptKind::OpenFile, String::new());
                return;
            }
            KeyCode::Char('p') => {
                let current = self.port_override.clone().unwrap_or_default();
                self.prompt(PromptKind::Port, current);
                return;
            }
            KeyCode::Char('x') if self.busy.is_some() => {
                self.cancel_job();
                return;
            }
            KeyCode::Char('M') => {
                self.switch_midi_backend();
                return;
            }
            KeyCode::Char('i') => {
                self.run(Command::MidiMonitor);
                return;
            }
            _ => {}
        }
        match self.screen {
            Screen::Devices => self.devices_key(key),
            Screen::Editor => self.editor_key(key),
            Screen::Browse => self.browse_key(key),
        }
    }

    /// Drop the result of the running job.
    ///
    /// The MIDI exchange cannot stop mid-request, so the worker ends the job
    /// after its current step.  The UI frees the job only when the worker
    /// reports, so a second job cannot open the same ports.
    pub fn cancel_job(&mut self) {
        if self.busy.is_none() {
            self.status = "No device job is running.".into();
            return;
        }
        self.cancel_requested = true;
        self.status = "Cancel requested. The job ends after its current step.".into();
    }

    /// Open the file list of the current device.
    pub fn open_browse(&mut self) {
        if self.class.is_none() {
            self.status = "Select a device first. Press o to open a path.".into();
            return;
        }
        self.browse_cursor = 0;
        self.screen = Screen::Browse;
    }

    fn browse_key(&mut self, key: KeyEvent) {
        let files = self.browse_files();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.browse_cursor = self.browse_cursor.saturating_sub(1)
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.browse_cursor = (self.browse_cursor + 1).min(files.len().saturating_sub(1))
            }
            KeyCode::Enter => match files.get(self.browse_cursor) {
                Some((_, path)) => {
                    let path = path.clone();
                    self.open_path(path);
                }
                None => self.status = "No file for this device. Press o to open a path.".into(),
            },
            KeyCode::Esc => {
                self.screen = if self.has_document() {
                    Screen::Editor
                } else {
                    Screen::Devices
                }
            }
            _ => {}
        }
    }

    fn prompt(&mut self, kind: PromptKind, input: String) {
        self.modal = Modal::Prompt { kind, input };
    }

    fn devices_key(&mut self, key: KeyEvent) {
        let rows = self.device_rows();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.device_cursor = self.device_cursor.saturating_sub(1)
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.device_cursor = (self.device_cursor + 1).min(rows.len().saturating_sub(1))
            }
            KeyCode::Char('c') => {
                self.tick();
                self.connected_only = !self.connected_only;
                self.device_cursor = 0;
            }
            KeyCode::Char('/') => self.prompt(PromptKind::Search, self.search.clone()),
            KeyCode::Char('b') => self.open_browse(),
            KeyCode::Esc if !self.search.is_empty() => {
                self.search.clear();
                self.device_cursor = 0;
                self.status = "Search cleared.".into();
            }
            KeyCode::Char('l') => self.cycle_link(),
            KeyCode::Enter => {
                let Some(&index) = rows.get(self.device_cursor) else {
                    return;
                };
                let device = self.catalog.devices[index].clone();
                match korg_app::open_device(&mut self.controller, &device, &self.products) {
                    Ok(status) => {
                        self.class = Some(device.class.clone());
                        self.path = None;
                        self.screen = Screen::Editor;
                        self.focus = Focus::Params;
                        self.param_cursor = 0;
                        self.control = None;
                        self.open_group = None;
                        self.status = status;
                        if self.auto_receive {
                            self.receive(Some(device.class.clone()));
                        }
                    }
                    Err(error) => self.status = error,
                }
            }
            KeyCode::Char('r') => {
                let Some(&index) = rows.get(self.device_cursor) else {
                    return;
                };
                let class = self.catalog.devices[index].class.clone();
                self.receive(Some(class));
            }
            KeyCode::Esc if self.view().scene_count > 0 => self.screen = Screen::Editor,
            _ => {}
        }
    }

    fn editor_key(&mut self, key: KeyEvent) {
        let view = self.view();
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => {
                self.screen = Screen::Devices;
                return;
            }
            KeyCode::Tab | KeyCode::BackTab => {
                let mut order = Vec::new();
                if self.panel().is_some() {
                    order.push(Focus::Panel);
                    order.push(Focus::List);
                }
                order.push(Focus::Params);
                let at = order.iter().position(|&f| f == self.focus).unwrap_or(0);
                let step = if key.code == KeyCode::BackTab {
                    order.len() - 1
                } else {
                    1
                };
                self.set_focus(order[(at + step) % order.len()]);
                return;
            }
            KeyCode::Char('[') | KeyCode::PageUp => {
                self.select_scene(view.selected_scene.checked_sub(1));
                return;
            }
            KeyCode::Char(']') | KeyCode::PageDown => {
                self.select_scene(Some(view.selected_scene + 1));
                return;
            }
            KeyCode::Char(c @ '1'..='9') => {
                self.select_scene(Some(c as usize - '1' as usize));
                return;
            }
            KeyCode::Char('u') => {
                self.dispatch(ControllerAction::Undo);
                return;
            }
            KeyCode::Char('r') if ctrl => {
                self.dispatch(ControllerAction::Redo);
                return;
            }
            KeyCode::Char('U') => {
                self.dispatch(ControllerAction::Redo);
                return;
            }
            KeyCode::Char('s') => {
                self.save_as();
                return;
            }
            KeyCode::Char('v') => {
                self.show_bindings = !self.show_bindings;
                return;
            }
            KeyCode::Char('e') => {
                self.toggle_global_page();
                return;
            }
            KeyCode::Char('r') => {
                self.receive(self.class.clone());
                return;
            }
            KeyCode::Char('w') => {
                self.confirm_write(false);
                return;
            }
            KeyCode::Char('W') => {
                self.confirm_write(true);
                return;
            }
            _ => {}
        }
        let step = if shift { 10 } else { 1 };
        match self.focus {
            Focus::Panel => self.panel_key(key.code),
            Focus::List => self.list_key(key.code),
            Focus::Params => {
                self.params_key(key.code, step, &view);
                self.sync_control();
            }
        }
    }

    fn set_focus(&mut self, focus: Focus) {
        self.focus = focus;
        match focus {
            Focus::List => {
                let lines = self.lines();
                self.list_cursor = lines
                    .iter()
                    .position(line_is_selected)
                    .unwrap_or_else(|| self.list_cursor.min(lines.len().saturating_sub(1)));
            }
            Focus::Panel if self.control.is_none() => {
                self.control = Some(0);
                self.follow_control();
            }
            Focus::Params => self.sync_control(),
            Focus::Panel => {}
        }
    }

    fn panel_key(&mut self, code: KeyCode) {
        let Some(panel) = self.panel() else {
            self.focus = Focus::Params;
            return;
        };
        let from = self.control.unwrap_or(0);
        let direction = match code {
            KeyCode::Left | KeyCode::Char('h') => (-1.0, 0.0),
            KeyCode::Right | KeyCode::Char('l') => (1.0, 0.0),
            KeyCode::Up | KeyCode::Char('k') => (0.0, -1.0),
            KeyCode::Down | KeyCode::Char('j') => (0.0, 1.0),
            KeyCode::Enter => {
                match self.selected_binding() {
                    Binding::Jog | Binding::Element(_) => self.focus = Focus::Params,
                    Binding::None => self.select_control(from),
                }
                return;
            }
            _ => return,
        };
        if let Some(next) = panel.neighbor(from, direction.0, direction.1) {
            self.select_control(next);
        }
    }

    /// Keys in the control list: rows, then chips along the open group.
    fn list_key(&mut self, code: KeyCode) {
        let lines = self.lines();
        if lines.is_empty() {
            self.focus = Focus::Params;
            return;
        }
        let cursor = self.list_cursor.min(lines.len() - 1);
        match code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.list_cursor = cursor.saturating_sub(1);
                self.enter_line();
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.list_cursor = (cursor + 1).min(lines.len() - 1);
                self.enter_line();
            }
            KeyCode::Right | KeyCode::Char('l') => match lines.get(cursor) {
                Some(TreeLine::Chips(_)) => self.step_chip(1),
                Some(TreeLine::Row(TreeRow::Group {
                    name, open: false, ..
                })) => {
                    self.open_group = Some((*name).to_owned());
                }
                _ => {}
            },
            KeyCode::Left | KeyCode::Char('h') => match lines.get(cursor) {
                Some(TreeLine::Chips(_)) => self.step_chip(-1),
                Some(TreeLine::Row(TreeRow::Group { .. })) => self.open_group = None,
                _ => {}
            },
            KeyCode::Enter | KeyCode::Char(' ') => self.activate_line(cursor),
            _ => {}
        }
    }

    /// Landing on a chip line selects a chip, so the parameters follow.
    fn enter_line(&mut self) {
        let lines = self.lines();
        let Some(TreeLine::Chips(chips)) = lines.get(self.list_cursor) else {
            return;
        };
        let index = chips
            .iter()
            .find(|chip| chip.selected)
            .or(chips.first())
            .map(|chip| chip.index);
        if let Some(index) = index {
            self.select_control(index);
        }
    }

    /// Move between the chips of the highlighted line.  Left of the first
    /// chip goes to the group's header and closes it.
    fn step_chip(&mut self, delta: i32) {
        let lines = self.lines();
        let Some(TreeLine::Chips(chips)) = lines.get(self.list_cursor) else {
            return;
        };
        let Some(at) = chips.iter().position(|chip| chip.selected) else {
            if let Some(chip) = chips.first() {
                self.select_control(chip.index);
            }
            return;
        };
        let next = at as i32 + delta;
        if next < 0 {
            self.collapse_to_group(chips[at].index);
        } else if let Some(chip) = chips.get(next as usize) {
            self.select_control(chip.index);
        }
    }

    /// Close the group of `index` and put the cursor on its header.
    fn collapse_to_group(&mut self, index: usize) {
        let Some(group) = self.panel().and_then(|panel| {
            panel
                .controls
                .get(index)
                .map(|control| control.group.clone())
        }) else {
            return;
        };
        self.open_group = None;
        let lines = self.lines();
        if let Some(position) = lines.iter().position(
            |line| matches!(line, TreeLine::Row(TreeRow::Group { name, .. }) if *name == group),
        ) {
            self.list_cursor = position;
        }
    }

    /// Enter or Space on the highlighted line.
    fn activate_line(&mut self, cursor: usize) {
        let lines = self.lines();
        let Some(line) = lines.get(cursor).cloned() else {
            return;
        };
        match line {
            TreeLine::Row(TreeRow::Group { name, .. }) => {
                self.open_group = korg_app::panel::toggle_group(self.open_group.as_deref(), &name);
                let lines = self.lines();
                if let Some(position) = lines.iter().position(|line| {
                    matches!(line, TreeLine::Row(TreeRow::Group { name: found, .. }) if *found == name)
                }) {
                    self.list_cursor = position;
                }
            }
            TreeLine::Row(TreeRow::Control { index, .. }) => self.select_control(index),
            TreeLine::Chips(chips) => {
                let index = chips
                    .iter()
                    .find(|chip| chip.selected)
                    .or(chips.first())
                    .map(|chip| chip.index);
                if let Some(index) = index {
                    self.select_control(index);
                }
            }
        }
    }

    /// Mouse input. Positions are checked against `hits` from the last frame.
    pub fn on_mouse(&mut self, event: MouseEvent) {
        let at = Position::new(event.column, event.row);
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => self.click(at),
            MouseEventKind::ScrollUp => self.scroll(at, 1),
            MouseEventKind::ScrollDown => self.scroll(at, -1),
            _ => {}
        }
    }

    fn click(&mut self, at: Position) {
        let double = self.last_click.is_some_and(|(time, place)| {
            place == at && time.elapsed() < Duration::from_millis(400)
        });
        self.last_click = Some((Instant::now(), at));
        let hits = self.hits.clone();
        match self.modal.clone() {
            Modal::Menu { menu, .. } => {
                self.modal = Modal::None;
                if let Some(item) = hits.menu_items.iter().position(|r| r.contains(at)) {
                    self.run(MENUS[menu].1[item].command);
                } else if let Some(title) = hits.menu_titles.iter().position(|r| r.contains(at)) {
                    if title != menu {
                        self.modal = Modal::Menu {
                            menu: title,
                            item: 0,
                        };
                    }
                }
                return;
            }
            Modal::Help => {
                self.modal = Modal::None;
                return;
            }
            Modal::Confirm { .. } => {
                let key = if hits.confirm_yes.is_some_and(|r| r.contains(at)) {
                    'y'
                } else if hits.confirm_no.is_some_and(|r| r.contains(at)) {
                    'n'
                } else {
                    return;
                };
                self.on_key(KeyEvent::new(KeyCode::Char(key), KeyModifiers::NONE));
                return;
            }
            Modal::Prompt { .. } => return,
            Modal::None => {}
        }
        if let Some(menu) = hits.menu_titles.iter().position(|r| r.contains(at)) {
            self.modal = Modal::Menu { menu, item: 0 };
            return;
        }
        match self.screen {
            Screen::Browse => {
                if let Some(row) = row_at(hits.browse, at) {
                    if row < self.browse_files().len() {
                        let again = row == self.browse_cursor;
                        self.browse_cursor = row;
                        if double && again {
                            self.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                        }
                    }
                }
            }
            Screen::Devices => {
                if let Some(row) = row_at(hits.devices, at) {
                    if row < self.device_rows().len() {
                        let again = row == self.device_cursor;
                        self.device_cursor = row;
                        if double && again {
                            self.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                        }
                    }
                }
            }
            Screen::Editor => {
                if let Some(scene) = hits.tabs.iter().position(|r| r.contains(at)) {
                    self.select_scene(Some(scene));
                } else if let Some(area) = hits.panel.filter(|r| r.contains(at)) {
                    self.click_panel(area, at);
                } else if let Some((_, index)) =
                    hits.chips.iter().find(|(rect, _)| rect.contains(at))
                {
                    self.click_chip(*index);
                } else if let Some((area, offset, _)) =
                    hits.tree.filter(|(area, _, _)| area.contains(at))
                {
                    if let Some(line) = row_at(Some((area, offset)), at) {
                        self.click_tree(line);
                    }
                } else if let Some((area, offset, value)) = hits.params {
                    if let Some(row) = row_at(Some((area, offset)), at) {
                        if row < self.param_rows() {
                            self.focus = Focus::Params;
                            let again = row == self.param_cursor;
                            self.param_cursor = row;
                            self.sync_control();
                            if again {
                                if let Some(delta) = value_click(value, at.x) {
                                    self.adjust_cursor(delta);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    fn click_panel(&mut self, area: Rect, at: Position) {
        let Some(panel) = self.panel() else {
            return;
        };
        let (sx, sy) = (
            panel.width / f64::from(area.width.max(1)),
            panel.height / f64::from(area.height.max(1)),
        );
        let x = (f64::from(at.x - area.x) + 0.5) * sx;
        let y = (f64::from(at.y - area.y) + 0.5) * sy;
        self.focus = Focus::Panel;
        match panel.hit(x, y, (sx / 2.0, sy / 2.0)) {
            Some(index) => self.select_control(index),
            None => {
                self.control = None;
                self.open_group = None;
            }
        }
    }

    fn click_tree(&mut self, line: usize) {
        if line >= self.lines().len() {
            return;
        }
        self.focus = Focus::List;
        self.list_cursor = line;
        self.activate_line(line);
    }

    fn click_chip(&mut self, index: usize) {
        self.focus = Focus::List;
        self.select_control(index);
        let at = self.lines().iter().position(|line| {
            matches!(line, TreeLine::Chips(chips) if chips.iter().any(|chip| chip.index == index))
        });
        if let Some(at) = at {
            self.list_cursor = at;
        }
    }

    fn scroll(&mut self, at: Position, delta: i32) {
        if self.modal != Modal::None {
            return;
        }
        let hits = self.hits.clone();
        match self.screen {
            Screen::Browse => {
                if hits.browse.is_some_and(|(r, _)| r.contains(at)) {
                    let key = if delta > 0 {
                        KeyCode::Up
                    } else {
                        KeyCode::Down
                    };
                    self.browse_key(KeyEvent::new(key, KeyModifiers::NONE));
                }
            }
            Screen::Devices => {
                if hits.devices.is_some_and(|(r, _)| r.contains(at)) {
                    let key = if delta > 0 {
                        KeyCode::Up
                    } else {
                        KeyCode::Down
                    };
                    self.devices_key(KeyEvent::new(key, KeyModifiers::NONE));
                }
            }
            Screen::Editor => {
                if hits.tabs.iter().any(|r| r.contains(at)) {
                    let selected = self.view().selected_scene;
                    let next = if delta > 0 {
                        selected.checked_sub(1)
                    } else {
                        Some(selected + 1)
                    };
                    self.select_scene(next);
                } else if let Some((area, offset, value)) = hits.params {
                    if let Some(row) = row_at(Some((area, offset)), at) {
                        if row >= self.param_rows() {
                            return;
                        }
                        self.focus = Focus::Params;
                        // The wheel over a value changes it; elsewhere it scrolls.
                        if at.x >= value.0 && at.x < value.1 {
                            self.param_cursor = row;
                            self.adjust_cursor(delta);
                        } else {
                            let len = self.param_rows();
                            self.param_cursor = (self.param_cursor as i32 - delta)
                                .clamp(0, len.saturating_sub(1) as i32)
                                as usize;
                        }
                        self.sync_control();
                    }
                }
            }
        }
    }

    fn select_scene(&mut self, index: Option<usize>) {
        let count = self.view().scene_count;
        if let Some(index) = index.filter(|&index| index < count) {
            self.dispatch(ControllerAction::SelectScene(index));
        }
    }

    fn params_key(&mut self, code: KeyCode, step: i32, view: &ViewModel) {
        let len = self.param_rows();
        match code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.param_cursor = self.param_cursor.saturating_sub(1)
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.param_cursor = (self.param_cursor + 1).min(len.saturating_sub(1))
            }
            KeyCode::Home | KeyCode::Char('g') => self.param_cursor = 0,
            KeyCode::End | KeyCode::Char('G') => self.param_cursor = len.saturating_sub(1),
            KeyCode::Left | KeyCode::Char('h') | KeyCode::Char('-') => self.adjust_cursor(-step),
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Char('+') | KeyCode::Char('=') => {
                self.adjust_cursor(step)
            }
            KeyCode::Char('H') => self.adjust_cursor(-10),
            KeyCode::Char('L') => self.adjust_cursor(10),
            KeyCode::Enter => {
                if let Some(fields) = self.control_fields() {
                    if let Some(field) = fields.get(self.param_cursor) {
                        self.prompt(PromptKind::Value, field.display());
                    }
                } else if let Some(param) = view.params.get(self.param_cursor) {
                    self.prompt(PromptKind::Value, param_display(param));
                }
            }
            _ => {}
        }
    }

    /// Step the row under the parameter cursor, schema row or Studio field.
    fn adjust_cursor(&mut self, delta: i32) {
        if self.control_fields().is_some() {
            self.adjust_field(delta);
        } else {
            let view = self.view();
            self.adjust(&view, delta);
        }
    }

    fn adjust_field(&mut self, delta: i32) {
        let Some(fields) = self.control_fields() else {
            return;
        };
        let Some(field) = fields.get(self.param_cursor) else {
            return;
        };
        let (min, max) = field.field.range();
        let next = match field.kind {
            FieldKind::Text { .. } => {
                self.status = format!("Press Enter to type the {}.", field.label);
                return;
            }
            // Choices and channels wrap, like a combo box.
            FieldKind::Channel
            | FieldKind::ChannelOnly
            | FieldKind::Choice(_)
            | FieldKind::ExternalChoice(_) => {
                (field.shown.unwrap_or(0) + delta).rem_euclid(max + 1)
            }
            // An empty slot starts from the bottom of the range.
            _ => field
                .shown
                .map_or(min, |shown| (shown + delta).clamp(min, max)),
        };
        if Some(next) != field.shown {
            self.set_field(field, next);
        }
    }

    /// Store a shown value in a page field of the selected scene, with undo.
    /// `korg_app::pages::store` clamps it, encodes it as the device does and
    /// refuses disabled fields and undecoded choices.
    fn set_field(&mut self, field: &ControlField, value: i32) {
        if !field.enabled {
            self.status = format!("{} is not used with the current settings.", field.label);
            return;
        }
        let index = self.view().selected_scene;
        let (Some((_, mut data)), Some(class), Some(panel)) =
            (self.scene_data(), self.class.clone(), self.panel())
        else {
            return;
        };
        let before = data.clone();
        if !korg_app::pages::store(&class, &panel, &mut data, &field.field, value) {
            self.status = format!("{} cannot be set to that value.", field.label);
        } else if data != before {
            self.dispatch(ControllerAction::SetSceneData { index, data });
        }
    }

    /// Store a text field of the selected scene, with undo.
    fn set_scene_text(&mut self, field: &ControlField, text: &str) {
        let index = self.view().selected_scene;
        let Some((_, mut data)) = self.scene_data() else {
            return;
        };
        let before = data.clone();
        if korg_app::pages::store_text(&mut data, &field.field, text) && data != before {
            self.dispatch(ControllerAction::SetSceneData { index, data });
        }
    }

    fn adjust(&mut self, view: &ViewModel, delta: i32) {
        let Some(param) = view.params.get(self.param_cursor) else {
            return;
        };
        if param.key == korg_core::DEVICE_NAME_KEY {
            self.status = format!("Press Enter to type the {}.", param.label);
            return;
        }
        let value = match param_channel(param) {
            // Channels wrap through the labels, Global included.
            Some(kind) => (param.value + delta).rem_euclid(kind.labels().len() as i32),
            None => match param.kind {
                // Enum values wrap around, like a combo box.
                ParameterKind::Enum => {
                    let span = param.max - param.min + 1;
                    param.min + (param.value - param.min + delta).rem_euclid(span.max(1))
                }
                ParameterKind::Number => (param.value + delta).clamp(param.min, param.max),
                ParameterKind::Text => {
                    self.status = format!("Press Enter to type the {}.", param.label);
                    return;
                }
            },
        };
        if value != param.value {
            self.set_param(param.element, param.index, value);
        }
    }

    /// Open a file and go to the editor.
    fn open_path(&mut self, path: PathBuf) {
        match korg_app::load_path(&mut self.controller, &self.catalog, &path) {
            Ok((class, status)) => {
                self.class = class;
                self.path = Some(path);
                self.screen = Screen::Editor;
                self.focus = Focus::Params;
                self.param_cursor = 0;
                self.control = None;
                self.open_group = None;
                self.status = status;
            }
            Err(error) => self.status = error,
        }
    }

    fn submit(&mut self, kind: PromptKind, input: String) {
        match kind {
            PromptKind::Port => {
                self.port_override = (!input.is_empty()).then_some(input);
                self.status = match &self.port_override {
                    Some(port) => format!("Device jobs use ports matching {port:?}."),
                    None => "Device jobs pick the port from the device name.".into(),
                };
            }
            PromptKind::Search => {
                self.search = input;
                self.device_cursor = 0;
                let count = self.device_rows().len();
                self.status = if self.search.is_empty() {
                    format!("{count} device(s).")
                } else {
                    format!("{count} device(s) match {:?}.", self.search)
                };
            }
            PromptKind::OpenFile if !input.is_empty() => {
                let path = expand(&input);
                self.open_path(path);
            }
            PromptKind::SaveFile if !input.is_empty() => self.save_to(expand(&input)),
            PromptKind::SaveSysex if !input.is_empty() => self.save_sysex_to(expand(&input)),
            PromptKind::SaveSceneFile if !input.is_empty() => self.save_scene_to(expand(&input)),
            PromptKind::Value => {
                if let Some(fields) = self.control_fields() {
                    let Some(field) = fields.get(self.param_cursor) else {
                        return;
                    };
                    if matches!(field.kind, FieldKind::Text { .. }) {
                        self.set_scene_text(field, &input);
                        return;
                    }
                    match prompt_field_value(&field.field, &input) {
                        Some(value) => self.set_field(field, value),
                        None => {
                            self.status = format!(
                                "{:?} is not a value for {} ({}).",
                                input,
                                field.label,
                                field.range()
                            )
                        }
                    }
                    return;
                }
                let view = self.view();
                let Some(param) = view.params.get(self.param_cursor) else {
                    return;
                };
                if param.key == korg_core::DEVICE_NAME_KEY {
                    let text: String = input.chars().filter(|c| matches!(c, ' '..='~')).collect();
                    self.dispatch(ControllerAction::SetGlobalText {
                        offset: param.offset,
                        text,
                    });
                    return;
                }
                if param.kind == ParameterKind::Text {
                    let text: String = input.chars().filter(|c| matches!(c, ' '..='~')).collect();
                    self.dispatch(ControllerAction::SetParameterText {
                        element: param.element,
                        index: param.index,
                        text,
                    });
                    return;
                }
                let value = match param_channel(param) {
                    Some(kind) => kind
                        .labels()
                        .iter()
                        .position(|label| label.eq_ignore_ascii_case(&input))
                        .map(|index| index as i32),
                    None => param
                        .choices
                        .iter()
                        .position(|choice| choice.eq_ignore_ascii_case(&input))
                        .map(|index| param.min + index as i32)
                        .or_else(|| input.parse::<i32>().ok()),
                };
                match value {
                    Some(value) if (param.min..=param.max).contains(&value) => {
                        self.set_param(param.element, param.index, value)
                    }
                    _ => {
                        self.status = format!(
                            "{:?} is not a value for {} ({}).",
                            input,
                            param.label,
                            param_range(param)
                        )
                    }
                }
            }
            _ => self.status = "Cancelled.".into(),
        }
    }

    /// The input and output ports to use for `class`: the override for both,
    /// else the two ports of the chosen link.
    fn ports_for(&self, class: Option<&str>) -> Result<(String, String), String> {
        if let Some(port) = &self.port_override {
            return Ok((port.clone(), port.clone()));
        }
        let device = class
            .and_then(|class| self.catalog.find_by_class(class))
            .ok_or("No device is selected. Press p to name a port.")?;
        let links = korg_app::device_links(&self.view().ports, device);
        korg_app::choose_link(&links, self.link)
            .map(|link| (link.input.clone(), link.output.clone()))
            .ok_or_else(|| {
                format!(
                    "No MIDI port matches {}. Connect it, or press p.",
                    device.device
                )
            })
    }

    fn receive(&mut self, class: Option<String>) {
        if self.view().dirty {
            self.status =
                "Unsaved changes. Save (s) or undo them before a receive replaces the document."
                    .into();
            return;
        }
        match self.ports_for(class.as_deref()) {
            Ok((input, output)) => self.queue(input, output, DeviceJob::ReceiveSceneSet, "Receive"),
            Err(error) => self.status = error,
        }
    }

    fn confirm_write(&mut self, whole_set: bool) {
        let view = self.view();
        let Some(set) = self.controller.session().document().scene_set.clone() else {
            self.status = "No scene set is loaded.".into();
            return;
        };
        let (input, output) = match self.ports_for(self.class.as_deref()) {
            Ok(ports) => ports,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        let ports = ports_text(&input, &output);
        let key = set.model.as_str().to_owned();
        let (job, text) = if whole_set {
            (
                DeviceJob::WriteSceneSet {
                    key,
                    scenes: set.scenes.iter().map(|scene| scene.data.clone()).collect(),
                },
                format!(
                    "Write all {} scenes to the device on {ports:?}, replacing what it holds?",
                    set.scenes.len()
                ),
            )
        } else {
            let index = view.selected_scene;
            let Some(scene) = set.scenes.get(index) else {
                return;
            };
            (
                DeviceJob::WriteScene {
                    key,
                    index: index as u8,
                    payload: scene.data.clone(),
                },
                format!(
                    "Write scene {} ({}) to the device on {ports:?}, replacing its scene {}?",
                    index + 1,
                    view.scene_labels.get(index).cloned().unwrap_or_default(),
                    index + 1
                ),
            )
        };
        if self.busy.is_some() {
            self.status = "A device job is already running.".into();
            return;
        }
        self.modal = Modal::Confirm {
            input,
            output,
            job,
            text,
        };
    }

    fn confirm_global_write(&mut self) {
        let Some(global) = self.controller.session().document().global.clone() else {
            self.status = "No global data is loaded.".into();
            return;
        };
        let (input, output) = match self.ports_for(self.class.as_deref()) {
            Ok(ports) => ports,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        if self.busy.is_some() {
            self.status = "A device job is already running.".into();
            return;
        }
        let job = DeviceJob::WriteGlobal {
            key: global.model.as_str().to_owned(),
            data: global.data,
        };
        let text = format!(
            "Write the global block to the device on {:?}?",
            ports_text(&input, &output)
        );
        if self.warn_before_write {
            self.modal = Modal::Confirm {
                input,
                output,
                job,
                text,
            };
        } else {
            self.queue(input, output, job, "Write");
        }
    }

    fn queue(&mut self, input: String, output: String, job: DeviceJob, label: &str) {
        if self.busy.is_some() {
            self.status = "A device job is already running.".into();
            return;
        }
        let active = self.monitor.active;
        self.restore = korg_app::restore_scene(&job, active);
        self.busy = Some(Busy {
            label: label.into(),
            step: format!("opening {:?}", ports_text(&input, &output)),
            started: Instant::now(),
        });
        self.status = format!("{label}: started.");
        self.pending = Some(JobRequest {
            input,
            output,
            active,
            job,
        });
    }
}

/// Whether `line` holds the selected control.
fn line_is_selected(line: &TreeLine) -> bool {
    match line {
        TreeLine::Row(TreeRow::Group { selected, .. })
        | TreeLine::Row(TreeRow::Control { selected, .. }) => *selected,
        TreeLine::Chips(chips) => chips.iter().any(|chip| chip.selected),
    }
}

/// `input`, or `input / output` when a link's two ports differ.
fn ports_text(input: &str, output: &str) -> String {
    if input == output {
        input.to_owned()
    } else {
        format!("{input} / {output}")
    }
}

/// The list row under `at`, given the rows area and the scroll offset.
fn row_at(rows: Option<(Rect, usize)>, at: Position) -> Option<usize> {
    let (area, offset) = rows?;
    area.contains(at)
        .then(|| usize::from(at.y - area.y) + offset)
}

/// A typed value for a page field: a choice or channel label, or a number
/// within the field's shown range.
fn prompt_field_value(field: &korg_app::pages::PageField, input: &str) -> Option<i32> {
    match field.kind {
        FieldKind::Channel
        | FieldKind::ChannelOnly
        | FieldKind::Choice(_)
        | FieldKind::ExternalChoice(_) => korg_app::pages::choice_index(field, input),
        FieldKind::Text { .. } => None,
        // A number, or a note name in a note field ("C#4").
        _ => field.parse_number(input),
    }
}

/// A click on a value cell: the left half lowers it, the right half raises it.
fn value_click(value: (u16, u16), x: u16) -> Option<i32> {
    let (start, end) = value;
    if x < start || x >= end {
        return None;
    }
    Some(if x < start + (end - start) / 2 { -1 } else { 1 })
}

fn expand(input: &str) -> PathBuf {
    match input.strip_prefix("~/") {
        Some(rest) => std::env::var_os("HOME")
            .map(|home| PathBuf::from(home).join(rest))
            .unwrap_or_else(|| PathBuf::from(input)),
        None => PathBuf::from(input),
    }
}
