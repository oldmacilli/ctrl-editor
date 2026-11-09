//! Slint shell for the KORG editor rewrite.

slint::include_modules!();

use korg_app::panel::{panel as device_panel, Binding as PanelBinding};
use korg_app::{DeviceJob, JobOutput, JobProgress};
use korg_core::{
    Clock, ControllerAction, ControllerInput, ControllerOutcome, DeviceTarget, EditorController,
    MeasuredSession, ReceivedPart, SystemClock, TickRate, TransferEvent, TransferState,
};
use korg_devices::{measured_layout_for_key, Catalog};
use korg_format::{Chunk, ChunkKey, Payload};
use korg_midi::{Direction, MidiTransport};
use korg_midi_midir::MidirTransport;
use korg_model::{ChunkKind, ClassId, DeviceInfo, GlobalData, ParameterKind, Scene, SceneSet};
use slint::{Model, ModelRc, SharedString, Timer, TimerMode, VecModel};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

thread_local! {
    static SELECTED_CONTROL: RefCell<Option<usize>> = const { RefCell::new(None) };
    static LAST_CLASS: RefCell<Option<String>> = const { RefCell::new(None) };
    static JOB_BUSY: Cell<bool> = const { Cell::new(false) };
    static JOB_RX: RefCell<Option<Receiver<JobMessage>>> = const { RefCell::new(None) };
    static JOB_GEN: Cell<u64> = const { Cell::new(0) };
    static AUTO_TIMER: RefCell<Option<Timer>> = const { RefCell::new(None) };
    static WATCHDOG: RefCell<Option<Timer>> = const { RefCell::new(None) };
    static TAB: Cell<i32> = const { Cell::new(0) };
    static PORT_IN: RefCell<String> = const { RefCell::new(String::new()) };
    static PORT_OUT: RefCell<String> = const { RefCell::new(String::new()) };
    static LINK: Cell<Option<korg_app::LinkKind>> = const { Cell::new(None) };
    /// The saved MIDI backend id; changed only by the MIDI button, so a
    /// `--midi-backend` flag or `CTRL_MIDI_BACKEND` is not saved.
    static MIDI_BACKEND: RefCell<String> = const { RefCell::new(String::new()) };
    /// The open group of the control list, and the selection that opened it:
    /// the list follows a new selection but a header click can close it.
    static OPEN_GROUP: RefCell<Option<String>> = const { RefCell::new(None) };
    static OPEN_FOR: Cell<Option<usize>> = const { Cell::new(None) };
    /// Listens for the device's scene announcements on the selected link.
    static SCENE_MONITOR: RefCell<Option<korg_app::monitor::SceneMonitor<MidirTransport>>> =
        const { RefCell::new(None) };
    /// The scene the running job leaves the device on.
    static JOB_RESTORE: Cell<Option<u8>> = const { Cell::new(None) };
    static RECEIVE_MODE: Cell<u8> = const { Cell::new(0) };
    static WRITE_MODE: Cell<u8> = const { Cell::new(0) };
    static CURRENT_PATH: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    static WARN_BEFORE_WRITE: Cell<bool> = const { Cell::new(true) };
    static AUTO_RECEIVE: Cell<bool> = const { Cell::new(false) };
    static GLOBAL_TAB: Cell<i32> = const { Cell::new(0) };
    static CALIBRATION_STEP: Cell<usize> = const { Cell::new(0) };
    static FIRMWARE: RefCell<HashMap<String, Option<String>>> = RefCell::new(HashMap::new());
    static CLIPBOARD: RefCell<Option<Vec<u8>>> = const { RefCell::new(None) };
    /// The running device job, for `korg_app::written_part` when it ends.
    static RUNNING_JOB: RefCell<Option<DeviceJob>> = const { RefCell::new(None) };
    /// Controls that sent a message recently (MIDI monitor), and when.
    static LIT: RefCell<Vec<(usize, Instant)>> = const { RefCell::new(Vec::new()) };
    static MONITOR_SHOWN: Cell<bool> = const { Cell::new(false) };
    /// Detect Devices running on a worker; its result arrives here.
    static DETECT_RX: RefCell<Option<Receiver<DetectResult>>> = const { RefCell::new(None) };
    static DETECT_TIMER: RefCell<Option<Timer>> = const { RefCell::new(None) };
}

/// What Detect Devices reports from its worker.
type DetectResult = Result<Vec<korg_app::detect::Detected>, String>;

/// How long a control stays lit after it sent a message.
const LIT_FOR: Duration = Duration::from_millis(400);
/// Monitor lines the GUI shows.
const MONITOR_ROWS: usize = 40;

enum JobMessage {
    Progress(JobProgress),
    Done(Result<JobOutput, String>),
}

type Controller = EditorController<MidirTransport, SystemClock>;

/// MIDI channel for SysEx.  The device's own global channel is in its global
/// data, which the editor may not have loaded.  Placeholder, not measured.
const SYSEX_CHANNEL: u8 = 0;
/// Reply timeout per transfer step.  The original app's value is not
/// recovered; this is a placeholder.
use korg_app::TRANSFER_TIMEOUT_MS;
/// Characters per chip line in the 176 px control list.
const CHIP_LINE_CHARS: usize = 22;

const EXTENSIONS: &[&str] = &[
    "microkey_data",
    "microkey25_data",
    "microkey2_data",
    "microkey25air_data",
    "nanokey2_data",
    "nanokey_st_data",
    "nanokey_st_set",
    "nanokey_st_glob",
    "nanopad2_data",
    "nanopad2_set",
    "nanopad2_glob",
    "nktrl2_data",
    "nktrl_st_data",
    "nktrl_st_set",
    "nktrl_st_glob",
    "taktile_data",
    "taktile_set",
    "taktile_glob",
    "tr_taktile_data",
    "tr_taktile_set",
    "tr_taktile_glob",
    "keystage_data",
    "keystage_set",
    "keystage_glob",
    "nanokey_fold_data",
    "nanokey_fold_set",
    "nanokey_fold_glob",
];

fn main() -> Result<(), slint::PlatformError> {
    if let Err(error) = korg_app::panel::validate_external_panels() {
        eprintln!("device panels: {error}");
        std::process::exit(1);
    }
    let catalog = Rc::new(Catalog::load().unwrap_or_else(|error| {
        eprintln!("device catalog: {error}");
        std::process::exit(1);
    }));
    let preferences = load_preferences();
    // Flag, then CTRL_MIDI_BACKEND, then the saved choice, then the
    // platform's own; before the first transport.
    let flag = std::env::args()
        .position(|argument| argument == "--midi-backend")
        .and_then(|index| std::env::args().nth(index + 1));
    if let Err(error) = korg_midi_midir::init_backend(flag.as_deref(), &preferences.midi_backend) {
        eprintln!("{error}");
        std::process::exit(2);
    }
    MIDI_BACKEND.with(|saved| *saved.borrow_mut() = preferences.midi_backend.clone());
    WARN_BEFORE_WRITE.set(preferences.warn_before_write);
    AUTO_RECEIVE.set(preferences.auto_receive);
    LINK.set(preferences.link);
    // The old build stored free text here. Drop it so a stale value cannot
    // break Receive; the dropdown selects a port that CoreMIDI reports.
    PORT_IN.with(|port| port.borrow_mut().clear());
    PORT_OUT.with(|port| port.borrow_mut().clear());
    save_preferences();
    let ui = AppWindow::new()?;
    ui.set_warn_before_write(preferences.warn_before_write);
    if std::env::var_os("KORG_SHOW_VALUES").is_some() {
        ui.set_show_values(true);
    }
    ui.set_port_name("".into());
    let controller = Rc::new(RefCell::new(EditorController::new(
        MidirTransport::new(),
        SystemClock::new(),
    )));
    let selected_class: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let device_view = Rc::new(Cell::new(1));
    let search_text = Rc::new(RefCell::new(String::new()));

    let connected = connected_classes(&controller, &catalog);
    let ports = controller.borrow().view().ports.clone();
    ui.set_total_devices(catalog.devices.len() as i32);
    ui.set_devices(ModelRc::new(VecModel::from(build_device_cards(
        &catalog, &connected, 1, &ports, "",
    ))));
    let card_count = ui.get_devices().row_count();
    ui.set_screen(0);
    ui.set_select_status("Select a device or open a file.".into());
    eprintln!("device cards: {card_count}, assets: {:?}", asset_dir());

    {
        let weak = ui.as_weak();
        let controller = controller.clone();
        let catalog = catalog.clone();
        let selected_class = selected_class.clone();
        ui.on_open_file(move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let Some(path) = rfd::FileDialog::new()
                .add_filter("KORG data", EXTENSIONS)
                .pick_file()
            else {
                ui.set_status("No file selected.".into());
                return;
            };
            let status = match load_path(&controller, &catalog, &path, &selected_class) {
                Ok(status) => {
                    CURRENT_PATH.with(|current| *current.borrow_mut() = Some(path.clone()));
                    ui.set_screen(1);
                    status
                }
                Err(error) => error,
            };
            ui.set_status(status.clone().into());
            ui.set_select_status(status.into());
            refresh(&ui, &controller.borrow(), &catalog, &selected_class);
        });
    }

    {
        let weak = ui.as_weak();
        let controller = controller.clone();
        let catalog = catalog.clone();
        let selected_class = selected_class.clone();
        ui.on_new_document(move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let Some(class) = current_class(&controller, &catalog, &selected_class) else {
                ui.set_status("Select a device first.".into());
                return;
            };
            let Some(device) = catalog.find_by_class(&class) else {
                ui.set_status("Unknown device.".into());
                return;
            };
            let Some(model) = device.file_key() else {
                ui.set_status("Device has no file key.".into());
                return;
            };
            let Some(scenes) = factory_scenes(device) else {
                ui.set_status("No factory scene set for this device.".into());
                return;
            };
            let element_size = scenes.first().map(|scene| scene.data.len()).unwrap_or(0) as u16;
            {
                let mut controller = controller.borrow_mut();
                controller.set_schema(korg_devices::parameter_schema(&class, "scene"));
                controller.set_device(Some(device_target(device)));
                controller.load(ControllerInput::SceneSet(SceneSet {
                    model,
                    element_size,
                    scenes,
                }));
            }
            *selected_class.borrow_mut() = Some(class);
            ui.set_screen(1);
            ui.set_status("New document from factory data.".into());
            refresh(&ui, &controller.borrow(), &catalog, &selected_class);
        });
    }

    {
        let weak = ui.as_weak();
        let controller = controller.clone();
        let catalog = catalog.clone();
        let selected_class = selected_class.clone();
        let auto_timer = Rc::new(Timer::default());
        let auto_timer = auto_timer.clone();
        ui.on_open_device(move |class| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let Some(device) = catalog.find_by_class(class.as_str()) else {
                ui.set_select_status("Unknown device.".into());
                return;
            };
            match open_device(&controller, device) {
                Ok(status) => {
                    *selected_class.borrow_mut() = Some(device.class.clone());
                    ui.set_select_status(status.clone().into());
                    ui.set_status(status.into());
                    ui.set_screen(1);
                    refresh(&ui, &controller.borrow(), &catalog, &selected_class);
                    if AUTO_RECEIVE.get()
                        && korg_app::port_matches(&controller.borrow().view().ports, device)
                            .is_some()
                    {
                        start_measured_job(
                            &ui,
                            &controller,
                            &catalog,
                            &selected_class,
                            &auto_timer,
                            effective_ports(&device.device),
                            DeviceJob::ReceiveSceneSet,
                        );
                    }
                }
                Err(error) => ui.set_select_status(error.into()),
            }
        });
    }

    {
        let weak = ui.as_weak();
        let controller = controller.clone();
        let catalog = catalog.clone();
        let view_flag = device_view.clone();
        let search_flag = search_text.clone();
        ui.on_rescan(move || {
            let outcome = controller.borrow_mut().tick();
            let _ = outcome;
            let connected = connected_classes(&controller, &catalog);
            let ports = controller.borrow().view().ports.clone();
            let search = search_flag.borrow().clone();
            let cards = build_device_cards(&catalog, &connected, view_flag.get(), &ports, &search);
            let count = cards.len();
            if let Some(ui) = weak.upgrade() {
                ui.set_devices(ModelRc::new(VecModel::from(cards)));
                let _ = count;
                ui.set_select_status("Select a device or open a file.".into());
            }
        });
    }

    {
        let weak = ui.as_weak();
        let controller = controller.clone();
        let catalog = catalog.clone();
        let view_flag = device_view.clone();
        let search_flag = search_text.clone();
        ui.on_device_view_changed(move |value| {
            view_flag.set(value);
            let _ = controller.borrow_mut().tick();
            let connected = connected_classes(&controller, &catalog);
            let ports = controller.borrow().view().ports.clone();
            let search = search_flag.borrow().clone();
            let cards = build_device_cards(&catalog, &connected, value, &ports, &search);
            let count = cards.len();
            if let Some(ui) = weak.upgrade() {
                ui.set_device_view(value);
                ui.set_devices(ModelRc::new(VecModel::from(cards)));
                ui.set_select_status(if value == 0 && count == 0 {
                    "No connected KORG device found. Switch to All devices.".into()
                } else {
                    "Select a device or open a file.".into()
                });
            }
        });
    }

    {
        let weak = ui.as_weak();
        let controller = controller.clone();
        let catalog = catalog.clone();
        let view_flag = device_view.clone();
        let search_flag = search_text.clone();
        ui.on_search_changed(move |text| {
            let text = text.to_string();
            *search_flag.borrow_mut() = text.clone();
            let connected = connected_classes(&controller, &catalog);
            let ports = controller.borrow().view().ports.clone();
            let cards = build_device_cards(&catalog, &connected, view_flag.get(), &ports, &text);
            let count = cards.len();
            if let Some(ui) = weak.upgrade() {
                ui.set_search_text(text.into());
                ui.set_devices(ModelRc::new(VecModel::from(cards)));
                let _ = count;
                ui.set_select_status("Select a device or open a file.".into());
            }
        });
    }

    {
        let weak = ui.as_weak();
        ui.on_show_devices(move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_screen(0);
            }
        });
    }

    {
        let weak = ui.as_weak();
        let controller = controller.clone();
        let catalog = catalog.clone();
        let selected_class = selected_class.clone();
        ui.on_save_file(move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let status = match save_path(&controller, &catalog, &selected_class, false) {
                Ok(status) => status,
                Err(error) => error,
            };
            ui.set_status(status.into());
            refresh(&ui, &controller.borrow(), &catalog, &selected_class);
        });
    }

    {
        let weak = ui.as_weak();
        let controller = controller.clone();
        let selected_class = selected_class.clone();
        ui.on_save_sysex(move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let Some(path) = rfd::FileDialog::new()
                .set_file_name("korg-dump.syx")
                .add_filter("SysEx dump", &["syx"])
                .save_file()
            else {
                ui.set_status("Save cancelled.".into());
                return;
            };
            let class = selected_class.borrow().clone();
            let status =
                match korg_app::save_sysex_path(&controller.borrow(), class.as_deref(), &path) {
                    Ok(status) => status,
                    Err(error) => error,
                };
            ui.set_status(status.into());
        });
    }

    {
        let weak = ui.as_weak();
        let controller = controller.clone();
        let catalog = catalog.clone();
        let selected_class = selected_class.clone();
        ui.on_save_as(move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let status = match save_path(&controller, &catalog, &selected_class, true) {
                Ok(status) => status,
                Err(error) => error,
            };
            ui.set_status(status.into());
            refresh(&ui, &controller.borrow(), &catalog, &selected_class);
        });
    }

    {
        let weak = ui.as_weak();
        let controller = controller.clone();
        let catalog = catalog.clone();
        let selected_class = selected_class.clone();
        ui.on_undo(move || {
            controller.borrow_mut().dispatch(ControllerAction::Undo);
            if let Some(ui) = weak.upgrade() {
                refresh(&ui, &controller.borrow(), &catalog, &selected_class);
            }
        });
    }

    {
        let weak = ui.as_weak();
        let controller = controller.clone();
        let catalog = catalog.clone();
        let selected_class = selected_class.clone();
        ui.on_revert(move || {
            controller
                .borrow_mut()
                .dispatch(ControllerAction::RevertToReceived);
            if let Some(ui) = weak.upgrade() {
                refresh(&ui, &controller.borrow(), &catalog, &selected_class);
                ui.set_status(
                    "Reverted to the data last received from the device; Undo brings the edits back."
                        .into(),
                );
            }
        });
    }

    {
        let weak = ui.as_weak();
        ui.on_monitor_changed(move |shown| {
            MONITOR_SHOWN.set(shown);
            if let Some(ui) = weak.upgrade() {
                ui.set_monitor_visible(shown);
                if shown {
                    set_monitor_rows(&ui);
                }
            }
        });
    }

    {
        let weak = ui.as_weak();
        ui.on_clear_monitor(move || {
            SCENE_MONITOR.with(|monitor| {
                if let Some(monitor) = monitor.borrow_mut().as_mut() {
                    monitor.clear_log();
                }
            });
            if let Some(ui) = weak.upgrade() {
                set_monitor_rows(&ui);
            }
        });
    }

    {
        let weak = ui.as_weak();
        let controller = controller.clone();
        let catalog = catalog.clone();
        let view_flag = device_view.clone();
        let search_flag = search_text.clone();
        ui.on_detect(move || {
            if JOB_BUSY.get() || DETECT_RX.with(|rx| rx.borrow().is_some()) {
                return;
            }
            if let Some(ui) = weak.upgrade() {
                ui.set_select_status("Asking every MIDI port which KORG device answers…".into());
            }
            let (tx, rx) = mpsc::channel();
            DETECT_RX.with(|slot| *slot.borrow_mut() = Some(rx));
            let worker_catalog = (*catalog).clone();
            std::thread::spawn(move || {
                let _ = tx.send(korg_app::detect::detect(
                    MidirTransport::new,
                    &worker_catalog,
                    korg_app::detect::PROBE_TIMEOUT,
                ));
            });
            let weak = weak.clone();
            let controller = controller.clone();
            let catalog = catalog.clone();
            let view_flag = view_flag.clone();
            let search_flag = search_flag.clone();
            let timer = Timer::default();
            timer.start(TimerMode::Repeated, Duration::from_millis(100), move || {
                let Some(result) =
                    DETECT_RX.with(|slot| slot.borrow().as_ref().and_then(|rx| rx.try_recv().ok()))
                else {
                    return;
                };
                DETECT_RX.with(|slot| slot.borrow_mut().take());
                let status = match result {
                    Err(error) => format!("Detect failed: {error}"),
                    Ok(found) => {
                        korg_app::detect::remember(&found);
                        // Firmware now comes from the replies.
                        FIRMWARE.with(|cache| cache.borrow_mut().clear());
                        let names: Vec<String> =
                            found.iter().map(|d| d.summary(&catalog)).collect();
                        match names.len() {
                            0 => "No KORG device answered.".to_owned(),
                            n => format!("Found {n}: {}", names.join("; ")),
                        }
                    }
                };
                if let Some(ui) = weak.upgrade() {
                    let _ = controller.borrow_mut().tick();
                    let connected = connected_classes(&controller, &catalog);
                    let ports = controller.borrow().view().ports.clone();
                    let search = search_flag.borrow().clone();
                    ui.set_devices(ModelRc::new(VecModel::from(build_device_cards(
                        &catalog,
                        &connected,
                        view_flag.get(),
                        &ports,
                        &search,
                    ))));
                    ui.set_select_status(status.into());
                }
                DETECT_TIMER.with(|slot| slot.borrow_mut().take());
            });
            DETECT_TIMER.with(|slot| *slot.borrow_mut() = Some(timer));
        });
    }

    {
        let weak = ui.as_weak();
        let controller = controller.clone();
        let catalog = catalog.clone();
        let selected_class = selected_class.clone();
        ui.on_redo(move || {
            controller.borrow_mut().dispatch(ControllerAction::Redo);
            if let Some(ui) = weak.upgrade() {
                refresh(&ui, &controller.borrow(), &catalog, &selected_class);
            }
        });
    }

    {
        let weak = ui.as_weak();
        let controller = controller.clone();
        let catalog = catalog.clone();
        let selected_class = selected_class.clone();
        ui.on_scene_selected(move |index| {
            controller
                .borrow_mut()
                .dispatch(ControllerAction::SelectScene(index as usize));
            if let Some(ui) = weak.upgrade() {
                refresh(&ui, &controller.borrow(), &catalog, &selected_class);
            }
        });
    }

    {
        let weak = ui.as_weak();
        let controller = controller.clone();
        let catalog = catalog.clone();
        let selected_class = selected_class.clone();
        ui.on_set_param(move |element, index, value| {
            if let (Ok(element), Ok(index)) = (usize::try_from(element), usize::try_from(index)) {
                controller
                    .borrow_mut()
                    .dispatch(ControllerAction::SetParameter {
                        element,
                        index,
                        value,
                    });
            }
            if let Some(ui) = weak.upgrade() {
                refresh(&ui, &controller.borrow(), &catalog, &selected_class);
            }
        });
    }

    {
        let weak = ui.as_weak();
        let controller = controller.clone();
        let catalog = catalog.clone();
        let selected_class = selected_class.clone();
        ui.on_set_param_choice(move |offset, label| {
            let class = selected_class.borrow().clone().unwrap_or_default();
            edit_scene(&controller, |data| {
                let Some((panel, field)) = selected_page_field(&class, data, offset as usize)
                else {
                    return false;
                };
                korg_app::pages::choice_index(&field, &label).is_some_and(|value| {
                    korg_app::pages::store(&class, &panel, data, &field, value)
                })
            });
            if let Some(ui) = weak.upgrade() {
                refresh(&ui, &controller.borrow(), &catalog, &selected_class);
            }
        });
    }

    {
        let weak = ui.as_weak();
        let controller = controller.clone();
        let catalog = catalog.clone();
        let selected_class = selected_class.clone();
        ui.on_set_param_byte(move |offset, value| {
            let class = selected_class.borrow().clone().unwrap_or_default();
            edit_scene(&controller, |data| {
                selected_page_field(&class, data, offset as usize).is_some_and(|(panel, field)| {
                    korg_app::pages::store(&class, &panel, data, &field, value)
                })
            });
            if let Some(ui) = weak.upgrade() {
                refresh(&ui, &controller.borrow(), &catalog, &selected_class);
            }
        });
    }

    {
        let weak = ui.as_weak();
        let controller = controller.clone();
        let catalog = catalog.clone();
        let selected_class = selected_class.clone();
        ui.on_set_param_text(move |offset, text| {
            let class = selected_class.borrow().clone().unwrap_or_default();
            edit_scene(&controller, |data| {
                selected_page_field(&class, data, offset as usize)
                    .is_some_and(|(_, field)| korg_app::pages::store_text(data, &field, &text))
            });
            if let Some(ui) = weak.upgrade() {
                refresh(&ui, &controller.borrow(), &catalog, &selected_class);
            }
        });
    }

    {
        let weak = ui.as_weak();
        let controller = controller.clone();
        let catalog = catalog.clone();
        let selected_class = selected_class.clone();
        ui.on_set_param_schema_text(move |element, index, text| {
            if let (Ok(element), Ok(index)) = (usize::try_from(element), usize::try_from(index)) {
                controller
                    .borrow_mut()
                    .dispatch(ControllerAction::SetParameterText {
                        element,
                        index,
                        text: text.to_string(),
                    });
            }
            if let Some(ui) = weak.upgrade() {
                refresh(&ui, &controller.borrow(), &catalog, &selected_class);
            }
        });
    }

    {
        let weak = ui.as_weak();
        let controller = controller.clone();
        let catalog = catalog.clone();
        let selected_class = selected_class.clone();
        ui.on_set_global_param(move |element, index, value| {
            if let (Ok(element), Ok(index)) = (usize::try_from(element), usize::try_from(index)) {
                controller
                    .borrow_mut()
                    .dispatch(ControllerAction::SetGlobalParameter {
                        element,
                        index,
                        value,
                    });
            }
            if let Some(ui) = weak.upgrade() {
                refresh(&ui, &controller.borrow(), &catalog, &selected_class);
            }
        });
    }

    {
        let weak = ui.as_weak();
        let controller = controller.clone();
        let catalog = catalog.clone();
        let selected_class = selected_class.clone();
        ui.on_set_global_param_label(move |element, index, label| {
            let row = controller
                .borrow()
                .view()
                .global_params
                .into_iter()
                .find(|row| row.element as i32 == element && row.index as i32 == index);
            let value = row.and_then(|row| {
                let row = global_row(&row);
                let position = (0..row.choices.row_count())
                    .position(|at| row.choices.row_data(at).as_deref() == Some(label.as_str()))?;
                Some(row.minimum + position as i32)
            });
            if let (Some(value), Ok(element), Ok(index)) =
                (value, usize::try_from(element), usize::try_from(index))
            {
                controller
                    .borrow_mut()
                    .dispatch(ControllerAction::SetGlobalParameter {
                        element,
                        index,
                        value,
                    });
            }
            if let Some(ui) = weak.upgrade() {
                refresh(&ui, &controller.borrow(), &catalog, &selected_class);
            }
        });
    }

    {
        let weak = ui.as_weak();
        let controller = controller.clone();
        let catalog = catalog.clone();
        let selected_class = selected_class.clone();
        ui.on_set_param_label(move |element, index, label| {
            let Ok(index) = usize::try_from(index) else {
                return;
            };
            let key = controller
                .borrow()
                .schema()
                .and_then(|schema| schema.parameters.get(index))
                .map(|parameter| parameter.key.clone())
                .unwrap_or_default();
            let value = {
                let controller = controller.borrow();
                controller.schema().and_then(|schema| {
                    let parameter = schema.parameters.get(index)?;
                    match korg_app::assign::channel_field(
                        &parameter.key,
                        parameter.kind,
                        parameter.max,
                    ) {
                        Some(kind) => kind
                            .labels()
                            .iter()
                            .position(|choice| choice.eq_ignore_ascii_case(&label))
                            .map(|index| index as i32),
                        None => parameter
                            .labels
                            .iter()
                            .position(|choice| choice == &label.to_string())
                            .map(|index| parameter.min + index as i32),
                    }
                })
            };
            let value = value.or_else(|| match (key.as_str(), label.as_str()) {
                ("AssignType", "Disable") => Some(0),
                ("AssignType", "Enable") => Some(1),
                _ => None,
            });
            if let Some(value) = value {
                controller
                    .borrow_mut()
                    .dispatch(ControllerAction::SetParameter {
                        element: element as usize,
                        index,
                        value,
                    });
            }
            if let Some(ui) = weak.upgrade() {
                refresh(&ui, &controller.borrow(), &catalog, &selected_class);
            }
        });
    }

    // The device announces scene changes; poll the monitor often enough
    // that the scene tab marker follows a button press.
    // It also feeds the MIDI monitor: lit controls and the message log.
    let scene_timer = Timer::default();
    {
        let weak = ui.as_weak();
        let controller = controller.clone();
        let selected_class = selected_class.clone();
        scene_timer.start(TimerMode::Repeated, Duration::from_millis(50), move || {
            let (changed, events) = SCENE_MONITOR.with(|monitor| {
                monitor
                    .borrow_mut()
                    .as_mut()
                    .map_or((false, Vec::new()), |monitor| {
                        (monitor.poll(), monitor.take_live())
                    })
            });
            let Some(ui) = weak.upgrade() else {
                return;
            };
            if let (true, Some(scene)) = (changed, device_scene()) {
                ui.set_device_scene(i32::from(scene));
                ui.set_status(format!("The device switched to scene {}.", scene + 1).into());
            }
            if ui.get_screen() == 1 {
                show_live(&ui, &controller, &selected_class, &events);
            }
        });
    }

    // One timer drives `EditorController::tick` at the rate it asks for:
    // fast during a transfer, slow to watch for hot-plug, stopped when idle.
    let timer = Rc::new(Timer::default());
    {
        let weak = ui.as_weak();
        let controller = controller.clone();
        let catalog = catalog.clone();
        let selected_class = selected_class.clone();
        let device_view = device_view.clone();
        let search_text = search_text.clone();
        let ticker = Rc::downgrade(&timer);
        timer.start(
            TimerMode::Repeated,
            Duration::from_millis(TickRate::Slow.interval_ms().unwrap_or(1_000)),
            move || {
                let Some(ui) = weak.upgrade() else {
                    return;
                };
                // Release the borrow before touching the UI.
                let outcome = controller.borrow_mut().tick();
                apply_outcome(&ui, &controller, &catalog, &selected_class, &outcome);
                if outcome.ports_changed && ui.get_screen() == 0 {
                    let connected = connected_classes(&controller, &catalog);
                    let ports = controller.borrow().view().ports.clone();
                    ui.set_devices(ModelRc::new(VecModel::from(build_device_cards(
                        &catalog,
                        &connected,
                        device_view.get(),
                        &ports,
                        &search_text.borrow(),
                    ))));
                }
                if let Some(timer) = ticker.upgrade() {
                    let rate = controller.borrow().wants_tick();
                    apply_rate(&timer, rate);
                }
            },
        );
    }

    let job_timer = Rc::new(Timer::default());
    {
        let weak = ui.as_weak();
        let controller = controller.clone();
        let catalog = catalog.clone();
        let selected_class = selected_class.clone();
        let job_timer = job_timer.clone();
        ui.on_receive_data(move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            start_receive(&ui, &controller, &catalog, &selected_class, &job_timer, 0);
        });
    }

    {
        let weak = ui.as_weak();
        let controller = controller.clone();
        let catalog = catalog.clone();
        let selected_class = selected_class.clone();
        let job_timer = job_timer.clone();
        ui.on_receive_scene_data(move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            start_receive(&ui, &controller, &catalog, &selected_class, &job_timer, 1);
        });
    }

    {
        let weak = ui.as_weak();
        let controller = controller.clone();
        let catalog = catalog.clone();
        let selected_class = selected_class.clone();
        ui.on_tab_selected(move |tab| {
            TAB.set(tab);
            if let Some(ui) = weak.upgrade() {
                refresh(&ui, &controller.borrow(), &catalog, &selected_class);
            }
        });
    }

    {
        let weak = ui.as_weak();
        let controller = controller.clone();
        let catalog = catalog.clone();
        let selected_class = selected_class.clone();
        ui.on_open_factory(move |path| {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let status = match load_path(&controller, &catalog, Path::new(&path), &selected_class) {
                Ok(status) => {
                    ui.set_screen(1);
                    status
                }
                Err(error) => error,
            };
            ui.set_status(status.into());
            refresh(&ui, &controller.borrow(), &catalog, &selected_class);
        });
    }

    {
        let weak = ui.as_weak();
        let controller = controller.clone();
        ui.on_copy_scene(move || {
            let controller_ref = controller.borrow();
            let document = controller_ref.session().document();
            let index = controller_ref.session().selected_scene();
            if let Some(scene) = document
                .scene_set
                .as_ref()
                .and_then(|set| set.scenes.get(index))
            {
                CLIPBOARD.with(|clipboard| *clipboard.borrow_mut() = Some(scene.data.clone()));
                if let Some(ui) = weak.upgrade() {
                    ui.set_status(format!("Copied scene {}.", index + 1).into());
                }
            }
        });
    }

    {
        let weak = ui.as_weak();
        let controller = controller.clone();
        let catalog = catalog.clone();
        let selected_class = selected_class.clone();
        ui.on_paste_scene(move || {
            let data = CLIPBOARD.with(|clipboard| clipboard.borrow().clone());
            let Some(data) = data else {
                if let Some(ui) = weak.upgrade() {
                    ui.set_status("Clipboard is empty.".into());
                }
                return;
            };
            let index = controller.borrow().session().selected_scene();
            controller
                .borrow_mut()
                .dispatch(ControllerAction::SetSceneData { index, data });
            if let Some(ui) = weak.upgrade() {
                ui.set_status(format!("Pasted scene {}.", index + 1).into());
                refresh(&ui, &controller.borrow(), &catalog, &selected_class);
            }
        });
    }

    {
        let weak = ui.as_weak();
        let controller = controller.clone();
        let catalog = catalog.clone();
        let selected_class = selected_class.clone();
        ui.on_cut_scene(move || {
            let data = {
                let controller_ref = controller.borrow();
                let document = controller_ref.session().document();
                let index = controller_ref.session().selected_scene();
                document
                    .scene_set
                    .as_ref()
                    .and_then(|set| set.scenes.get(index))
                    .map(|scene| scene.data.clone())
            };
            let Some(data) = data else {
                if let Some(ui) = weak.upgrade() {
                    ui.set_status("No scene is selected.".into());
                }
                return;
            };
            CLIPBOARD.with(|clipboard| *clipboard.borrow_mut() = Some(data.clone()));
            let index = controller.borrow().session().selected_scene();
            controller
                .borrow_mut()
                .dispatch(ControllerAction::SetSceneData {
                    index,
                    data: vec![0u8; data.len()],
                });
            if let Some(ui) = weak.upgrade() {
                ui.set_status(format!("Cut scene {}.", index + 1).into());
                refresh(&ui, &controller.borrow(), &catalog, &selected_class);
            }
        });
    }

    {
        let weak = ui.as_weak();
        let controller = controller.clone();
        let catalog = catalog.clone();
        let selected_class = selected_class.clone();
        ui.on_clear_scene(move || {
            let (index, len) = {
                let controller_ref = controller.borrow();
                let session = controller_ref.session();
                let index = session.selected_scene();
                let len = session
                    .document()
                    .scene_set
                    .as_ref()
                    .and_then(|set| set.scenes.get(index))
                    .map(|scene| scene.data.len());
                (index, len)
            };
            let Some(len) = len else {
                if let Some(ui) = weak.upgrade() {
                    ui.set_status("No scene is selected.".into());
                }
                return;
            };
            controller
                .borrow_mut()
                .dispatch(ControllerAction::SetSceneData {
                    index,
                    data: vec![0u8; len],
                });
            if let Some(ui) = weak.upgrade() {
                ui.set_status(format!("Cleared scene {}.", index + 1).into());
                refresh(&ui, &controller.borrow(), &catalog, &selected_class);
            }
        });
    }

    {
        let weak = ui.as_weak();
        let controller = controller.clone();
        let catalog = catalog.clone();
        let selected_class = selected_class.clone();
        ui.on_port_in_changed(move |text| {
            PORT_IN.with(|port| *port.borrow_mut() = text.to_string());
            LINK.set(Some(korg_app::LinkKind::of_port(&text)));
            save_preferences();
            if let Some(ui) = weak.upgrade() {
                refresh(&ui, &controller.borrow(), &catalog, &selected_class);
            }
        });
    }

    {
        ui.on_port_out_changed(move |text| {
            PORT_OUT.with(|port| *port.borrow_mut() = text.to_string());
            save_preferences();
        });
    }

    let midi = Rc::new(RefCell::new(korg_app::MidiBackendSwitch::new(
        korg_midi_midir::backend_info(korg_midi_midir::backend()),
        korg_midi_midir::backends().len(),
        korg_midi_midir::switch_to_next_backend,
    )));
    show_midi_backend(&ui, &midi.borrow());
    if midi.borrow().current().problem.is_some() {
        ui.set_select_status(midi.borrow().status().into());
    }
    {
        let weak = ui.as_weak();
        let controller = controller.clone();
        let catalog = catalog.clone();
        let selected_class = selected_class.clone();
        let midi = midi.clone();
        ui.on_midi_backend_switch(move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            if JOB_BUSY.get() {
                ui.set_status("A transfer is running. Switch the MIDI backend after it.".into());
                return;
            }
            let switched = midi.borrow_mut().switch().cloned();
            let status = match switched {
                Ok(info) => {
                    MIDI_BACKEND.with(|saved| *saved.borrow_mut() = info.id.clone());
                    save_preferences();
                    // Port names differ between backends: take the pair from
                    // the link again once the next scan lists the new ports.
                    PORT_IN.with(|port| port.borrow_mut().clear());
                    PORT_OUT.with(|port| port.borrow_mut().clear());
                    SCENE_MONITOR.with(|monitor| {
                        if let Some(monitor) = monitor.borrow_mut().as_mut() {
                            monitor.disconnect();
                        }
                    });
                    korg_app::midi_backend::status(&info)
                }
                Err(message) => message.to_owned(),
            };
            show_midi_backend(&ui, &midi.borrow());
            ui.set_status(status.clone().into());
            ui.set_select_status(status.into());
            refresh(&ui, &controller.borrow(), &catalog, &selected_class);
        });
    }

    {
        let weak = ui.as_weak();
        let controller = controller.clone();
        let catalog = catalog.clone();
        let selected_class = selected_class.clone();
        ui.on_link_changed(move |label| {
            let kind = [korg_app::LinkKind::Usb, korg_app::LinkKind::Bluetooth]
                .into_iter()
                .find(|kind| kind.label() == label.as_str());
            LINK.set(kind);
            // Clear both so refresh takes the pair from the chosen link.
            PORT_IN.with(|port| port.borrow_mut().clear());
            PORT_OUT.with(|port| port.borrow_mut().clear());
            if let Some(ui) = weak.upgrade() {
                refresh(&ui, &controller.borrow(), &catalog, &selected_class);
                ui.set_status(format!("Link: {label}.").into());
            }
            save_preferences();
        });
    }

    {
        ui.on_warn_changed(move |value| {
            WARN_BEFORE_WRITE.set(value);
            save_preferences();
        });
    }

    {
        ui.on_auto_receive_changed(move |value| {
            AUTO_RECEIVE.set(value);
            save_preferences();
        });
    }

    {
        let weak = ui.as_weak();
        ui.on_values_changed(move |value| {
            if let Some(ui) = weak.upgrade() {
                ui.set_show_values(value);
            }
        });
    }

    {
        let weak = ui.as_weak();
        let controller = controller.clone();
        let catalog = catalog.clone();
        let selected_class = selected_class.clone();
        ui.on_group_toggled(move |name| {
            OPEN_GROUP.with(|open| {
                let next = korg_app::panel::toggle_group(open.borrow().as_deref(), &name);
                *open.borrow_mut() = next;
            });
            if let Some(ui) = weak.upgrade() {
                refresh(&ui, &controller.borrow(), &catalog, &selected_class);
            }
        });
    }

    {
        let weak = ui.as_weak();
        let controller = controller.clone();
        let catalog = catalog.clone();
        let selected_class = selected_class.clone();
        ui.on_control_selected(move |index| {
            SELECTED_CONTROL.with(|selected| *selected.borrow_mut() = Some(index as usize));
            if let Some(ui) = weak.upgrade() {
                refresh(&ui, &controller.borrow(), &catalog, &selected_class);
            }
        });
    }

    {
        let weak = ui.as_weak();
        let controller = controller.clone();
        let catalog = catalog.clone();
        let selected_class = selected_class.clone();
        let job_timer = job_timer.clone();
        ui.on_calibrate(move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let class = match selected_class.borrow().clone() {
                Some(class) => class,
                None => {
                    ui.set_status("Select a device first.".into());
                    return;
                }
            };
            if class != "X14800" {
                ui.set_status("Touch-pad calibration is only for the nanoKEY Studio.".into());
                return;
            }
            if CALIBRATION_STEP.get() >= 4 {
                CALIBRATION_STEP.set(0);
            }
            let command = [0x01u8, 0x03, 0x02, 0x04][CALIBRATION_STEP.get()];
            let ports = effective_ports("nanoKEY Studio");
            start_measured_job(
                &ui,
                &controller,
                &catalog,
                &selected_class,
                &job_timer,
                ports,
                DeviceJob::Calibrate { command },
            );
        });
    }

    {
        let weak = ui.as_weak();
        let controller = controller.clone();
        let catalog = catalog.clone();
        let selected_class = selected_class.clone();
        let job_timer = job_timer.clone();
        ui.on_receive_global(move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let Some(class) = current_class(&controller, &catalog, &selected_class) else {
                ui.set_status("Open a device first.".into());
                return;
            };
            let Some(device) = catalog.find_by_class(&class) else {
                ui.set_status("Unknown device.".into());
                return;
            };
            start_measured_job(
                &ui,
                &controller,
                &catalog,
                &selected_class,
                &job_timer,
                effective_ports(&device.device),
                DeviceJob::ReceiveGlobal,
            );
        });
    }

    {
        let weak = ui.as_weak();
        let controller = controller.clone();
        let catalog = catalog.clone();
        let selected_class = selected_class.clone();
        let job_timer = job_timer.clone();
        ui.on_write_data(move || {
            WRITE_MODE.set(0);
            let Some(ui) = weak.upgrade() else {
                return;
            };
            if WARN_BEFORE_WRITE.get() {
                ui.set_confirm_write_visible(true);
                ui.set_status("Confirm the write.".into());
            } else {
                start_write(&ui, &controller, &catalog, &selected_class, &job_timer);
            }
        });
    }

    {
        let weak = ui.as_weak();
        let controller = controller.clone();
        let catalog = catalog.clone();
        let selected_class = selected_class.clone();
        let job_timer = job_timer.clone();
        ui.on_write_scene_data(move || {
            WRITE_MODE.set(1);
            let Some(ui) = weak.upgrade() else {
                return;
            };
            if WARN_BEFORE_WRITE.get() {
                ui.set_confirm_write_visible(true);
                ui.set_status("Confirm the scene write.".into());
            } else {
                start_write(&ui, &controller, &catalog, &selected_class, &job_timer);
            }
        });
    }

    {
        let weak = ui.as_weak();
        ui.on_global_tab_selected(move |tab| {
            GLOBAL_TAB.set(tab);
            if let Some(ui) = weak.upgrade() {
                ui.set_global_tab(tab);
            }
        });
    }

    {
        let weak = ui.as_weak();
        let controller = controller.clone();
        let catalog = catalog.clone();
        let selected_class = selected_class.clone();
        ui.on_set_global_choice(move |role, index| {
            let (Ok(role), Ok(index)) = (usize::try_from(role), usize::try_from(index)) else {
                return;
            };
            let Some(class) = current_class(&controller, &catalog, &selected_class) else {
                return;
            };
            let fields = korg_devices::global_fields(&class);
            let Some((offset, value)) =
                korg_devices::global_ui_choice_selection(fields, role, index)
            else {
                return;
            };
            controller
                .borrow_mut()
                .dispatch(ControllerAction::SetGlobalByte { offset, value });
            if let Some(ui) = weak.upgrade() {
                refresh(&ui, &controller.borrow(), &catalog, &selected_class);
            }
        });
    }

    {
        let weak = ui.as_weak();
        let controller = controller.clone();
        let catalog = catalog.clone();
        let selected_class = selected_class.clone();
        ui.on_set_global_byte(move |role, value| {
            let Ok(role) = usize::try_from(role) else {
                return;
            };
            let Some(class) = current_class(&controller, &catalog, &selected_class) else {
                return;
            };
            let fields = korg_devices::global_fields(&class);
            let Some((offset, value)) =
                korg_devices::global_ui_number_selection(fields, role, value)
            else {
                return;
            };
            controller
                .borrow_mut()
                .dispatch(ControllerAction::SetGlobalByte { offset, value });
            if let Some(ui) = weak.upgrade() {
                refresh(&ui, &controller.borrow(), &catalog, &selected_class);
            }
        });
    }

    {
        let weak = ui.as_weak();
        let controller = controller.clone();
        let catalog = catalog.clone();
        let selected_class = selected_class.clone();
        ui.on_set_global_text(move |offset, text| {
            if offset < 0 {
                return;
            }
            controller
                .borrow_mut()
                .dispatch(ControllerAction::SetGlobalText {
                    offset: offset as usize,
                    text: text.to_string(),
                });
            if let Some(ui) = weak.upgrade() {
                refresh(&ui, &controller.borrow(), &catalog, &selected_class);
            }
        });
    }

    {
        let weak = ui.as_weak();
        let controller = controller.clone();
        let catalog = catalog.clone();
        let selected_class = selected_class.clone();
        ui.on_set_global_bpm(move |value| {
            let Some(class) = current_class(&controller, &catalog, &selected_class) else {
                return;
            };
            let Some(range) =
                korg_devices::global_ui_bpm_range(korg_devices::global_fields(&class))
            else {
                return;
            };
            if !(range.min..=range.max).contains(&i64::from(value)) {
                return;
            }
            dispatch_ui_bpm(&mut controller.borrow_mut(), value);
            if let Some(ui) = weak.upgrade() {
                refresh(&ui, &controller.borrow(), &catalog, &selected_class);
            }
        });
    }

    {
        let weak = ui.as_weak();
        let controller = controller.clone();
        let catalog = catalog.clone();
        let selected_class = selected_class.clone();
        let job_timer = job_timer.clone();
        ui.on_write_global(move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            if controller.borrow().session().document().global.is_none() {
                ui.set_status("No global data is loaded.".into());
                return;
            }
            if WARN_BEFORE_WRITE.get() {
                ui.set_confirm_global_write_visible(true);
                ui.set_status("Confirm the global write.".into());
            } else {
                start_global_write(&ui, &controller, &catalog, &selected_class, &job_timer);
            }
        });
    }

    {
        let weak = ui.as_weak();
        let controller = controller.clone();
        let catalog = catalog.clone();
        let selected_class = selected_class.clone();
        let job_timer = job_timer.clone();
        ui.on_confirm_global_write(move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            ui.set_confirm_global_write_visible(false);
            start_global_write(&ui, &controller, &catalog, &selected_class, &job_timer);
        });
    }

    {
        let weak = ui.as_weak();
        ui.on_cancel_global_write(move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_confirm_global_write_visible(false);
                ui.set_status("Write cancelled.".into());
            }
        });
    }

    {
        let weak = ui.as_weak();
        let controller = controller.clone();
        let catalog = catalog.clone();
        let selected_class = selected_class.clone();
        ui.on_confirm_write(move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            ui.set_confirm_write_visible(false);
            start_write(&ui, &controller, &catalog, &selected_class, &job_timer);
        });
    }

    {
        let weak = ui.as_weak();
        ui.on_cancel_write(move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_confirm_write_visible(false);
                ui.set_status("Write cancelled.".into());
            }
        });
    }

    {
        let weak = ui.as_weak();
        let controller = controller.clone();
        let catalog = catalog.clone();
        let selected_class = selected_class.clone();
        let timer = timer.clone();
        ui.on_cancel_transfer(move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let outcome = controller
                .borrow_mut()
                .dispatch(ControllerAction::CancelTransfer);
            JOB_GEN.set(JOB_GEN.get().wrapping_add(1));
            AUTO_TIMER.with(|slot| slot.borrow_mut().take());
            JOB_RX.with(|cell| cell.borrow_mut().take());
            JOB_BUSY.set(false);
            ui.set_transfer_busy(false);
            ui.set_status("Transfer cancelled.".into());
            apply_outcome(&ui, &controller, &catalog, &selected_class, &outcome);
            let rate = controller.borrow().wants_tick();
            apply_rate(&timer, rate);
        });
    }

    if let Some(index) = std::env::args().position(|argument| argument == "--open-device") {
        if let Some(class) = std::env::args().nth(index + 1) {
            if let Some(device) = catalog.find_by_class(&class) {
                match open_device(&controller, device) {
                    Ok(status) => {
                        *selected_class.borrow_mut() = Some(device.class.clone());
                        ui.set_status(status.into());
                        ui.set_screen(1);
                    }
                    Err(error) => ui.set_select_status(error.into()),
                }
            }
        }
    }

    refresh(&ui, &controller.borrow(), &catalog, &selected_class);
    if let Some(index) = std::env::args().position(|argument| argument == "--open-file") {
        if let Some(path) = std::env::args().nth(index + 1) {
            match load_path(&controller, &catalog, Path::new(&path), &selected_class) {
                Ok(status) => {
                    CURRENT_PATH.with(|current| *current.borrow_mut() = Some(PathBuf::from(path)));
                    ui.set_status(status.into());
                    ui.set_screen(1);
                    refresh(&ui, &controller.borrow(), &catalog, &selected_class);
                }
                Err(error) => ui.set_status(error.into()),
            }
        }
    }

    if let Some(index) = std::env::args().position(|argument| argument == "--tab") {
        if let Some(tab) = std::env::args()
            .nth(index + 1)
            .and_then(|tab| tab.parse().ok())
        {
            TAB.set(tab);
            ui.set_tab(tab);
        }
    }
    if std::env::args().any(|argument| argument == "--values") {
        ui.set_show_values(true);
    }
    ui.run()
}

fn load_path(
    controller: &Rc<RefCell<Controller>>,
    catalog: &Catalog,
    path: &Path,
    selected_class: &RefCell<Option<String>>,
) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let chunk =
        korg_format::decode(&bytes).map_err(|error| format!("{}: {error}", path.display()))?;
    let model = chunk.key.model.clone();
    let extension = path.extension().and_then(|extension| extension.to_str());
    let class = catalog
        .find_by_file_key(&model)
        .into_iter()
        .find(|device| {
            extension.is_none_or(|extension| {
                device
                    .extensions
                    .iter()
                    .any(|candidate| candidate == extension)
            })
        })
        .map(|device| device.class.clone());
    *selected_class.borrow_mut() = class.clone();
    let payload_name = if matches!(&chunk.payload, Payload::Global(_)) {
        "global"
    } else {
        "scene"
    };
    let schema = class
        .as_deref()
        .and_then(|class| korg_devices::parameter_schema(class, payload_name));

    let target = class
        .as_deref()
        .and_then(|class| catalog.find_by_class(class))
        .map(device_target);
    let mut controller = controller.borrow_mut();
    controller.set_schema(schema);
    controller.set_device(target);
    match chunk.payload {
        Payload::SceneSet(scenes) => {
            let element_size = scenes.first().map(|scene| scene.len()).unwrap_or(0) as u16;
            let scenes = scenes
                .into_iter()
                .map(|data| Scene {
                    name: korg_model::scene_name(&model, &data),
                    data,
                })
                .collect();
            controller.load(ControllerInput::SceneSet(SceneSet {
                model,
                element_size,
                scenes,
            }));
        }
        Payload::Scene(data) => {
            let element_size = data.len() as u16;
            let name = korg_model::scene_name(&model, &data);
            controller.load(ControllerInput::SceneSet(SceneSet {
                model,
                element_size,
                scenes: vec![Scene { name, data }],
            }));
        }
        Payload::Global(data) => {
            controller.load(ControllerInput::Global(GlobalData { model, data }));
        }
    }
    Ok(format!("Loaded {} ({})", path.display(), chunk.key))
}

/// Prefer the vendor scene-set extension.  Older models ship only `_data`.
fn preferred_scene_extension(extensions: &[String]) -> Option<&str> {
    extensions
        .iter()
        .find(|extension| extension.ends_with("_set"))
        .or_else(|| {
            extensions
                .iter()
                .find(|extension| extension.ends_with("_data"))
        })
        .map(String::as_str)
}

fn save_path(
    controller: &Rc<RefCell<Controller>>,
    catalog: &Catalog,
    selected_class: &RefCell<Option<String>>,
    save_as: bool,
) -> Result<String, String> {
    let (chunk, extension) = {
        let controller = controller.borrow();
        let document = controller.session().document();
        if let Some(scene_set) = &document.scene_set {
            let extension = selected_class
                .borrow()
                .as_deref()
                .and_then(|class| catalog.find_by_class(class))
                .and_then(|device| preferred_scene_extension(&device.extensions).map(str::to_owned))
                .unwrap_or_else(|| "korg_data".to_owned());
            (
                Chunk {
                    key: ChunkKey::new(scene_set.model.clone(), ChunkKind::SceneSet),
                    payload: Payload::SceneSet(
                        scene_set
                            .scenes
                            .iter()
                            .map(|scene| scene.data.clone())
                            .collect(),
                    ),
                },
                extension,
            )
        } else if let Some(global) = &document.global {
            let extension = selected_class
                .borrow()
                .as_deref()
                .and_then(|class| catalog.find_by_class(class))
                .and_then(|device| {
                    device
                        .extensions
                        .iter()
                        .find(|extension| extension.ends_with("_glob"))
                        .cloned()
                })
                .unwrap_or_else(|| "korg_glob".to_owned());
            (
                Chunk {
                    key: ChunkKey::new(global.model.clone(), ChunkKind::GlobalData),
                    payload: Payload::Global(global.data.clone()),
                },
                extension,
            )
        } else {
            return Err("No document is loaded.".to_owned());
        }
    };

    let bytes = korg_format::encode(&chunk).map_err(|error| error.to_string())?;
    let current = CURRENT_PATH.with(|path| path.borrow().clone());
    let path = if !save_as { current } else { None };
    let path = match path {
        Some(path) => path,
        None => {
            let device_name = selected_class
                .borrow()
                .as_deref()
                .and_then(|class| catalog.find_by_class(class))
                .map(|device| device.device.clone())
                .unwrap_or_else(|| "KORG data".to_owned());
            let kind = if extension.ends_with("_set") {
                "Scene Set"
            } else if extension.ends_with("_glob") {
                "Global Data"
            } else {
                "Scene Data"
            };
            let mut dialog = rfd::FileDialog::new()
                .set_file_name(format!("{device_name} {kind}.{extension}"))
                .add_filter("KORG data", &[extension.as_str()]);
            let directory = CURRENT_PATH.with(|path| {
                path.borrow()
                    .as_ref()
                    .and_then(|path| path.parent().map(Path::to_path_buf))
            });
            let directory = directory.or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .map(|home| home.join("Documents"))
                    .filter(|path| path.is_dir())
            });
            if let Some(directory) = directory {
                dialog = dialog.set_directory(directory);
            }
            let Some(path) = dialog.save_file() else {
                return Ok("Save cancelled.".to_owned());
            };
            path
        }
    };
    std::fs::write(&path, bytes).map_err(|error| format!("{}: {error}", path.display()))?;
    CURRENT_PATH.with(|current| *current.borrow_mut() = Some(path.clone()));
    controller
        .borrow_mut()
        .dispatch(ControllerAction::MarkSaved);
    Ok(format!("Saved {}", path.display()))
}

fn progress_text(progress: &JobProgress) -> String {
    match progress {
        JobProgress::Opened { input, output } => format!("Receive: {input} -> {output}"),
        JobProgress::Identified { model, key } => format!("Device {model:#06x}, key {key}"),
        JobProgress::Receiving => "Receiving scene set...".to_owned(),
        JobProgress::WritingGlobal => "Writing global...".to_owned(),
        JobProgress::VerifyingGlobal => "Verifying global...".to_owned(),
        JobProgress::Writing { scene, of } => format!("Writing scene {} of {of}...", scene + 1),
        JobProgress::Verifying { scene, of } => format!("Verifying scene {} of {of}...", scene + 1),
    }
}

fn start_write(
    ui: &AppWindow,
    controller: &Rc<RefCell<Controller>>,
    catalog: &Rc<Catalog>,
    selected_class: &Rc<RefCell<Option<String>>>,
    job_timer: &Rc<Timer>,
) {
    let (key, index, scenes) = {
        let controller_ref = controller.borrow();
        let document = controller_ref.session().document();
        let selected = controller_ref.session().selected_scene();
        let Some(set) = document.scene_set.as_ref() else {
            ui.set_status("No scene set is loaded.".into());
            return;
        };
        (
            set.model.to_string(),
            selected,
            set.scenes
                .iter()
                .map(|scene| scene.data.clone())
                .collect::<Vec<_>>(),
        )
    };
    let Some(class) = current_class(controller, catalog, selected_class) else {
        ui.set_status("Open a device first.".into());
        return;
    };
    let Some(device) = catalog.find_by_class(&class) else {
        ui.set_status("Unknown device.".into());
        return;
    };
    let job = if WRITE_MODE.get() == 1 {
        let Some(payload) = scenes.get(index).cloned() else {
            ui.set_status("Selected scene is missing.".into());
            return;
        };
        DeviceJob::WriteScene {
            key,
            index: index as u8,
            payload,
        }
    } else {
        DeviceJob::WriteSceneSet { key, scenes }
    };
    start_measured_job(
        ui,
        controller,
        catalog,
        selected_class,
        job_timer,
        effective_ports(&device.device),
        job,
    );
}

fn start_global_write(
    ui: &AppWindow,
    controller: &Rc<RefCell<Controller>>,
    catalog: &Rc<Catalog>,
    selected_class: &Rc<RefCell<Option<String>>>,
    job_timer: &Rc<Timer>,
) {
    let (key, data) = {
        let controller_ref = controller.borrow();
        let Some(global) = controller_ref.session().document().global.as_ref() else {
            ui.set_status("No global data is loaded.".into());
            return;
        };
        (global.model.to_string(), global.data.clone())
    };
    let Some(class) = current_class(controller, catalog, selected_class) else {
        ui.set_status("Open a device first.".into());
        return;
    };
    let Some(device) = catalog.find_by_class(&class) else {
        ui.set_status("Unknown device.".into());
        return;
    };
    start_measured_job(
        ui,
        controller,
        catalog,
        selected_class,
        job_timer,
        effective_ports(&device.device),
        DeviceJob::WriteGlobal { key, data },
    );
}

fn apply_receive(
    ui: &AppWindow,
    controller: &Rc<RefCell<Controller>>,
    catalog: &Rc<Catalog>,
    selected_class: &Rc<RefCell<Option<String>>>,
    job_timer: Rc<Timer>,
    result: Result<JobOutput, String>,
) {
    match result {
        Ok(JobOutput::SceneSet { key, scenes }) => {
            let element_size = scenes.first().map(Vec::len).unwrap_or(0) as u16;
            let count = scenes.len();
            if RECEIVE_MODE.get() == 1 {
                let index = controller.borrow().session().selected_scene();
                let Some(data) = scenes.get(index).cloned() else {
                    ui.set_status(format!("Device has no scene {}.", index + 1).into());
                    refresh(ui, &controller.borrow(), catalog, selected_class);
                    return;
                };
                let mut controller = controller.borrow_mut();
                controller.dispatch(ControllerAction::SetSceneData { index, data });
                controller.dispatch(ControllerAction::MarkSaved);
                controller.dispatch(ControllerAction::MarkReceived(ReceivedPart::Scene(index)));
                ui.set_status(format!("Loaded scene {} from the device.", index + 1).into());
            } else {
                let set = SceneSet {
                    model: key.clone(),
                    element_size,
                    scenes: scenes
                        .into_iter()
                        .map(|data| Scene {
                            name: korg_model::scene_name(&key, &data),
                            data,
                        })
                        .collect(),
                };
                controller.borrow_mut().load(ControllerInput::SceneSet(set));
                controller
                    .borrow_mut()
                    .dispatch(ControllerAction::MarkReceived(ReceivedPart::SceneSet));
                if let Some(device) = catalog.find_by_file_key(&key).first() {
                    *selected_class.borrow_mut() = Some(device.class.clone());
                }
                ui.set_screen(1);
                ui.set_status(format!("Received {count} scenes.").into());
                if AUTO_RECEIVE.get() {
                    if let Some(device) = catalog.find_by_file_key(&key).first() {
                        let device_name = device.device.clone();
                        let weak = ui.as_weak();
                        let controller = controller.clone();
                        let catalog = catalog.clone();
                        let selected_class = selected_class.clone();
                        let job_timer = job_timer.clone();
                        let timer = Timer::default();
                        timer.start(
                            TimerMode::SingleShot,
                            Duration::from_millis(100),
                            move || {
                                if let Some(ui) = weak.upgrade() {
                                    start_measured_job(
                                        &ui,
                                        &controller,
                                        &catalog,
                                        &selected_class,
                                        &job_timer,
                                        effective_ports(&device_name),
                                        DeviceJob::ReceiveGlobal,
                                    );
                                }
                            },
                        );
                        AUTO_TIMER.with(|slot| *slot.borrow_mut() = Some(timer));
                    }
                }
            }
        }
        Ok(JobOutput::Global { key, data }) => {
            let schema = selected_class
                .borrow()
                .as_deref()
                .and_then(|class| korg_devices::parameter_schema(class, "global"));
            if schema.is_some() {
                controller.borrow_mut().set_schema(schema);
            }
            controller
                .borrow_mut()
                .load(ControllerInput::Global(GlobalData { model: key, data }));
            controller
                .borrow_mut()
                .dispatch(ControllerAction::MarkReceived(ReceivedPart::Global));
            ui.set_status("Received global data.".into());
        }
        Ok(JobOutput::Written { scenes }) => {
            ui.set_status(format!("Wrote {scenes} scenes; read back identical.").into());
        }
        Ok(JobOutput::GlobalWritten) => {
            ui.set_status("Wrote global; read back identical.".into());
        }
        Ok(JobOutput::CalibrationDone) => {
            let step = CALIBRATION_STEP.get() + 1;
            CALIBRATION_STEP.set(step);
            let text = match step {
                1 => "Touch the top-left corner, then press Calibrate again.",
                2 => "Touch the bottom-right corner, then press Calibrate again.",
                3 => "Finishing calibration...",
                _ => "Calibration complete. Reboot the nanoKEY Studio.",
            };
            ui.set_status(text.into());
        }
        Ok(JobOutput::Scene { index, .. }) => {
            ui.set_status(format!("Received scene {}.", index + 1).into());
        }
        Err(error) => ui.set_status(format!("Receive failed: {error}").into()),
    }
    refresh(ui, &controller.borrow(), catalog, selected_class);
}

/// Decode the fields the Global page shows.  X14800's layout is measured;
/// other models keep the editor defaults until their layout is decoded.
fn global_display(
    class: Option<&str>,
    data: Option<&[u8]>,
) -> (String, String, String, String, String, String) {
    let fields = class.and_then(korg_devices::global_fields);
    let offsets = korg_devices::global_ui_offsets(fields);
    let choices = korg_devices::global_ui_choices(fields);
    let byte = |role: usize| {
        data.and_then(|data| {
            usize::try_from(offsets[role])
                .ok()
                .and_then(|offset| data.get(offset))
        })
    };
    let mut channel = "1".to_owned();
    let controller = "Native".to_owned();
    let battery = byte(1).map_or_else(
        || "Unknown".to_owned(),
        |value| {
            choices[1]
                .iter()
                .find(|choice| choice.value == *value)
                .map_or_else(
                    || format!("Unknown ({value})"),
                    |choice| choice.label.clone(),
                )
        },
    );
    let mut wireless = "nanoKONTROL Studio".to_owned();
    let clock = byte(2).map_or_else(
        || "Unknown".to_owned(),
        |value| {
            choices[2]
                .iter()
                .find(|choice| choice.value == *value)
                .map_or_else(
                    || format!("Unknown ({value})"),
                    |choice| choice.label.clone(),
                )
        },
    );
    let mut bpm = "120.0".to_owned();
    if class == Some("X14800") {
        wireless = "nanoKEY Studio".to_owned();
        if let Some(data) = data {
            if let Some(value) = byte(0) {
                channel = (value + 1).to_string();
            }
            let name_range = fields
                .and_then(|f| f.name_offset.zip(f.name_width))
                .and_then(|(offset, width)| {
                    offset
                        .checked_add(width)
                        .map(|end| offset as usize..end as usize)
                });
            let name: Vec<u8> = name_range
                .and_then(|range| data.get(range))
                .unwrap_or_default()
                .iter()
                .copied()
                .take_while(|byte| *byte != 0)
                .collect();
            if !name.is_empty() {
                wireless = String::from_utf8_lossy(&name).trim().to_owned();
            }
            let bpm_offset = fields
                .and_then(|f| f.bpm_offset)
                .map(|offset| offset as usize);
            if let Some((low, high)) = bpm_offset
                .and_then(|offset| Some((data.get(offset)?, data.get(offset.checked_add(1)?)?)))
            {
                let tenths = u16::from(*low) | (u16::from(*high) << 8);
                bpm = format!("{}.{}", tenths / 10, tenths % 10);
            }
        }
    }
    (channel, controller, battery, wireless, clock, bpm)
}

/// Fit a panel picture into a canvas, centred, and keep the overlay scale.
#[cfg(test)]
fn contain_panel(canvas: (f64, f64), panel: (f64, f64)) -> (f64, f64, f64) {
    let scale = (canvas.0 / panel.0).min(canvas.1 / panel.1);
    let ox = (canvas.0 - panel.0 * scale) / 2.0;
    let oy = (canvas.1 - panel.1 * scale) / 2.0;
    (scale, ox, oy)
}

/// Format BPM tenths as `whole.fraction`, for example 1205 -> "120.5".
#[cfg(test)]
fn format_bpm_tenths(tenths: i32) -> String {
    format!("{}.{}", tenths.div_euclid(10), tenths.rem_euclid(10))
}

fn effective_ports(device: &str) -> (String, String) {
    let input = PORT_IN.with(|port| port.borrow().clone());
    let output = PORT_OUT.with(|port| port.borrow().clone());
    let input = if input.trim().is_empty() {
        device.to_owned()
    } else {
        input
    };
    let output = if output.trim().is_empty() {
        device.to_owned()
    } else {
        output
    };
    (input, output)
}

fn product_root() -> PathBuf {
    // One definition for both front ends (`korg-app/src/document.rs`).
    korg_app::products_dir()
}

/// The MIDI button: "MIDI: JACK", its tooltip, and whether it can switch.
fn show_midi_backend(ui: &AppWindow, midi: &korg_app::MidiBackendSwitch) {
    ui.set_midi_backend_label(midi.button_label().into());
    let note = if midi.can_switch() {
        format!("{} Click to switch.", midi.status())
    } else {
        midi.status()
    };
    ui.set_midi_backend_note(note.into());
    ui.set_midi_backend_enabled(midi.can_switch());
}

fn load_preferences() -> korg_app::preferences::Preferences {
    korg_app::preferences::load()
}

fn save_preferences() {
    let preferences = korg_app::preferences::Preferences {
        warn_before_write: WARN_BEFORE_WRITE.get(),
        auto_receive: AUTO_RECEIVE.get(),
        input_port: PORT_IN.with(|port| port.borrow().clone()),
        output_port: PORT_OUT.with(|port| port.borrow().clone()),
        link: LINK.get(),
        midi_backend: MIDI_BACKEND.with(|saved| saved.borrow().clone()),
    };
    korg_app::preferences::save(&preferences);
}

fn current_class(
    controller: &Rc<RefCell<Controller>>,
    catalog: &Rc<Catalog>,
    selected_class: &Rc<RefCell<Option<String>>>,
) -> Option<String> {
    selected_class.borrow().clone().or_else(|| {
        controller
            .borrow()
            .view()
            .device
            .as_ref()
            .and_then(|model| {
                catalog
                    .find_by_file_key(model)
                    .first()
                    .map(|device| device.class.clone())
            })
    })
}

fn start_receive(
    ui: &AppWindow,
    controller: &Rc<RefCell<Controller>>,
    catalog: &Rc<Catalog>,
    selected_class: &Rc<RefCell<Option<String>>>,
    job_timer: &Rc<Timer>,
    mode: u8,
) {
    let Some(class) = current_class(controller, catalog, selected_class) else {
        ui.set_status("Open a device first.".into());
        return;
    };
    let Some(device) = catalog.find_by_class(&class) else {
        ui.set_status("Unknown device.".into());
        return;
    };
    RECEIVE_MODE.set(mode);
    start_measured_job(
        ui,
        controller,
        catalog,
        selected_class,
        job_timer,
        effective_ports(&device.device),
        DeviceJob::ReceiveSceneSet,
    );
}

fn start_measured_job(
    ui: &AppWindow,
    controller: &Rc<RefCell<Controller>>,
    catalog: &Rc<Catalog>,
    selected_class: &Rc<RefCell<Option<String>>>,
    job_timer: &Rc<Timer>,
    ports: (String, String),
    job: DeviceJob,
) {
    JOB_BUSY.set(true);
    ui.set_transfer_busy(true);
    ui.set_status("Opening port...".into());
    let (tx, rx) = mpsc::channel();
    let timer = job_timer.clone();
    let timer_handle = timer.clone();
    let job_timer = job_timer.clone();
    let weak = ui.as_weak();
    let controller = controller.clone();
    let catalog = catalog.clone();
    let selected_class = selected_class.clone();
    timer.start(TimerMode::Repeated, Duration::from_millis(100), move || {
        let Some(rx) = JOB_RX.with(|cell| cell.borrow_mut().take()) else {
            timer_handle.stop();
            return;
        };
        let mut keep = true;
        loop {
            match rx.try_recv() {
                Ok(JobMessage::Progress(progress)) => {
                    if let Some(ui) = weak.upgrade() {
                        ui.set_status(progress_text(&progress).into());
                    }
                }
                Ok(JobMessage::Done(result)) => {
                    keep = false;
                    JOB_BUSY.set(false);
                    // A write read back identical: the device holds it now.
                    if let (Ok(_), Some(job)) =
                        (&result, RUNNING_JOB.with(|slot| slot.borrow_mut().take()))
                    {
                        let part =
                            korg_app::written_part(&job, controller.borrow().session().document());
                        if let Some(part) = part {
                            controller
                                .borrow_mut()
                                .dispatch(ControllerAction::MarkReceived(part));
                        }
                    }
                    if let (Ok(_), Some(scene)) = (&result, JOB_RESTORE.take()) {
                        SCENE_MONITOR.with(|monitor| {
                            if let Some(monitor) = monitor.borrow_mut().as_mut() {
                                monitor.set_active(scene);
                            }
                        });
                    }
                    if let Some(ui) = weak.upgrade() {
                        ui.set_device_scene(device_scene().map_or(-1, i32::from));
                        ui.set_transfer_busy(false);
                        apply_receive(
                            &ui,
                            &controller,
                            &catalog,
                            &selected_class,
                            job_timer.clone(),
                            result,
                        );
                    }
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    keep = false;
                    JOB_BUSY.set(false);
                    break;
                }
            }
        }
        if keep {
            JOB_RX.with(|cell| *cell.borrow_mut() = Some(rx));
        } else {
            timer_handle.stop();
        }
    });
    JOB_RX.with(|cell| *cell.borrow_mut() = Some(rx));
    let generation = JOB_GEN.get().wrapping_add(1);
    JOB_GEN.set(generation);
    {
        let weak = ui.as_weak();
        let watchdog = Timer::default();
        watchdog.start(TimerMode::SingleShot, Duration::from_secs(30), move || {
            if JOB_GEN.get() == generation && JOB_BUSY.get() {
                JOB_GEN.set(generation.wrapping_add(1));
                JOB_RX.with(|cell| cell.borrow_mut().take());
                JOB_BUSY.set(false);
                if let Some(ui) = weak.upgrade() {
                    ui.set_transfer_busy(false);
                    ui.set_status("Transfer timed out; cancelled.".into());
                }
            }
        });
        WATCHDOG.with(|slot| *slot.borrow_mut() = Some(watchdog));
    }
    let active_scene = device_scene();
    JOB_RESTORE.set(korg_app::restore_scene(&job, active_scene));
    RUNNING_JOB.with(|slot| *slot.borrow_mut() = Some(job.clone()));
    std::thread::spawn(move || {
        let mut progress = |progress: JobProgress| {
            let _ = tx.send(JobMessage::Progress(progress));
        };
        let result = korg_app::run_job_restoring(
            MidirTransport::new(),
            SystemClock::new(),
            &ports.0,
            &ports.1,
            TRANSFER_TIMEOUT_MS,
            job,
            active_scene,
            &mut progress,
        );
        let _ = tx.send(JobMessage::Done(result));
    });
}

/// The scene active on the device, when an announcement or a job told us.
fn device_scene() -> Option<u8> {
    SCENE_MONITOR.with(|monitor| monitor.borrow().as_ref().and_then(|m| m.active))
}

/// Keep the scene monitor on the selected link, or off without one.
fn follow_device_scene(ui: &AppWindow, device: bool, input: Option<&str>, output: Option<&str>) {
    SCENE_MONITOR.with(|monitor| {
        let mut monitor = monitor.borrow_mut();
        let monitor = monitor
            .get_or_insert_with(|| korg_app::monitor::SceneMonitor::new(MidirTransport::new));
        match (device, input, output) {
            // A failed connect keeps the wanted ports; `poll` retries them,
            // so a Bluetooth link that is down now is picked up later.
            (true, Some(input), Some(output)) => {
                let _ = monitor.connect(input, output);
            }
            _ => monitor.disconnect(),
        }
    });
    ui.set_device_scene(device_scene().map_or(-1, i32::from));
}

/// One row of a device page (`korg_app::pages`) for the parameter blocks:
/// kind 0 number, 1 choice, 2 heading, 3 text.  A number shows -1 for an
/// empty slot and -2 when another field makes it unused.
fn page_row(row: &korg_app::pages::PageRow, data: &[u8]) -> ParamRow {
    use korg_app::assign::FieldKind;
    let field = match row {
        korg_app::pages::PageRow::Heading(label) => {
            return ParamRow {
                index: -1,
                element: 0,
                offset: 0,
                byte_offset: -1,
                key: "".into(),
                label: (*label).into(),
                value: 0,
                minimum: 0,
                maximum: 0,
                kind: 2,
                choices: ModelRc::default(),
                display: "".into(),
                hint: "".into(),
            }
        }
        korg_app::pages::PageRow::Field(field) => field,
    };
    let (minimum, maximum) = field.range();
    let (kind, value) = match field.kind {
        FieldKind::Text { .. } | FieldKind::TextPadded { .. } => (3, 0),
        FieldKind::Number { .. } | FieldKind::Biased { .. } | FieldKind::OptionalNumber { .. } => (
            0,
            if field.enabled {
                field.shown(data).unwrap_or(-1)
            } else {
                -2
            },
        ),
        _ => (1, field.shown(data).unwrap_or(0)),
    };
    ParamRow {
        index: -1,
        element: 0,
        offset: field.offset as i32,
        byte_offset: field.offset as i32,
        key: field.label.into(),
        label: field.label.into(),
        value,
        minimum,
        maximum,
        kind,
        choices: ModelRc::new(VecModel::from(
            field
                .kind
                .labels()
                .into_iter()
                .map(SharedString::from)
                .collect::<Vec<_>>(),
        )),
        display: field.display(data).into(),
        // A note number shows its name beside the spinner.
        hint: field
            .note_name(data)
            .unwrap_or_else(|| field.kind.range_hint())
            .into(),
    }
}

/// The page field at `offset` for the selected control, with the panel it
/// is stored against; `None` for devices without decoded pages.
fn selected_page_field(
    class: &str,
    data: &[u8],
    offset: usize,
) -> Option<(korg_app::panel::Panel, korg_app::pages::PageField)> {
    let panel = device_panel(class)?;
    let control = SELECTED_CONTROL
        .with(|selected| *selected.borrow())
        .and_then(|index| panel.controls.get(index));
    let field = korg_app::pages::field_at(class, &panel, control, data, offset)?;
    Some((panel, field))
}

/// Apply `edit` to the selected scene's bytes and dispatch the result.
fn edit_scene(controller: &Rc<RefCell<Controller>>, edit: impl FnOnce(&mut Vec<u8>) -> bool) {
    let index = controller.borrow().session().selected_scene();
    let Some(mut data) = controller
        .borrow()
        .session()
        .document()
        .scene_set
        .as_ref()
        .and_then(|set| set.scenes.get(index))
        .map(|scene| scene.data.clone())
    else {
        return;
    };
    if edit(&mut data) {
        controller
            .borrow_mut()
            .dispatch(ControllerAction::SetSceneData { index, data });
    }
}

/// A global schema row for the generic Global page: channels and labelled
/// enums as combos, everything else as a number.
fn global_row(parameter: &korg_core::ParamView) -> ParamRow {
    let channel = korg_app::assign::channel_field(&parameter.key, parameter.kind, parameter.max);
    let choices: Vec<String> = match channel {
        Some(kind) => kind.labels(),
        None => parameter.choices.clone(),
    };
    let minimum = if channel.is_some() { 0 } else { parameter.min };
    ParamRow {
        element: parameter.element as i32,
        index: parameter.index as i32,
        offset: parameter.offset as i32,
        byte_offset: -1,
        key: parameter.key.as_str().into(),
        label: parameter.label.as_str().into(),
        value: parameter.value,
        minimum,
        maximum: parameter.max,
        kind: if parameter.kind == ParameterKind::Text {
            3
        } else {
            i32::from(!choices.is_empty())
        },
        display: choices
            .get(usize::try_from(parameter.value - minimum).unwrap_or(usize::MAX))
            .cloned()
            .unwrap_or_else(|| parameter.display.clone())
            .into(),
        hint: channel
            .map_or_else(
                || format!("{}..{}", parameter.min, parameter.max),
                |kind| kind.range_hint(),
            )
            .into(),
        choices: ModelRc::new(VecModel::from(
            choices
                .into_iter()
                .map(SharedString::from)
                .collect::<Vec<_>>(),
        )),
    }
}

fn refresh(
    ui: &AppWindow,
    controller: &Controller,
    catalog: &Catalog,
    selected_class: &RefCell<Option<String>>,
) {
    let view = controller.view();
    let device = selected_class
        .borrow()
        .as_deref()
        .and_then(|class| catalog.find_by_class(class));
    let options_for = |direction: Direction| -> Vec<String> {
        let mut names = Vec::new();
        for port in view.ports.iter().filter(|port| port.direction == direction) {
            if let Some(device) = device {
                if !korg_app::port_name_matches_device(device, &port.name) {
                    continue;
                }
            }
            if !names.iter().any(|name| name == &port.name) {
                names.push(port.name.clone());
            }
        }
        names
    };
    let links = device
        .map(|device| korg_app::device_links(&view.ports, device))
        .unwrap_or_default();
    let chosen = korg_app::choose_link(&links, LINK.get());
    let select = |current: String, options: &[String], direction: Direction| -> Option<String> {
        if !current.is_empty() && options.iter().any(|name| name == &current) {
            Some(current)
        } else if let Some(link) = chosen {
            Some(match direction {
                Direction::Input => link.input.clone(),
                Direction::Output => link.output.clone(),
            })
        } else {
            device
                .and_then(|device| {
                    korg_app::port_matches_direction(&view.ports, device, Some(direction))
                })
                .map(str::to_owned)
        }
    };
    let input_options = options_for(Direction::Input);
    let output_options = options_for(Direction::Output);
    let input = select(
        PORT_IN.with(|port| port.borrow().clone()),
        &input_options,
        Direction::Input,
    );
    let output = select(
        PORT_OUT.with(|port| port.borrow().clone()),
        &output_options,
        Direction::Output,
    );
    // Keep both directions on one link: a request sent over USB is never
    // answered on Bluetooth.
    let input_link = input.as_deref().and_then(|name| {
        links
            .iter()
            .find(|link| link.kind == korg_app::LinkKind::of_port(name))
    });
    let output = match (input_link, output) {
        (Some(link), Some(name)) if korg_app::LinkKind::of_port(&name) != link.kind => {
            Some(link.output.clone())
        }
        (_, output) => output,
    };
    ui.set_link_index(
        input_link
            .and_then(|current| links.iter().position(|link| link.kind == current.kind))
            .map_or(-1, |index| index as i32),
    );
    ui.set_link_options(ModelRc::new(VecModel::from(
        links
            .iter()
            .map(|link| SharedString::from(link.label()))
            .collect::<Vec<_>>(),
    )));
    if let Some(name) = &input {
        PORT_IN.with(|port| *port.borrow_mut() = name.clone());
    }
    if let Some(name) = &output {
        PORT_OUT.with(|port| *port.borrow_mut() = name.clone());
    }
    follow_device_scene(ui, device.is_some(), input.as_deref(), output.as_deref());
    let class = device.map(|device| device.class.as_str()).unwrap_or("");
    ui.set_wireless_on(
        korg_app::wireless::WirelessState::of_links(&links)
            == korg_app::wireless::WirelessState::On,
    );
    ui.set_wireless_how(
        korg_app::wireless::toggle_instruction(class)
            .unwrap_or_default()
            .into(),
    );
    ui.set_wireless_note(
        korg_app::wireless::toggle_note(class)
            .unwrap_or_default()
            .into(),
    );
    ui.set_port_name(input.clone().unwrap_or_default().as_str().into());
    ui.set_port_in_index(
        input
            .as_ref()
            .and_then(|name| input_options.iter().position(|option| option == name))
            .map_or(-1, |index| index as i32),
    );
    ui.set_port_out_index(
        output
            .as_ref()
            .and_then(|name| output_options.iter().position(|option| option == name))
            .map_or(-1, |index| index as i32),
    );
    ui.set_port_in_options(ModelRc::new(VecModel::from(
        input_options
            .into_iter()
            .map(SharedString::from)
            .collect::<Vec<_>>(),
    )));
    ui.set_port_out_options(ModelRc::new(VecModel::from(
        output_options
            .into_iter()
            .map(SharedString::from)
            .collect::<Vec<_>>(),
    )));
    if std::env::var_os("KORG_DEBUG").is_some() {
        eprintln!(
            "view: device={:?}, scenes={}, params={}",
            view.device,
            view.scene_count,
            view.params.len()
        );
    }
    let class = selected_class.borrow().clone().or_else(|| {
        view.device.as_ref().and_then(|model| {
            catalog
                .find_by_file_key(model)
                .first()
                .map(|device| device.class.clone())
        })
    });
    let device_name = class
        .as_deref()
        .and_then(|class| catalog.find_by_class(class))
        .map(|device| device.device.clone())
        .or_else(|| view.device.as_ref().map(|model| model.to_string()))
        .unwrap_or_else(|| "No device".to_owned());

    let panel = class.as_deref().and_then(device_panel);
    LAST_CLASS.with(|last| {
        let mut last = last.borrow_mut();
        if last.as_deref() != class.as_deref() {
            SELECTED_CONTROL.with(|selected| *selected.borrow_mut() = None);
            TAB.set(if panel.is_some() { 1 } else { 0 });
            OPEN_FOR.set(None);
            *last = class.clone();
        }
    });
    let selected = panel.as_ref().map(|panel| {
        SELECTED_CONTROL.with(|cell| {
            let mut index = cell.borrow_mut();
            if index.is_none_or(|value| value >= panel.controls.len()) {
                *index = std::env::var("KORG_SELECT_CONTROL")
                    .ok()
                    .and_then(|value| value.parse::<usize>().ok())
                    .filter(|value| *value < panel.controls.len())
                    .or_else(|| panel.find(PanelBinding::Jog))
                    .or(Some(0));
            }
            index.unwrap_or(0)
        })
    });
    if let (Some(panel), Some(index)) = (panel.as_ref(), selected) {
        if OPEN_FOR.get() != Some(index) {
            OPEN_FOR.set(Some(index));
            let group = panel
                .controls
                .get(index)
                .map(|control| control.group.clone());
            OPEN_GROUP.with(|open| *open.borrow_mut() = group);
        }
    }
    let class_str = class.as_deref().unwrap_or("");
    let scene_bytes = controller
        .session()
        .document()
        .scene_set
        .as_ref()
        .and_then(|set| set.scenes.get(controller.session().selected_scene()))
        .map(|scene| scene.data.clone())
        .unwrap_or_default();
    let kind_color = |index: usize| -> slint::Color {
        let message = panel.as_ref().and_then(|panel| {
            korg_app::assign::control_message(class_str, &panel.controls[index], &scene_bytes)
        });
        let (r, g, b) = message.map_or((0xe8, 0xe8, 0xe8), |message| message.kind.rgb());
        slint::Color::from_rgb_u8(r, g, b)
    };
    let tooltip_for = |index: usize| -> SharedString {
        panel
            .as_ref()
            .map(|panel| korg_app::assign::details(class_str, &panel.controls[index], &scene_bytes))
            .unwrap_or_default()
            .into()
    };
    let tree: Vec<ControlTreeRow> = panel
        .as_ref()
        .map(|panel| {
            let open = OPEN_GROUP.with(|open| open.borrow().clone());
            panel
                .tree_lines(open.as_deref(), selected, CHIP_LINE_CHARS)
                .into_iter()
                .map(|line| {
                    let empty = ModelRc::new(VecModel::from(Vec::<TreeChip>::new()));
                    match line {
                        korg_app::panel::TreeLine::Row(korg_app::panel::TreeRow::Group {
                            name,
                            count,
                            open,
                            selected,
                        }) => ControlTreeRow {
                            header: true,
                            label: name.into(),
                            count: count as i32,
                            open,
                            selected: false,
                            contains: selected,
                            index: -1,
                            nested: false,
                            color: slint::Color::from_rgb_u8(0xd0, 0xd0, 0xd0),
                            tooltip: "".into(),
                            chips: empty,
                        },
                        korg_app::panel::TreeLine::Row(korg_app::panel::TreeRow::Control {
                            index,
                            label,
                            nested,
                            selected,
                        }) => ControlTreeRow {
                            header: false,
                            label: label.into(),
                            count: 0,
                            open: false,
                            selected,
                            contains: false,
                            index: index as i32,
                            nested,
                            color: kind_color(index),
                            tooltip: tooltip_for(index),
                            chips: empty,
                        },
                        korg_app::panel::TreeLine::Chips(chips) => ControlTreeRow {
                            header: false,
                            label: "".into(),
                            count: 0,
                            open: false,
                            selected: false,
                            contains: false,
                            index: -1,
                            nested: true,
                            color: slint::Color::from_rgb_u8(0xe8, 0xe8, 0xe8),
                            tooltip: "".into(),
                            chips: ModelRc::new(VecModel::from(
                                chips
                                    .into_iter()
                                    .map(|chip| TreeChip {
                                        index: chip.index as i32,
                                        label: chip.label.into(),
                                        selected: chip.selected,
                                        color: kind_color(chip.index),
                                        tooltip: tooltip_for(chip.index),
                                    })
                                    .collect::<Vec<_>>(),
                            )),
                        },
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    ui.set_control_tree(ModelRc::new(VecModel::from(tree)));
    ui.set_control_details(
        selected
            .map(tooltip_for)
            .unwrap_or_else(|| "Click a control on the device.".into()),
    );
    let controls: Vec<PanelControl> = panel
        .as_ref()
        .map(|panel| {
            let mut items: Vec<(usize, &korg_app::panel::Control)> =
                panel.controls.iter().enumerate().collect();
            // The original lists knobs and sliders first, then the strip
            // buttons, transport, scene and finally the jog wheel.
            let class_name = class.as_deref().unwrap_or("");
            let rank = |control: &korg_app::panel::Control| match class_name {
                "X10820" => match control.label.as_str() {
                    "Common" => 0,
                    "Keyboard" => 1,
                    "Pitch Button" => 2,
                    "Sustain Button" => 3,
                    "Mod Button" => 4,
                    _ => 5,
                },
                "X14800" => match (control.group.as_str(), control.label.as_str()) {
                    ("Keyboard", _) => 0,
                    ("Trigger Pad", _) => 1,
                    (_, "Chord Pad") => 2,
                    ("Knob", _) => 3,
                    ("X-Y Pad", _) => 4,
                    (_, "Pitch/Mod") => 5,
                    (_, "Touch Scale") => 6,
                    (_, "Sustain") => 7,
                    ("Button", _) => 8,
                    _ => 9,
                },
                _ => match control.group.as_str() {
                    "Knob" => 0,
                    "Slider" => 1,
                    "Track Mute" => 2,
                    "Track Solo" => 3,
                    "Track Rec" => 4,
                    "Track Select" => 5,
                    "Transport" => 6,
                    "Scene" => 7,
                    "Jog Wheel" => 8,
                    _ => 9,
                },
            };
            let natural = |label: &str| -> (String, i64) {
                let tail = label.rsplit(' ').next().unwrap_or(label);
                match tail.parse::<i64>() {
                    Ok(number) => (label.trim_end_matches(tail).to_owned(), number),
                    Err(_) => (label.to_owned(), 0),
                }
            };
            items.sort_by(|(_, a), (_, b)| {
                (rank(a), natural(&a.label)).cmp(&(rank(b), natural(&b.label)))
            });
            items
                .into_iter()
                .map(|(index, control)| PanelControl {
                    index: index as i32,
                    label: control.label.as_str().into(),
                    group: control.group.clone().into(),
                    x: control.x as f32,
                    y: control.y as f32,
                    w: control.w as f32,
                    h: control.h as f32,
                    selected: selected == Some(index),
                    lit: lit_now().contains(&index),
                    tooltip: korg_app::assign::details(class_str, control, &scene_bytes).into(),
                    message_color: {
                        let kind = if class_str == "X14800" && control.group == "X-Y Pad" {
                            Some(korg_app::assign::MessageKind::ControlChange)
                        } else {
                            korg_app::assign::control_message(class_str, control, &scene_bytes)
                                .map(|message| message.kind)
                        };
                        let (r, g, b) = kind.map_or((0x80, 0x80, 0x80), |kind| kind.rgb());
                        slint::Color::from_rgb_u8(r, g, b)
                    },
                    badge: if class_str == "X14800" && control.group == "X-Y Pad" {
                        format!(
                            "CC{}/{}",
                            scene_bytes.get(174).copied().unwrap_or(0),
                            scene_bytes.get(178).copied().unwrap_or(0)
                        )
                    } else {
                        korg_app::assign::control_message(class_str, control, &scene_bytes)
                            .map(|message| message.badge())
                            .unwrap_or_default()
                    }
                    .into(),
                    axis_x: if class_str == "X14800" && control.group == "X-Y Pad" {
                        format!("CC{}", scene_bytes.get(174).copied().unwrap_or(0)).into()
                    } else {
                        "".into()
                    },
                    axis_y: if class_str == "X14800" && control.group == "X-Y Pad" {
                        format!("{}", scene_bytes.get(178).copied().unwrap_or(0)).into()
                    } else {
                        "".into()
                    },
                    pad_touch: if class_str == "X14800" && control.group == "X-Y Pad" {
                        let enabled = scene_bytes.get(169).copied().unwrap_or(1);
                        if enabled == 1 {
                            "—".into()
                        } else {
                            format!("CC{}", scene_bytes.get(170).copied().unwrap_or(0)).into()
                        }
                    } else {
                        "".into()
                    },
                    value: if class_str == "X14800" && control.group == "X-Y Pad" {
                        format!(
                            "CC{}/{}",
                            scene_bytes.get(174).copied().unwrap_or(0),
                            scene_bytes.get(178).copied().unwrap_or(0)
                        )
                    } else {
                        korg_app::assign::control_message(class_str, control, &scene_bytes)
                            .map(|message| message.badge())
                            .unwrap_or_default()
                    }
                    .into(),
                })
                .collect()
        })
        .unwrap_or_default();
    let selected_binding = panel
        .as_ref()
        .and_then(|panel| selected.and_then(|index| panel.controls.get(index)))
        .map(|control| control.binding);
    let control_label = panel
        .as_ref()
        .and_then(|panel| selected.and_then(|index| panel.controls.get(index)))
        .map(|control| {
            if control.label.starts_with(&format!("{} ", control.group))
                || control.group == control.label
            {
                control.label.clone()
            } else {
                format!("{} · {}", control.group, control.label)
            }
        })
        .unwrap_or_default();

    let scenes: Vec<SceneTab> = view
        .scene_labels
        .iter()
        .enumerate()
        .map(|(index, label)| SceneTab {
            index: index as i32,
            label: label.as_str().into(),
        })
        .collect();

    let selected_element = match selected_binding {
        Some(PanelBinding::Element(element)) => Some(element),
        _ => None,
    };
    let selected_control = panel
        .as_ref()
        .and_then(|panel| selected.and_then(|index| panel.controls.get(index)));
    let scene_bytes = controller
        .session()
        .document()
        .scene_set
        .as_ref()
        .and_then(|set| set.scenes.get(controller.session().selected_scene()))
        .map(|scene| scene.data.clone());
    let rows: Vec<ParamRow> = view
        .params
        .iter()
        // The selected control's element, as the TUI filters it.
        .filter(|parameter| selected_element.is_none_or(|element| parameter.element == element))
        .map(|parameter| {
            let channel =
                korg_app::assign::channel_field(&parameter.key, parameter.kind, parameter.max);
            let choices: Vec<String> = match channel {
                Some(kind) => kind.labels(),
                None => parameter.choices.clone(),
            };
            let hint = match channel {
                Some(kind) => kind.range_hint(),
                None if parameter.kind == ParameterKind::Text => "text".to_owned(),
                None if !choices.is_empty() => format!("{} choices", choices.len()),
                None => format!("{}..{}", parameter.min, parameter.max),
            };
            let display = match channel {
                Some(_) => choices
                    .get(usize::try_from(parameter.value).unwrap_or(usize::MAX))
                    .cloned()
                    .unwrap_or_else(|| parameter.value.to_string()),
                None => parameter.display.clone(),
            };
            ParamRow {
                element: parameter.element as i32,
                index: parameter.index as i32,
                offset: parameter.offset as i32,
                byte_offset: -1,
                key: parameter.key.as_str().into(),
                label: parameter.label.as_str().into(),
                value: parameter.value,
                minimum: if channel.is_some() { 0 } else { parameter.min },
                maximum: if channel.is_some() {
                    choices.len() as i32 - 1
                } else {
                    parameter.max
                },
                kind: if channel.is_some() {
                    1
                } else {
                    match parameter.kind {
                        ParameterKind::Number => 0,
                        ParameterKind::Enum if !choices.is_empty() => 1,
                        ParameterKind::Enum => 0,
                        ParameterKind::Text => 3,
                    }
                },
                choices: ModelRc::new(VecModel::from(
                    choices
                        .into_iter()
                        .map(SharedString::from)
                        .collect::<Vec<_>>(),
                )),
                display: display.into(),
                hint: hint.into(),
            }
        })
        .collect();
    // Devices with decoded pages (korg_app::pages): the page the selected
    // control opens, the same rows the TUI shows.
    let page = match (panel.as_ref(), scene_bytes.as_deref()) {
        (Some(panel), Some(data)) => {
            korg_app::pages::fields(class_str, panel, selected_control, data)
                .map(|rows| (rows, data))
        }
        _ => None,
    };
    let rows = match page {
        Some((page, data)) => page.iter().map(|row| page_row(row, data)).collect(),
        None => rows,
    };

    let offline = class
        .as_deref()
        .and_then(|class| catalog.find_by_class(class))
        .is_some_and(|device| korg_app::port_matches(&view.ports, device).is_none());
    let transfer_enabled = !offline
        && view
            .device
            .as_ref()
            .and_then(|model| measured_layout_for_key(model.as_str()))
            .is_some();
    ui.set_offline(offline);

    let browse_files: Vec<BrowseFile> = class
        .as_deref()
        .and_then(|class| catalog.find_by_class(class))
        .map(|device| {
            korg_app::device_files(device, &product_root())
                .into_iter()
                .map(|(label, path)| BrowseFile {
                    label: label.into(),
                    path: path.to_string_lossy().into_owned().into(),
                })
                .collect()
        })
        .unwrap_or_default();
    ui.set_browse_files(ModelRc::new(VecModel::from(browse_files)));
    ui.set_browse_path(product_root().to_string_lossy().into_owned().into());
    ui.set_device_name(device_name.into());
    ui.set_dirty(view.dirty);
    ui.set_can_undo(view.can_undo);
    ui.set_can_redo(view.can_redo);
    ui.set_can_revert(view.can_revert);
    ui.set_scenes(ModelRc::new(VecModel::from(scenes)));
    ui.set_scene_index(view.selected_scene as i32);
    let mut blocks: Vec<ParamBlock> = Vec::new();
    let mut title: Option<String> = None;
    let mut block_rows: Vec<ParamRow> = Vec::new();
    for row in rows.clone() {
        if row.kind == 2 {
            if title.is_some() || !block_rows.is_empty() {
                blocks.push(ParamBlock {
                    title: title.take().unwrap_or_default().into(),
                    rows: ModelRc::new(VecModel::from(block_rows)),
                });
                block_rows = Vec::new();
            }
            title = Some(row.label.to_string());
        } else {
            block_rows.push(row);
        }
    }
    if title.is_some() || !block_rows.is_empty() {
        blocks.push(ParamBlock {
            title: title.unwrap_or_default().into(),
            rows: ModelRc::new(VecModel::from(block_rows)),
        });
    }
    ui.set_param_blocks(ModelRc::new(VecModel::from(blocks)));
    ui.set_params(ModelRc::new(VecModel::from(rows)));
    ui.set_global_params(ModelRc::new(VecModel::from(
        view.global_params
            .iter()
            .map(global_row)
            .collect::<Vec<_>>(),
    )));
    ui.set_transfer_enabled(transfer_enabled);
    ui.set_transfer_busy(
        JOB_BUSY.get()
            || matches!(
                view.transfer,
                TransferState::Sending(_) | TransferState::Waiting(_)
            ),
    );
    ui.set_tab(TAB.get());
    ui.set_warn_before_write(WARN_BEFORE_WRITE.get());
    ui.set_auto_receive(AUTO_RECEIVE.get());
    ui.set_can_calibrate(class.as_deref() == Some("X14800") && !offline);
    let global_data = controller
        .session()
        .document()
        .global
        .as_ref()
        .map(|global| global.data.clone());
    ui.set_global_length(global_data.as_ref().map_or(0, |data| data.len() as i32));
    let (channel, controller_mode, battery, wireless, clock, bpm) =
        global_display(class.as_deref(), global_data.as_deref());
    ui.set_global_channel(channel.into());
    ui.set_global_controller(controller_mode.into());
    ui.set_global_battery(battery.into());
    ui.set_global_name(wireless.into());
    ui.set_global_clock(clock.into());
    ui.set_global_bpm(bpm.into());
    ui.set_global_class(class.as_deref().unwrap_or("").into());
    ui.set_global_tab(GLOBAL_TAB.get());
    let bytes = global_data.as_deref().unwrap_or(&[]);
    let global_fields = class.as_deref().and_then(korg_devices::global_fields);
    ui.set_global_name_label(
        global_fields
            .and_then(|fields| fields.name_label.as_deref())
            .unwrap_or_default()
            .into(),
    );
    ui.set_global_bpm_label(
        global_fields
            .and_then(|fields| fields.bpm_label.as_deref())
            .unwrap_or_default()
            .into(),
    );
    ui.set_global_chord_pad_label(
        global_fields
            .and_then(|fields| fields.chord_pad_label.as_deref())
            .unwrap_or_default()
            .into(),
    );
    ui.set_global_easy_scale_label(
        global_fields
            .and_then(|fields| fields.easy_scale_label.as_deref())
            .unwrap_or_default()
            .into(),
    );
    ui.set_global_binding_labels(ModelRc::new(VecModel::from(
        korg_devices::global_ui_labels(global_fields)
            .into_iter()
            .map(SharedString::from)
            .collect::<Vec<_>>(),
    )));
    let number_ranges = (0..korg_devices::GLOBAL_UI_ROLES.len())
        .map(|role| {
            let range = korg_devices::global_ui_number_range(global_fields, role);
            GlobalNumberRange {
                minimum: range.and_then(|r| i32::try_from(r.min).ok()).unwrap_or(0),
                maximum: range.and_then(|r| i32::try_from(r.max).ok()).unwrap_or(0),
                bias: range.and_then(|r| i32::try_from(r.bias).ok()).unwrap_or(0),
                available: range.is_some_and(|r| {
                    i32::try_from(r.min).is_ok()
                        && i32::try_from(r.max).is_ok()
                        && i32::try_from(r.bias).is_ok()
                }),
            }
        })
        .collect::<Vec<_>>();
    ui.set_global_number_ranges(ModelRc::new(VecModel::from(number_ranges)));
    let bpm_range = korg_devices::global_ui_bpm_range(global_fields);
    ui.set_global_bpm_available(bpm_range.is_some());
    ui.set_global_bpm_minimum(bpm_range.map_or(0, |r| r.min as i32));
    ui.set_global_bpm_maximum(bpm_range.map_or(0, |r| r.max as i32));
    let offsets = korg_devices::global_ui_offsets(global_fields);
    let byte = |role: usize| {
        usize::try_from(offsets[role])
            .ok()
            .and_then(|offset| bytes.get(offset))
    };
    ui.set_global_binding_offsets(ModelRc::new(VecModel::from(offsets.to_vec())));
    let choice_lists = korg_devices::global_ui_choices(global_fields)
        .into_iter()
        .enumerate()
        .map(|(role, choices)| {
            let selected = byte(role)
                .and_then(|value| choices.iter().position(|choice| choice.value == *value))
                .and_then(|index| i32::try_from(index).ok())
                .unwrap_or(-1);
            GlobalChoiceList {
                labels: ModelRc::new(VecModel::from(
                    choices
                        .into_iter()
                        .map(|choice| SharedString::from(choice.label.as_str()))
                        .collect::<Vec<_>>(),
                )),
                selected,
            }
        })
        .collect::<Vec<_>>();
    ui.set_global_choice_lists(ModelRc::new(VecModel::from(choice_lists)));
    ui.set_global_bytes(ModelRc::new(VecModel::from(
        bytes
            .iter()
            .map(|byte| i32::from(*byte))
            .collect::<Vec<_>>(),
    )));
    let channel_value = byte(0).map_or(1, |value| i32::from(*value) + 1);
    ui.set_global_channel_value(channel_value);
    ui.set_global_clock_index(match byte(2) {
        Some(0) => 0,
        Some(1) => 1,
        Some(2) => 2,
        _ => 0,
    });
    let bpm_value = global_fields
        .and_then(|fields| fields.bpm_offset)
        .and_then(|offset| {
            let offset = offset as usize;
            Some(i32::from(
                u16::from(*bytes.get(offset)?)
                    | (u16::from(*bytes.get(offset.checked_add(1)?)?) << 8),
            ))
        })
        .unwrap_or(1200);
    ui.set_global_bpm_value(bpm_value);
    ui.set_global_battery_index(i32::from(*byte(1).unwrap_or(&0)));
    ui.set_global_scrub_index(i32::from(*byte(20).unwrap_or(&0)));
    ui.set_global_mode_index(i32::from(*byte(19).unwrap_or(&0)));
    ui.set_global_key_curve(i32::from(*byte(3).unwrap_or(&0)));
    ui.set_global_pad_curve(i32::from(*byte(5).unwrap_or(&0)));
    ui.set_global_touch_curve(i32::from(*byte(9).unwrap_or(&0)));
    let name_range = global_fields
        .and_then(|fields| fields.name_offset.zip(fields.name_width))
        .and_then(|(offset, width)| {
            offset
                .checked_add(width)
                .map(|end| offset as usize..end as usize)
        });
    let name_bytes: Vec<u8> = name_range
        .and_then(|range| bytes.get(range))
        .unwrap_or_default()
        .iter()
        .copied()
        .take_while(|byte| *byte != 0)
        .collect();
    ui.set_global_name_text(String::from_utf8_lossy(&name_bytes).trim().into());
    ui.set_panel_visible(panel.is_some());
    if let Some(panel) = &panel {
        ui.set_panel_width(panel.width as f32);
        ui.set_panel_height(panel.height as f32);
    }
    let panel_image = class.as_deref().and_then(|class| {
        let path = asset_dir()?.join(korg_devices::external_image(class, "external-panel")?);
        if path.is_file() {
            slint::Image::load_from_path(&path).ok()
        } else {
            None
        }
    });
    ui.set_panel_has_image(panel_image.is_some());
    ui.set_panel_image(panel_image.unwrap_or_default());
    ui.set_panel_controls(ModelRc::new(VecModel::from(controls)));
    ui.set_control_index(selected.unwrap_or(0) as i32);
    ui.set_control_label(control_label.into());
}

/// What the transfer engine needs to know about a catalog device.
fn device_target(device: &DeviceInfo) -> DeviceTarget {
    DeviceTarget {
        class: ClassId::new(device.class.clone()),
        descriptor: korg_devices::sysex_descriptor(&device.class),
        channel: SYSEX_CHANNEL,
        scene_payload_size: device.scene_payload_size.map(usize::from),
        global_payload_size: device.global_payload_size.map(usize::from),
        timeout_ms: TRANSFER_TIMEOUT_MS,
    }
}

/// Set the timer to the rate the controller asks for.
fn apply_rate(timer: &Timer, rate: TickRate) {
    match rate.interval_ms() {
        None => timer.stop(),
        Some(ms) => {
            let interval = Duration::from_millis(ms);
            if timer.interval() != interval {
                timer.set_interval(interval);
            }
            if !timer.running() {
                timer.restart();
            }
        }
    }
}

/// Status text for a transfer event.  `None` leaves the status as it is.
fn transfer_status(event: &TransferEvent) -> Option<String> {
    let phase_name = |phase: &korg_sysex::Phase| match phase {
        korg_sysex::Phase::ModeCheck => "checking the mode",
        korg_sysex::Phase::Data => "transferring data",
        korg_sysex::Phase::Write => "writing",
    };
    match event {
        TransferEvent::None => None,
        TransferEvent::Sent(phase) => Some(format!("Transfer: {}...", phase_name(phase))),
        TransferEvent::Ack(_) => None,
        TransferEvent::Done => Some("Write complete.".to_owned()),
        TransferEvent::Received(payload) => Some(format!("Received {} bytes.", payload.len())),
        TransferEvent::Failed(error) => Some(format!("Transfer failed: {error}")),
    }
}

/// Show a controller outcome: error or transfer status, then the view.
fn apply_outcome(
    ui: &AppWindow,
    controller: &Rc<RefCell<Controller>>,
    catalog: &Catalog,
    selected_class: &RefCell<Option<String>>,
    outcome: &ControllerOutcome,
) {
    if let Some(error) = &outcome.error {
        ui.set_status(error.to_string().into());
    } else if let Some(text) = outcome.transfer_event.as_ref().and_then(transfer_status) {
        ui.set_status(text.into());
    }
    if outcome.view_changed || outcome.error.is_some() {
        refresh(ui, &controller.borrow(), catalog, selected_class);
    }
}

/// The UI keeps BPM as scaled tenths; the controller expects the same unit.
fn dispatch_ui_bpm<T: MidiTransport, C: Clock>(
    controller: &mut EditorController<T, C>,
    value: i32,
) {
    controller.dispatch(ControllerAction::SetGlobalBpm(value.max(0) as u16));
}

fn open_device<T: MidiTransport, C: Clock>(
    controller: &Rc<RefCell<EditorController<T, C>>>,
    device: &DeviceInfo,
) -> Result<String, String> {
    korg_app::open_device(
        &mut controller.borrow_mut(),
        device,
        &korg_app::products_dir(),
    )
}

fn factory_scenes(device: &DeviceInfo) -> Option<Vec<Scene>> {
    let root = std::env::var_os("KORG_PRODUCTS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/Applications/KORG KONTROL EDITOR/Products"));
    let file = preferred_scene_extension(&device.files)?;
    let bytes = std::fs::read(root.join(file)).ok()?;
    let chunk = korg_format::decode(&bytes).ok()?;
    let model = device.file_key()?;
    match chunk.payload {
        Payload::SceneSet(scenes) => Some(
            scenes
                .into_iter()
                .map(|data| Scene {
                    name: korg_model::scene_name(&model, &data),
                    data,
                })
                .collect(),
        ),
        Payload::Scene(data) => Some(vec![Scene {
            name: korg_model::scene_name(&model, &data),
            data,
        }]),
        Payload::Global(_) => None,
    }
}

fn connected_classes(controller: &Rc<RefCell<Controller>>, catalog: &Catalog) -> Vec<String> {
    let ports = controller
        .borrow_mut()
        .transfer_mut()
        .transport_mut()
        .list_ports()
        .unwrap_or_default();
    catalog
        .devices
        .iter()
        .filter(|device| {
            ports
                .iter()
                .any(|port| korg_app::port_name_matches_device(device, &port.name))
        })
        .map(|device| device.class.clone())
        .collect()
}

fn firmware_for(device: &DeviceInfo, ports: &[korg_midi::PortInfo]) -> Option<String> {
    if let Some(found) = korg_app::detect::detected_class(&device.class) {
        return Some(found.reply.firmware());
    }
    FIRMWARE.with(|cache| {
        if let Some(cached) = cache.borrow().get(&device.class) {
            return cached.clone();
        }
        let needle = device.device.to_lowercase();
        let find = |direction| {
            ports
                .iter()
                .find(|port| {
                    port.direction == direction && port.name.to_lowercase().contains(&needle)
                })
                .map(|port| port.id.clone())
        };
        let version = match (
            find(korg_midi::Direction::Input),
            find(korg_midi::Direction::Output),
        ) {
            (Some(input), Some(output)) => {
                let mut session = MeasuredSession::new(
                    MidirTransport::new(),
                    input,
                    output,
                    SystemClock::new(),
                    500,
                );
                session
                    .open()
                    .ok()
                    .and_then(|()| session.mode_check().ok())
                    .map(|identity| identity.firmware())
            }
            _ => None,
        };
        cache
            .borrow_mut()
            .insert(device.class.clone(), version.clone());
        version
    })
}

fn build_device_cards(
    catalog: &Catalog,
    connected: &[String],
    view: i32,
    ports: &[korg_midi::PortInfo],
    search: &str,
) -> Vec<DeviceCard> {
    let assets = asset_dir();
    catalog
        .devices
        .iter()
        .filter(|device| {
            korg_app::device_passes_filter(device, connected, view == 0, ports, search)
        })
        .map(|device| {
            let enabled = device.scenes.is_some() && device.scene_payload_size.is_some();
            // One row per link; a port outside every link keeps its direction.
            let links = korg_app::device_links(ports, device);
            let mut device_ports: Vec<DevicePort> = links
                .iter()
                .map(|link| DevicePort {
                    kind: format!("{} ⇅", link.label()).into(),
                    name: link.ports_text().into(),
                })
                .collect();
            device_ports.extend(
                ports
                    .iter()
                    .filter(|port| korg_app::port_name_matches_device(device, &port.name))
                    .filter(|port| {
                        !links
                            .iter()
                            .any(|link| link.input == port.name || link.output == port.name)
                    })
                    .map(|port| DevicePort {
                        kind: if port.direction == Direction::Input {
                            "IN ↓"
                        } else {
                            "OUT ↑"
                        }
                        .into(),
                        name: port.name.as_str().into(),
                    }),
            );
            let port_text = if !links.is_empty() {
                links
                    .iter()
                    .map(|link| link.label())
                    .collect::<Vec<_>>()
                    .join(" + ")
            } else if !device_ports.is_empty() {
                format!("{} port(s)", device_ports.len())
            } else {
                String::new()
            };
            let detail = if enabled {
                let firmware = firmware_for(device, ports)
                    .map(|version| format!(" · fw {version}"))
                    .unwrap_or_default();
                format!(
                    "{} · {} scene(s) · {}{firmware}",
                    device.class,
                    device.scenes.unwrap_or(1),
                    if port_text.is_empty() {
                        "no port".to_owned()
                    } else {
                        port_text.clone()
                    },
                )
            } else {
                format!(
                    "{} · data model not extracted · use Open file...",
                    device.class
                )
            };
            let image = assets.as_ref().and_then(|directory| {
                let path = directory.join(korg_devices::external_image(
                    &device.class,
                    "external-thumbnail",
                )?);
                if path.is_file() {
                    slint::Image::load_from_path(&path).ok()
                } else {
                    None
                }
            });
            DeviceCard {
                class: device.class.as_str().into(),
                name: device.device.as_str().into(),
                detail: detail.into(),
                has_image: image.is_some(),
                image: image.unwrap_or_default(),
                connected: connected.contains(&device.class),
                ports: ModelRc::new(VecModel::from(device_ports)),
                enabled,
            }
        })
        .collect()
}

fn asset_dir() -> Option<PathBuf> {
    if let Some(directory) = std::env::var_os("KORG_DEVICE_ASSETS") {
        let path = PathBuf::from(directory);
        if path.is_dir() {
            return Some(path);
        }
    }
    if let Some(path) = korg_app::paths::device_assets_dir() {
        if path.is_dir() {
            return Some(path);
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        let path = PathBuf::from(home).join("Developer/_ANALYZE_/OUT/korg-device-assets");
        if path.is_dir() {
            return Some(path);
        }
    }
    let path = PathBuf::from("assets/devices");
    if path.is_dir() {
        return Some(path);
    }
    None
}

/// Controls lit by recent messages, indices into the panel's controls.
fn lit_now() -> Vec<usize> {
    let now = Instant::now();
    LIT.with(|lit| {
        lit.borrow()
            .iter()
            .filter(|(_, at)| now.duration_since(*at) < LIT_FOR)
            .map(|(index, _)| *index)
            .collect()
    })
}

/// Light the controls that sent `events`, unlight expired ones, and refresh
/// the monitor lines.  Updates rows in place: a full `refresh` per message
/// would rebuild the parameter pane under the user's cursor.
fn show_live(
    ui: &AppWindow,
    controller: &Rc<RefCell<Controller>>,
    selected_class: &Rc<RefCell<Option<String>>>,
    events: &[korg_app::monitor::LiveEvent],
) {
    let now = Instant::now();
    let class = selected_class.borrow().clone();
    if let Some((class, panel)) = class
        .as_deref()
        .and_then(|class| korg_app::panel::panel(class).map(|panel| (class, panel)))
    {
        let controller = controller.borrow();
        let document = controller.session().document();
        let scene = document
            .scene_set
            .as_ref()
            .and_then(|set| set.scenes.get(controller.session().selected_scene()))
            .map(|scene| scene.data.clone())
            .unwrap_or_default();
        let global = korg_app::live::global_channel(
            class,
            document
                .global
                .as_ref()
                .map(|global| global.data.as_slice()),
        );
        LIT.with(|lit| {
            let mut lit = lit.borrow_mut();
            lit.retain(|(_, at)| now.duration_since(*at) < LIT_FOR);
            for event in events {
                for index in
                    korg_app::live::lit_controls(class, &panel, &scene, &event.message, global)
                {
                    lit.retain(|(lit, _)| *lit != index);
                    lit.push((index, now));
                }
            }
        });
    }
    let lit = lit_now();
    let controls = ui.get_panel_controls();
    for row in 0..controls.row_count() {
        if let Some(mut control) = controls.row_data(row) {
            let on = lit.contains(&(control.index as usize));
            if control.lit != on {
                control.lit = on;
                controls.set_row_data(row, control);
            }
        }
    }
    if MONITOR_SHOWN.get() && (!events.is_empty() || ui.get_monitor_title().is_empty()) {
        set_monitor_rows(ui);
    }
}

fn set_monitor_rows(ui: &AppWindow) {
    let (title, rows) = SCENE_MONITOR.with(|monitor| {
        let monitor = monitor.borrow();
        let Some(monitor) = monitor.as_ref() else {
            return ("MIDI Monitor · not connected".to_owned(), Vec::new());
        };
        let title = match &monitor.ports {
            Some((input, _)) => format!("MIDI Monitor · {input}"),
            None => "MIDI Monitor · not connected".to_owned(),
        };
        // Newest first, so the latest message is always in view.
        let rows = monitor
            .log
            .iter()
            .rev()
            .take(MONITOR_ROWS)
            .map(|event| {
                let (r, g, b) = event
                    .message
                    .kind()
                    .map_or((0xd8, 0xd8, 0xd8), |kind| kind.rgb());
                MonitorRow {
                    time: event.time_text().into(),
                    text: event.message.describe().into(),
                    hex: event.message.hex().into(),
                    color: slint::Color::from_rgb_u8(r, g, b),
                }
            })
            .collect::<Vec<_>>();
        (title, rows)
    });
    ui.set_monitor_title(title.into());
    ui.set_monitor_rows(ModelRc::new(VecModel::from(rows)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use korg_core::TransferKind;

    #[test]
    fn opens_nano_pad_offline_with_expected_scene_shape() {
        let controller = Rc::new(RefCell::new(EditorController::new(
            korg_midi::MockTransport::new(Vec::new()),
            korg_core::ManualClock::new(),
        )));
        let catalog = Catalog::load().unwrap();
        let device = catalog.find_by_class("X10830").unwrap();
        open_device(&controller, device).unwrap();
        {
            let controller = controller.borrow();
            let scene_set = controller.session().document().scene_set.as_ref().unwrap();
            assert_eq!(scene_set.scenes.len(), 4);
            assert_eq!(scene_set.element_size, 97);
            assert!(controller.schema().is_some());
        }
        // The mock transport has no ports, so the open path is offline.
        let ports = controller
            .borrow_mut()
            .transfer_mut()
            .transport_mut()
            .list_ports()
            .unwrap();
        assert!(ports.is_empty());
    }

    #[test]
    fn opening_a_device_sets_the_transfer_target() {
        let controller = Rc::new(RefCell::new(EditorController::new(
            korg_midi::MockTransport::new(Vec::new()),
            korg_core::ManualClock::new(),
        )));
        let catalog = Catalog::load().unwrap();
        let device = catalog.find_by_class("X10830").unwrap();
        open_device(&controller, device).unwrap();
        // A write needs these descriptor keys.  With all of them the core
        // starts the transfer; without them it refuses before sending.  This
        // stays valid when the decompile fills in `mode_check`.
        let descriptor = korg_devices::sysex_descriptor("X10830").unwrap();
        let ready = descriptor.model_id.is_some()
            && descriptor.length_style.is_some()
            && [korg_core::MODE_CHECK_KEY, "scene_data", "scene_data_write"]
                .iter()
                .all(|key| descriptor.commands.get(*key).copied().flatten().is_some());
        let outcome = controller
            .borrow_mut()
            .dispatch(ControllerAction::Write(TransferKind::SceneData));
        if ready {
            assert_eq!(outcome.error, None);
            assert_eq!(controller.borrow().wants_tick(), TickRate::Fast);
        } else {
            assert!(matches!(
                outcome.error,
                Some(korg_core::ControllerError::Transfer(
                    korg_core::TransferError::DescriptorIncomplete { .. }
                ))
            ));
            assert_eq!(controller.borrow().wants_tick(), TickRate::Slow);
        }
        // Dispatch only plans; bytes go out on the first tick.
        assert!(controller.borrow().transfer().transport().sent.is_empty());

        let target = device_target(device);
        assert_eq!(target.class.as_str(), "X10830");
        assert_eq!(target.scene_payload_size, Some(97));
        assert!(target.descriptor.is_some());

        // A complete descriptor yields a plan, whatever the shipped one holds
        // today.  The 0x10 here is a synthetic test value, not a device fact.
        let mut complete = target;
        let commands = &mut complete.descriptor.as_mut().unwrap().commands;
        for key in [korg_core::MODE_CHECK_KEY, "scene_data", "scene_data_write"] {
            let byte = commands.get(key).copied().flatten().unwrap_or(0x10);
            commands.insert(key.to_owned(), Some(byte));
        }
        controller.borrow_mut().set_device(Some(complete));
        let outcome = controller
            .borrow_mut()
            .dispatch(ControllerAction::Write(TransferKind::SceneData));
        assert_eq!(outcome.error, None);
        assert_eq!(controller.borrow().wants_tick(), TickRate::Fast);
    }

    #[test]
    fn transfer_events_have_status_text() {
        assert_eq!(transfer_status(&TransferEvent::None), None);
        assert_eq!(
            transfer_status(&TransferEvent::Sent(korg_sysex::Phase::ModeCheck)).as_deref(),
            Some("Transfer: checking the mode...")
        );
        assert_eq!(
            transfer_status(&TransferEvent::Received(vec![0; 97])).as_deref(),
            Some("Received 97 bytes.")
        );
        assert!(
            transfer_status(&TransferEvent::Failed(korg_core::TransferError::NoReply))
                .unwrap()
                .starts_with("Transfer failed:")
        );
    }

    #[test]
    fn builds_all_device_cards() {
        let catalog = Catalog::load().unwrap();
        let cards = build_device_cards(&catalog, &[], 1, &[], "");
        assert_eq!(cards.len(), 16);
        assert!(cards.iter().filter(|card| card.has_image).count() >= 14);
        assert!(
            cards
                .iter()
                .find(|card| card.class == "X10830")
                .unwrap()
                .enabled
        );
        let pad = cards.iter().find(|card| card.class == "X10830").unwrap();
        assert!(pad.enabled);
        assert!(!pad.connected);
        assert!(
            !cards
                .iter()
                .find(|card| card.class == "X21212")
                .unwrap()
                .enabled
        );
    }

    #[test]
    fn prefers_the_vendor_scene_set_extension() {
        let extensions = vec![
            "nktrl_st_data".to_owned(),
            "nktrl_st_set".to_owned(),
            "nktrl_st_glob".to_owned(),
        ];
        assert_eq!(preferred_scene_extension(&extensions), Some("nktrl_st_set"));
        let data_only = vec!["nanokey2_data".to_owned()];
        assert_eq!(preferred_scene_extension(&data_only), Some("nanokey2_data"));
        assert_eq!(preferred_scene_extension(&[]), None);
    }

    #[test]
    fn ui_bpm_callback_does_not_scale_twice() {
        let mut controller = EditorController::new(
            korg_midi::MockTransport::new(Vec::new()),
            korg_core::ManualClock::new(),
        );
        controller.load(ControllerInput::Global(GlobalData {
            model: korg_model::FileModelKey::new("1480").unwrap(),
            data: vec![0u8; 128],
        }));
        dispatch_ui_bpm(&mut controller, 1210);
        let data = &controller
            .session()
            .document()
            .global
            .as_ref()
            .unwrap()
            .data;
        assert_eq!(u16::from(data[57]) | (u16::from(data[58]) << 8), 1210);
    }

    #[test]
    fn unknown_global_choice_bytes_are_not_reported_as_defaults() {
        let mut data = vec![0; 128];
        let fields = korg_devices::global_fields("X14800").unwrap();
        let offsets = korg_devices::global_ui_offsets(Some(fields));
        data[usize::try_from(offsets[1]).unwrap()] = 255;
        data[usize::try_from(offsets[2]).unwrap()] = 255;
        let (_, _, battery, _, clock, _) = global_display(Some("X14800"), Some(&data));
        assert_eq!(
            (battery.as_str(), clock.as_str()),
            ("Unknown (255)", "Unknown (255)")
        );
        let (_, _, battery, _, clock, _) = global_display(Some("X14800"), None);
        assert_eq!((battery.as_str(), clock.as_str()), ("Unknown", "Unknown"));
    }

    #[test]
    fn battery_global_controls_use_battery_roles_and_labels() {
        let labels = korg_devices::global_ui_labels(korg_devices::global_fields("X14800"));
        assert_eq!(labels[10], "Auto LED Off"); // USB
        assert_eq!(labels[14], "Auto Power Off"); // Battery
        assert_eq!(labels[11], "LED Brightness"); // USB
        assert_eq!(labels[16], "LED Brightness"); // Battery

        // Keep the repeated battery panel bound to its own descriptor roles.
        let ui = include_str!("../ui/app.slint");
        assert!(ui.contains("glabel(14); width: 150px; horizontal-alignment: right; color: #f0f0f0; vertical-alignment: center; } CompactCombo { width: 130px; model: root.global-choice-lists[14].labels;"));
        assert!(ui.contains("glabel(15); width: 150px; horizontal-alignment: right; color: #f0f0f0; vertical-alignment: center; } CompactCombo { width: 130px; model: root.global-choice-lists[15].labels;"));
        assert!(ui.contains("glabel(16); width: 150px; horizontal-alignment: right; color: #f0f0f0; vertical-alignment: center; } CompactSpin { width: 92px; minimum: root.grange(16).minimum;"));
        assert!(ui.contains("glabel(17); width: 150px; horizontal-alignment: right; color: #f0f0f0; vertical-alignment: center; } CompactCombo { width: 130px; model: root.global-choice-lists[17].labels;"));
        assert!(ui.contains("glabel(18); width: 150px; horizontal-alignment: right; color: #f0f0f0; vertical-alignment: center; } CompactCombo { width: 130px; model: root.global-choice-lists[18].labels;"));
    }

    #[test]
    fn global_display_uses_relocated_external_bindings() {
        const CHILD: &str = "KORG_GLOBAL_BINDING_TEST_CHILD";
        if std::env::var_os(CHILD).is_some() {
            let mut data = vec![0; 128];
            data[7] = 3;
            data[4] = 9;
            data[6] = 2;
            let (channel, _, battery, _, clock, _) = global_display(Some("X14800"), Some(&data));
            assert_eq!(
                (channel.as_str(), battery.as_str(), clock.as_str()),
                ("4", "Rechargeable", "External-USB")
            );
            let offsets = korg_devices::global_ui_offsets(korg_devices::global_fields("X14800"));
            let fields = korg_devices::global_fields("X14800");
            let range = korg_devices::global_ui_number_range(fields, 0).unwrap();
            assert_eq!((range.min, range.max, range.bias), (2, 8, -1));
            assert_eq!(
                korg_devices::global_ui_number_selection(fields, 0, 2),
                Some((7, 1))
            );
            assert_eq!(korg_devices::global_ui_number_selection(fields, 0, 9), None);
            assert_eq!(
                korg_devices::global_ui_choice_selection(
                    korg_devices::global_fields("X14800"),
                    1,
                    1
                ),
                Some((4, 9))
            );
            let mut controller = EditorController::new(
                korg_midi::MockTransport::new(Vec::new()),
                korg_core::ManualClock::new(),
            );
            controller.load(ControllerInput::Global(GlobalData {
                model: korg_model::FileModelKey::new("1480").unwrap(),
                data: vec![0; 128],
            }));
            assert!(controller
                .dispatch(ControllerAction::SetGlobalByte {
                    offset: offsets[0] as usize,
                    value: 5,
                })
                .error
                .is_none());
            let global = &controller
                .session()
                .document()
                .global
                .as_ref()
                .unwrap()
                .data;
            assert_eq!((global[7], global[0]), (5, 0));
            return;
        }
        let source =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../descriptors/korg");
        let root =
            std::env::temp_dir().join(format!("korg-ui-global-bindings-{}", std::process::id()));
        let destination = root.join("korg");
        std::fs::create_dir_all(&destination).unwrap();
        for entry in std::fs::read_dir(source).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|extension| extension == "kdl") {
                std::fs::copy(&path, destination.join(path.file_name().unwrap())).unwrap();
            }
        }
        let file = destination.join("nanokey-studio.global.inc.kdl");
        let data = std::fs::read_to_string(&file).unwrap();
        let modified = data
            .replace(
                "binding \"channel\" label=\"Global MIDI Channel\" offset=0 min=1 max=16",
                "binding \"channel\" label=\"Global MIDI Channel\" offset=7 min=2 max=8",
            )
            .replace(
                "binding \"battery\" label=\"Battery Type\" offset=1",
                "binding \"battery\" label=\"Battery Type\" offset=4",
            )
            .replace(
                "binding \"clock\" label=\"MIDI Clock Source\" offset=56",
                "binding \"clock\" label=\"MIDI Clock Source\" offset=6",
            );
        let modified = modified.replace(
            "choice \"Ni-MH\" value=1",
            "choice \"Rechargeable\" value=9",
        );
        assert_ne!(data, modified);
        std::fs::write(file, modified).unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("tests::global_display_uses_relocated_external_bindings")
            .env(CHILD, "1")
            .env("KORG_DESCRIPTOR_DIR", &root)
            .status()
            .unwrap();
        std::fs::remove_dir_all(root).unwrap();
        assert!(status.success());
    }

    #[test]
    fn opened_device_has_editable_global_defaults() {
        let controller = Rc::new(RefCell::new(EditorController::new(
            korg_midi::MockTransport::new(Vec::new()),
            korg_core::ManualClock::new(),
        )));
        let catalog = Catalog::load().unwrap();
        let device = catalog.find_by_class("X14800").unwrap();
        open_device(&controller, device).unwrap();
        let outcome = controller
            .borrow_mut()
            .dispatch(ControllerAction::SetGlobalByte {
                offset: 0,
                value: 5,
            });
        assert_eq!(outcome.error, None);
        assert_eq!(
            controller
                .borrow()
                .session()
                .document()
                .global
                .as_ref()
                .unwrap()
                .data[0],
            5
        );
    }

    #[test]
    fn bpm_display_keeps_one_decimal() {
        assert_eq!(format_bpm_tenths(1205), "120.5");
        assert_eq!(format_bpm_tenths(200), "20.0");
    }

    #[test]
    fn panel_contain_keeps_picture_inside_canvas() {
        for canvas in [(900.0, 300.0), (400.0, 400.0), (500.0, 294.0)] {
            let panel = (500.0, 294.0);
            let (scale, ox, oy) = contain_panel(canvas, panel);
            assert!(ox >= 0.0 && oy >= 0.0, "{canvas:?}: {scale} {ox} {oy}");
            assert!(ox + panel.0 * scale <= canvas.0 + 0.001);
            assert!(oy + panel.1 * scale <= canvas.1 + 0.001);
        }
    }

    #[test]
    fn x14800_control_groups_have_the_original_labels() {
        let cases: &[(&str, &[&str])] = &[
            ("Keyboard", &["MIDI Channel"]),
            ("Chord", &["MIDI Channel"]),
            (
                "XY",
                &[
                    "Common",
                    "MIDI Channel",
                    "Touch",
                    "Enable",
                    "CC Number",
                    "Off Value",
                    "On Value",
                    "X-axis",
                    "Assign Type",
                    "CC Number",
                    "Left Value",
                    "Right Value",
                    "Y-axis",
                    "Assign Type",
                    "CC Number",
                    "Lower Value",
                    "Upper Value",
                ],
            ),
            (
                "PitchMod",
                &["Mod", "Enable", "CC Number", "Lower Value", "Upper Value"],
            ),
            (
                "TouchScale",
                &[
                    "Common",
                    "MIDI Channel",
                    "Y-axis",
                    "Enable",
                    "CC Number",
                    "Lower Value",
                    "Upper Value",
                ],
            ),
            (
                "Sustain",
                &[
                    "MIDI Channel",
                    "Assign Type",
                    "CC Number",
                    "Off Value",
                    "On Value",
                    "Button Behavior",
                ],
            ),
            (
                "Pad",
                &[
                    "MIDI Channel",
                    "Assign Type",
                    "Note/CC/Prog Number 1",
                    "Note/CC/Prog Number 2",
                    "Note/CC/Prog Number 3",
                    "Note/CC/Prog Number 4",
                    "Pad Behavior",
                ],
            ),
            (
                "Knob",
                &[
                    "MIDI Channel",
                    "Enable",
                    "CC Number",
                    "Left Value",
                    "Right Value",
                ],
            ),
        ];
        let panel = device_panel("X14800").unwrap();
        let data = vec![0u8; 256];
        for (group, expected) in cases {
            let control_label = match *group {
                "Chord" => "Chord Pad",
                "XY" => "X-Y Pad",
                "PitchMod" => "Pitch/Mod",
                "TouchScale" => "Touch Scale",
                "Pad" => "Pad 1",
                "Knob" => "Knob 1",
                other => other,
            };
            let control = panel.controls.iter().find(|c| c.label == control_label);
            let labels: Vec<String> = korg_app::pages::fields("X14800", &panel, control, &data)
                .unwrap()
                .iter()
                .map(|row| page_row(row, &data).label.to_string())
                .collect();
            assert_eq!(&labels, expected, "{group}");
        }
    }

    #[test]
    fn device_search_matches_only_the_devices_own_ports() {
        let catalog = Catalog::load().unwrap();
        let ports = vec![korg_midi::PortInfo {
            id: korg_midi::PortId::new("nano-out"),
            name: "nanoKEY Studio CTRL".into(),
            direction: Direction::Output,
        }];
        let cards = build_device_cards(&catalog, &[], 1, &ports, "nano");
        assert!(!cards.is_empty());
        assert!(
            cards
                .iter()
                .all(|card| card.name.to_lowercase().contains("nano")),
            "{:?}",
            cards
                .iter()
                .map(|card| card.name.as_str())
                .collect::<Vec<_>>()
        );
        let studio = build_device_cards(&catalog, &[], 1, &ports, "studio");
        let mut names: Vec<&str> = studio.iter().map(|card| card.name.as_str()).collect();
        names.sort();
        assert_eq!(names, ["nanoKEY Studio", "nanoKONTROL Studio"]);
    }

    #[test]
    fn device_card_shows_one_row_per_link() {
        let catalog = Catalog::load().unwrap();
        let mut ports = Vec::new();
        for name in ["nanoKONTROL Studio CTRL", "nanoKONTROL Studio Bluetooth"] {
            for direction in [Direction::Input, Direction::Output] {
                ports.push(korg_midi::PortInfo {
                    id: korg_midi::PortId::new(format!("{name}#0")),
                    name: name.into(),
                    direction,
                });
            }
        }
        let cards = build_device_cards(&catalog, &[], 1, &ports, "nanoKONTROL Studio");
        let card = cards.iter().find(|card| card.class == "X14802").unwrap();
        let rows: Vec<(String, String)> = card
            .ports
            .iter()
            .map(|port| (port.kind.to_string(), port.name.to_string()))
            .collect();
        assert_eq!(
            rows,
            [
                ("USB ⇅".to_owned(), "nanoKONTROL Studio CTRL".to_owned()),
                (
                    "Bluetooth ⇅".to_owned(),
                    "nanoKONTROL Studio Bluetooth".to_owned()
                ),
            ]
        );
        assert!(card.detail.contains("USB + Bluetooth"), "{}", card.detail);
    }
}
