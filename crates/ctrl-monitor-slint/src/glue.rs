//! The glue between the bridge `MonitorBridge` and a [`MonitorController`]
//! (docs/MONITOR.md §5), as a macro: Slint generates the bridge's types in
//! the crate that compiles the `.slint` files, so the glue has to be
//! expanded there. This crate expands it for its own `MonitorWindow`; an
//! editor that embeds `MonitorPane` in its window expands it next to its
//! own `slint::include_modules!()`.
//!
//! In the editor's `.slint` root, export the bridge and its structs:
//!
//! ```slint,ignore
//! import { MonitorPane } from "@ctrl-monitor/monitor.slint";
//! export { MonitorBridge, MonitorRow, MonitorField, MonitorHex, MonitorChip,
//!          MonitorAlert, MonitorType, MonitorGroup } from "@ctrl-monitor/monitor.slint";
//! ```
//!
//! In its `build.rs`, map `@ctrl-monitor` to this crate's `ui/` directory,
//! which Cargo passes as `DEP_CTRL_MONITOR_SLINT_UI` (`links` metadata):
//!
//! ```ignore
//! let ui = std::env::var("DEP_CTRL_MONITOR_SLINT_UI").unwrap();
//! let config = slint_build::CompilerConfiguration::new()
//!     .with_library_paths([("ctrl-monitor".into(), ui.into())].into());
//! slint_build::compile_with_config("ui/app.slint", config).unwrap();
//! ```
//!
//! In its Rust code:
//!
//! ```ignore
//! slint::include_modules!();
//! ctrl_monitor_slint::monitor_glue!();
//! // monitor_glue::MonitorController::new(&main_window, store, ViewState::default())
//! ```
//!
//! [`MonitorController`]: crate::MonitorController

/// Expands `mod monitor_glue` with `MonitorController` and the conversions
/// into the bridge's structs, for the Slint types in scope where it is
/// expanded (see the module docs).
#[macro_export]
macro_rules! monitor_glue {
    () => {
        #[allow(dead_code)]
        pub mod monitor_glue {
            use std::cell::RefCell;
            use std::rc::{Rc, Weak};
            use std::sync::{Arc, Mutex};

            use $crate::convert::UiEvent;
            use $crate::ctrl_monitor::{
                Alert, AlertKind, Chip, Class, Details, Row, Store, Tone, View, ViewState,
            };
            use $crate::slint::{
                ComponentHandle, Global, Model, ModelRc, SharedString, Timer, TimerMode, VecModel,
            };

            use super::{
                MonitorAlert, MonitorBridge, MonitorChip, MonitorField, MonitorGroup, MonitorHex,
                MonitorRow, MonitorType,
            };

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
                    tone: $crate::convert::color(r.tone, dark),
                    editor: r.editor,
                    expandable: r.expandable,
                    selected: r.selected,
                    expanded: r.expanded,
                    odd,
                    spark: $crate::convert::spark_path(&r.spark).into(),
                    title,
                    fields: ModelRc::new(VecModel::from(fields)),
                    hex: ModelRc::new(VecModel::from(hex)),
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
                    port: format!("{} {}", $crate::convert::glyph(c.dir), c.port).into(),
                    value: c
                        .bpm
                        .map(|b| format!("{b:.1} BPM"))
                        .unwrap_or_default()
                        .into(),
                    count: format!("×{}", c.count).into(),
                    tone: $crate::convert::color(tone, dark),
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
                    tone: $crate::convert::color(tone, dark),
                    jump: a.last_seq.is_some(),
                }
            }

            /// The filter pane: a column per group, its types inside.
            pub fn groups(state: &ViewState) -> Vec<MonitorGroup> {
                $crate::convert::COLUMNS
                    .iter()
                    .enumerate()
                    .map(|(col, &(name, tri))| {
                        let types: Vec<MonitorType> = Class::ALL
                            .iter()
                            .enumerate()
                            .filter(|(_, &c)| $crate::convert::column(c) == col)
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

            /// What a shown row was made from; a row is written again only when this
            /// changes.
            #[derive(Debug, Clone, PartialEq)]
            #[doc(hidden)]
            pub struct Shown {
                pub row: Row,
                pub details: Option<Details>,
                pub odd: bool,
                pub dark: bool,
            }

            /// Brings `model` (mirrored by `shown`) to `new`, writing only what
            /// changed. Rows that scrolled off the top are removed, not rewritten.
            /// Returns the rows written or inserted.
            #[doc(hidden)]
            pub fn sync_rows(
                model: &VecModel<MonitorRow>,
                shown: &mut Vec<Shown>,
                new: Vec<Shown>,
            ) -> usize {
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
                        model.set_row_data(i, row(&n.row, n.details.as_ref(), n.odd, n.dark));
                        writes += 1;
                    }
                }
                while model.row_count() > new.len() {
                    model.remove(model.row_count() - 1);
                }
                for n in &new[common..] {
                    model.push(row(&n.row, n.details.as_ref(), n.odd, n.dark));
                    writes += 1;
                }
                *shown = new;
                writes
            }

            /// The pane's state that lives in Rust: the view, the store, the models.
            #[doc(hidden)]
            pub struct Inner {
                pub view: View,
                pub store: Arc<Mutex<Store>>,
                pub rows: Rc<VecModel<MonitorRow>>,
                pub shown: Vec<Shown>,
                pub chips: Rc<VecModel<MonitorChip>>,
                pub alerts: Rc<VecModel<MonitorAlert>>,
                pub alert_seqs: Vec<Option<u64>>,
                pub groups: Rc<VecModel<MonitorGroup>>,
                pub groups_for: Option<ViewState>,
            }

            /// The bridge values one refresh computes, apart from the models.
            #[derive(Debug, Clone, PartialEq)]
            #[doc(hidden)]
            pub struct Scalars {
                pub paused: bool,
                pub waiting: i32,
                pub fold: bool,
                pub raw: bool,
                pub only_described: bool,
                pub following: bool,
                pub time_format: &'static str,
                pub channel: i32,
                pub status: String,
            }

            impl Inner {
                /// Takes in new events and brings the models up to date for a list of
                /// `lines` lines.
                pub fn refresh(&mut self, lines: usize, dark: bool) -> Scalars {
                    let store = self.store.clone();
                    let store = $crate::lock(&store);
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
                        .map(|d| 1 + d.as_ref().map_or(0, $crate::convert::detail_lines))
                        .collect();
                    let anchor = if self.view.is_following() {
                        rows.len().checked_sub(1)
                    } else {
                        rows.iter().position(|r| r.selected)
                    };
                    let keep = $crate::convert::fit(&heights, lines, anchor);
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

                    $crate::sync_small(
                        &self.chips,
                        self.view.chips().iter().map(|c| chip(c, dark)).collect(),
                    );
                    let alerts = self.view.alerts(&store);
                    self.alert_seqs = alerts.iter().map(|a| a.last_seq).collect();
                    $crate::sync_small(
                        &self.alerts,
                        alerts.iter().map(|a| alert(a, dark)).collect(),
                    );
                    let state = self.view.state();
                    if self.groups_for.as_ref() != Some(state) {
                        self.groups.set_vec(groups(state));
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
                        time_format: $crate::convert::time_label(state.time),
                        channel: $crate::convert::channel_index(state.channels),
                        status: $crate::status(self.view.len(), stats.events),
                    }
                }

                pub fn handle(&mut self, event: UiEvent) {
                    let seqs: Vec<u64> = self.shown.iter().map(|s| s.row.seq).collect();
                    let commands = $crate::convert::commands(
                        event,
                        self.view.state(),
                        &seqs,
                        &self.alert_seqs,
                    );
                    let store = self.store.clone();
                    let mut store = $crate::lock(&store);
                    for c in commands {
                        self.view.apply(c, &mut store);
                    }
                }
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
                        b.set_mono_family($crate::mono_family().into());
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
                    timer.start(TimerMode::Repeated, $crate::REFRESH, move || tick());
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
        }
    };
}
