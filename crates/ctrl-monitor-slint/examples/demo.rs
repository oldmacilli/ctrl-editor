//! The monitor window fed by a synthetic producer thread.
//!
//! `cargo run -p ctrl-monitor-slint --example demo`

#[path = "support/feed.rs"]
mod feed;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use ctrl_monitor::{tap, Clock, Collector, ParserConfig, Stamp, Store, TapConfig, ViewState};
use ctrl_monitor_slint::{MonitorController, MonitorWindow};
use slint::ComponentHandle;

fn main() -> Result<(), slint::PlatformError> {
    let mut store = Store::default();
    let ports = feed::Ports::register(&mut store);
    let store = Arc::new(Mutex::new(store));

    let clock = Clock::new();
    let (mut tap, reader) = tap(TapConfig::default());
    let mut collector = Collector::new(ParserConfig::default());
    collector.add(reader);
    collector.describe_with(Box::new(feed::Demo));
    let _collector = collector.spawn(store.clone(), clock, Duration::from_millis(5));

    // The producer: ten seconds of traffic, again and again, in real time.
    std::thread::spawn(move || {
        let cycle = 10;
        let mut n = 0u64;
        loop {
            for p in feed::script(n * cycle * 1000, cycle, n == 0) {
                let due = Duration::from_millis(p.ms);
                let now = Duration::from_nanos(clock.now().host_ns);
                if due > now {
                    std::thread::sleep(due - now);
                }
                let at: Stamp = clock.now();
                tap.push(at, ports.of(p.via), p.dir, &p.bytes);
            }
            n += 1;
        }
    });

    let window = MonitorWindow::new()?;
    let _monitor = MonitorController::new(&window, store, ViewState::default());
    window.run()
}
