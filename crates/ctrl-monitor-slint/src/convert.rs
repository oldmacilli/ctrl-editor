//! From the view model to the Slint structs, and from the pane's callbacks
//! to [`Command`]s. Plain functions, so the tests need no window.

use ctrl_monitor::{
    Alert, AlertKind, ChannelMask, Chip, Class, Command, Details, Dir, Group, Row, TimeFormat,
    Tone, ViewState,
};
use std::ops::Range;

use slint::{Color, ModelRc, SharedString, VecModel};

use crate::{
    MonitorAlert, MonitorChip, MonitorField, MonitorGroup, MonitorHex, MonitorRow, MonitorType,
};

/// A tone in the pane's colour scheme.
pub fn color(tone: Tone, dark: bool) -> Color {
    let (r, g, b) = tone.rgb(dark);
    Color::from_rgb_u8(r, g, b)
}

/// A spark line as SVG path commands in a 100 × 20 box, from the bottom
/// up. It spans the values' range, at least 16 steps of 0..=127, so a
/// small wobble stays small. Empty below two values.
pub fn spark_path(values: &[u8]) -> String {
    if values.len() < 2 {
        return String::new();
    }
    let lo = values.iter().copied().min().unwrap_or(0).min(127) as f32;
    let hi = values.iter().copied().max().unwrap_or(0).min(127) as f32;
    let span = (hi - lo).max(16.0);
    let bottom = ((lo + hi - span) / 2.0).clamp(0.0, 127.0 - span);
    let step = 100.0 / (values.len() - 1) as f32;
    let mut s = String::with_capacity(values.len() * 12);
    for (i, &v) in values.iter().enumerate() {
        let x = i as f32 * step;
        let y = 19.0 - (v.min(127) as f32 - bottom) * 18.0 / span;
        if !s.is_empty() {
            s.push(' ');
        }
        s.push_str(&format!("{} {x:.1} {y:.1}", if i == 0 { "M" } else { "L" }));
    }
    s
}

/// The fields of an expanded row: the event's fields, then each
/// annotation as a heading with its own.
pub fn fields(details: &Details) -> Vec<MonitorField> {
    let field = |k: &str, v: &str, heading| MonitorField {
        key: k.into(),
        value: v.into(),
        heading,
    };
    let mut out: Vec<MonitorField> = details
        .fields
        .iter()
        .map(|(k, v)| field(k, v, false))
        .collect();
    for (name, fs) in &details.notes {
        out.push(field(name, "", true));
        out.extend(fs.iter().map(|(k, v)| field(k, v, false)));
    }
    out
}

/// List lines (rows of the list) an expanded row's details take below it,
/// to keep it on screen. Matches `Details` in ui/monitor.slint: small text
/// lines of about 16 px, 56 px of title and padding, 22 px rows.
pub fn detail_lines(details: &Details) -> usize {
    let lines = details.fields.len()
        + details
            .notes
            .iter()
            .map(|(_, f)| f.len() + 1)
            .sum::<usize>()
        + details.hex.len();
    (lines * 16 + 56).div_ceil(22)
}

/// The rows that fit in `lines` lines, given each row's height in lines:
/// cut from the bottom down to `anchor` (the selected row), then from the
/// top. Without an anchor, from the bottom.
pub fn fit(heights: &[usize], lines: usize, anchor: Option<usize>) -> Range<usize> {
    let mut range = 0..heights.len();
    let mut total: usize = heights.iter().sum();
    let anchor = anchor.unwrap_or(0).min(heights.len().saturating_sub(1));
    while total > lines && range.len() > 1 && range.end - 1 > anchor {
        range.end -= 1;
        total -= heights[range.end];
    }
    while total > lines && range.len() > 1 && range.start < anchor {
        total -= heights[range.start];
        range.start += 1;
    }
    range
}

/// One row as the pane draws it. `odd` stripes it; `details` fill it
/// while it is expanded.
pub fn row(r: &Row, details: Option<&Details>, odd: bool, dark: bool) -> MonitorRow {
    let (title, fields, hex) = match details.filter(|_| r.expanded) {
        Some(d) => (
            d.title.as_str().into(),
            fields(d),
            d.hex
                .iter()
                .map(|h| MonitorHex {
                    offset: h.offset.as_str().into(),
                    hex: h.hex.as_str().into(),
                    ascii: h.ascii.as_str().into(),
                })
                .collect(),
        ),
        None => (SharedString::default(), Vec::new(), Vec::new()),
    };
    MonitorRow {
        time: r.time.as_str().into(),
        dir: r.dir.into(),
        port: r.port.as_str().into(),
        channel: r.channel.as_str().into(),
        message: r.message.as_str().into(),
        value: r.value.as_str().into(),
        detail: r.detail.as_str().into(),
        count: if r.count > 1 {
            format!("×{}", r.count).into()
        } else {
            SharedString::default()
        },
        tone: color(r.tone, dark),
        editor: r.editor,
        expandable: r.expandable,
        selected: r.selected,
        expanded: r.expanded,
        odd,
        spark: spark_path(&r.spark).into(),
        title,
        fields: ModelRc::new(VecModel::from(fields)),
        hex: ModelRc::new(VecModel::from(hex)),
    }
}

fn glyph(dir: Dir) -> &'static str {
    match dir {
        Dir::In => "←",
        Dir::Out => "→",
        Dir::Thru => "⇄",
    }
}

/// A folded clock or active sense: "Clock 120.0 BPM ×384 ← nanoKEY2".
pub fn chip(c: &Chip, dark: bool) -> MonitorChip {
    let tone = if c.class == Class::Clock {
        Tone::Realtime
    } else {
        Tone::Common
    };
    MonitorChip {
        label: c.class.name().into(),
        port: format!("{} {}", glyph(c.dir), c.port).into(),
        value: c
            .bpm
            .map(|b| format!("{b:.1} BPM"))
            .unwrap_or_default()
            .into(),
        count: format!("×{}", c.count).into(),
        tone: color(tone, dark),
    }
}

/// Lost data and damaged messages are red; what is only cut short amber.
pub fn alert(a: &Alert, dark: bool) -> MonitorAlert {
    let tone = match a.kind {
        AlertKind::Evicted | AlertKind::SysExTruncated => Tone::Program,
        _ => Tone::Invalid,
    };
    MonitorAlert {
        title: a.title.into(),
        count: format!("×{}", a.count).into(),
        tone: color(tone, dark),
        jump: a.last_seq.is_some(),
    }
}

/// The filter pane's column a type belongs to: 0 voice, 1 system common,
/// 2 real time, 3 other.
pub fn column(class: Class) -> usize {
    match class.group() {
        Group::Voice => 0,
        Group::Common => 1,
        Group::Realtime => 2,
        Group::SysEx | Group::Invalid => 3,
    }
}

const COLUMNS: [(&str, bool); 4] = [
    ("Voice", true),
    ("System Common", true),
    ("Real Time", true),
    ("Other", false),
];

fn classes_in(col: usize) -> impl Iterator<Item = Class> {
    Class::ALL.into_iter().filter(move |&c| column(c) == col)
}

/// The filter pane: a column per group, its types inside.
pub fn groups(state: &ViewState) -> Vec<MonitorGroup> {
    COLUMNS
        .iter()
        .enumerate()
        .map(|(col, &(name, tri))| {
            let types: Vec<MonitorType> = Class::ALL
                .iter()
                .enumerate()
                .filter(|(_, &c)| column(c) == col)
                .map(|(i, &c)| MonitorType {
                    name: c.name().into(),
                    index: i as i32,
                    on: state.types.contains(c),
                })
                .collect();
            let on = types.iter().filter(|t| t.on).count();
            MonitorGroup {
                name: name.into(),
                tri,
                state: if on == 0 {
                    0
                } else if on == types.len() {
                    2
                } else {
                    1
                },
                types: ModelRc::new(VecModel::from(types)),
            }
        })
        .collect()
}

pub fn time_label(time: TimeFormat) -> &'static str {
    match time {
        TimeFormat::SinceStart => "Since start",
        TimeFormat::Delta => "Δ previous",
        TimeFormat::Wall { .. } => "Wall clock",
        TimeFormat::HostNs => "Host ns",
        TimeFormat::Beat => "Beat",
    }
}

/// The channel choice: 0 all, else the one channel shown. A mask of
/// several channels (set elsewhere) shows as all.
pub fn channel_index(mask: ChannelMask) -> i32 {
    (1..=16u8)
        .find(|&c| mask == ChannelMask::only(c))
        .map_or(0, i32::from)
}

/// What the pane reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiEvent {
    TogglePause,
    Clear,
    ToggleFold,
    ToggleRaw,
    CycleTime,
    Search(String),
    ToggleOnlyDescribed,
    /// 0 all, 1..=16.
    Channel(i32),
    /// An index into `Class::ALL`.
    ToggleType(i32),
    /// A filter column.
    ToggleGroup(i32),
    /// 0 up, 1 down, 2 page up, 3 page down, 4 home, 5 end, with a count;
    /// 6 scroll by the (signed) count of lines, the selection stays.
    Nav(i32, i32),
    /// An index into the rows shown.
    RowClicked(i32),
    /// Double click on a row, or Enter (-1: the selected row).
    RowActivated(i32),
    /// An index into the alerts shown.
    AlertClicked(i32),
}

/// The commands a pane event stands for. `rows` are the `seq`s of the rows
/// shown, `alerts` the alerts' `last_seq`s, in the pane's order.
pub fn commands(
    event: UiEvent,
    state: &ViewState,
    rows: &[u64],
    alerts: &[Option<u64>],
) -> Vec<Command> {
    let row = |i: i32| usize::try_from(i).ok().and_then(|i| rows.get(i)).copied();
    match event {
        UiEvent::TogglePause => vec![Command::TogglePause],
        UiEvent::Clear => vec![Command::Clear],
        UiEvent::ToggleFold => vec![Command::ToggleFold],
        UiEvent::ToggleRaw => vec![Command::ToggleRaw],
        UiEvent::CycleTime => vec![Command::CycleTime],
        UiEvent::Search(s) => vec![Command::Search(s)],
        UiEvent::ToggleOnlyDescribed => vec![Command::ToggleOnlyDescribed],
        UiEvent::Channel(c) => vec![Command::Channel(
            u8::try_from(c).ok().filter(|c| (1..=16).contains(c)),
        )],
        UiEvent::ToggleType(i) => usize::try_from(i)
            .ok()
            .and_then(|i| Class::ALL.get(i))
            .map(|&c| vec![Command::ToggleType(c)])
            .unwrap_or_default(),
        UiEvent::ToggleGroup(g) => {
            let Ok(col) = usize::try_from(g) else {
                return Vec::new();
            };
            // All shown: hide them all. Otherwise show them all. One
            // command, so the view is built once.
            let all = classes_in(col).all(|c| state.types.contains(c));
            let types = classes_in(col).fold(state.types, |t, c| t.with(c, !all));
            vec![Command::SetTypes(types)]
        }
        UiEvent::Nav(6, n) => vec![Command::Scroll(n as i64)],
        UiEvent::Nav(code, n) => {
            let c = match code {
                0 => Command::Up,
                1 => Command::Down,
                2 => Command::PageUp,
                3 => Command::PageDown,
                4 => Command::Home,
                5 => Command::End,
                _ => return Vec::new(),
            };
            vec![c; n.clamp(1, 50) as usize]
        }
        UiEvent::RowClicked(i) => row(i).map(Command::Select).into_iter().collect(),
        UiEvent::RowActivated(-1) => vec![Command::ToggleExpand],
        UiEvent::RowActivated(i) => row(i)
            .map(|s| vec![Command::Select(s), Command::ToggleExpand])
            .unwrap_or_default(),
        UiEvent::AlertClicked(i) => usize::try_from(i)
            .ok()
            .and_then(|i| alerts.get(i).copied().flatten())
            .map(Command::Select)
            .into_iter()
            .collect(),
    }
}
