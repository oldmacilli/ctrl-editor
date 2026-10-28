//! Renders the monitor window without a screen into PNGs, light and dark.
//!
//! `cargo run -p ctrl-monitor-slint --example snapshot -- <dir>`

#[path = "support/feed.rs"]
mod feed;
#[path = "support/headless.rs"]
mod headless;

use std::path::PathBuf;

use ctrl_monitor_slint::convert::UiEvent;
use ctrl_monitor_slint::MonitorBridge;
use slint::{ComponentHandle, Model};

fn save(window: &ctrl_monitor_slint::MonitorWindow, path: &PathBuf) {
    let shot = window.window().take_snapshot().expect("snapshot");
    let img = image::RgbaImage::from_raw(shot.width(), shot.height(), shot.as_bytes().to_vec())
        .expect("pixels");
    img.save(path).expect("write png");
    println!("{}", path.display());
}

fn main() {
    let dir = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| ".".into()));
    headless::init();
    let (window, monitor) = headless::window(headless::store(6), 1000, 720);

    // Open the device's identity reply, the way a double click does.
    let rows = window.global::<MonitorBridge>().get_rows();
    if let Some(i) = rows
        .iter()
        .position(|r| r.message == "SysEx" && r.value.starts_with("Identity Reply"))
    {
        monitor.handle(UiEvent::RowActivated(i as i32));
    }
    headless::settle(&window, &monitor);
    save(&window, &dir.join("ctrl-monitor-light.png"));

    window.set_color_scheme(slint::language::ColorScheme::Dark);
    headless::settle(&window, &monitor);
    save(&window, &dir.join("ctrl-monitor-dark.png"));

    // Narrow, filters open, the clock hidden.
    window.set_color_scheme(slint::language::ColorScheme::Light);
    window.set_filters_open(true);
    monitor.handle(UiEvent::RowActivated(-1));
    monitor.handle(UiEvent::Nav(5, 1));
    monitor.handle(UiEvent::ToggleType(
        ctrl_monitor::Class::ALL
            .iter()
            .position(|&c| c == ctrl_monitor::Class::Clock)
            .unwrap() as i32,
    ));
    window.window().set_size(slint::PhysicalSize::new(600, 560));
    headless::settle(&window, &monitor);
    save(&window, &dir.join("ctrl-monitor-narrow.png"));
}
