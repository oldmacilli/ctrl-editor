//! The view model every front end draws (docs/MONITOR.md §5): rows after
//! the view filter, folding, chips for clock and active sense, the alert
//! strip, scrolling and selection, details with a hex view.
//!
//! Plain data and [`Command`]s: the Slint component and the ratatui widget
//! map their input to the same commands and draw the same rows, so they
//! cannot drift apart.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt::Write as _;
use std::ops::Range;

use crate::describe::{describe, hex, ChannelMask, Class, Group, Style, TypeMask};
use crate::model::{Annotation, Dir, Domain, Event, Msg, PortIdx, PortKind, SysExEnd};
use crate::store::Store;

/// How the time column is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TimeFormat {
    /// Seconds since the monitor started: `12.345`.
    #[default]
    SinceStart,
    /// Since the row above: `+0.012`.
    Delta,
    /// Wall clock `12:03:44.120`; the front end passes the local offset.
    Wall { utc_offset_min: i32 },
    /// Host time in nanoseconds.
    HostNs,
    /// Link beat where the producer gives one.
    Beat,
}

/// The view filter (§5.6): hides, never deletes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewState {
    pub types: TypeMask,
    pub channels: ChannelMask,
    /// `None` shows every port.
    pub ports: Option<BTreeSet<PortIdx>>,
    pub show_in: bool,
    pub show_out: bool,
    pub show_thru: bool,
    /// Only messages a descriptor or device named.
    pub only_described: bool,
    /// Case-insensitive, over message, value and description.
    pub search: String,
    /// Clock and active sense become chips; runs of one controller one row.
    pub fold: bool,
    pub time: TimeFormat,
    pub style: Style,
}

impl Default for ViewState {
    fn default() -> Self {
        ViewState {
            types: TypeMask::ALL,
            channels: ChannelMask::ALL,
            ports: None,
            show_in: true,
            show_out: true,
            show_thru: true,
            only_described: false,
            search: String::new(),
            fold: true,
            time: TimeFormat::SinceStart,
            style: Style::default(),
        }
    }
}

/// Colour roles; front ends map them to their theme ([`Tone::rgb`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tone {
    Note,
    Control,
    Program,
    Bend,
    Pressure,
    Common,
    Realtime,
    SysEx,
    Invalid,
    Hid,
}

impl Tone {
    fn of(class: Class, msg: &Msg) -> Tone {
        if matches!(msg, Msg::Hid { .. }) {
            return Tone::Hid;
        }
        match class {
            Class::NoteOn | Class::NoteOff => Tone::Note,
            Class::Control => Tone::Control,
            Class::Program => Tone::Program,
            Class::PitchBend => Tone::Bend,
            Class::PolyPressure | Class::ChannelPressure => Tone::Pressure,
            _ => match class.group() {
                Group::Common => Tone::Common,
                Group::Realtime => Tone::Realtime,
                Group::SysEx => Tone::SysEx,
                _ => Tone::Invalid,
            },
        }
    }

    /// The default palette. Note, control and program keep the KORG
    /// editor's green, blue and orange.
    pub fn rgb(self, dark: bool) -> (u8, u8, u8) {
        let (light, dark_) = match self {
            Tone::Note => ((0x1e, 0x8e, 0x3e), (0x5c, 0xd6, 0x7c)),
            Tone::Control => ((0x1a, 0x6f, 0xc4), (0x6a, 0xb0, 0xff)),
            Tone::Program => ((0xc4, 0x6a, 0x00), (0xff, 0xa8, 0x4a)),
            Tone::Bend => ((0x00, 0x8a, 0x8a), (0x4a, 0xd6, 0xd6)),
            Tone::Pressure => ((0x6b, 0x7a, 0x1a), (0xc0, 0xd0, 0x5a)),
            Tone::Common => ((0x7a, 0x3d, 0xc4), (0xb8, 0x8a, 0xff)),
            Tone::Realtime => ((0x70, 0x70, 0x70), (0x9a, 0x9a, 0x9a)),
            Tone::SysEx => ((0x00, 0x7a, 0x6e), (0x3a, 0xc8, 0xb4)),
            Tone::Invalid => ((0xc4, 0x1e, 0x2e), (0xff, 0x6a, 0x6a)),
            Tone::Hid => ((0x8a, 0x5a, 0x2a), (0xd6, 0xa8, 0x7a)),
        };
        if dark {
            dark_
        } else {
            light
        }
    }
}

/// A run of one controller folded into one row.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CcRun {
    key: (PortIdx, Dir, u8, u8),
    first: u8,
    last: u8,
    min: u8,
    max: u8,
    /// The latest values, oldest first, for a spark line.
    recent: VecDeque<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RowData {
    first: u64,
    last: u64,
    count: u32,
    cc: Option<CcRun>,
}

/// Values a spark line keeps.
pub const SPARK: usize = 32;

/// One row as the front ends draw it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// The first event's `seq`: selection, details and expansion use it.
    pub seq: u64,
    /// Events folded into the row (1 for most).
    pub count: u32,
    pub time: String,
    /// `←` from the port, `→` to it, `⇄` thru.
    pub dir: &'static str,
    pub port: String,
    /// `1`..`16`, or empty.
    pub channel: String,
    /// The type: "Note On", "Control", "SysEx".
    pub message: String,
    /// "C4 vel 100", "7 Channel Volume (coarse) 12→98", "Korg, 560 bytes".
    pub value: String,
    /// What a descriptor or the device says: "KORG nanoKONTROL Studio:
    /// scene-write · scene 3".
    pub detail: String,
    pub tone: Tone,
    /// The editor sent it.
    pub editor: bool,
    /// Has details beyond the row: SysEx, invalid bytes, annotations.
    pub expandable: bool,
    pub selected: bool,
    pub expanded: bool,
    /// The folded controller's latest values.
    pub spark: Vec<u8>,
}

/// Folded clock or active sense of one port.
#[derive(Debug, Clone, PartialEq)]
pub struct Chip {
    pub port: String,
    pub dir: Dir,
    pub class: Class,
    pub count: u64,
    /// From the clock's spacing, 24 per quarter note.
    pub bpm: Option<f64>,
    pub last_ns: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AlertKind {
    /// The tap's ring was full.
    Dropped,
    /// Payloads pushed out of the store while their events are kept.
    Evicted,
    SysExCut,
    SysExTimeout,
    SysExTruncated,
    Invalid,
    /// A reply whose `ok-when` failed.
    NotOk,
}

impl AlertKind {
    pub fn title(self) -> &'static str {
        match self {
            AlertKind::Dropped => "Messages dropped (monitor too slow)",
            AlertKind::Evicted => "Old SysEx bytes no longer kept",
            AlertKind::SysExCut => "SysEx cut by another message",
            AlertKind::SysExTimeout => "SysEx without end (timed out)",
            AlertKind::SysExTruncated => "SysEx too long, truncated",
            AlertKind::Invalid => "Bytes that are no message",
            AlertKind::NotOk => "Device reported an error",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alert {
    pub kind: AlertKind,
    pub title: &'static str,
    pub count: u64,
    /// The latest event of this kind, to jump to.
    pub last_seq: Option<u64>,
}

/// One line of the hex view: 16 bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HexLine {
    pub offset: String,
    pub hex: String,
    pub ascii: String,
}

/// Everything known about one event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Details {
    pub title: String,
    pub fields: Vec<(String, String)>,
    /// Each annotation: a heading and its fields.
    pub notes: Vec<(String, Vec<(String, String)>)>,
    pub hex: Vec<HexLine>,
}

/// What a front end can ask of the view. Keys and buttons map to these.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    TogglePause,
    /// Forget everything recorded.
    Clear,
    ToggleFold,
    /// Note On velocity 0 as sent; raw bytes over names.
    ToggleRaw,
    CycleTime,
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    /// Back to the newest row, following again.
    End,
    /// Open or close the selected row's details.
    ToggleExpand,
    ToggleType(Class),
    /// Show one channel, or all.
    Channel(Option<u8>),
    Search(String),
    ToggleOnlyDescribed,
    /// Select the row holding this event.
    Select(u64),
}

/// A chip and the smoothed clock spacing behind its tempo.
struct ChipState {
    chip: Chip,
    avg_ns: Option<u64>,
}

pub struct View {
    state: ViewState,
    rows: VecDeque<RowData>,
    max_rows: usize,
    /// The next store event to look at.
    next: u64,
    paused: bool,
    chips: BTreeMap<(PortIdx, Dir, Class), ChipState>,
    alerts: BTreeMap<AlertKind, (u64, Option<u64>)>,
    follow: bool,
    top: usize,
    page: usize,
    selected: Option<u64>,
    expanded: BTreeSet<u64>,
}

impl View {
    pub fn new(state: ViewState) -> Self {
        View {
            state,
            rows: VecDeque::new(),
            max_rows: 100_000,
            next: 0,
            paused: false,
            chips: BTreeMap::new(),
            alerts: BTreeMap::new(),
            follow: true,
            top: 0,
            page: 20,
            selected: None,
            expanded: BTreeSet::new(),
        }
    }

    pub fn state(&self) -> &ViewState {
        &self.state
    }

    /// A new filter: the rows are built again from what the store keeps.
    pub fn set_state(&mut self, state: ViewState, store: &Store) {
        self.state = state;
        self.rebuild(store);
    }

    fn rebuild(&mut self, store: &Store) {
        self.rows.clear();
        self.chips.clear();
        self.next = store.first_seq();
        let paused = std::mem::replace(&mut self.paused, false);
        self.update(store);
        self.paused = paused;
        if !self.follow {
            self.top = self.top.min(self.rows.len().saturating_sub(1));
        }
    }

    /// Takes in what the store recorded since the last call. While paused
    /// the rows stand still; [`View::pending`] counts what waits.
    pub fn update(&mut self, store: &Store) {
        let first = store.first_seq();
        while self.rows.front().is_some_and(|r| r.first < first) {
            self.rows.pop_front();
            self.top = self.top.saturating_sub(1);
        }
        if self.paused {
            return;
        }
        self.next = self.next.max(first);
        let from = self.next;
        for event in store.since(from) {
            self.take(store, event);
        }
        self.next = store.next_seq();
        while self.rows.len() > self.max_rows {
            self.rows.pop_front();
            self.top = self.top.saturating_sub(1);
        }
    }

    /// Events recorded while paused.
    pub fn pending(&self, store: &Store) -> u64 {
        if self.paused {
            store.next_seq() - self.next.max(store.first_seq())
        } else {
            0
        }
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    pub fn is_following(&self) -> bool {
        self.follow
    }

    fn take(&mut self, store: &Store, e: &Event) {
        let class = Class::of(&e.msg, self.state.style.raw);
        self.note_alerts(store, e);
        if !self.passes(store, e, class) {
            return;
        }
        if self.state.fold && matches!(class, Class::Clock | Class::ActiveSense) {
            self.chip(store, e, class);
            return;
        }
        if self.state.fold {
            if let Msg::Voice { status, d1, d2 } = e.msg {
                if status & 0xF0 == 0xB0 && !e.annotated {
                    let key = (e.port, e.dir, status & 0x0F, d1);
                    if let Some(row) = self.rows.back_mut() {
                        if let Some(run) = row.cc.as_mut().filter(|r| r.key == key) {
                            run.last = d2;
                            run.min = run.min.min(d2);
                            run.max = run.max.max(d2);
                            if run.recent.len() == SPARK {
                                run.recent.pop_front();
                            }
                            run.recent.push_back(d2);
                            row.last = e.seq;
                            row.count += 1;
                            return;
                        }
                    }
                    self.rows.push_back(RowData {
                        first: e.seq,
                        last: e.seq,
                        count: 1,
                        cc: Some(CcRun {
                            key,
                            first: d2,
                            last: d2,
                            min: d2,
                            max: d2,
                            recent: VecDeque::from([d2]),
                        }),
                    });
                    return;
                }
            }
        }
        self.rows.push_back(RowData {
            first: e.seq,
            last: e.seq,
            count: 1,
            cc: None,
        });
    }

    fn passes(&self, store: &Store, e: &Event, class: Class) -> bool {
        let s = &self.state;
        let dir = match e.dir {
            Dir::In => s.show_in,
            Dir::Out => s.show_out,
            Dir::Thru => s.show_thru,
        };
        if !dir || !s.types.contains(class) || !s.channels.passes(&e.msg) {
            return false;
        }
        if s.ports.as_ref().is_some_and(|p| !p.contains(&e.port)) {
            return false;
        }
        let notes = store.notes(e.seq);
        if s.only_described
            && !notes
                .iter()
                .any(|n| matches!(n, Annotation::Described { .. }))
        {
            return false;
        }
        if !s.search.is_empty() {
            let needle = s.search.to_lowercase();
            let d = describe(&e.msg, e.msg.blob().and_then(|b| store.bytes(b)), &s.style);
            let hay = format!("{} {} {}", d.class.name(), d.value, detail(notes)).to_lowercase();
            if !hay.contains(&needle) {
                return false;
            }
        }
        true
    }

    fn chip(&mut self, store: &Store, e: &Event, class: Class) {
        let port = store
            .port_ref(e.port)
            .map(|p| p.name.clone())
            .unwrap_or_default();
        let ChipState { chip, avg_ns } =
            self.chips
                .entry((e.port, e.dir, class))
                .or_insert_with(|| ChipState {
                    chip: Chip {
                        port,
                        dir: e.dir,
                        class,
                        count: 0,
                        bpm: None,
                        last_ns: 0,
                    },
                    avg_ns: None,
                });
        if class == Class::Clock && chip.count > 0 {
            let dt = e.at.host_ns.saturating_sub(chip.last_ns);
            // Smooth over about a quarter note.
            let avg = match *avg_ns {
                None => dt,
                Some(a) => (a * 23 + dt) / 24,
            };
            *avg_ns = Some(avg);
            if avg > 0 {
                chip.bpm = Some(60e9 / (avg as f64 * 24.0));
            }
        }
        chip.count += 1;
        chip.last_ns = e.at.host_ns;
    }

    fn note_alerts(&mut self, store: &Store, e: &Event) {
        let kind = match e.msg {
            Msg::SysEx {
                end: SysExEnd::Cut(_),
                ..
            } => Some(AlertKind::SysExCut),
            Msg::SysEx {
                end: SysExEnd::Timeout,
                ..
            } => Some(AlertKind::SysExTimeout),
            Msg::SysEx {
                end: SysExEnd::Truncated,
                ..
            } => Some(AlertKind::SysExTruncated),
            Msg::Invalid { .. } => Some(AlertKind::Invalid),
            _ => None,
        };
        let not_ok = store.notes(e.seq).iter().any(|n| match n {
            Annotation::Described { fields, .. } => {
                fields.iter().any(|(k, v)| k == "ok" && v == "no")
            }
            _ => false,
        });
        for kind in kind.into_iter().chain(not_ok.then_some(AlertKind::NotOk)) {
            let a = self.alerts.entry(kind).or_insert((0, None));
            a.0 += 1;
            a.1 = Some(e.seq);
        }
    }

    /// The alert strip: exceptions only, with counts.
    pub fn alerts(&self, store: &Store) -> Vec<Alert> {
        let stats = store.stats();
        let mut out = Vec::new();
        for (kind, count) in [
            (AlertKind::Dropped, stats.dropped),
            (AlertKind::Evicted, stats.evicted_payloads),
        ] {
            if count > 0 {
                out.push(Alert {
                    kind,
                    title: kind.title(),
                    count,
                    last_seq: None,
                });
            }
        }
        for (&kind, &(count, last_seq)) in &self.alerts {
            out.push(Alert {
                kind,
                title: kind.title(),
                count,
                last_seq,
            });
        }
        out
    }

    /// Clock and active sense, folded.
    pub fn chips(&self) -> Vec<Chip> {
        self.chips.values().map(|c| c.chip.clone()).collect()
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// The rows a front end with `height` lines shows now, and sets the
    /// page size for paging.
    pub fn window(&mut self, height: usize) -> Range<usize> {
        self.page = height.max(1);
        let len = self.rows.len();
        let start = if self.follow {
            len.saturating_sub(height)
        } else {
            self.top.min(len.saturating_sub(height))
        };
        start..(start + height).min(len)
    }

    /// Rows in `range`, ready to draw.
    pub fn rows(&self, store: &Store, range: Range<usize>) -> Vec<Row> {
        let range = range.start.min(self.rows.len())..range.end.min(self.rows.len());
        let mut out = Vec::with_capacity(range.len());
        let mut prev_ns = range
            .start
            .checked_sub(1)
            .and_then(|i| self.rows.get(i))
            .and_then(|r| store.get(r.first))
            .map(|e| e.at.host_ns);
        for data in self.rows.range(range) {
            let Some(e) = store.get(data.first) else {
                continue;
            };
            out.push(self.row(store, data, e, prev_ns));
            prev_ns = Some(e.at.host_ns);
        }
        out
    }

    fn row(&self, store: &Store, data: &RowData, e: &Event, prev_ns: Option<u64>) -> Row {
        let style = &self.state.style;
        let bytes = e.msg.blob().and_then(|b| store.bytes(b));
        let d = describe(&e.msg, bytes, style);
        let port = store.port_ref(e.port);
        let mut value = d.value;
        let mut spark = Vec::new();
        if let Some(run) = &data.cc {
            if data.count > 1 {
                let v = |n: u8| crate::describe::value_text(n as u32, style.values);
                let name = match crate::describe::controller_name(run.key.3) {
                    Some(n) => format!("{} {n}", v(run.key.3)),
                    None => v(run.key.3),
                };
                value = format!(
                    "{name} {}→{} ({}…{})",
                    v(run.first),
                    v(run.last),
                    v(run.min),
                    v(run.max)
                );
            }
            spark = run.recent.iter().copied().collect();
        }
        if style.raw {
            if let Some(w) = store.wire(&e.msg) {
                value = hex(&w, 24);
            }
        }
        let notes = store.notes(e.seq);
        Row {
            seq: data.first,
            count: data.count,
            time: self.time(e, prev_ns),
            dir: match e.dir {
                Dir::In => "←",
                Dir::Out => "→",
                Dir::Thru => "⇄",
            },
            port: port.map(|p| p.name.clone()).unwrap_or_default(),
            channel: d.channel.map(|c| c.to_string()).unwrap_or_default(),
            message: d.class.name().to_string(),
            value,
            detail: detail(notes),
            tone: Tone::of(d.class, &e.msg),
            editor: port.is_some_and(|p| p.kind == PortKind::Editor),
            expandable: !notes.is_empty() || e.msg.blob().is_some(),
            selected: self.selected == Some(data.first),
            expanded: self.expanded.contains(&data.first),
            spark,
        }
    }

    fn time(&self, e: &Event, prev_ns: Option<u64>) -> String {
        let ns = e.at.host_ns;
        match self.state.time {
            TimeFormat::SinceStart => {
                format!("{}.{:03}", ns / 1_000_000_000, ns / 1_000_000 % 1000)
            }
            TimeFormat::Delta => match prev_ns {
                Some(p) => {
                    let d = ns.saturating_sub(p);
                    format!("+{}.{:03}", d / 1_000_000_000, d / 1_000_000 % 1000)
                }
                None => "—".into(),
            },
            TimeFormat::Wall { utc_offset_min } => {
                let ms = (e.at.wall_ms + utc_offset_min as i64 * 60_000).rem_euclid(86_400_000);
                format!(
                    "{:02}:{:02}:{:02}.{:03}",
                    ms / 3_600_000,
                    ms / 60_000 % 60,
                    ms / 1000 % 60,
                    ms % 1000
                )
            }
            TimeFormat::HostNs => ns.to_string(),
            TimeFormat::Beat => match e.at.domain {
                Domain::LinkBeat(b) => format!("{b:.2}"),
                _ => "—".into(),
            },
        }
    }

    pub fn selected(&self) -> Option<u64> {
        self.selected
    }

    fn selected_index(&self) -> Option<usize> {
        let s = self.selected?;
        self.rows.binary_search_by_key(&s, |r| r.first).ok()
    }

    fn select_index(&mut self, i: usize) {
        if let Some(r) = self.rows.get(i) {
            self.selected = Some(r.first);
            self.follow = i + 1 == self.rows.len();
            if !self.follow {
                // Keep the selection on screen.
                if i < self.top {
                    self.top = i;
                } else if i >= self.top + self.page {
                    self.top = i + 1 - self.page;
                }
            }
        }
    }

    /// Carries out a command. `Clear` empties the store too.
    pub fn apply(&mut self, command: Command, store: &mut Store) {
        let last = self.rows.len().saturating_sub(1);
        match command {
            Command::TogglePause => {
                self.paused = !self.paused;
                if !self.paused {
                    self.update(store);
                }
            }
            Command::Clear => {
                store.clear();
                self.alerts.clear();
                self.selected = None;
                self.expanded.clear();
                self.follow = true;
                self.top = 0;
                self.rebuild(store);
            }
            Command::ToggleFold => {
                let mut s = self.state.clone();
                s.fold = !s.fold;
                self.set_state(s, store);
            }
            Command::ToggleRaw => {
                let mut s = self.state.clone();
                s.style.raw = !s.style.raw;
                self.set_state(s, store);
            }
            Command::CycleTime => {
                self.state.time = match self.state.time {
                    TimeFormat::SinceStart => TimeFormat::Delta,
                    TimeFormat::Delta => TimeFormat::Wall { utc_offset_min: 0 },
                    TimeFormat::Wall { .. } => TimeFormat::HostNs,
                    TimeFormat::HostNs => TimeFormat::Beat,
                    TimeFormat::Beat => TimeFormat::SinceStart,
                };
            }
            Command::Up => {
                let i = self.selected_index().map_or(last, |i| i.saturating_sub(1));
                self.select_index(i);
            }
            Command::Down => {
                let i = self.selected_index().map_or(last, |i| (i + 1).min(last));
                self.select_index(i);
            }
            Command::PageUp => {
                let i = self.selected_index().unwrap_or(last);
                self.select_index(i.saturating_sub(self.page));
            }
            Command::PageDown => {
                let i = self.selected_index().unwrap_or(last);
                self.select_index((i + self.page).min(last));
            }
            Command::Home => {
                self.top = 0;
                self.select_index(0);
            }
            Command::End => {
                self.follow = true;
                self.selected = None;
            }
            Command::ToggleExpand => {
                if let Some(s) = self.selected {
                    if !self.expanded.remove(&s) {
                        self.expanded.insert(s);
                    }
                }
            }
            Command::ToggleType(class) => {
                let mut s = self.state.clone();
                s.types = s.types.with(class, !s.types.contains(class));
                self.set_state(s, store);
            }
            Command::Channel(channel) => {
                let mut s = self.state.clone();
                s.channels = channel.map_or(ChannelMask::ALL, ChannelMask::only);
                self.set_state(s, store);
            }
            Command::Search(text) => {
                let mut s = self.state.clone();
                s.search = text;
                self.set_state(s, store);
            }
            Command::ToggleOnlyDescribed => {
                let mut s = self.state.clone();
                s.only_described = !s.only_described;
                self.set_state(s, store);
            }
            Command::Select(seq) => {
                // The row that holds the event: the last row starting at or before it.
                let i = match self.rows.binary_search_by_key(&seq, |r| r.first) {
                    Ok(i) => Some(i),
                    Err(0) => None,
                    Err(i) => Some(i - 1).filter(|&i| self.rows[i].last >= seq),
                };
                if let Some(i) = i {
                    self.select_index(i);
                }
            }
        }
    }

    /// The details of the event `seq`.
    pub fn details(&self, store: &Store, seq: u64) -> Option<Details> {
        let e = store.get(seq)?;
        let style = &self.state.style;
        let bytes = e.msg.blob().and_then(|b| store.bytes(b));
        let d = describe(&e.msg, bytes, style);
        let port = store.port_ref(e.port);
        let mut fields = vec![
            ("Time".to_string(), self.time(e, None)),
            (
                "Port".to_string(),
                port.map(|p| format!("{} ({:?})", p.name, p.kind))
                    .unwrap_or_default(),
            ),
            (
                "Direction".to_string(),
                match e.dir {
                    Dir::In => "from the port",
                    Dir::Out => "to the port",
                    Dir::Thru => "thru",
                }
                .to_string(),
            ),
            ("Type".to_string(), d.class.name().to_string()),
        ];
        if let Some(c) = d.channel {
            fields.push(("Channel".into(), c.to_string()));
        }
        if !d.value.is_empty() {
            fields.push(("Value".into(), d.value.clone()));
        }
        if let Msg::SysEx { len, end, .. } = e.msg {
            fields.push(("Size".into(), format!("{len} bytes")));
            fields.push((
                "End".into(),
                match end {
                    SysExEnd::Complete => "F7".into(),
                    SysExEnd::Cut(s) => format!("cut by {s:02X}"),
                    SysExEnd::Timeout => "timed out".into(),
                    SysExEnd::Truncated => "truncated".into(),
                },
            ));
            if let Some(m) = bytes.and_then(crate::describe::manufacturer) {
                fields.push(("Manufacturer".into(), m.into()));
            }
        }
        let notes = store
            .notes(seq)
            .iter()
            .map(|n| match n {
                Annotation::Described { name, fields } => (name.clone(), fields.clone()),
                Annotation::Control { label, id } => {
                    ("Control".into(), vec![(label.clone(), id.clone())])
                }
                Annotation::State(s) => ("State".into(), vec![(s.clone(), String::new())]),
                Annotation::Rule { gate, action, rule } => (
                    format!("Gate {gate}"),
                    vec![
                        ("action".into(), action.clone()),
                        ("rule".into(), rule.clone().unwrap_or_default()),
                    ],
                ),
                Annotation::Path {
                    tier,
                    hops,
                    latency_us,
                } => {
                    let mut f = vec![("tier".into(), tier.clone())];
                    if let Some(l) = latency_us {
                        f.push(("latency".into(), format!("{l} µs")));
                    }
                    for h in hops {
                        let dt = h.dt_us.map(|d| format!(" +{d} µs")).unwrap_or_default();
                        f.push((h.label.clone(), format!("{}{dt}", h.location)));
                    }
                    ("Path".into(), f)
                }
                Annotation::Joined { pieces } => {
                    ("Joined".into(), vec![("pieces".into(), pieces.to_string())])
                }
            })
            .collect();
        let wire = store.wire(&e.msg);
        Some(Details {
            title: format!(
                "{} · {}",
                d.class.name(),
                port.map(|p| p.name.as_str()).unwrap_or("")
            ),
            fields,
            notes,
            hex: wire.map(|w| hex_lines(&w)).unwrap_or_default(),
        })
    }
}

/// What the detail column says: the first description, a control, a state.
fn detail(notes: &[Annotation]) -> String {
    let mut s = String::new();
    for n in notes {
        let part = match n {
            Annotation::Described { name, fields } => {
                let shown: Vec<String> = fields
                    .iter()
                    .filter(|(k, _)| k != "also")
                    .take(3)
                    .map(|(k, v)| format!("{k} {v}"))
                    .collect();
                if shown.is_empty() {
                    name.clone()
                } else {
                    format!("{name} · {}", shown.join(", "))
                }
            }
            Annotation::Control { label, .. } => format!("◉ {label}"),
            Annotation::State(state) => state.clone(),
            Annotation::Rule { gate, action, .. } => format!("{gate}: {action}"),
            Annotation::Path {
                tier, latency_us, ..
            } => match latency_us {
                Some(l) => format!("{tier} {l} µs"),
                None => tier.clone(),
            },
            Annotation::Joined { pieces } => format!("joined from {pieces}"),
        };
        if !s.is_empty() {
            s.push_str(" · ");
        }
        s.push_str(&part);
    }
    s
}

/// 16 bytes a line: offset, hex, printable ASCII.
pub fn hex_lines(bytes: &[u8]) -> Vec<HexLine> {
    bytes
        .chunks(16)
        .enumerate()
        .map(|(i, chunk)| {
            let mut h = String::with_capacity(48);
            for (j, b) in chunk.iter().enumerate() {
                if j > 0 {
                    h.push(if j == 8 { '\u{2002}' } else { ' ' });
                }
                let _ = write!(h, "{b:02X}");
            }
            HexLine {
                offset: format!("{:04X}", i * 16),
                hex: h,
                ascii: chunk
                    .iter()
                    .map(|&b| {
                        if (0x20..0x7F).contains(&b) {
                            b as char
                        } else {
                            '·'
                        }
                    })
                    .collect(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{PortRef, Stamp};
    use crate::parser::Parsed;
    use crate::store::StoreConfig;

    struct Fixture {
        store: Store,
        port: PortIdx,
        view: View,
    }

    fn fixture() -> Fixture {
        let mut store = Store::new(StoreConfig::default());
        let port = store.port(PortRef::new(PortKind::Alsa, "a", "nanoKEY2"));
        Fixture {
            store,
            port,
            view: View::new(ViewState::default()),
        }
    }

    fn at(ms: u64) -> Stamp {
        Stamp {
            host_ns: ms * 1_000_000,
            wall_ms: 1_000 * 3600 * 25 + ms as i64,
            ..Stamp::default()
        }
    }

    impl Fixture {
        fn voice(&mut self, ms: u64, status: u8, d1: u8, d2: u8) -> u64 {
            self.store
                .record(
                    self.port,
                    Dir::In,
                    Parsed::Voice {
                        at: at(ms),
                        status,
                        d1,
                        d2,
                    },
                )
                .unwrap()
        }
        fn clock(&mut self, ms: u64) {
            self.store.record(
                self.port,
                Dir::In,
                Parsed::Realtime {
                    at: at(ms),
                    status: 0xF8,
                },
            );
        }
        fn rows(&mut self) -> Vec<Row> {
            self.view.update(&self.store);
            let len = self.view.len();
            self.view.rows(&self.store, 0..len)
        }
        fn texts(&mut self) -> Vec<String> {
            self.rows()
                .iter()
                .map(|r| format!("{} {}", r.message, r.value))
                .collect()
        }
    }

    #[test]
    fn clock_becomes_a_chip_with_its_tempo() {
        let mut f = fixture();
        // 120 BPM: 24 clocks per half second.
        for i in 0..100 {
            f.clock(i * 500 / 24);
        }
        f.voice(3000, 0x90, 60, 100);
        assert_eq!(f.texts(), ["Note On C4 vel 100"]);
        let chips = f.view.chips();
        assert_eq!(chips.len(), 1);
        assert_eq!(chips[0].count, 100);
        let bpm = chips[0].bpm.unwrap();
        assert!((bpm - 120.0).abs() < 2.0, "{bpm}");
        // Without folding, every clock is a row.
        f.view.apply(Command::ToggleFold, &mut f.store);
        assert_eq!(f.rows().len(), 101);
    }

    #[test]
    fn a_run_of_one_controller_is_one_row_until_something_else_comes() {
        let mut f = fixture();
        for (i, v) in [10, 40, 25, 30].into_iter().enumerate() {
            f.voice(i as u64, 0xB0, 7, v);
        }
        f.voice(10, 0x90, 60, 1);
        f.voice(11, 0xB0, 7, 31);
        let rows = f.rows();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].count, 4);
        assert_eq!(rows[0].value, "7 Channel Volume (coarse) 10→30 (10…40)");
        assert_eq!(rows[0].spark, [10, 40, 25, 30]);
        assert_eq!(rows[2].value, "7 Channel Volume (coarse) = 31");
    }

    #[test]
    fn filters_hide_and_bring_back_without_losing_events() {
        let mut f = fixture();
        f.voice(0, 0x90, 60, 1);
        f.voice(1, 0x99, 36, 1);
        f.voice(2, 0xC0, 4, 0);
        assert_eq!(f.rows().len(), 3);
        f.view.apply(Command::Channel(Some(10)), &mut f.store);
        assert_eq!(f.texts(), ["Note On C2 vel 1"]);
        f.view.apply(Command::Channel(None), &mut f.store);
        f.view
            .apply(Command::ToggleType(Class::Program), &mut f.store);
        assert_eq!(f.rows().len(), 2);
        f.view.apply(Command::Search("c4".into()), &mut f.store);
        assert_eq!(f.texts(), ["Note On C4 vel 1"]);
        f.view.apply(Command::Search(String::new()), &mut f.store);
        f.view
            .apply(Command::ToggleType(Class::Program), &mut f.store);
        assert_eq!(f.rows().len(), 3);
    }

    #[test]
    fn pause_freezes_the_rows_and_counts_what_waits() {
        let mut f = fixture();
        f.voice(0, 0x90, 60, 1);
        f.rows();
        f.view.apply(Command::TogglePause, &mut f.store);
        f.voice(1, 0x90, 61, 1);
        f.voice(2, 0x90, 62, 1);
        assert_eq!(f.rows().len(), 1);
        assert_eq!(f.view.pending(&f.store), 2);
        f.view.apply(Command::TogglePause, &mut f.store);
        assert_eq!(f.rows().len(), 3);
        assert_eq!(f.view.pending(&f.store), 0);
    }

    #[test]
    fn alerts_count_exceptions_whatever_the_filter() {
        let mut f = fixture();
        f.view
            .apply(Command::ToggleType(Class::SysEx), &mut f.store);
        let cut = Parsed::SysEx {
            at: at(0),
            bytes: vec![0xF0, 0x42],
            end: SysExEnd::Cut(0x90),
        };
        let seq = f.store.record(f.port, Dir::In, cut).unwrap();
        f.store.record(
            f.port,
            Dir::In,
            Parsed::Invalid {
                at: at(1),
                bytes: vec![1, 2],
            },
        );
        f.store.add_dropped(3);
        f.rows();
        let alerts: Vec<(AlertKind, u64)> = f
            .view
            .alerts(&f.store)
            .iter()
            .map(|a| (a.kind, a.count))
            .collect();
        assert_eq!(
            alerts,
            [
                (AlertKind::Dropped, 3),
                (AlertKind::SysExCut, 1),
                (AlertKind::Invalid, 1)
            ]
        );
        assert_eq!(f.view.alerts(&f.store)[1].last_seq, Some(seq));
    }

    #[test]
    fn selection_scrolls_and_end_follows_again() {
        let mut f = fixture();
        for n in 0..50 {
            f.voice(n, 0x90, n as u8, 1);
        }
        f.rows();
        assert_eq!(f.view.window(10), 40..50);
        f.view.apply(Command::Up, &mut f.store);
        assert_eq!(f.view.selected(), Some(49));
        f.view.apply(Command::Up, &mut f.store);
        assert!(!f.view.is_following());
        f.view.apply(Command::PageUp, &mut f.store);
        assert_eq!(f.view.selected(), Some(38));
        assert_eq!(f.view.window(10), 38..48);
        f.voice(60, 0x90, 1, 1);
        f.rows();
        assert_eq!(f.view.window(10), 38..48, "new rows do not move a reader");
        f.view.apply(Command::End, &mut f.store);
        assert_eq!(f.view.window(10), 41..51);
        assert_eq!(f.view.selected(), None);
    }

    #[test]
    fn selecting_an_event_inside_a_folded_run_selects_its_row() {
        let mut f = fixture();
        f.voice(0, 0x90, 60, 1);
        let first = f.voice(1, 0xB0, 1, 1);
        let inner = f.voice(2, 0xB0, 1, 2);
        f.rows();
        f.view.apply(Command::Select(inner), &mut f.store);
        assert_eq!(f.view.selected(), Some(first));
        f.view.apply(Command::ToggleExpand, &mut f.store);
        assert!(f.rows()[1].expanded);
    }

    #[test]
    fn evicted_events_leave_the_view() {
        let mut store = Store::new(StoreConfig {
            events: 3,
            bytes: 100,
        });
        let port = store.port(PortRef::new(PortKind::Alsa, "a", "A"));
        let mut view = View::new(ViewState::default());
        for n in 0..5 {
            store.record(
                port,
                Dir::In,
                Parsed::Voice {
                    at: at(n),
                    status: 0x90,
                    d1: n as u8,
                    d2: 1,
                },
            );
            view.update(&store);
        }
        assert_eq!(view.len(), 3);
        assert_eq!(view.rows(&store, 0..3)[0].seq, 2);
    }

    #[test]
    fn time_formats() {
        let mut f = fixture();
        f.voice(1500, 0x90, 60, 1);
        f.voice(1512, 0x90, 61, 1);
        assert_eq!(f.rows()[1].time, "1.512");
        f.view.apply(Command::CycleTime, &mut f.store);
        assert_eq!(f.rows()[1].time, "+0.012");
        f.view.set_state(
            ViewState {
                time: TimeFormat::Wall { utc_offset_min: 60 },
                ..ViewState::default()
            },
            &f.store,
        );
        // 25 h + 1.5 s after the epoch, one hour east.
        assert_eq!(f.rows()[0].time, "02:00:01.500");
    }

    #[test]
    fn details_show_fields_and_the_bytes() {
        let mut f = fixture();
        let bytes = vec![
            0xF0, 0x42, 0x40, 0x00, 0x01, 0x37, 0x41, 0x42, 0x43, 0, 0, 0, 0, 0, 0, 0, 0, 0xF7,
        ];
        let seq = f
            .store
            .record(
                f.port,
                Dir::In,
                Parsed::SysEx {
                    at: at(0),
                    bytes,
                    end: SysExEnd::Complete,
                },
            )
            .unwrap();
        f.store.annotate(
            seq,
            Annotation::Described {
                name: "KORG x: scene".into(),
                fields: vec![("scene".into(), "3".into())],
            },
        );
        let row = &f.rows()[0];
        assert_eq!(row.detail, "KORG x: scene · scene 3");
        assert!(row.expandable);
        let d = f.view.details(&f.store, seq).unwrap();
        assert!(d
            .fields
            .contains(&("Manufacturer".to_string(), "Korg".to_string())));
        assert!(d.fields.contains(&("End".to_string(), "F7".to_string())));
        assert_eq!(
            d.notes,
            [(
                "KORG x: scene".to_string(),
                vec![("scene".to_string(), "3".to_string())]
            )]
        );
        assert_eq!(d.hex.len(), 2);
        assert_eq!(d.hex[0].offset, "0000");
        assert_eq!(d.hex[0].ascii, "·B@··7ABC·······");
        assert_eq!(d.hex[1].hex, "00 F7");
    }

    #[test]
    fn raw_shows_bytes_and_note_on_velocity_zero() {
        let mut f = fixture();
        f.voice(0, 0x90, 60, 0);
        assert_eq!(f.texts(), ["Note Off C4 vel 0"]);
        f.view.apply(Command::ToggleRaw, &mut f.store);
        assert_eq!(f.texts(), ["Note On 90 3C 00"]);
    }
}
