//! The Slint front end of the shared MIDI monitor (docs/MONITOR.md §5).
//!
//! `ui/monitor.slint` holds the pane (`MonitorPane`) and a window around it
//! (`MonitorWindow`); both read the `MonitorBridge` global. A
//! [`MonitorController`] owns a [`View`] over a shared [`Store`], refreshes
//! the bridge on a timer and turns the bridge's callbacks into
//! [`Command`](ctrl_monitor::Command)s. The conversions live in [`convert`].
//!
//! ```no_run
//! use std::sync::{Arc, Mutex};
//! use ctrl_monitor::{Store, ViewState};
//! use ctrl_monitor_slint::{MonitorController, MonitorWindow};
//! use slint::ComponentHandle;
//!
//! let store = Arc::new(Mutex::new(Store::default()));
//! let window = MonitorWindow::new().unwrap();
//! let _monitor = MonitorController::new(&window, store, ViewState::default());
//! window.run().unwrap();
//! ```

use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use ctrl_monitor::{Details, Row, Store, View, ViewState};
use slint::{ComponentHandle, Global, Model, ModelRc, Timer, TimerMode, VecModel};

pub mod convert;

use convert::UiEvent;

slint::include_modules!();

/// How often the pane takes in new events.
pub const REFRESH: Duration = Duration::from_millis(50);

/// The monospace family for time and hex.
pub fn mono_family() -> &'static str {
    if cfg!(target_os = "macos") {
        "Menlo"
    } else if cfg!(target_os = "windows") {
        "Consolas"
    } else {
        "DejaVu Sans Mono"
    }
}

/// What a shown row was made from; a row is written again only when this
/// changes.
#[derive(Debug, Clone, PartialEq)]
struct Shown {
    row: Row,
    details: Option<Details>,
    odd: bool,
    dark: bool,
}

/// Brings `model` (mirrored by `shown`) to `new`, writing only what
/// changed. Rows that scrolled off the top are removed, not rewritten.
/// Returns the rows written or inserted.
fn sync_rows(model: &VecModel<MonitorRow>, shown: &mut Vec<Shown>, new: Vec<Shown>) -> usize {
    // The window moved down: drop the rows above the first new one.
    if let Some(first) = new.first() {
        if let Some(k) = shown.iter().position(|s| s.row.seq == first.row.seq) {
            for _ in 0..k {
                model.remove(0);
            }
            shown.drain(..k);
        }
    }
    let mut writes = 0;
    let common = shown.len().min(new.len());
    for (i, n) in new.iter().enumerate().take(common) {
        if shown[i] != *n {
            model.set_row_data(i, convert::row(&n.row, n.details.as_ref(), n.odd, n.dark));
            writes += 1;
        }
    }
    while model.row_count() > new.len() {
        model.remove(model.row_count() - 1);
    }
    for n in &new[common..] {
        model.push(convert::row(&n.row, n.details.as_ref(), n.odd, n.dark));
        writes += 1;
    }
    *shown = new;
    writes
}

/// Replaces a small model's rows when they differ.
fn sync_small<T: Clone + PartialEq + 'static>(model: &VecModel<T>, new: Vec<T>) -> bool {
    let same = model.row_count() == new.len()
        && new
            .iter()
            .enumerate()
            .all(|(i, n)| model.row_data(i).as_ref() == Some(n));
    if !same {
        model.set_vec(new);
    }
    !same
}

/// The pane's state that lives in Rust: the view, the store, the models.
struct Inner {
    view: View,
    store: Arc<Mutex<Store>>,
    rows: Rc<VecModel<MonitorRow>>,
    shown: Vec<Shown>,
    chips: Rc<VecModel<MonitorChip>>,
    alerts: Rc<VecModel<MonitorAlert>>,
    alert_seqs: Vec<Option<u64>>,
    groups: Rc<VecModel<MonitorGroup>>,
    groups_for: Option<ViewState>,
}

/// The bridge values one refresh computes, apart from the models.
#[derive(Debug, Clone, PartialEq)]
struct Scalars {
    paused: bool,
    waiting: i32,
    fold: bool,
    raw: bool,
    only_described: bool,
    following: bool,
    time_format: &'static str,
    channel: i32,
    status: String,
}

fn lock(store: &Mutex<Store>) -> MutexGuard<'_, Store> {
    // A producer that panicked mid-write leaves a store that is still
    // readable; keep showing it.
    store.lock().unwrap_or_else(|e| e.into_inner())
}

impl Inner {
    /// Takes in new events and brings the models up to date for a list of
    /// `lines` lines.
    fn refresh(&mut self, lines: usize, dark: bool) -> Scalars {
        let store = self.store.clone();
        let store = lock(&store);
        self.view.update(&store);
        let lines = lines.max(1);
        // View keeps the selection in a smaller window by itself.
        let range = self.view.window(lines);
        let rows = self.view.rows(&store, range.clone());
        let details: Vec<Option<Details>> = rows
            .iter()
            .map(|r| {
                r.expanded
                    .then(|| self.view.details(&store, r.seq))
                    .flatten()
            })
            .collect();
        // Expanded rows take more lines: keep the ones that fit around the
        // selection, or the newest while following.
        let heights: Vec<usize> = details
            .iter()
            .map(|d| 1 + d.as_ref().map_or(0, convert::detail_lines))
            .collect();
        let anchor = if self.view.is_following() {
            rows.len().checked_sub(1)
        } else {
            rows.iter().position(|r| r.selected)
        };
        let keep = convert::fit(&heights, lines, anchor);
        let new: Vec<Shown> = rows
            .into_iter()
            .zip(details)
            .skip(keep.start)
            .take(keep.len())
            .map(|(row, details)| Shown {
                details,
                odd: row.stripe,
                dark,
                row,
            })
            .collect();
        sync_rows(&self.rows, &mut self.shown, new);

        sync_small(
            &self.chips,
            self.view
                .chips()
                .iter()
                .map(|c| convert::chip(c, dark))
                .collect(),
        );
        let alerts = self.view.alerts(&store);
        self.alert_seqs = alerts.iter().map(|a| a.last_seq).collect();
        sync_small(
            &self.alerts,
            alerts.iter().map(|a| convert::alert(a, dark)).collect(),
        );
        let state = self.view.state();
        if self.groups_for.as_ref() != Some(state) {
            self.groups.set_vec(convert::groups(state));
            self.groups_for = Some(state.clone());
        }
        let stats = store.stats();
        Scalars {
            paused: self.view.is_paused(),
            waiting: i32::try_from(self.view.pending(&store)).unwrap_or(i32::MAX),
            fold: state.fold,
            raw: state.style.raw,
            only_described: state.only_described,
            following: self.view.is_following(),
            time_format: convert::time_label(state.time),
            channel: convert::channel_index(state.channels),
            status: status(self.view.len(), stats.events),
        }
    }

    fn handle(&mut self, event: UiEvent) {
        let seqs: Vec<u64> = self.shown.iter().map(|s| s.row.seq).collect();
        let commands = convert::commands(event, self.view.state(), &seqs, &self.alert_seqs);
        let store = self.store.clone();
        let mut store = lock(&store);
        for c in commands {
            self.view.apply(c, &mut store);
        }
    }
}

/// "1 204 rows · 18 377 events".
fn status(rows: usize, events: usize) -> String {
    let n = |v: usize| {
        let s = v.to_string();
        let mut out = String::new();
        for (i, ch) in s.chars().enumerate() {
            if i > 0 && (s.len() - i).is_multiple_of(3) {
                out.push('\u{202F}');
            }
            out.push(ch);
        }
        out
    };
    format!("{} rows · {} events", n(rows), n(events))
}

/// Drives one monitor pane: owns its [`View`], refreshes the
/// `MonitorBridge` every [`REFRESH`] and carries out what the pane asks.
/// Dropping it stops the refresh.
pub struct MonitorController {
    inner: Rc<RefCell<Inner>>,
    refresh: Box<dyn Fn()>,
    timer: Timer,
}

impl MonitorController {
    /// Binds the bridge of `ui`, a component of this crate that contains a
    /// `MonitorPane` (such as [`MonitorWindow`]), and starts the timer.
    pub fn new<C>(ui: &C, store: Arc<Mutex<Store>>, state: ViewState) -> Self
    where
        C: ComponentHandle + 'static,
        for<'a> MonitorBridge<'a>: Global<'a, C>,
    {
        let inner = Rc::new(RefCell::new(Inner {
            view: View::new(state),
            store,
            rows: Rc::new(VecModel::default()),
            shown: Vec::new(),
            chips: Rc::new(VecModel::default()),
            alerts: Rc::new(VecModel::default()),
            alert_seqs: Vec::new(),
            groups: Rc::new(VecModel::default()),
            groups_for: None,
        }));
        {
            let b = ui.global::<MonitorBridge>();
            let i = inner.borrow();
            b.set_rows(ModelRc::from(i.rows.clone()));
            b.set_chips(ModelRc::from(i.chips.clone()));
            b.set_alerts(ModelRc::from(i.alerts.clone()));
            b.set_groups(ModelRc::from(i.groups.clone()));
            b.set_mono_family(mono_family().into());
        }

        let weak_ui = ui.as_weak();
        let weak: Weak<RefCell<Inner>> = Rc::downgrade(&inner);
        let refresh: Rc<dyn Fn()> = Rc::new(move || {
            let (Some(ui), Some(inner)) = (weak_ui.upgrade(), weak.upgrade()) else {
                return;
            };
            let b = ui.global::<MonitorBridge>();
            let lines = usize::try_from(b.get_visible_lines()).unwrap_or(1);
            let Ok(mut inner) = inner.try_borrow_mut() else {
                return;
            };
            let s = inner.refresh(lines, b.get_dark());
            drop(inner);
            b.set_paused(s.paused);
            b.set_waiting(s.waiting);
            b.set_fold(s.fold);
            b.set_raw(s.raw);
            b.set_only_described(s.only_described);
            b.set_following(s.following);
            b.set_time_format(s.time_format.into());
            b.set_channel(s.channel);
            b.set_status(s.status.into());
        });

        let b = ui.global::<MonitorBridge>();
        let on = |event: fn() -> UiEvent| {
            let inner = Rc::downgrade(&inner);
            let refresh = refresh.clone();
            move || {
                if let Some(i) = inner.upgrade() {
                    i.borrow_mut().handle(event());
                }
                refresh();
            }
        };
        let on_int = |event: fn(i32) -> UiEvent| {
            let inner = Rc::downgrade(&inner);
            let refresh = refresh.clone();
            move |v: i32| {
                if let Some(i) = inner.upgrade() {
                    i.borrow_mut().handle(event(v));
                }
                refresh();
            }
        };
        b.on_toggle_pause(on(|| UiEvent::TogglePause));
        b.on_clear(on(|| UiEvent::Clear));
        b.on_toggle_fold(on(|| UiEvent::ToggleFold));
        b.on_toggle_raw(on(|| UiEvent::ToggleRaw));
        b.on_cycle_time(on(|| UiEvent::CycleTime));
        b.on_toggle_only_described(on(|| UiEvent::ToggleOnlyDescribed));
        b.on_channel_chosen(on_int(UiEvent::Channel));
        b.on_toggle_type(on_int(UiEvent::ToggleType));
        b.on_toggle_group(on_int(UiEvent::ToggleGroup));
        b.on_row_clicked(on_int(UiEvent::RowClicked));
        b.on_row_activated(on_int(UiEvent::RowActivated));
        b.on_alert_clicked(on_int(UiEvent::AlertClicked));
        {
            let inner = Rc::downgrade(&inner);
            let refresh = refresh.clone();
            b.on_search(move |text| {
                if let Some(i) = inner.upgrade() {
                    i.borrow_mut().handle(UiEvent::Search(text.to_string()));
                }
                refresh();
            });
        }
        {
            let inner = Rc::downgrade(&inner);
            let refresh = refresh.clone();
            b.on_nav(move |code, n| {
                if let Some(i) = inner.upgrade() {
                    i.borrow_mut().handle(UiEvent::Nav(code, n));
                }
                refresh();
            });
        }

        refresh();
        let timer = Timer::default();
        let tick = refresh.clone();
        timer.start(TimerMode::Repeated, REFRESH, move || tick());
        MonitorController {
            inner,
            refresh: Box::new(move || refresh()),
            timer,
        }
    }

    /// Refreshes now, without waiting for the timer (tests, or an editor
    /// that just sent something).
    pub fn refresh(&self) {
        (self.refresh)();
    }

    /// Carries out a pane event as if the pane had reported it.
    pub fn handle(&self, event: UiEvent) {
        self.inner.borrow_mut().handle(event);
        self.refresh();
    }

    /// Reads the view, e.g. for its filter.
    pub fn with_view<R>(&self, f: impl FnOnce(&View) -> R) -> R {
        f(&self.inner.borrow().view)
    }

    /// Stops the periodic refresh; [`MonitorController::refresh`] still works.
    pub fn stop(&self) {
        self.timer.stop();
    }
}

#[cfg(test)]
mod tests;
