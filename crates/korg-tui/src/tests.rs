//! Key flows and rendering, without a terminal or MIDI hardware.

use crate::app::{App, Command, Focus, JobMessage, Modal, Screen};
use korg_app::monitor::SceneMonitor;
use korg_app::panel::Binding;
use korg_app::panel::TreeRow;
use korg_app::{DeviceJob, JobOutput, JobProgress, LinkKind};
use korg_core::{EditorController, ManualClock};
use korg_devices::Catalog;
use korg_midi::{Direction, MockTransport, PortId, PortInfo};
use korg_model::FileModelKey;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::style::Color;
use ratatui::Terminal;
use std::path::PathBuf;

type TestApp = App<MockTransport, ManualClock>;

fn app_with_ports(ports: Vec<PortInfo>) -> TestApp {
    let controller = EditorController::new(MockTransport::new(ports.clone()), ManualClock::new());
    let monitor = SceneMonitor::new(move || MockTransport::new(ports));
    let mut app = App::new(
        controller,
        monitor,
        Catalog::load().unwrap(),
        PathBuf::from("/nonexistent-products"),
    );
    app.auto_receive = false;
    // The tests must not write the real preferences file, and must not depend
    // on the link the real file holds.
    app.write_preferences = false;
    app.link = None;
    app.tick(); // first port scan
    app
}

fn app() -> TestApp {
    app_with_ports(Vec::new())
}

/// Like `app_with_ports`, but the monitor's own transport already holds the
/// measured scene-change announcement (scene 3).
fn app_with_announcement(ports: Vec<PortInfo>) -> TestApp {
    let controller = EditorController::new(MockTransport::new(ports.clone()), ManualClock::new());
    let mut transport = MockTransport::new(ports);
    transport.push_packet([
        0xF0, 0x42, 0x40, 0x00, 0x01, 0x37, 0x02, 0x00, 0x00, 0x4F, 0x02, 0xF7,
    ]);
    let mut app = App::new(
        controller,
        SceneMonitor::new(move || transport),
        Catalog::load().unwrap(),
        PathBuf::from("/nonexistent-products"),
    );
    app.auto_receive = false;
    app.write_preferences = false;
    app.link = None;
    app.tick();
    app
}

fn studio_ports() -> Vec<PortInfo> {
    both("nanoKONTROL Studio")
}

/// The four ports CoreMIDI lists when the Studio is on USB and Bluetooth.
fn studio_link_ports() -> Vec<PortInfo> {
    let mut ports = both("nanoKONTROL Studio CTRL");
    ports.extend(both("nanoKONTROL Studio Bluetooth"));
    ports
}

fn both(name: &str) -> Vec<PortInfo> {
    [("in", Direction::Input), ("out", Direction::Output)]
        .into_iter()
        .map(|(id, direction)| PortInfo {
            id: PortId::new(format!("{name} {id}")),
            name: name.into(),
            direction,
        })
        .collect()
}

/// Put the device list cursor on `class`.
fn select(app: &mut TestApp, class: &str) {
    let row = app
        .device_rows()
        .iter()
        .position(|&index| app.catalog.devices[index].class == class)
        .unwrap();
    app.device_cursor = row;
}

/// The rows the shared panel builds for the app's list state.
fn tree_rows(app: &TestApp) -> Vec<TreeRow> {
    app.panel()
        .unwrap()
        .tree(app.open_group.as_deref(), app.control)
}

/// Index of the control labelled `label` (for example "Knob 3").
fn knob(app: &TestApp, label: &str) -> usize {
    app.panel()
        .unwrap()
        .controls
        .iter()
        .position(|control| control.label == label)
        .unwrap()
}

/// Open `class` and queue its receive job.
fn receive_job(app: &mut TestApp) -> crate::app::JobRequest {
    select(app, "X14802");
    press(app, KeyCode::Enter);
    assert_eq!(app.screen, Screen::Editor, "{}", app.status);
    press(app, KeyCode::Char('r'));
    app.take_job().expect("job queued")
}

fn press(app: &mut TestApp, code: KeyCode) {
    app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
}

fn keys(app: &mut TestApp, text: &str) {
    for c in text.chars() {
        press(app, KeyCode::Char(c));
    }
}

/// Type `text` into the prompt for the row under the parameter cursor.
fn type_value(app: &mut TestApp, row: usize, text: &str) {
    app.param_cursor = row;
    press(app, KeyCode::Enter);
    assert!(matches!(app.modal, Modal::Prompt { .. }), "{}", app.status);
    for _ in 0..20 {
        press(app, KeyCode::Backspace);
    }
    keys(app, text);
    press(app, KeyCode::Enter);
}

fn open(app: &mut TestApp, class: &str) {
    let row = app
        .device_rows()
        .iter()
        .position(|&index| app.catalog.devices[index].class == class)
        .unwrap();
    app.device_cursor = row;
    press(app, KeyCode::Enter);
    assert_eq!(app.screen, Screen::Editor, "{}", app.status);
}

/// Render `app` and keep the clickable areas, as the main loop does.
fn render(app: &mut TestApp) -> String {
    let mut terminal = Terminal::new(TestBackend::new(140, 40)).unwrap();
    let mut hits = Default::default();
    terminal
        .draw(|frame| hits = crate::ui::draw(frame, app))
        .unwrap();
    app.hits = hits;
    text_of(&terminal)
}

fn mouse(app: &mut TestApp, kind: MouseEventKind, column: u16, row: u16) {
    app.on_mouse(MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    });
}

fn click(app: &mut TestApp, column: u16, row: u16) {
    mouse(app, MouseEventKind::Down(MouseButton::Left), column, row);
}

/// The cell over panel coordinates `(x, y)` in the last frame.
fn panel_cell(app: &TestApp, x: f64, y: f64) -> (u16, u16) {
    let area = app.hits.panel.expect("schematic drawn");
    let panel = app.panel().unwrap();
    (
        area.x + (x / panel.width * f64::from(area.width)) as u16,
        area.y + (y / panel.height * f64::from(area.height)) as u16,
    )
}

fn screen(app: &TestApp) -> String {
    let mut terminal = Terminal::new(TestBackend::new(140, 40)).unwrap();
    terminal
        .draw(|frame| {
            crate::ui::draw(frame, app);
        })
        .unwrap();
    text_of(&terminal)
}

fn text_of(terminal: &Terminal<TestBackend>) -> String {
    let buffer = terminal.backend().buffer();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn device_screen_lists_the_catalog_and_marks_connected_devices() {
    let app = app_with_ports(studio_ports());
    let text = screen(&app);
    assert!(text.contains("nanoKONTROL Studio"));
    assert!(text.contains("1 connected"));
    assert!(text.contains("● nanoKONTROL Studio"));
}

#[test]
fn connected_only_filters_the_list() {
    let mut app = app_with_ports(studio_ports());
    press(&mut app, KeyCode::Char('c'));
    let rows = app.device_rows();
    assert_eq!(rows.len(), 1);
    assert_eq!(app.catalog.devices[rows[0]].class, "X14802");
}

#[test]
fn editing_a_parameter_marks_dirty_and_undo_reverts() {
    let mut app = app();
    open(&mut app, "X10820");
    // No control selected: the nanoKEY2 Common page, as in the GUI.
    let field = |app: &TestApp| app.control_fields().expect("nanoKEY2 page")[0].clone();
    assert_eq!(field(&app).label, "Key Mode MIDI Channel");
    let before = field(&app).display();
    press(&mut app, KeyCode::Right);
    assert!(app.view().dirty);
    assert_ne!(field(&app).display(), before);
    press(&mut app, KeyCode::Char('u'));
    assert_eq!(field(&app).display(), before);
    assert!(!app.view().dirty);
    press(&mut app, KeyCode::Char('U'));
    assert!(app.view().dirty);
}

#[test]
fn typed_values_are_range_checked() {
    let mut app = app();
    open(&mut app, "X10820");
    type_value(&mut app, 0, "17");
    assert!(app.status.contains("is not a value"), "{}", app.status);
    assert!(!app.view().dirty);
}

#[test]
fn studio_jog_fields_follow_the_type() {
    let mut app = app();
    open(&mut app, "X14802");
    app.select_control(knob(&app, "Jog Wheel"));
    let labels = |app: &TestApp| -> Vec<&'static str> {
        app.control_fields()
            .expect("studio fields")
            .iter()
            .map(|field| field.label)
            .collect()
    };
    // The jog is an ordinary control: its rows come from studio_fields.
    assert!(labels(&app).contains(&"Type"), "{:?}", labels(&app));
    for _ in 0..7 {
        if labels(&app).contains(&"Min Value") {
            break;
        }
        press(&mut app, KeyCode::Right);
    }
    let continuous = labels(&app);
    assert!(
        continuous.contains(&"Min Value") && continuous.contains(&"Max Value"),
        "{continuous:?}"
    );
    assert!(!continuous.contains(&"CW CC Number"), "{continuous:?}");
    // No Assign uses only Type and MIDI Channel.
    for _ in 0..7 {
        if !labels(&app).contains(&"Acceleration") {
            break;
        }
        press(&mut app, KeyCode::Right);
    }
    assert_eq!(labels(&app), ["Type", "MIDI Channel"]);
}

#[test]
fn studio_jog_min_max_clamp() {
    let mut app = app();
    open(&mut app, "X14802");
    app.select_control(knob(&app, "Jog Wheel"));
    for _ in 0..7 {
        if app
            .control_fields()
            .expect("studio fields")
            .iter()
            .any(|field| field.label == "Min Value")
        {
            break;
        }
        press(&mut app, KeyCode::Right);
    }
    let row = |app: &TestApp, label: &str| -> usize {
        app.control_fields()
            .expect("studio fields")
            .iter()
            .position(|field| field.label == label)
            .unwrap()
    };
    let min_row = row(&app, "Min Value");
    let max_row = row(&app, "Max Value");
    type_value(&mut app, min_row, "0");
    type_value(&mut app, max_row, "10");
    assert_eq!(app.scene_bytes()[520], 10);
    // Minimum cannot pass maximum.
    type_value(&mut app, min_row, "120");
    assert_eq!(app.scene_bytes()[519], 10, "minimum clamped to maximum");
    assert!(app.scene_bytes()[520] >= app.scene_bytes()[519]);
    // Maximum cannot drop below minimum.
    type_value(&mut app, max_row, "3");
    assert_eq!(app.scene_bytes()[520], 10, "maximum clamped to minimum");
    assert!(app.scene_bytes()[520] >= app.scene_bytes()[519]);
}

#[test]
fn studio_jog_channel_offers_global() {
    let mut app = app();
    open(&mut app, "X14802");
    app.select_control(knob(&app, "Jog Wheel"));
    let fields = app.control_fields().expect("studio fields");
    let row = fields
        .iter()
        .position(|field| field.label == "MIDI Channel")
        .expect("jog channel");
    assert_eq!(fields[row].range(), "1..16, Global");
    type_value(&mut app, row, "Global");
    assert_eq!(app.scene_bytes()[512], 16);
    assert!(app.view().jog.is_some(), "the jog block stays decodable");
}

#[test]
fn studio_scene_name_is_typed_and_relabels_the_scene() {
    let mut app = app();
    open(&mut app, "X14802");
    app.select_control(knob(&app, "Scene"));
    let fields = app.control_fields().expect("studio fields");
    assert_eq!(fields[0].label, "Scene Name");
    type_value(&mut app, 0, "GONZO");
    assert_eq!(&app.scene_bytes()[..12], b"GONZO       ");
    let view = app.view();
    assert_eq!(view.scene_labels[view.selected_scene], "GONZO");
    assert_eq!(app.control_fields().unwrap()[0].display(), "GONZO");
    press(&mut app, KeyCode::Char('u'));
    assert_ne!(&app.scene_bytes()[..5], b"GONZO", "undo restores the name");
}

#[test]
fn a_write_needs_confirmation_and_n_sends_nothing() {
    let mut app = app_with_ports(studio_ports());
    open(&mut app, "X14802");
    press(&mut app, KeyCode::Char('2'));
    press(&mut app, KeyCode::Char('w'));
    let Modal::Confirm { text, .. } = &app.modal else {
        panic!("no confirmation: {}", app.status);
    };
    assert!(text.contains("scene 2"));
    assert!(screen(&app).contains("Write to device"));
    press(&mut app, KeyCode::Char('n'));
    assert!(app.take_job().is_none());
    assert!(app.status.contains("Nothing was sent"));

    press(&mut app, KeyCode::Char('w'));
    press(&mut app, KeyCode::Char('y'));
    let request = app.take_job().expect("job");
    assert_eq!(request.input, "nanoKONTROL Studio");
    assert_eq!(request.output, "nanoKONTROL Studio");
    assert!(matches!(
        request.job,
        DeviceJob::WriteScene { index: 1, ref key, ref payload } if key == "1482" && payload.len() == 560
    ));
    assert!(app.busy.is_some());
}

#[test]
fn write_all_sends_every_scene() {
    let mut app = app_with_ports(studio_ports());
    open(&mut app, "X14802");
    press(&mut app, KeyCode::Char('W'));
    press(&mut app, KeyCode::Char('y'));
    let request = app.take_job().unwrap();
    assert!(
        matches!(request.job, DeviceJob::WriteSceneSet { ref scenes, .. } if scenes.len() == 5)
    );
}

#[test]
fn no_port_means_no_job() {
    let mut app = app();
    open(&mut app, "X14802");
    press(&mut app, KeyCode::Char('w'));
    assert_eq!(app.modal, Modal::None);
    assert!(app.status.contains("No MIDI port matches"));
}

#[test]
fn receive_refuses_to_replace_unsaved_edits() {
    let mut app = app_with_ports(studio_ports());
    open(&mut app, "X14802");
    app.select_control(knob(&app, "Knob 3"));
    press(&mut app, KeyCode::Right);
    assert!(app.view().dirty);
    press(&mut app, KeyCode::Char('r'));
    assert!(app.take_job().is_none());
    assert!(app.status.contains("Unsaved"));
}

#[test]
fn a_received_scene_set_becomes_a_clean_document() {
    let mut app = app_with_ports(studio_ports());
    press(&mut app, KeyCode::Char('c'));
    press(&mut app, KeyCode::Char('r'));
    let request = app.take_job().unwrap();
    assert_eq!(request.job, DeviceJob::ReceiveSceneSet);
    app.on_job(JobMessage::Progress(JobProgress::Receiving));
    assert!(screen(&app).contains("Receive: receiving"));
    let mut scene = vec![0u8; 560];
    scene[..5].copy_from_slice(b"GONZO");
    app.on_job(JobMessage::Done(Ok(JobOutput::SceneSet {
        key: FileModelKey::new("1482").unwrap(),
        scenes: vec![scene; 5],
    })));
    assert!(app.busy.is_none());
    assert_eq!(app.class.as_deref(), Some("X14802"));
    assert_eq!(app.screen, Screen::Editor);
    let view = app.view();
    assert_eq!(view.scene_labels[0], "GONZO");
    assert!(!view.dirty);
    assert!(app.status.contains("Received 5 scenes"));
}

#[test]
fn a_failed_job_reports_the_error() {
    let mut app = app_with_ports(studio_ports());
    open(&mut app, "X14802");
    press(&mut app, KeyCode::Char('r'));
    app.take_job().unwrap();
    app.on_job(JobMessage::Done(Err("no reply within 3000 ms".into())));
    assert_eq!(app.status, "Receive failed: no reply within 3000 ms");
}

#[test]
fn quitting_with_unsaved_edits_needs_a_second_q() {
    let mut app = app();
    open(&mut app, "X10820");
    press(&mut app, KeyCode::Right);
    press(&mut app, KeyCode::Char('q'));
    assert!(!app.quit);
    press(&mut app, KeyCode::Char('q'));
    assert!(app.quit);
}

#[test]
fn save_then_open_round_trips_through_a_file() {
    let dir = std::env::temp_dir().join(format!("korg-tui-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("set.nktrl_st_set");
    let mut app = app();
    open(&mut app, "X14802");
    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Right);
    let jog = app.view().jog.unwrap();
    press(&mut app, KeyCode::Char('s'));
    let Modal::Prompt { input, .. } = &app.modal else {
        panic!()
    };
    assert!(input.ends_with(".nktrl_st_set"), "{input}");
    for _ in 0..input.len() {
        press(&mut app, KeyCode::Backspace);
    }
    keys(&mut app, &path.display().to_string());
    press(&mut app, KeyCode::Enter);
    assert!(app.status.starts_with("Saved"), "{}", app.status);
    assert!(!app.view().dirty);

    let mut other = self::app();
    press(&mut other, KeyCode::Char('o'));
    keys(&mut other, &path.display().to_string());
    press(&mut other, KeyCode::Enter);
    assert_eq!(other.class.as_deref(), Some("X14802"), "{}", other.status);
    assert_eq!(other.view().jog, Some(jog));
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn help_opens_and_any_key_closes_it() {
    let mut app = app();
    press(&mut app, KeyCode::Char('?'));
    assert!(screen(&app).contains("Help"));
    press(&mut app, KeyCode::Char('x'));
    assert_eq!(app.modal, Modal::None);
}

#[test]
fn renders_in_a_small_terminal_without_panicking() {
    let mut app = app_with_ports(studio_ports());
    open(&mut app, "X14802");
    press(&mut app, KeyCode::Char('?'));
    let mut terminal = Terminal::new(TestBackend::new(30, 8)).unwrap();
    terminal
        .draw(|frame| {
            crate::ui::draw(frame, &app);
        })
        .unwrap();
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::F(10));
    terminal
        .draw(|frame| {
            crate::ui::draw(frame, &app);
        })
        .unwrap();
}

#[test]
fn prompts_ignore_alt_chords() {
    let mut app = app();
    press(&mut app, KeyCode::Char('o'));
    app.on_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::ALT));
    keys(&mut app, "x");
    assert_eq!(
        app.modal,
        Modal::Prompt {
            kind: crate::app::PromptKind::OpenFile,
            input: "x".into()
        }
    );
}

#[test]
fn the_details_show_the_link_set_with_p() {
    let mut app = app_with_ports(studio_ports());
    select(&mut app, "X14802");
    assert!(screen(&app).contains("Link       USB · nanoKONTROL Studio"));
    press(&mut app, KeyCode::Char('p'));
    keys(&mut app, "nanoKONTROL Studio Bluetooth");
    press(&mut app, KeyCode::Enter);
    let text = screen(&app);
    assert!(
        text.contains("Link       nanoKONTROL Studio Bluetooth  (set with p)"),
        "{text}"
    );
}

#[test]
fn the_details_offer_the_next_link() {
    let mut app = app_with_ports(studio_link_ports());
    select(&mut app, "X14802");
    let text = screen(&app);
    assert!(
        text.contains("Link       USB · nanoKONTROL Studio CTRL  (l: Bluetooth)"),
        "{text}"
    );
}

#[test]
fn a_job_uses_the_input_and_output_of_the_usb_link() {
    let mut app = app_with_ports(studio_link_ports());
    let job = receive_job(&mut app);
    assert_eq!(job.input, "nanoKONTROL Studio CTRL");
    assert_eq!(job.output, "nanoKONTROL Studio CTRL");
}

#[test]
fn l_cycles_to_bluetooth_and_the_job_follows() {
    let mut app = app_with_ports(studio_link_ports());
    select(&mut app, "X14802");
    press(&mut app, KeyCode::Char('l'));
    assert_eq!(app.link, Some(LinkKind::Bluetooth));
    assert_eq!(app.status, "Link: Bluetooth");
    assert!(screen(&app).contains("Link       Bluetooth · nanoKONTROL Studio Bluetooth"));
    let job = receive_job(&mut app);
    assert_eq!(job.input, "nanoKONTROL Studio Bluetooth");
    assert_eq!(job.output, "nanoKONTROL Studio Bluetooth");
}

#[test]
fn bluetooth_alone_is_used_and_l_reports_one_link() {
    let mut app = app_with_ports(both("nanoKONTROL Studio Bluetooth"));
    select(&mut app, "X14802");
    press(&mut app, KeyCode::Char('l'));
    assert_eq!(app.link, None);
    assert_eq!(app.status, "Only one link connected");
    let job = receive_job(&mut app);
    assert_eq!(job.input, "nanoKONTROL Studio Bluetooth");
    assert_eq!(job.output, "nanoKONTROL Studio Bluetooth");
}

#[test]
fn the_port_override_still_wins_over_the_link() {
    let mut app = app_with_ports(studio_link_ports());
    press(&mut app, KeyCode::Char('p'));
    keys(&mut app, "nanoKONTROL Studio CTRL");
    press(&mut app, KeyCode::Enter);
    select(&mut app, "X14802");
    press(&mut app, KeyCode::Char('l'));
    let job = receive_job(&mut app);
    assert_eq!(job.input, "nanoKONTROL Studio CTRL");
    assert_eq!(job.output, "nanoKONTROL Studio CTRL");
}

#[test]
fn the_details_show_wireless_on_for_a_bluetooth_link() {
    let mut app = app_with_ports(studio_link_ports());
    select(&mut app, "X14802");
    let text = screen(&app);
    assert!(
        text.contains("Wireless   On (Bluetooth link to this computer)"),
        "{text}"
    );
    assert!(
        text.contains("on device: Hold Scene and press Cycle"),
        "{text}"
    );
}

#[test]
fn a_usb_only_link_leaves_wireless_unknown() {
    let mut app = app_with_ports(studio_ports());
    select(&mut app, "X14802");
    let text = screen(&app);
    assert!(
        text.contains("Wireless   Unknown (no Bluetooth link to this computer)"),
        "{text}"
    );
}

#[test]
fn a_device_without_a_wireless_instruction_has_no_line() {
    let mut app = app_with_ports(both("nanoKEY2"));
    select(&mut app, "X10820");
    assert!(!screen(&app).contains("Wireless"));
}

#[test]
fn an_unconnected_device_has_no_wireless_line() {
    let mut app = app();
    select(&mut app, "X14802");
    assert!(!screen(&app).contains("Wireless"));
}

#[test]
fn the_device_list_marks_each_link() {
    let mut app = app_with_ports(studio_link_ports());
    select(&mut app, "X14802");
    assert!(screen(&app).contains("USB + Bluetooth"));
}

#[test]
fn the_device_list_draws_a_schematic() {
    let mut app = app();
    let row = app
        .device_rows()
        .iter()
        .position(|&index| app.catalog.devices[index].class == "X14802")
        .unwrap();
    app.device_cursor = row;
    assert!(screen(&app).contains("Schematic"));
}

#[test]
fn scenes_copy_paste_and_clear() {
    let mut app = app();
    open(&mut app, "X14802");
    let scene = |app: &TestApp, index: usize| {
        app.controller
            .session()
            .document()
            .scene_set
            .as_ref()
            .unwrap()
            .scenes[index]
            .data
            .clone()
    };
    let first = scene(&app, 0);
    app.run(Command::Copy);
    press(&mut app, KeyCode::Char('2'));
    app.run(Command::Paste);
    assert_eq!(scene(&app, 1), first);
    let len = first.len();
    app.run(Command::Clear);
    assert_eq!(scene(&app, 1), vec![0u8; len]);
}

#[test]
fn new_builds_a_clean_factory_document() {
    let mut app = app();
    open(&mut app, "X14802");
    app.run(Command::New);
    assert_eq!(app.screen, Screen::Editor, "{}", app.status);
    assert!(
        app.status.starts_with("Opened nanoKONTROL Studio"),
        "{}",
        app.status
    );
    assert!(!app.view().dirty);
    assert_eq!(app.view().scene_count, 5);
}

#[test]
fn schema_channels_show_one_based_and_store_raw() {
    let mut app = app();
    open(&mut app, "X14800");
    let view = app.view();
    let row = view.params.first().expect("a schema row");
    assert_eq!(row.key, "MidiCh");
    assert_eq!(crate::app::param_display(row), "1");
    assert_eq!(crate::app::param_range(row), "1..16, Global");
    // The first left step wraps to Global and stores the raw byte 16.
    app.param_cursor = 0;
    press(&mut app, KeyCode::Left);
    assert_eq!(app.scene_bytes()[16], 16);
    let view = app.view();
    assert_eq!(crate::app::param_display(&view.params[0]), "Global");
    // One more step wraps back to channel 1 (stored 0).
    press(&mut app, KeyCode::Right);
    let view = app.view();
    assert_eq!(crate::app::param_display(&view.params[0]), "1");
    assert_eq!(app.scene_bytes()[16], 0);
}

#[test]
fn menus_open_with_f10_and_explain_what_is_missing() {
    let mut app = app();
    press(&mut app, KeyCode::F(10));
    assert_eq!(app.modal, Modal::Menu { menu: 0, item: 0 });
    press(&mut app, KeyCode::Right);
    press(&mut app, KeyCode::Enter); // Edit → Undo
    assert_eq!(app.modal, Modal::None);
    assert_eq!(app.status, "Open a device or a file first.");
    open(&mut app, "X14802");
    app.run(Command::Copy);
    assert_eq!(app.status, "Copied scene 1.");
    app.run(Command::About);
    assert!(app.status.starts_with("korg-tui"));
}

#[test]
fn clicking_a_menu_title_and_item_runs_the_command() {
    let mut app = app();
    open(&mut app, "X10820");
    press(&mut app, KeyCode::Right);
    assert!(app.view().dirty);
    render(&mut app);
    let edit = app.hits.menu_titles[1];
    click(&mut app, edit.x + 1, edit.y);
    assert_eq!(app.modal, Modal::Menu { menu: 1, item: 0 });
    render(&mut app);
    let undo = app.hits.menu_items[0];
    click(&mut app, undo.x + 1, undo.y);
    assert_eq!(app.modal, Modal::None);
    assert!(!app.view().dirty);
}

#[test]
fn clicking_the_jog_wheel_on_the_schematic_shows_its_fields() {
    let mut app = app();
    open(&mut app, "X14802");
    render(&mut app);
    let (column, row) = panel_cell(&app, 82.0, 245.0);
    click(&mut app, column, row);
    assert_eq!(app.selected_binding(), Binding::Jog);
    assert_eq!(app.focus, Focus::Panel);
    let text = render(&mut app);
    assert!(text.contains("Jog Wheel · Jog Wheel"), "{text}");
    assert!(text.contains("Type"), "{text}");
    assert!(text.contains("MIDI Channel"), "{text}");
}

#[test]
fn clicking_a_nanokontrol_control_shows_its_fields() {
    let mut app = app();
    open(&mut app, "X14802");
    render(&mut app);
    let (column, row) = panel_cell(&app, 180.0, 160.0); // Knob 1
    click(&mut app, column, row);
    assert_eq!(app.focus, Focus::Panel);
    assert!(app.status.contains("Knob 1"), "{}", app.status);
    let text = render(&mut app);
    assert!(!text.contains("are not decoded yet"), "{text}");
    assert!(text.contains("Knob/Slider Enable"), "{text}");
    assert!(text.contains("1..16, Global"), "{text}");
    // Arrows walk the panel from there.
    press(&mut app, KeyCode::Right);
    assert!(app.status.starts_with("Knob 2"), "{}", app.status);
}

#[test]
fn clicking_a_scene_tab_selects_it() {
    let mut app = app();
    open(&mut app, "X14802");
    render(&mut app);
    let tab = app.hits.tabs[3];
    click(&mut app, tab.x + 1, tab.y);
    assert_eq!(app.view().selected_scene, 3);
    mouse(&mut app, MouseEventKind::ScrollUp, tab.x + 1, tab.y);
    assert_eq!(app.view().selected_scene, 2);
}

#[test]
fn double_click_opens_a_device() {
    let mut app = app();
    render(&mut app);
    let (rows, _) = app.hits.devices.unwrap();
    let row = app
        .device_rows()
        .iter()
        .position(|&index| app.catalog.devices[index].class == "X10820")
        .unwrap() as u16;
    click(&mut app, rows.x + 2, rows.y + row);
    assert_eq!(app.screen, Screen::Devices);
    click(&mut app, rows.x + 2, rows.y + row);
    assert_eq!(app.screen, Screen::Editor);
    assert_eq!(app.class.as_deref(), Some("X10820"));
}

#[test]
fn parameter_rows_follow_the_selected_control() {
    let mut app = app();
    open(&mut app, "X10830");
    render(&mut app);
    // Pad 10: second row, second column (element 9).
    let (column, row) = panel_cell(&app, 280.0, 132.0);
    click(&mut app, column, row);
    assert_eq!(app.selected_binding(), Binding::Element(9));
    // Only the selected pad's rows, as the GUI shows them.
    let view = app.view();
    assert!(!view.params.is_empty());
    assert!(view.params.iter().all(|param| param.element == 9));
    // The cursor stays on this control's rows.
    press(&mut app, KeyCode::Tab); // panel → list
    press(&mut app, KeyCode::Tab); // list → params
    for _ in 0..view.params.len() + 2 {
        press(&mut app, KeyCode::Down);
    }
    assert_eq!(app.selected_binding(), Binding::Element(9));
    assert!(app.param_cursor < view.params.len());
}

#[test]
fn clicking_a_selected_value_steps_it() {
    let mut app = app();
    open(&mut app, "X10820");
    render(&mut app);
    let (rows, offset, value) = app.hits.params.unwrap();
    assert_eq!(offset, 0);
    let before = app.view().params[0].value;
    click(&mut app, value.1 - 1, rows.y); // right half: +1
    assert_ne!(app.view().params[0].value, before);
    click(&mut app, value.0, rows.y); // left half: back
    assert_eq!(app.view().params[0].value, before);
}

#[test]
fn the_control_list_starts_folded_into_groups() {
    let mut app = app();
    open(&mut app, "X14802");
    assert_eq!(tree_rows(&app).len(), 9);
    let text = screen(&app);
    assert!(text.contains("▸ Knob (8)"), "{text}");
}

#[test]
fn selecting_a_control_opens_its_group() {
    let mut app = app();
    open(&mut app, "X14802");
    let index = knob(&app, "Knob 3");
    app.select_control(index);
    assert_eq!(app.open_group.as_deref(), Some("Knob"));
    let rows = tree_rows(&app);
    let header = rows
        .iter()
        .position(|row| matches!(row, TreeRow::Group { name, .. } if *name == "Knob"))
        .unwrap();
    assert_eq!(
        rows[header],
        TreeRow::Group {
            name: "Knob".into(),
            count: 8,
            open: true,
            selected: true
        }
    );
    assert_eq!(
        rows[header + 3],
        TreeRow::Control {
            index,
            label: "3".into(),
            nested: true,
            selected: true
        }
    );
    let text = screen(&app);
    assert!(text.contains("▾ Knob (8)"), "{text}");
}

#[test]
fn a_header_click_toggles_the_group() {
    let mut app = app();
    open(&mut app, "X14802");
    let header = tree_rows(&app)
        .iter()
        .position(|row| matches!(row, TreeRow::Group { name, .. } if *name == "Knob"))
        .unwrap() as u16;
    render(&mut app);
    let (area, offset, _) = app.hits.tree.expect("list drawn");
    assert_eq!(offset, 0);
    click(&mut app, area.x + 1, area.y + header);
    assert_eq!(app.focus, Focus::List);
    assert_eq!(app.open_group.as_deref(), Some("Knob"));
    render(&mut app);
    let (area, offset, _) = app.hits.tree.expect("list drawn");
    assert_eq!(offset, 0);
    click(&mut app, area.x + 1, area.y + header);
    assert_eq!(app.open_group, None);
}

#[test]
fn enter_toggles_the_header_and_the_arrows_select_members() {
    let mut app = app();
    open(&mut app, "X14802");
    let header = tree_rows(&app)
        .iter()
        .position(|row| matches!(row, TreeRow::Group { name, .. } if *name == "Knob"))
        .unwrap();
    press(&mut app, KeyCode::BackTab); // params → list
    assert_eq!(app.focus, Focus::List);
    assert_eq!(app.list_cursor, 0);
    for _ in 0..header {
        press(&mut app, KeyCode::Down);
    }
    assert_eq!(app.list_cursor, header);
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.open_group.as_deref(), Some("Knob"));
    press(&mut app, KeyCode::Down); // onto the chip line; the first chip wins
    assert_eq!(app.control, Some(knob(&app, "Knob 1")));
    press(&mut app, KeyCode::Right);
    assert_eq!(app.control, Some(knob(&app, "Knob 2")));
    press(&mut app, KeyCode::Left);
    assert_eq!(app.control, Some(knob(&app, "Knob 1")));
    press(&mut app, KeyCode::Left); // left of the first chip closes the group
    assert_eq!(app.open_group, None);
    press(&mut app, KeyCode::Right); // → on the header opens it
    assert_eq!(app.open_group.as_deref(), Some("Knob"));
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.open_group, None);
}

#[test]
fn an_open_group_shows_numbered_chips_only() {
    let mut app = app();
    open(&mut app, "X14802");
    app.open_group = Some("Track Mute".into());
    let text = screen(&app);
    assert!(text.contains("▾ Track Mute (8)"), "{text}");
    assert!(text.contains("  1 2 3 4 5 6 7 8"), "{text}");
    // The info lines moved to the status line under the schematic.
    assert!(!text.contains("Group  "), "{text}");
    assert!(!text.contains("Parameters for this control"), "{text}");
    // The chips carry the colour of their MIDI message kind.
    render(&mut app);
    let &(rect, _) = app.hits.chips.first().expect("chips drawn");
    let mut terminal = Terminal::new(TestBackend::new(140, 40)).unwrap();
    terminal
        .draw(|frame| {
            crate::ui::draw(frame, &app);
        })
        .unwrap();
    assert_eq!(
        terminal.backend().buffer()[(rect.x, rect.y)].fg,
        Color::Rgb(0x7f, 0xb8, 0xff)
    );
}

#[test]
fn the_status_line_describes_the_selected_control() {
    let mut app = app();
    open(&mut app, "X14802");
    app.select_control(knob(&app, "Mute 1"));
    let text = screen(&app);
    assert!(text.contains("Mute 1 · Control Change 0 · ch 1"), "{text}");
    assert!(!text.contains("Click a control on the device"), "{text}");
}

#[test]
fn clicking_a_chip_selects_its_control() {
    let mut app = app();
    open(&mut app, "X14802");
    app.open_group = Some("Track Mute".into());
    render(&mut app);
    let &(rect, index) = app.hits.chips.first().expect("chips drawn");
    click(&mut app, rect.x, rect.y);
    assert_eq!(app.control, Some(index));
    assert_eq!(app.focus, Focus::List);
    assert_eq!(index, knob(&app, "Mute 1"));
}

/// The monitor opens links on a worker thread; tick until it reports `link`.
fn wait_for_monitor_link(app: &mut TestApp, link: &str) {
    for _ in 0..2_000 {
        app.tick();
        if app
            .monitor
            .ports
            .as_ref()
            .is_some_and(|(input, _)| input == link)
        {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    panic!("the monitor did not open {link}: {:?}", app.monitor.ports);
}

#[test]
fn the_monitor_follows_the_announced_scene() {
    let mut app = app_with_announcement(studio_link_ports());
    open(&mut app, "X14802");
    // Connect on the chosen link; the worker then reads the announcement.
    for _ in 0..2_000 {
        app.tick();
        if app.monitor.active == Some(2) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert_eq!(app.monitor.active, Some(2));
    assert_eq!(app.status, "The device switched to scene 3.");
    let text = screen(&app);
    assert!(text.contains("3 Scene 3 ●"), "{text}");
}

#[test]
fn the_monitor_follows_the_chosen_link() {
    let mut app = app_with_ports(studio_link_ports());
    open(&mut app, "X14802");
    wait_for_monitor_link(&mut app, "nanoKONTROL Studio CTRL");
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Char('l'));
    wait_for_monitor_link(&mut app, "nanoKONTROL Studio Bluetooth");
}

#[test]
fn a_successful_job_restores_the_scene_it_found() {
    let mut app = app_with_ports(studio_link_ports());
    open(&mut app, "X14802");
    assert_eq!(app.monitor.active, None);
    press(&mut app, KeyCode::Char('r'));
    let job = app.take_job().expect("job queued");
    assert!(matches!(job.job, DeviceJob::ReceiveSceneSet));
    assert_eq!(job.active, None);
    let scene = app.scene_bytes().to_vec();
    app.on_job(JobMessage::Done(Ok(JobOutput::SceneSet {
        key: FileModelKey::new("1482").unwrap(),
        scenes: vec![scene; 5],
    })));
    assert_eq!(app.monitor.active, Some(0));
}

#[test]
fn the_control_tree_sits_left_of_the_parameters() {
    let mut app = app();
    open(&mut app, "X14802");
    app.select_control(knob(&app, "Knob 3"));
    let text = screen(&app);
    let line = text
        .lines()
        .find(|line| line.contains(" Controls ") && line.contains(" Parameters "))
        .expect("tree and parameters share a row");
    assert!(
        line.find(" Controls ").unwrap() < line.find(" Parameters ").unwrap(),
        "{line}"
    );
}

#[test]
fn studio_fields_replace_the_schema_rows() {
    let mut app = app();
    open(&mut app, "X14802");
    app.select_control(knob(&app, "Knob 3"));
    let fields = app.control_fields().expect("studio fields");
    let labels: Vec<&str> = fields.iter().map(|field| field.label).collect();
    assert_eq!(
        labels,
        [
            "MIDI Channel",
            "Knob/Slider Enable",
            "CC Number",
            "Left/Lower Value",
            "Right/Upper Value"
        ]
    );
    let text = screen(&app);
    assert!(text.contains("CC Number"), "{text}");
    assert!(!text.contains("OnOff"), "{text}");
}

#[test]
fn studio_buttons_have_six_fields() {
    let mut app = app();
    open(&mut app, "X14802");
    app.select_control(knob(&app, "Mute 6"));
    let fields = app.control_fields().expect("studio fields");
    assert_eq!(fields.len(), 6);
    assert_eq!(fields[5].label, "Button Behavior");
    assert_eq!(fields[5].offset, 253);
    assert_eq!(app.param_rows(), 6);
}

#[test]
fn editing_button_behavior_writes_the_scene_byte() {
    let mut app = app();
    open(&mut app, "X14802");
    app.select_control(knob(&app, "Mute 6"));
    // Button Behavior is the last row.
    app.param_cursor = 5;
    let before = app.scene_bytes()[253];
    press(&mut app, KeyCode::Right);
    let after = app.scene_bytes()[253];
    assert_ne!(before, after);
    assert!(after <= 1, "stored choice {after}");
    press(&mut app, KeyCode::Char('u'));
    assert_eq!(app.scene_bytes()[253], before);
}

#[test]
fn the_editor_menu_switches_the_link() {
    let mut app = app_with_ports(studio_link_ports());
    open(&mut app, "X14802");
    assert_eq!(app.screen, Screen::Editor, "{}", app.status);
    app.run(Command::Link);
    assert_eq!(app.link, Some(LinkKind::Bluetooth));
    assert_eq!(app.status, "Link: Bluetooth");
    app.run(Command::Link);
    assert_eq!(app.link, Some(LinkKind::Usb));
}

#[test]
fn search_filters_the_device_list() {
    let mut app = app_with_ports(studio_ports());
    let all = app.device_rows().len();
    app.search = "studio".into();
    let rows = app.device_rows();
    assert!(!rows.is_empty());
    assert!(rows
        .iter()
        .any(|&index| app.catalog.devices[index].class == "X14802"));
    assert!(rows.len() < all);
    app.search = "no-such-device".into();
    assert!(app.device_rows().is_empty());
    app.search = "nanoKONTROL".into();
    press(&mut app, KeyCode::Esc);
    assert!(app.search.is_empty(), "Esc clears the filter");
    assert_eq!(app.device_rows().len(), all);
}

#[test]
fn the_search_prompt_sets_the_filter() {
    let mut app = app();
    press(&mut app, KeyCode::Char('/'));
    keys(&mut app, "keystage");
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.search, "keystage");
    let rows = app.device_rows();
    assert!(!rows.is_empty());
    for &index in &rows {
        let device = &app.catalog.devices[index];
        assert!(
            device.device.to_lowercase().contains("keystage")
                || device.class.to_lowercase().contains("keystage"),
            "{} does not match",
            device.device
        );
    }
    assert!(screen(&app).contains("search \"keystage\""));
}

#[test]
fn browse_lists_the_device_files() {
    let mut app = app();
    open(&mut app, "X20870");
    app.run(Command::Browse);
    assert_eq!(app.screen, Screen::Browse);
    let files = app.browse_files();
    assert_eq!(files.len(), 2, "{files:?}");
    assert!(files
        .iter()
        .any(|(label, _)| label == "Keystage Scene Set.keystage_set"));
    let frame = screen(&app);
    assert!(frame.contains("Keystage Scene Set.keystage_set"), "{frame}");
    press(&mut app, KeyCode::Down);
    assert_eq!(app.browse_cursor, 1);
    press(&mut app, KeyCode::Up);
    assert_eq!(app.browse_cursor, 0);
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.screen, Screen::Editor);
}

#[test]
fn cancelling_a_job_drops_its_result() {
    let mut app = app_with_ports(studio_ports());
    press(&mut app, KeyCode::Char('c'));
    press(&mut app, KeyCode::Char('r'));
    app.take_job().expect("job");
    assert!(app.busy.is_some());
    press(&mut app, KeyCode::Char('x'));
    assert!(app.cancel_requested);
    assert!(app.status.contains("Cancel requested"));
    // The worker cannot stop mid-request; its late result must not apply.
    app.on_job(JobMessage::Done(Ok(JobOutput::GlobalWritten)));
    assert!(app.busy.is_none());
    assert!(!app.cancel_requested);
    assert_eq!(app.status, "Cancelled. The job result was dropped.");
    assert_eq!(app.screen, Screen::Devices);
}

#[test]
fn cancelling_without_a_job_says_so() {
    let mut app = app();
    app.run(Command::CancelTransfer);
    assert_eq!(app.status, "No device job is running.");
}

#[test]
fn the_editor_shows_control_bindings_on_request() {
    let mut app = app();
    open(&mut app, "X14802");
    app.control = Some(0);
    assert!(!app.show_bindings);
    press(&mut app, KeyCode::Char('v'));
    assert!(app.show_bindings);
    // The band lists the open group, so the line stays short.
    app.open_group = Some("Knob".into());
    let frame = screen(&app);
    assert!(frame.contains("Knob 1"), "{frame}");
    press(&mut app, KeyCode::Char('v'));
    assert!(!app.show_bindings);
}

#[test]
fn esc_closes_a_prompt_without_applying_it() {
    let mut app = app();
    press(&mut app, KeyCode::Char('/'));
    keys(&mut app, "key");
    assert!(matches!(app.modal, Modal::Prompt { .. }));
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.modal, Modal::None);
    assert!(app.search.is_empty(), "Esc must not apply the text");
    assert_eq!(app.status, "Cancelled.");
}

#[test]
fn received_global_data_opens_the_global_page_beside_the_scenes() {
    let mut app = app_with_ports(studio_ports());
    open(&mut app, "X14802");
    let scene_before = app.scene_bytes().to_vec();
    app.run(Command::ReceiveGlobal);
    app.take_job().expect("job");
    // The device's global block received on 2026-10-28, fields only.
    let mut data = vec![0xffu8; 128];
    data[..4].copy_from_slice(&[0, 0, 1, 0]);
    data[8..34].copy_from_slice(b"nanoKONTROL Studio        ");
    app.on_job(JobMessage::Done(Ok(JobOutput::Global {
        key: FileModelKey::new("1482").unwrap(),
        data,
    })));
    assert!(app.global_page);
    let view = app.view();
    assert_eq!(view.params[0].label, "Wireless Device Name");
    assert_eq!(view.params[0].display, "nanoKONTROL Studio");
    let mode = view
        .params
        .iter()
        .position(|row| row.label == "Controller Mode")
        .unwrap();
    type_value(&mut app, mode, "Live");
    type_value(&mut app, 0, "Studio B");
    let global = app
        .controller
        .session()
        .document()
        .global
        .clone()
        .unwrap()
        .data;
    assert_eq!(global[1], 3);
    assert_eq!(&global[8..34], b"Studio B                  ");
    assert_eq!(app.scene_bytes(), scene_before, "the scene is untouched");
    // Back on the scene page with no control: the Common fields.
    press(&mut app, KeyCode::Char('e'));
    assert!(!app.global_page);
    let fields = app.control_fields().expect("common fields");
    assert_eq!(fields[0].label, "Scene Name");
    assert_eq!(fields[1].label, "LED Mode");
    assert_eq!(
        app.unavailable(Command::Calibrate),
        Some("Touch-pad calibration is only for the nanoKEY Studio.")
    );
}

fn backend(id: &str, label: &str, problem: Option<&str>) -> korg_midi::BackendInfo {
    korg_midi::BackendInfo {
        id: id.into(),
        label: label.into(),
        problem: problem.map(Into::into),
    }
}

/// ALSA first; the switch goes to a JACK without a server, then back.
fn alsa_and_jack(app: &mut TestApp) {
    let mut on_jack = false;
    app.midi =
        korg_app::MidiBackendSwitch::new(backend("alsa", "ALSA sequencer", None), 2, move || {
            on_jack = !on_jack;
            if on_jack {
                backend(
                    "jack",
                    "JACK",
                    Some("JACK is not available: no JACK server answers"),
                )
            } else {
                backend("alsa", "ALSA sequencer", None)
            }
        });
}

#[test]
fn m_switches_the_midi_backend_and_the_header_shows_it() {
    let mut app = app();
    alsa_and_jack(&mut app);
    assert!(render(&mut app).contains("MIDI: ALSA sequencer"));
    press(&mut app, KeyCode::Char('M'));
    assert_eq!(app.midi.current().id, "jack");
    assert!(
        app.status.contains("no JACK server answers"),
        "{}",
        app.status
    );
    assert!(app.status.contains("switch back"), "{}", app.status);
    let screen = render(&mut app);
    assert!(screen.contains("MIDI: JACK (not available)"), "{screen}");
    // The menu item does the same.
    app.run(Command::MidiBackend);
    assert_eq!(app.midi.current().id, "alsa");
    assert!(render(&mut app).contains("MIDI: ALSA sequencer"));
}

#[test]
fn a_build_with_one_backend_says_so() {
    let mut app = app();
    press(&mut app, KeyCode::Char('M'));
    assert_eq!(app.status, korg_app::midi_backend::ONE_BACKEND);
}

#[test]
fn nanokey2_pages_match_the_gui() {
    let mut app = app();
    open(&mut app, "X10820");
    let labels = |app: &TestApp| -> Vec<&'static str> {
        app.control_fields()
            .expect("nanoKEY2 page")
            .iter()
            .map(|field| field.label)
            .collect()
    };
    app.select_control(knob(&app, "Octave Up"));
    assert_eq!(labels(&app), ["Key Mode MIDI Channel"]);
    app.select_control(knob(&app, "Pitch Up"));
    assert_eq!(labels(&app), ["Pitch Bend Speed"]);
    app.select_control(knob(&app, "Keyboard"));
    assert_eq!(
        labels(&app),
        ["Transpose", "Velocity Curve", "Const Velocity Value"]
    );
    // Transpose is shown -12..12 and stored with a bias of 64.
    type_value(&mut app, 0, "-3");
    assert_eq!(app.scene_bytes()[8], 61);
    assert_eq!(app.control_fields().unwrap()[0].display(), "-3");
    // Const Velocity Value is used only with the Const curve.
    type_value(&mut app, 1, "Normal");
    assert!(!app.control_fields().unwrap()[2].enabled);
    type_value(&mut app, 1, "Const");
    assert!(app.control_fields().unwrap()[2].enabled);
    app.select_control(knob(&app, "Sustain Button"));
    type_value(&mut app, 1, "Toggle");
    assert_eq!(app.scene_bytes()[26], 1);
}

#[test]
fn nanokey_studio_pages_match_the_gui() {
    let mut app = app();
    open(&mut app, "X14800");
    app.select_control(knob(&app, "X-Y Pad"));
    let fields = app.control_fields().expect("nanoKEY Studio page");
    assert_eq!(fields[0].section, Some("Common"));
    assert_eq!(fields[1].section, Some("Touch"));
    assert_eq!(fields[5].section, Some("X-axis"));
    app.select_control(knob(&app, "Pad 2"));
    let fields = app.control_fields().unwrap();
    assert_eq!(fields[0].offset, 40);
    assert_eq!(fields[1].label, "Assign Type");
    assert_eq!(fields[6].label, "Pad Behavior");
    let text = screen(&app);
    assert!(text.contains("Note/CC/Prog Number 1"), "{text}");
}

/// An app whose monitor transport already holds `packets`.
fn app_with_packets(ports: Vec<PortInfo>, packets: Vec<Vec<u8>>) -> TestApp {
    let controller = EditorController::new(MockTransport::new(ports.clone()), ManualClock::new());
    let mut transport = MockTransport::new(ports);
    for packet in packets {
        transport.push_packet(packet);
    }
    let mut app = App::new(
        controller,
        SceneMonitor::new(move || transport),
        Catalog::load().unwrap(),
        PathBuf::from("/nonexistent-products"),
    );
    app.auto_receive = false;
    app.write_preferences = false;
    app.link = None;
    app.tick();
    app
}

#[test]
fn the_midi_monitor_lights_the_control_that_sent() {
    // What Track Select 6 sends in the factory scene 1.
    let mut probe = app();
    open(&mut probe, "X14802");
    let panel = probe.panel().unwrap();
    let select = knob(&probe, "Select 6");
    let sent =
        korg_app::assign::control_message("X14802", &panel.controls[select], probe.scene_bytes())
            .unwrap();
    let channel = sent.channel.filter(|c| *c < 16).unwrap_or(0);
    let packet = vec![0xB0 | channel, sent.number.unwrap(), 0x7F];

    let mut app = app_with_packets(studio_link_ports(), vec![packet]);
    open(&mut app, "X14802");
    press(&mut app, KeyCode::Char('i'));
    assert!(app.monitor_shown);
    let mut lit = Vec::new();
    for _ in 0..2_000 {
        app.tick();
        lit = app.lit_controls();
        if !lit.is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(lit.contains(&select), "lit: {lit:?}");
    let text = screen(&app);
    assert!(
        text.contains("MIDI Monitor · nanoKONTROL Studio CTRL"),
        "{text}"
    );
    assert!(
        text.contains(&format!("CC {} = 127", sent.number.unwrap())),
        "{text}"
    );
}

#[test]
fn revert_puts_back_the_received_scene_set() {
    let mut app = app_with_ports(studio_link_ports());
    open(&mut app, "X14802");
    assert!(!app.view().can_revert);
    let scene = app.scene_bytes().to_vec();
    app.on_job(JobMessage::Done(Ok(JobOutput::SceneSet {
        key: FileModelKey::new("1482").unwrap(),
        scenes: vec![scene.clone(); 5],
    })));
    app.run(Command::RevertReceived);
    assert!(
        app.status.starts_with("Nothing to revert"),
        "{}",
        app.status
    );
    // Rename the scene: the Scene control's first field.
    let index = knob(&app, "Scene");
    app.select_control(index);
    app.focus = Focus::Params;
    type_value(&mut app, 0, "EDITED");
    assert_ne!(app.scene_bytes(), scene.as_slice());
    assert!(app.view().can_revert);
    app.run(Command::RevertReceived);
    assert_eq!(app.scene_bytes(), scene.as_slice(), "{}", app.status);
    // One undo step brings the edit back.
    press(&mut app, KeyCode::Char('u'));
    assert_ne!(app.scene_bytes(), scene.as_slice());
}

#[test]
fn a_verified_write_counts_as_received() {
    let mut app = app_with_ports(studio_link_ports());
    open(&mut app, "X14802");
    let index = knob(&app, "Scene");
    app.select_control(index);
    app.focus = Focus::Params;
    type_value(&mut app, 0, "WRITTEN");
    let written = app.scene_bytes().to_vec();
    app.warn_before_write = false;
    press(&mut app, KeyCode::Char('w'));
    if matches!(app.modal, Modal::Confirm { .. }) {
        press(&mut app, KeyCode::Char('y'));
    }
    let job = app.take_job().expect("write queued");
    assert!(matches!(job.job, DeviceJob::WriteScene { .. }));
    app.on_job(JobMessage::Done(Ok(JobOutput::Written { scenes: 1 })));
    type_value(&mut app, 0, "LATER");
    app.run(Command::RevertReceived);
    assert_eq!(app.scene_bytes(), written.as_slice(), "{}", app.status);
}
