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

use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use ctrl_monitor::Store;
use slint::{Model, VecModel};

#[doc(hidden)]
pub use ctrl_monitor;
#[doc(hidden)]
pub use slint;

pub mod convert;
#[macro_use]
mod glue;

slint::include_modules!();
monitor_glue!();

pub use monitor_glue::MonitorController;

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

/// Replaces a small model's rows when they differ.
#[doc(hidden)]
pub fn sync_small<T: Clone + PartialEq + 'static>(model: &VecModel<T>, new: Vec<T>) -> bool {
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

#[doc(hidden)]
pub fn lock(store: &Mutex<Store>) -> MutexGuard<'_, Store> {
    // A producer that panicked mid-write leaves a store that is still
    // readable; keep showing it.
    store.lock().unwrap_or_else(|e| e.into_inner())
}

/// "1 204 rows · 18 377 events".
#[doc(hidden)]
pub fn status(rows: usize, events: usize) -> String {
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

#[cfg(test)]
mod tests;
