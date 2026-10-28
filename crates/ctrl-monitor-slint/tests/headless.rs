//! The window without a screen: Slint's testing backend with its software
//! renderer. One test, because the backend is set once per thread.

#[path = "../examples/support/feed.rs"]
#[allow(dead_code)]
mod feed;
#[path = "../examples/support/headless.rs"]
mod headless;

use ctrl_monitor_slint::MonitorBridge;
use i_slint_backend_testing::ElementHandle;
use slint::{ComponentHandle, Model};

#[test]
fn the_window_shows_rows_alerts_and_reacts() {
    headless::init();
    let (window, monitor) = headless::window(headless::store(6), 1000, 720);
    let bridge = window.global::<MonitorBridge>();

    // Rows arrived, newest at the bottom, following.
    let rows = bridge.get_rows();
    assert!(rows.row_count() > 10, "{} rows", rows.row_count());
    assert!(bridge.get_following());
    assert!(bridge.get_visible_lines() >= 20);
    let last = rows.row_data(rows.row_count() - 1).unwrap();
    assert!(last.time.starts_with("5."), "newest last: {}", last.time);

    // A row is an element with its message and value as label.
    let label = format!("{} {}", last.message, last.value);
    assert_eq!(
        ElementHandle::find_by_accessible_label(&window, &label).count(),
        1,
        "{label}"
    );

    // Chips and the alert strip.
    let chips: Vec<_> = bridge.get_chips().iter().map(|c| c.label).collect();
    assert_eq!(chips, ["Clock", "Active Sense"]);
    let alert = ElementHandle::find_by_accessible_label(&window, "SysEx cut by another message")
        .next()
        .expect("alert shown");

    // Clicking the alert selects the cut SysEx and stops following.
    alert.invoke_accessible_default_action();
    let selected: Vec<_> = bridge
        .get_rows()
        .iter()
        .filter(|r| r.selected)
        .map(|r| r.message.to_string())
        .collect();
    assert_eq!(selected, ["SysEx"]);
    assert!(!bridge.get_following());

    // The pause button through its accessible action.
    ElementHandle::find_by_accessible_label(&window, "Pause")
        .next()
        .expect("pause button")
        .invoke_accessible_default_action();
    assert!(bridge.get_paused());
    headless::settle(&window, &monitor);
    assert!(ElementHandle::find_by_accessible_label(&window, "Resume")
        .next()
        .is_some());

    // It renders.
    let shot = window.window().take_snapshot().expect("snapshot");
    assert_eq!((shot.width(), shot.height()), (1000, 720));
}
