//! A window without a screen: Slint's testing backend with its software
//! renderer, fed with `seconds` of the synthetic script.

use std::sync::{Arc, Mutex};

use ctrl_monitor::{tap, Collector, ParserConfig, Stamp, Store, TapConfig, ViewState};
use ctrl_monitor_slint::{MonitorController, MonitorWindow};
use i_slint_backend_testing::{TestingBackend, TestingBackendOptions};
use slint::ComponentHandle;

use super::feed;

pub fn init() {
    slint::platform::set_platform(Box::new(TestingBackend::new(TestingBackendOptions {
        mock_time: true,
        threading: false,
        renderer_name: Some("software".into()),
    })))
    .expect("platform already initialised");
}

pub fn store(seconds: u64) -> Arc<Mutex<Store>> {
    let mut store = Store::default();
    let ports = feed::Ports::register(&mut store);
    let (mut tap, reader) = tap(TapConfig {
        slots: 1 << 16,
        bytes: 1 << 22,
    });
    let mut collector = Collector::new(ParserConfig::default());
    collector.add(reader);
    collector.describe_with(Box::new(feed::Demo));
    let mut last = 0;
    for p in feed::script(0, seconds, true) {
        let at = Stamp {
            host_ns: p.ms * 1_000_000,
            wall_ms: 1_767_225_600_000 + p.ms as i64,
            ..Stamp::default()
        };
        tap.push(at, ports.of(p.via), p.dir, &p.bytes);
        last = at.host_ns;
    }
    collector.drain(&mut store, last + 1_000_000_000);
    Arc::new(Mutex::new(store))
}

pub fn window(
    store: Arc<Mutex<Store>>,
    width: u32,
    height: u32,
) -> (MonitorWindow, MonitorController) {
    let window = MonitorWindow::new().expect("window");
    window
        .window()
        .set_size(slint::PhysicalSize::new(width, height));
    window.show().expect("show");
    let monitor = MonitorController::new(&window, store, ViewState::default());
    settle(&window, &monitor);
    (window, monitor)
}

/// Lets the pane lay itself out and measure its list, then refreshes.
pub fn settle(window: &MonitorWindow, monitor: &MonitorController) {
    for _ in 0..3 {
        let _ = window.window().take_snapshot();
        slint::platform::update_timers_and_animations();
        monitor.refresh();
    }
}
