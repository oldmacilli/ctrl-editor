//! The native backend (ALSA sequencer, CoreMIDI): a program sends to the
//! monitor's virtual input (docs/MONITOR.md §6.2). Touches no device.
//!
//!     CTRL_MIDI_NATIVE_TEST=1 cargo test -p ctrl-monitor-midi -- --ignored
#![cfg(unix)]

use std::time::{Duration, Instant};

use ctrl_midi::{Backend, SysexPolicy};
use ctrl_monitor::{Msg, StoreConfig};
use ctrl_monitor_midi::Monitor;

#[test]
#[ignore = "opens the platform's MIDI system; CTRL_MIDI_NATIVE_TEST=1"]
fn a_program_sends_to_the_virtual_input() {
    if std::env::var_os("CTRL_MIDI_NATIVE_TEST").is_none() {
        eprintln!("CTRL_MIDI_NATIVE_TEST is not set: skipped");
        return;
    }
    let backend = Backend::native();
    let monitor = Monitor::start(StoreConfig::default(), Vec::new());
    let virt = monitor
        .virtual_input(backend, "lb-monitor-native-test")
        .unwrap();
    std::thread::sleep(Duration::from_millis(300));
    let port = ctrl_midi::ports(backend)
        .unwrap()
        .into_iter()
        .find(|p| p.dir == ctrl_midi::Dir::Out && p.display.contains("lb-monitor-native-test"))
        .expect("the virtual input is listed for other programs");
    let mut program = ctrl_midi::open_output(backend, &port, SysexPolicy::default()).unwrap();
    program.send(&[0xB2, 1, 64]).unwrap();
    program.send(&[0xF0, 0x7E, 0x7F, 0x06, 0x01, 0xF7]).unwrap();
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        let n = monitor
            .store
            .lock()
            .unwrap()
            .events()
            .filter(|e| e.port == virt.port)
            .count();
        if n == 2 {
            break;
        }
        assert!(Instant::now() < end, "{n} of 2 events");
        std::thread::sleep(Duration::from_millis(10));
    }
    let store = monitor.store.lock().unwrap();
    let msgs: Vec<Msg> = store.events().map(|e| e.msg).collect();
    assert_eq!(
        msgs[0],
        Msg::Voice {
            status: 0xB2,
            d1: 1,
            d2: 64
        }
    );
    assert!(matches!(msgs[1], Msg::SysEx { len: 6, .. }));
}
