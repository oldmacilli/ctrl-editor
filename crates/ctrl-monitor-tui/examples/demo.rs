//! The monitor pane fed by a synthetic producer: notes, a controller sweep,
//! clock at 120 BPM, SysEx now and then, one cut SysEx.
//!
//!     cargo run -p ctrl-monitor-tui --example demo
//!
//! q quits; ? lists the keys.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use ctrl_monitor::{
    tap, CdlDescriber, Clock, Collector, Dir, ParserConfig, PortKind, PortRef, Store, StoreConfig,
    TapConfig, View, ViewState,
};
use ctrl_monitor_tui::{key_input, MonitorPane};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};

fn main() -> std::io::Result<()> {
    let store = Arc::new(Mutex::new(Store::new(StoreConfig::default())));
    let clock = Clock::new();
    let (keys, synth) = {
        let mut s = store.lock().unwrap();
        (
            s.port(PortRef::new(PortKind::Alsa, "demo:0", "nanoKONTROL Studio")),
            s.port(PortRef::new(PortKind::Jack, "demo:1", "Synth")),
        )
    };
    let (mut tx, rx) = tap(TapConfig::default());
    let mut collector = Collector::new(ParserConfig::default());
    collector.add(rx);
    collector.describe_with(Box::new(CdlDescriber::embedded()));
    let _running = collector.spawn(store.clone(), clock, Duration::from_millis(5));

    std::thread::spawn(move || {
        let mut tick = 0u64;
        loop {
            let now = clock.now();
            tx.push(now, synth, Dir::Out, &[0xF8]);
            if tick % 12 == 0 {
                let note = 48 + (tick / 12 % 24) as u8;
                tx.push(now, keys, Dir::In, &[0x90, note, 100]);
            }
            if tick % 12 == 6 {
                let note = 48 + (tick / 12 % 24) as u8;
                tx.push(now, keys, Dir::In, &[0x80, note, 0]);
            }
            if tick % 96 < 32 {
                tx.push(now, keys, Dir::In, &[0xB0, 7, ((tick % 32) * 4) as u8]);
            }
            if tick % 240 == 120 {
                tx.push(now, synth, Dir::Out, &[0xF0, 0x7E, 0x7F, 0x06, 0x01, 0xF7]);
            }
            if tick % 480 == 300 {
                let mut dump = vec![0xF0, 0x42, 0x40, 0x00, 0x01, 0x37];
                dump.extend((0..600).map(|i| (i % 128) as u8));
                dump.push(0xF7);
                tx.push(now, keys, Dir::In, &dump);
            }
            if tick == 400 {
                tx.push(now, keys, Dir::In, &[0xF0, 0x41, 0x10, 0x90, 60, 1]);
            }
            tick += 1;
            // 24 clocks a quarter note at 120 BPM.
            std::thread::sleep(Duration::from_micros(20_833));
        }
    });

    let mut view = View::new(ViewState::default());
    let mut pane = MonitorPane::default();
    pane.title = "MIDI Monitor (demo)".into();
    let mut terminal = ratatui::init();
    let result = loop {
        {
            let store = store.lock().unwrap();
            view.update(&store);
            if let Err(e) =
                terminal.draw(|f| pane.render(f.area(), f.buffer_mut(), &mut view, &store))
            {
                break Err(e);
            }
        }
        match event::poll(Duration::from_millis(50)) {
            Ok(true) => {}
            Ok(false) => continue,
            Err(e) => break Err(e),
        }
        let Ok(Event::Key(key)) = event::read() else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        let mut store = store.lock().unwrap();
        if pane.is_typing() {
            pane.type_key(key, &mut view, &mut store);
        } else if key.code == KeyCode::Char('q') {
            break Ok(());
        } else if let Some(input) = key_input(key) {
            pane.handle(input, &mut view, &mut store);
        }
    };
    ratatui::restore();
    result
}
