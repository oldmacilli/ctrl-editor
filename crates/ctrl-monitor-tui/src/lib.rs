//! The shared MIDI monitor in a terminal (docs/MONITOR.md §5): a ratatui
//! pane over [`ctrl_monitor::View`] and the key map that turns keys into
//! the same [`Command`]s the Slint component sends.
//!
//! ```ignore
//! let mut pane = MonitorPane::default();
//! terminal.draw(|f| pane.render(f.area(), f.buffer_mut(), &mut view, &store))?;
//! if let Some(input) = key_input(key) { pane.handle(input, &mut view, &mut store) }
//! ```

use ctrl_monitor::{Alert, Chip, Class, Command, Details, Dir, Row, Store, TimeFormat, Tone, View};
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget};

/// What a key asks for: a view command, or something the pane does itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    Command(Command),
    /// Start typing a search; Enter applies it, Esc drops it.
    StartSearch,
    /// Show one channel: the next digits typed choose it.
    StartChannel,
    ToggleHelp,
}

/// The key map. The help text ([`HELP`]) lists the same keys.
pub fn key_input(key: KeyEvent) -> Option<Input> {
    use Command as C;
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    Some(Input::Command(match key.code {
        KeyCode::Char(' ') => C::TogglePause,
        KeyCode::Char('l') if ctrl => C::Clear,
        KeyCode::Char('f') => C::ToggleFold,
        KeyCode::Char('r') => C::ToggleRaw,
        KeyCode::Char('t') => C::CycleTime,
        KeyCode::Char('d') => C::ToggleOnlyDescribed,
        KeyCode::Up | KeyCode::Char('k') => C::Up,
        KeyCode::Down | KeyCode::Char('j') => C::Down,
        KeyCode::PageUp => C::PageUp,
        KeyCode::PageDown => C::PageDown,
        KeyCode::Home | KeyCode::Char('g') => C::Home,
        KeyCode::End | KeyCode::Char('G') => C::End,
        KeyCode::Enter => C::ToggleExpand,
        KeyCode::Char('/') => return Some(Input::StartSearch),
        KeyCode::Char('c') => return Some(Input::StartChannel),
        KeyCode::Char('?') => return Some(Input::ToggleHelp),
        _ => return None,
    }))
}

/// Keys, as the help overlay shows them.
pub const HELP: &[(&str, &str)] = &[
    ("Space", "pause / resume (nothing is lost)"),
    ("↑↓ j k", "select a row"),
    ("PgUp PgDn", "page"),
    ("Home g / End G", "oldest / newest, follow again"),
    ("Enter", "details and bytes of the selected row"),
    ("f", "fold clock, active sense and controller runs"),
    ("r", "raw: bytes, Note On velocity 0 as sent"),
    ("t", "time: since start, delta, clock, host ns, beat"),
    ("d", "only messages a descriptor named"),
    ("/", "search"),
    ("c", "channel: type 1–16, Enter; empty for all"),
    ("Ctrl-L", "clear"),
    ("?", "this help"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
enum Prompt {
    Search(String),
    Channel(String),
}

/// Pane state that is the terminal's own: prompts, help, colours.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitorPane {
    /// Use the dark palette.
    pub dark: bool,
    /// The title above the pane.
    pub title: String,
    prompt: Option<Prompt>,
    help: bool,
}

impl Default for MonitorPane {
    fn default() -> Self {
        MonitorPane {
            dark: true,
            title: "MIDI Monitor".into(),
            prompt: None,
            help: false,
        }
    }
}

fn rgb(tone: Tone, dark: bool) -> Color {
    let (r, g, b) = tone.rgb(dark);
    Color::Rgb(r, g, b)
}

const SPARK_GLYPHS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// The last `width` values as a spark line.
pub fn spark(values: &[u8], width: usize) -> String {
    let start = values.len().saturating_sub(width);
    values[start..]
        .iter()
        .map(|&v| SPARK_GLYPHS[(v as usize * 8 / 128).min(7)])
        .collect()
}

fn time_name(t: TimeFormat) -> &'static str {
    match t {
        TimeFormat::SinceStart => "since start",
        TimeFormat::Delta => "delta",
        TimeFormat::Wall { .. } => "clock",
        TimeFormat::HostNs => "host ns",
        TimeFormat::Beat => "beat",
    }
}

fn fit(s: &str, width: usize) -> String {
    let n = s.chars().count();
    if n <= width {
        format!("{s:<width$}")
    } else if width == 0 {
        String::new()
    } else {
        let mut t: String = s.chars().take(width - 1).collect();
        t.push('…');
        t
    }
}

impl MonitorPane {
    /// True while a prompt takes the keys: send them to [`Self::type_key`].
    pub fn is_typing(&self) -> bool {
        self.prompt.is_some()
    }

    pub fn handle(&mut self, input: Input, view: &mut View, store: &mut Store) {
        match input {
            Input::Command(c) => view.apply(c, store),
            Input::StartSearch => self.prompt = Some(Prompt::Search(view.state().search.clone())),
            Input::StartChannel => self.prompt = Some(Prompt::Channel(String::new())),
            Input::ToggleHelp => self.help = !self.help,
        }
    }

    /// A key while a prompt is open.
    pub fn type_key(&mut self, key: KeyEvent, view: &mut View, store: &mut Store) {
        let Some(prompt) = self.prompt.as_mut() else {
            return;
        };
        let text = match prompt {
            Prompt::Search(t) | Prompt::Channel(t) => t,
        };
        match key.code {
            KeyCode::Esc => self.prompt = None,
            KeyCode::Backspace => {
                text.pop();
            }
            KeyCode::Char(c) => text.push(c),
            KeyCode::Enter => {
                let command = match self.prompt.take() {
                    Some(Prompt::Search(t)) => Command::Search(t),
                    Some(Prompt::Channel(t)) => {
                        Command::Channel(t.trim().parse().ok().filter(|c| (1..=16).contains(c)))
                    }
                    None => return,
                };
                view.apply(command, store);
            }
            _ => {}
        }
    }

    /// Draws the pane into `area`.
    pub fn render(&self, area: Rect, buf: &mut Buffer, view: &mut View, store: &Store) {
        let dim = Style::default().fg(Color::DarkGray);
        let block = Block::default()
            .borders(Borders::ALL)
            .title(Line::from(vec![
                Span::styled(
                    format!(" {} ", self.title),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::styled(self.status(view, store), dim),
            ]));
        let inner = block.inner(area);
        block.render(area, buf);
        if inner.height == 0 {
            return;
        }

        let chips = view.chips();
        let alerts = view.alerts(store);
        let mut constraints = Vec::new();
        if !chips.is_empty() {
            constraints.push(Constraint::Length(1));
        }
        if !alerts.is_empty() {
            constraints.push(Constraint::Length(1));
        }
        constraints.push(Constraint::Length(1)); // header
        constraints.push(Constraint::Min(1)); // rows
        if self.prompt.is_some() {
            constraints.push(Constraint::Length(1));
        }
        let parts = Layout::vertical(constraints).split(inner);
        let mut i = 0;
        if !chips.is_empty() {
            Paragraph::new(self.chip_line(&chips)).render(parts[i], buf);
            i += 1;
        }
        if !alerts.is_empty() {
            Paragraph::new(self.alert_line(&alerts)).render(parts[i], buf);
            i += 1;
        }
        let widths = Columns::for_width(inner.width as usize);
        Paragraph::new(Line::styled(
            widths.header(),
            dim.add_modifier(Modifier::BOLD),
        ))
        .render(parts[i], buf);
        let list = parts[i + 1];
        if let Some(prompt) = &self.prompt {
            let (label, text) = match prompt {
                Prompt::Search(t) => ("search: ", t),
                Prompt::Channel(t) => ("channel (1–16, empty for all): ", t),
            };
            Paragraph::new(Line::from(vec![
                Span::styled(label, Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(format!("{text}▏")),
            ]))
            .render(parts[i + 2], buf);
        }
        if self.help {
            self.render_help(list, buf);
            return;
        }
        self.render_rows(list, buf, view, store, &widths);
    }

    fn status(&self, view: &View, store: &Store) -> String {
        let s = view.state();
        let mut parts = Vec::new();
        if view.is_paused() {
            parts.push(format!("⏸ paused, {} waiting", view.pending(store)));
        } else if view.is_following() {
            parts.push("● live".to_string());
        } else {
            parts.push("● live (scrolled)".to_string());
        }
        parts.push(format!("time {}", time_name(s.time)));
        if s.fold {
            parts.push("fold".into());
        }
        if s.style.raw {
            parts.push("raw".into());
        }
        if s.only_described {
            parts.push("described only".into());
        }
        if !s.search.is_empty() {
            parts.push(format!("“{}”", s.search));
        }
        let hidden = Class::ALL.iter().filter(|c| !s.types.contains(**c)).count();
        if hidden > 0 {
            parts.push(format!("{hidden} types hidden"));
        }
        if s.channels != ctrl_monitor::ChannelMask::ALL {
            parts.push("one channel".into());
        }
        parts.push("? help".into());
        format!(" {} ", parts.join(" · "))
    }

    fn chip_line(&self, chips: &[Chip]) -> Line<'static> {
        let mut spans = Vec::new();
        for c in chips {
            let label = match (c.class, c.bpm) {
                (Class::Clock, Some(bpm)) => format!(" ♩ {bpm:.1} BPM ×{} ", c.count),
                (Class::Clock, None) => format!(" ♩ Clock ×{} ", c.count),
                _ => format!(" ♥ Active Sense ×{} ", c.count),
            };
            let dir = if c.dir == Dir::Out { "→" } else { "←" };
            spans.push(Span::styled(
                format!("{label}{dir} {} ", c.port),
                Style::default()
                    .fg(rgb(Tone::Realtime, self.dark))
                    .add_modifier(Modifier::REVERSED),
            ));
            spans.push(Span::raw(" "));
        }
        Line::from(spans)
    }

    fn alert_line(&self, alerts: &[Alert]) -> Line<'static> {
        let red = rgb(Tone::Invalid, self.dark);
        let mut spans = vec![Span::styled("⚠ ", Style::default().fg(red))];
        for (i, a) in alerts.iter().enumerate() {
            if i > 0 {
                spans.push(Span::raw("  "));
            }
            spans.push(Span::styled(
                format!("{} ×{}", a.title, a.count),
                Style::default().fg(red),
            ));
        }
        Line::from(spans)
    }

    fn render_rows(
        &self,
        area: Rect,
        buf: &mut Buffer,
        view: &mut View,
        store: &Store,
        widths: &Columns,
    ) {
        let height = area.height as usize;
        if view.is_empty() {
            Paragraph::new(Line::styled(
                "Nothing yet: messages appear here as they arrive.",
                Style::default().fg(Color::DarkGray),
            ))
            .render(area, buf);
            return;
        }
        // An expanded row takes lines from the window: ask for fewer rows.
        let mut range = view.window(height);
        let mut rows = view.rows(store, range.clone());
        let mut details: Vec<(u64, Details)> = rows
            .iter()
            .filter(|r| r.expanded)
            .filter_map(|r| view.details(store, r.seq).map(|d| (r.seq, d)))
            .collect();
        let extra: usize = details.iter().map(|(_, d)| detail_lines(d)).sum();
        if extra > 0 && rows.len() + extra > height {
            let fewer = height.saturating_sub(extra).max(1);
            range = if view.is_following() || range.start + fewer >= range.end {
                range.end.saturating_sub(fewer)..range.end
            } else {
                range.start..range.start + fewer
            };
            rows = view.rows(store, range);
            details.retain(|(seq, _)| rows.iter().any(|r| r.seq == *seq));
        }
        let mut lines = Vec::with_capacity(height);
        for row in &rows {
            lines.push(self.row_line(row, widths));
            if let Some((_, d)) = details.iter().find(|(s, _)| *s == row.seq) {
                lines.extend(self.detail_block(d));
            }
        }
        let skip = lines.len().saturating_sub(height);
        Paragraph::new(lines.into_iter().skip(skip).collect::<Vec<_>>()).render(area, buf);
    }

    fn row_line(&self, row: &Row, w: &Columns) -> Line<'static> {
        let tone = Style::default().fg(rgb(row.tone, self.dark));
        let dim = Style::default().fg(Color::DarkGray);
        // The fold count and spark line are kept whole; the value is cut.
        let fold = if row.count > 1 {
            format!(" ×{} {}", row.count, spark(&row.spark, 8))
        } else {
            String::new()
        };
        let value = fit(&row.value, w.value.saturating_sub(fold.chars().count()));
        let mut spans = vec![
            Span::styled(fit(&row.time, w.time), dim),
            Span::raw(" "),
            Span::styled(
                row.dir.to_string(),
                if row.editor {
                    tone.add_modifier(Modifier::BOLD)
                } else {
                    dim
                },
            ),
            Span::raw(" "),
            Span::styled(fit(&row.port, w.port), dim),
            Span::raw(" "),
            Span::styled(fit(&row.channel, 2), dim),
            Span::raw(" "),
            Span::styled(
                fit(&row.message, w.message),
                tone.add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(value, tone),
            Span::styled(fold, dim),
        ];
        if w.detail > 0 {
            spans.push(Span::raw(" "));
            spans.push(Span::raw(fit(&row.detail, w.detail)));
        }
        if row.expandable {
            spans.push(Span::styled(if row.expanded { " ▾" } else { " ▸" }, dim));
        }
        let mut line = Line::from(spans);
        if row.selected {
            line = line.style(Style::default().add_modifier(Modifier::REVERSED));
        } else if row.stripe {
            line = line.style(Style::default().bg(if self.dark {
                Color::Rgb(0x1c, 0x1c, 0x1c)
            } else {
                Color::Rgb(0xf4, 0xf4, 0xf4)
            }));
        }
        line
    }

    fn detail_block(&self, d: &Details) -> Vec<Line<'static>> {
        let dim = Style::default().fg(Color::DarkGray);
        let bold = Style::default().add_modifier(Modifier::BOLD);
        let mut out = Vec::new();
        for (k, v) in &d.fields {
            out.push(Line::from(vec![
                Span::styled(format!("    {k:<13}"), dim),
                Span::raw(v.clone()),
            ]));
        }
        for (heading, fields) in &d.notes {
            out.push(Line::styled(format!("    {heading}"), bold));
            for (k, v) in fields {
                out.push(Line::from(vec![
                    Span::styled(format!("      {k:<11}"), dim),
                    Span::raw(v.clone()),
                ]));
            }
        }
        for h in &d.hex {
            out.push(Line::from(vec![
                Span::styled(format!("    {}  ", h.offset), dim),
                Span::raw(format!("{:<48} ", h.hex)),
                Span::styled(h.ascii.clone(), dim),
            ]));
        }
        out
    }

    fn render_help(&self, area: Rect, buf: &mut Buffer) {
        let lines: Vec<Line> = HELP
            .iter()
            .map(|(k, v)| {
                Line::from(vec![
                    Span::styled(
                        format!(" {k:<16}"),
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(*v),
                ])
            })
            .collect();
        Paragraph::new(lines).render(area, buf);
    }
}

fn detail_lines(d: &Details) -> usize {
    d.fields.len() + d.notes.iter().map(|(_, f)| 1 + f.len()).sum::<usize>() + d.hex.len()
}

/// Column widths for a pane width.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Columns {
    time: usize,
    port: usize,
    message: usize,
    value: usize,
    detail: usize,
}

impl Columns {
    fn for_width(width: usize) -> Columns {
        // time dir port ch message value [detail] marker
        let time = 9;
        let message = 16;
        let fixed = time + 1 + 1 + 1 + 1 + 2 + 1 + message + 1 + 2;
        let rest = width.saturating_sub(fixed);
        let port = (rest / 6).clamp(6, 18);
        let rest = rest.saturating_sub(port + 1);
        // The value first (a folded controller run needs about 44); the
        // description gets what is left once that is 24 or more.
        let (value, detail) = if rest >= 44 + 1 + 24 {
            (44, rest - 45)
        } else {
            (rest, 0)
        };
        Columns {
            time,
            port,
            message,
            value,
            detail,
        }
    }

    fn header(&self) -> String {
        let mut s = format!(
            "{} {} {} {} {} {}",
            fit("time", self.time),
            " ",
            fit("port", self.port),
            fit("ch", 2),
            fit("message", self.message),
            fit("value", self.value)
        );
        if self.detail > 0 {
            s.push(' ');
            s.push_str(&fit("description", self.detail));
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctrl_monitor::{
        Annotation, Parsed, PortKind, PortRef, Stamp, StoreConfig, SysExEnd, ViewState,
    };

    fn at(ms: u64) -> Stamp {
        Stamp {
            host_ns: ms * 1_000_000,
            ..Stamp::default()
        }
    }

    fn screen(pane: &MonitorPane, view: &mut View, store: &Store, w: u16, h: u16) -> String {
        let area = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(area);
        view.update(store);
        pane.render(area, &mut buf, view, store);
        (0..h)
            .map(|y| {
                (0..w)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn setup() -> (Store, View, MonitorPane, ctrl_monitor::PortIdx) {
        let mut store = Store::new(StoreConfig::default());
        let port = store.port(PortRef::new(PortKind::Alsa, "a", "nanoKONTROL Studio"));
        (
            store,
            View::new(ViewState::default()),
            MonitorPane::default(),
            port,
        )
    }

    #[test]
    fn rows_chips_and_alerts_are_drawn() {
        let (mut store, mut view, pane, port) = setup();
        for i in 0..48 {
            store.record(
                port,
                Dir::In,
                Parsed::Realtime {
                    at: at(i * 500 / 24),
                    status: 0xF8,
                },
            );
        }
        store.record(
            port,
            Dir::In,
            Parsed::Voice {
                at: at(2000),
                status: 0x90,
                d1: 60,
                d2: 100,
            },
        );
        for (i, v) in [0u8, 40, 80, 127].iter().enumerate() {
            store.record(
                port,
                Dir::In,
                Parsed::Voice {
                    at: at(2100 + i as u64),
                    status: 0xB0,
                    d1: 7,
                    d2: *v,
                },
            );
        }
        store.record(
            port,
            Dir::In,
            Parsed::SysEx {
                at: at(3000),
                bytes: vec![0xF0, 0x42, 0x01],
                end: SysExEnd::Cut(0x90),
            },
        );
        let s = screen(&pane, &mut view, &store, 120, 12);
        // Whole-millisecond spacing: about 120 BPM.
        assert!(s.contains("120.6 BPM ×48"), "{s}");
        assert!(s.contains("SysEx cut by another message ×1"), "{s}");
        assert!(s.contains("Note On"), "{s}");
        assert!(s.contains("C4 vel 100"), "{s}");
        assert!(s.contains("0→127") && s.contains("×4 ▁▃▆█"), "{s}");
        assert!(s.contains("Korg, 3 bytes, no F7"), "{s}");
        assert!(s.contains("● live"), "{s}");
    }

    #[test]
    fn enter_opens_the_details_with_bytes() {
        let (mut store, mut view, mut pane, port) = setup();
        let seq = store
            .record(
                port,
                Dir::In,
                Parsed::SysEx {
                    at: at(0),
                    bytes: vec![0xF0, 0x7E, 0x7F, 0x06, 0x01, 0xF7],
                    end: SysExEnd::Complete,
                },
            )
            .unwrap();
        store.annotate(
            seq,
            Annotation::Described {
                name: "KORG x: identity".into(),
                fields: vec![],
            },
        );
        view.update(&store);
        for k in [KeyCode::Up, KeyCode::Enter] {
            pane.handle(key_input(key(k)).unwrap(), &mut view, &mut store);
        }
        let s = screen(&pane, &mut view, &store, 120, 16);
        assert!(s.contains("Identity Request, 6 bytes"), "{s}");
        assert!(s.contains("KORG x: identity"), "{s}");
        assert!(s.contains("0000  F0 7E 7F 06 01 F7"), "{s}");
        assert!(s.contains("▾"), "{s}");
    }

    #[test]
    fn prompts_search_and_choose_a_channel() {
        let (mut store, mut view, mut pane, port) = setup();
        store.record(
            port,
            Dir::In,
            Parsed::Voice {
                at: at(0),
                status: 0x90,
                d1: 60,
                d2: 1,
            },
        );
        store.record(
            port,
            Dir::In,
            Parsed::Voice {
                at: at(1),
                status: 0x99,
                d1: 36,
                d2: 1,
            },
        );
        view.update(&store);
        pane.handle(
            key_input(key(KeyCode::Char('c'))).unwrap(),
            &mut view,
            &mut store,
        );
        assert!(pane.is_typing());
        for k in [KeyCode::Char('1'), KeyCode::Char('0'), KeyCode::Enter] {
            pane.type_key(key(k), &mut view, &mut store);
        }
        let s = screen(&pane, &mut view, &store, 100, 8);
        assert!(s.contains("C2 vel 1") && !s.contains("C4 vel 1"), "{s}");
        assert!(s.contains("one channel"), "{s}");
        pane.handle(Input::StartChannel, &mut view, &mut store);
        pane.type_key(key(KeyCode::Enter), &mut view, &mut store);
        pane.handle(Input::StartSearch, &mut view, &mut store);
        for c in "c4".chars() {
            pane.type_key(key(KeyCode::Char(c)), &mut view, &mut store);
        }
        assert!(screen(&pane, &mut view, &store, 100, 8).contains("search: c4▏"));
        pane.type_key(key(KeyCode::Enter), &mut view, &mut store);
        let s = screen(&pane, &mut view, &store, 100, 8);
        assert!(s.contains("C4 vel 1") && !s.contains("C2 vel 1"), "{s}");
    }

    #[test]
    fn pause_shows_what_waits_and_help_lists_the_keys() {
        let (mut store, mut view, mut pane, port) = setup();
        view.update(&store);
        pane.handle(
            key_input(key(KeyCode::Char(' '))).unwrap(),
            &mut view,
            &mut store,
        );
        store.record(
            port,
            Dir::In,
            Parsed::Voice {
                at: at(0),
                status: 0x90,
                d1: 60,
                d2: 1,
            },
        );
        let s = screen(&pane, &mut view, &store, 100, 8);
        assert!(s.contains("paused, 1 waiting"), "{s}");
        assert!(s.contains("Nothing yet"), "{s}");
        pane.handle(
            key_input(key(KeyCode::Char('?'))).unwrap(),
            &mut view,
            &mut store,
        );
        let s = screen(&pane, &mut view, &store, 100, 20);
        assert!(s.contains("pause / resume"), "{s}");
    }

    #[test]
    fn every_help_entry_has_a_key_and_narrow_panes_still_draw() {
        assert_eq!(HELP.len(), 13);
        let (mut store, mut view, pane, port) = setup();
        store.record(
            port,
            Dir::Out,
            Parsed::Voice {
                at: at(0),
                status: 0xC0,
                d1: 4,
                d2: 0,
            },
        );
        let s = screen(&pane, &mut view, &store, 40, 5);
        assert!(s.contains("Program"), "{s}");
    }

    #[test]
    fn spark_uses_the_last_values() {
        assert_eq!(spark(&[0, 127], 8), "▁█");
        assert_eq!(spark(&[0, 0, 0, 64], 2), "▁▅");
    }
}
