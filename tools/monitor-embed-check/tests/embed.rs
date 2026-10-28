//! The pane inside another crate's window shows the store's messages.

use std::sync::{Arc, Mutex};

use ctrl_monitor::{Dir, Parsed, PortKind, PortRef, Stamp, Store, ViewState};
use monitor_embed_check::monitor_glue::MonitorController;
use monitor_embed_check::{EditorWindow, MonitorBridge};
use slint::{ComponentHandle, Model};

#[test]
fn an_editor_window_embeds_the_pane_and_its_glue() {
    i_slint_backend_testing::init_no_event_loop();
    let mut store = Store::default();
    let port = store.port(PortRef::new(PortKind::Alsa, "k", "nanoKONTROL Studio"));
    store.record(
        port,
        Dir::In,
        Parsed::Voice {
            at: Stamp::default(),
            status: 0x90,
            d1: 60,
            d2: 100,
        },
    );
    let window = EditorWindow::new().unwrap();
    window.show().unwrap();
    let monitor =
        MonitorController::new(&window, Arc::new(Mutex::new(store)), ViewState::default());
    monitor.refresh();
    let rows = window.global::<MonitorBridge>().get_rows();
    assert_eq!(rows.row_count(), 1);
    assert_eq!(rows.row_data(0).unwrap().value, "C4 vel 100");
}
