use super::*;
use convert::*;
use ctrl_monitor::{
    tap, Alert, AlertKind, ChannelMask, Chip, Class, Collector, Command, Details, Dir, HexLine,
    ParserConfig, PortKind, PortRef, Row, Stamp, Store, TapConfig, Tone, View, ViewState,
};
use monitor_glue::*;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

/// A store fed through the real tap and collector: `(ms, bytes)` on one
/// input port.
fn store_with(packets: &[(u64, &[u8])]) -> Store {
    let mut store = Store::default();
    let port = store.port(PortRef::new(PortKind::Alsa, "a", "nanoKEY2"));
    let (mut tap, reader) = tap(TapConfig::default());
    let mut collector = Collector::new(ParserConfig::default());
    collector.add(reader);
    let mut last = 0;
    for &(ms, bytes) in packets {
        let at = Stamp {
            host_ns: ms * 1_000_000,
            ..Stamp::default()
        };
        assert!(tap.push(at, port, Dir::In, bytes));
        last = at.host_ns;
    }
    collector.drain(&mut store, last);
    store
}

fn plain_row() -> Row {
    Row {
        seq: 7,
        count: 1,
        time: "1.250".into(),
        dir: "←",
        port: "nanoKEY2".into(),
        channel: "1".into(),
        message: "Note On".into(),
        value: "C4 vel 100".into(),
        detail: String::new(),
        tone: Tone::Note,
        editor: false,
        expandable: false,
        selected: false,
        expanded: false,
        stripe: false,
        spark: Vec::new(),
    }
}

#[test]
fn a_row_carries_its_text_and_its_tone_in_the_scheme() {
    let r = plain_row();
    let light = row(&r, None, false, false);
    let dark = row(&r, None, true, true);
    assert_eq!(light.message, "Note On");
    assert_eq!(light.value, "C4 vel 100");
    assert_eq!(light.time, "1.250");
    assert_eq!(light.dir, "←");
    assert_eq!(light.channel, "1");
    let (r_, g, b) = Tone::Note.rgb(false);
    assert_eq!(light.tone, slint::Color::from_rgb_u8(r_, g, b));
    let (r_, g, b) = Tone::Note.rgb(true);
    assert_eq!(dark.tone, slint::Color::from_rgb_u8(r_, g, b));
    assert_ne!(light.tone, dark.tone);
    assert!(!light.odd && dark.odd);
    assert_eq!(light.count, "");
    assert_eq!(light.spark, "");
    assert_eq!(light.fields.row_count(), 0);
}

#[test]
fn a_folded_run_shows_its_count_and_a_spark_line() {
    let mut r = plain_row();
    r.count = 37;
    r.message = "Control".into();
    r.tone = Tone::Control;
    r.spark = vec![0, 64, 127];
    r.editor = true;
    let s = row(&r, None, false, false);
    assert_eq!(s.count, "×37");
    assert_eq!(s.spark, "M 0.0 19.0 L 50.0 9.9 L 100.0 1.0");
    assert!(s.editor);
    let (r_, g, b) = Tone::Control.rgb(false);
    assert_eq!(s.tone, slint::Color::from_rgb_u8(r_, g, b));
}

#[test]
fn spark_lines_need_two_values() {
    assert_eq!(spark_path(&[]), "");
    assert_eq!(spark_path(&[5]), "");
    assert_eq!(spark_path(&[127, 127]), "M 0.0 1.0 L 100.0 1.0");
    // A range fills the box; one under 16 steps sits centred in 16.
    assert_eq!(spark_path(&[10, 26]), "M 0.0 19.0 L 100.0 1.0");
    assert_eq!(spark_path(&[10, 14]), "M 0.0 12.2 L 100.0 7.8");
}

#[test]
fn an_expanded_row_carries_its_details_and_hex() {
    let mut r = plain_row();
    r.expanded = true;
    r.expandable = true;
    let d = Details {
        title: "SysEx · nanoKEY2".into(),
        fields: vec![("Size".into(), "6 bytes".into())],
        notes: vec![(
            "KORG: scene-write".into(),
            vec![("scene".into(), "3".into())],
        )],
        hex: vec![HexLine {
            offset: "0000".into(),
            hex: "F0 7E 7F 06 01 F7".into(),
            ascii: "·~····".into(),
        }],
    };
    let s = row(&r, Some(&d), false, false);
    assert_eq!(s.title, "SysEx · nanoKEY2");
    let f: Vec<_> = s
        .fields
        .iter()
        .map(|f| (f.key.to_string(), f.value.to_string(), f.heading))
        .collect();
    assert_eq!(
        f,
        [
            ("Size".into(), "6 bytes".into(), false),
            ("KORG: scene-write".into(), String::new(), true),
            ("scene".into(), "3".into(), false),
        ]
    );
    assert_eq!(s.hex.row_count(), 1);
    assert_eq!(s.hex.row_data(0).unwrap().hex, "F0 7E 7F 06 01 F7");
    assert_eq!(detail_lines(&d), 6, "four text lines and the frame");
    // Collapsed rows drop what they carried.
    r.expanded = false;
    assert_eq!(row(&r, Some(&d), false, false).fields.row_count(), 0);
}

#[test]
fn chips_show_tempo_count_and_port() {
    let c = Chip {
        port: "nanoKEY2".into(),
        dir: Dir::In,
        class: Class::Clock,
        count: 384,
        bpm: Some(120.0),
        last_ns: 0,
    };
    let s = chip(&c, false);
    assert_eq!(s.label, "Clock");
    assert_eq!(s.value, "120.0 BPM");
    assert_eq!(s.count, "×384");
    assert_eq!(s.port, "← nanoKEY2");
    let sense = chip(
        &Chip {
            class: Class::ActiveSense,
            bpm: None,
            ..c
        },
        true,
    );
    assert_eq!(sense.label, "Active Sense");
    assert_eq!(sense.value, "");
}

#[test]
fn alerts_are_red_or_amber_and_know_whether_they_jump() {
    let cut = alert(
        &Alert {
            kind: AlertKind::SysExCut,
            title: AlertKind::SysExCut.title(),
            count: 2,
            last_seq: Some(9),
        },
        false,
    );
    assert_eq!(cut.title, "SysEx cut by another message");
    assert_eq!(cut.count, "×2");
    assert!(cut.jump);
    assert_eq!(cut.tone, color(Tone::Invalid, false));
    let evicted = alert(
        &Alert {
            kind: AlertKind::Evicted,
            title: AlertKind::Evicted.title(),
            count: 1,
            last_seq: None,
        },
        false,
    );
    assert!(!evicted.jump);
    assert_eq!(evicted.tone, color(Tone::Program, false));
}

#[test]
fn filter_groups_are_tri_state_over_their_types() {
    let mut state = ViewState::default();
    let g = groups(&state);
    let names: Vec<_> = g.iter().map(|g| g.name.to_string()).collect();
    assert_eq!(names, ["Voice", "System Common", "Real Time", "Other"]);
    assert!(g.iter().all(|g| g.state == 2));
    assert_eq!(g[0].types.row_count(), 7);
    assert_eq!(g[1].types.row_count(), 4);
    assert_eq!(g[2].types.row_count(), 6);
    let other: Vec<_> = g[3].types.iter().map(|t| t.name.to_string()).collect();
    assert_eq!(other, ["SysEx", "Invalid"]);
    assert!(!g[3].tri);
    // Every type once, at its place in Class::ALL.
    let mut idx: Vec<i32> = g
        .iter()
        .flat_map(|g| g.types.iter().map(|t| t.index).collect::<Vec<_>>())
        .collect();
    idx.sort();
    assert_eq!(idx, (0..19).collect::<Vec<_>>());

    state.types = state.types.with(Class::Clock, false);
    let g = groups(&state);
    assert_eq!(g[2].state, 1);
    let clock = g[2].types.iter().find(|t| t.name == "Clock").unwrap();
    assert!(!clock.on);
}

#[test]
fn the_channel_choice_reads_the_mask() {
    assert_eq!(channel_index(ChannelMask::ALL), 0);
    assert_eq!(channel_index(ChannelMask::only(1)), 1);
    assert_eq!(channel_index(ChannelMask::only(16)), 16);
    assert_eq!(
        channel_index(ChannelMask::only(2).with(3, true)),
        0,
        "several channels show as all"
    );
}

#[test]
fn every_pane_event_maps_to_its_command() {
    let state = ViewState::default();
    let rows = [10, 20, 30];
    let alerts = [None, Some(42)];
    let c = |e| commands(e, &state, &rows, &alerts);
    assert_eq!(c(UiEvent::TogglePause), [Command::TogglePause]);
    assert_eq!(c(UiEvent::Clear), [Command::Clear]);
    assert_eq!(c(UiEvent::ToggleFold), [Command::ToggleFold]);
    assert_eq!(c(UiEvent::ToggleRaw), [Command::ToggleRaw]);
    assert_eq!(c(UiEvent::CycleTime), [Command::CycleTime]);
    assert_eq!(
        c(UiEvent::ToggleOnlyDescribed),
        [Command::ToggleOnlyDescribed]
    );
    assert_eq!(
        c(UiEvent::Search("vol".into())),
        [Command::Search("vol".into())]
    );
    assert_eq!(c(UiEvent::Channel(0)), [Command::Channel(None)]);
    assert_eq!(c(UiEvent::Channel(10)), [Command::Channel(Some(10))]);
    assert_eq!(c(UiEvent::Channel(17)), [Command::Channel(None)]);
    assert_eq!(
        c(UiEvent::ToggleType(3)),
        [Command::ToggleType(Class::ALL[3])]
    );
    assert_eq!(c(UiEvent::ToggleType(19)), []);
    assert_eq!(c(UiEvent::Nav(0, 1)), [Command::Up]);
    assert_eq!(c(UiEvent::Nav(1, 3)), vec![Command::Down; 3]);
    assert_eq!(c(UiEvent::Nav(2, 1)), [Command::PageUp]);
    assert_eq!(c(UiEvent::Nav(3, 1)), [Command::PageDown]);
    assert_eq!(c(UiEvent::Nav(4, 1)), [Command::Home]);
    assert_eq!(c(UiEvent::Nav(5, 1)), [Command::End]);
    assert_eq!(c(UiEvent::Nav(9, 1)), []);
    assert_eq!(c(UiEvent::RowClicked(1)), [Command::Select(20)]);
    assert_eq!(c(UiEvent::RowClicked(3)), []);
    assert_eq!(
        c(UiEvent::RowActivated(2)),
        [Command::Select(30), Command::ToggleExpand]
    );
    assert_eq!(c(UiEvent::RowActivated(-1)), [Command::ToggleExpand]);
    assert_eq!(c(UiEvent::AlertClicked(1)), [Command::Select(42)]);
    assert_eq!(c(UiEvent::AlertClicked(0)), [], "nothing to jump to");
}

#[test]
fn a_group_toggle_hides_all_or_shows_all_in_one_command() {
    let mut state = ViewState::default();
    let [Command::SetTypes(off)] = commands(UiEvent::ToggleGroup(2), &state, &[], &[])[..] else {
        panic!("one SetTypes");
    };
    let realtime = [
        Class::Clock,
        Class::Start,
        Class::Continue,
        Class::Stop,
        Class::ActiveSense,
        Class::Reset,
    ];
    assert!(realtime.iter().all(|c| !off.contains(*c)));
    assert!(off.contains(Class::NoteOn), "other groups stay");
    state.types = state.types.with(Class::Clock, false);
    let [Command::SetTypes(on)] = commands(UiEvent::ToggleGroup(2), &state, &[], &[])[..] else {
        panic!("one SetTypes");
    };
    assert!(realtime.iter().all(|c| on.contains(*c)));
}

#[test]
fn the_wheel_scrolls_without_selecting() {
    let state = ViewState::default();
    assert_eq!(
        commands(UiEvent::Nav(6, -3), &state, &[], &[]),
        [Command::Scroll(-3)]
    );
    assert_eq!(
        commands(UiEvent::Nav(6, 2), &state, &[], &[]),
        [Command::Scroll(2)]
    );
}

#[test]
fn the_time_format_has_a_label_for_each() {
    let mut v = View::new(ViewState::default());
    let mut store = Store::default();
    let mut seen = Vec::new();
    for _ in 0..5 {
        seen.push(time_label(v.state().time));
        v.apply(Command::CycleTime, &mut store);
    }
    assert_eq!(
        seen,
        ["Since start", "Δ previous", "Wall clock", "Host ns", "Beat"]
    );
}

fn shown(seq: u64, value: &str) -> Shown {
    let mut row = plain_row();
    row.seq = seq;
    row.value = value.into();
    Shown {
        row,
        details: None,
        odd: false,
        dark: false,
    }
}

#[test]
fn rows_are_written_only_when_they_change() {
    let model = VecModel::<MonitorRow>::default();
    let mut mirror = Vec::new();
    let first: Vec<_> = (0..4).map(|s| shown(s, "a")).collect();
    assert_eq!(sync_rows(&model, &mut mirror, first.clone()), 4);
    assert_eq!(sync_rows(&model, &mut mirror, first.clone()), 0);

    // One row changed in place.
    let mut changed = first.clone();
    changed[2] = shown(2, "b");
    assert_eq!(sync_rows(&model, &mut mirror, changed), 1);
    assert_eq!(model.row_data(2).unwrap().value, "b");

    // Following: two rows scrolled off, two arrived.
    let scrolled = vec![shown(2, "b"), shown(3, "a"), shown(4, "c"), shown(5, "d")];
    assert_eq!(sync_rows(&model, &mut mirror, scrolled), 2);
    let values: Vec<_> = model.iter().map(|r| r.value.to_string()).collect();
    assert_eq!(values, ["b", "a", "c", "d"]);

    // Fewer lines.
    assert_eq!(sync_rows(&model, &mut mirror, vec![shown(4, "c")]), 0);
    assert_eq!(model.row_count(), 1);
}

#[test]
fn small_models_are_replaced_only_when_they_differ() {
    let model = VecModel::<MonitorAlert>::default();
    let a = MonitorAlert {
        title: "x".into(),
        ..Default::default()
    };
    assert!(sync_small(&model, vec![a.clone()]));
    assert!(!sync_small(&model, vec![a.clone()]));
    assert!(sync_small(&model, vec![]));
}

#[test]
fn the_view_model_comes_through_from_bytes_to_rows() {
    let mut packets: Vec<(u64, &[u8])> = vec![(0, &[0x90, 60, 100]), (5, &[0x80, 60, 0])];
    for i in 0..48 {
        packets.push((10 + i * 20, &[0xF8]));
    }
    packets.push((1000, &[0xF0, 0x7E, 0x7F, 0x06, 0x01, 0xF7]));
    packets.push((1010, &[0xF0, 0x42, 0x01]));
    packets.push((1011, &[0x90, 64, 90]));
    let store = Arc::new(Mutex::new(store_with(&packets)));
    let mut inner = Inner {
        view: View::new(ViewState::default()),
        store: store.clone(),
        rows: Rc::new(VecModel::default()),
        shown: Vec::new(),
        chips: Rc::new(VecModel::default()),
        alerts: Rc::new(VecModel::default()),
        alert_seqs: Vec::new(),
        groups: Rc::new(VecModel::default()),
        groups_for: None,
    };
    let s = inner.refresh(30, false);
    let messages: Vec<_> = inner.rows.iter().map(|r| r.message.to_string()).collect();
    assert_eq!(
        messages,
        ["Note On", "Note Off", "SysEx", "SysEx", "Note On"],
        "clock folds into a chip"
    );
    assert_eq!(inner.chips.row_count(), 1);
    assert_eq!(inner.chips.row_data(0).unwrap().count, "×48");
    let alerts: Vec<_> = inner.alerts.iter().map(|a| a.title.to_string()).collect();
    assert_eq!(alerts, ["SysEx cut by another message"]);
    assert!(s.following && !s.paused && s.fold);
    assert_eq!(s.status, "5 rows · 53 events");
    assert_eq!(inner.groups.row_count(), 4);
    // Stripes alternate, and stay with their rows.
    let odd: Vec<_> = inner.rows.iter().map(|r| r.odd).collect();
    assert_eq!(odd, [false, true, false, true, false]);

    // The alert jumps to the cut SysEx; Enter opens it.
    inner.handle(UiEvent::AlertClicked(0));
    inner.handle(UiEvent::RowActivated(-1));
    inner.refresh(30, false);
    let open: Vec<_> = inner.rows.iter().filter(|r| r.expanded).collect();
    assert_eq!(open.len(), 1);
    assert!(open[0].selected);
    assert_eq!(open[0].message, "SysEx");
    assert!(open[0].hex.row_count() > 0);

    // Pause: new events wait and are counted.
    inner.handle(UiEvent::TogglePause);
    {
        let mut st = store.lock().unwrap();
        let port = ctrl_monitor::PortIdx(0);
        st.record_msg(
            Stamp::default(),
            port,
            Dir::In,
            ctrl_monitor::Msg::Voice {
                status: 0x90,
                d1: 1,
                d2: 1,
            },
        );
    }
    let s = inner.refresh(30, true);
    assert!(s.paused);
    assert_eq!(s.waiting, 1);
    assert_eq!(inner.rows.row_count(), 5);
    let (r, g, b) = Tone::Note.rgb(true);
    assert_eq!(
        inner.rows.row_data(0).unwrap().tone,
        slint::Color::from_rgb_u8(r, g, b),
        "dark scheme recolours"
    );
}

#[test]
fn status_groups_thousands() {
    assert_eq!(
        status(1204, 18377),
        "1\u{202F}204 rows · 18\u{202F}377 events"
    );
    assert_eq!(status(0, 999), "0 rows · 999 events");
}

#[test]
fn expanded_rows_fit_around_the_selection() {
    // Ten one-line rows, the fourth expanded to five lines, eight lines.
    let mut h = vec![1; 10];
    h[3] = 5;
    // Selected: cut below it first, then above.
    assert_eq!(fit(&h, 8, Some(3)), 0..4);
    assert_eq!(fit(&h, 6, Some(3)), 2..4);
    // Following: the newest row stays.
    assert_eq!(fit(&h, 8, Some(9)), 4..10);
    assert_eq!(fit(&h, 20, Some(9)), 0..10);
    // Never fewer than one row, even when it alone is too tall.
    assert_eq!(fit(&[30], 8, Some(0)), 0..1);
    assert_eq!(fit(&[], 8, None), 0..0);
}
