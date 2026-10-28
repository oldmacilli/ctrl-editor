//! On a JACK server (jackd2, or PipeWire's libjack): the monitor watches a
//! conversation between two other clients without taking part, and another
//! client sends to the monitor's virtual input (docs/MONITOR.md §6.1, §6.2).
//!
//!     CTRL_MIDI_JACK_TEST=1 cargo test -p ctrl-monitor-midi -- --ignored
//!
//! tools/linux-check runs it under both servers.
#![cfg(target_os = "linux")]

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ctrl_midi::{Backend, Port, SysexPolicy};
use ctrl_monitor::{Dir, Msg, StoreConfig, SysExEnd};
use ctrl_monitor_midi::Monitor;
use jack::{Client, ClientOptions, Control, MidiIn, MidiOut, ProcessScope, RawMidi};

type Queue = Arc<Mutex<Vec<Vec<u8>>>>;

struct Sender {
    port: jack::Port<MidiOut>,
    queue: Queue,
}

impl jack::ProcessHandler for Sender {
    fn process(&mut self, _: &Client, ps: &ProcessScope) -> Control {
        let mut w = self.port.writer(ps);
        // A test client: a lock in the process callback is fine here.
        if let Ok(mut q) = self.queue.try_lock() {
            for msg in q.drain(..) {
                let _ = w.write(&RawMidi {
                    time: 0,
                    bytes: &msg,
                });
            }
        }
        Control::Continue
    }
}

struct Receiver {
    port: jack::Port<MidiIn>,
    got: Queue,
}

impl jack::ProcessHandler for Receiver {
    fn process(&mut self, _: &Client, ps: &ProcessScope) -> Control {
        if let Ok(mut g) = self.got.try_lock() {
            for e in self.port.iter(ps) {
                g.push(e.bytes.to_vec());
            }
        }
        Control::Continue
    }
}

fn until(what: &str, mut ok: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(5);
    while !ok() {
        assert!(Instant::now() < end, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn sysex(len: usize) -> Vec<u8> {
    let mut m = vec![0xF0, 0x42, 0x40, 0x00, 0x01, 0x37];
    m.extend((0..len - 7).map(|i| (i % 128) as u8));
    m.push(0xF7);
    m
}

#[test]
#[ignore = "needs a JACK server; CTRL_MIDI_JACK_TEST=1"]
fn watches_two_other_clients_and_offers_a_virtual_input() {
    if std::env::var_os("CTRL_MIDI_JACK_TEST").is_none() {
        eprintln!("CTRL_MIDI_JACK_TEST is not set: skipped");
        return;
    }
    // Two clients that know nothing of the monitor, talking.
    let (sender, _) = Client::new("test-sender", ClientOptions::NO_START_SERVER).unwrap();
    let (receiver, _) = Client::new("test-receiver", ClientOptions::NO_START_SERVER).unwrap();
    let out = sender.register_port("out", MidiOut::default()).unwrap();
    let inp = receiver.register_port("in", MidiIn::default()).unwrap();
    let (out_name, in_name) = (out.name().unwrap(), inp.name().unwrap());
    let queue: Queue = Arc::default();
    let got: Queue = Arc::default();
    let sender = sender
        .activate_async(
            (),
            Sender {
                port: out,
                queue: queue.clone(),
            },
        )
        .unwrap();
    let _receiver = receiver
        .activate_async(
            (),
            Receiver {
                port: inp,
                got: got.clone(),
            },
        )
        .unwrap();
    sender
        .as_client()
        .connect_ports_by_name(&out_name, &in_name)
        .unwrap();

    let monitor = Monitor::start(StoreConfig::default(), Vec::new());
    let watched = Port {
        name: out_name.clone(),
        display: out_name.clone(),
        dir: ctrl_midi::Dir::In,
    };
    let watch = monitor.watch(Backend::Jack, &watched).unwrap();
    std::thread::sleep(Duration::from_millis(300));

    let dump = sysex(3002);
    queue
        .lock()
        .unwrap()
        .extend([vec![0xB0, 7, 100], dump.clone()]);

    until("the receiver's messages", || got.lock().unwrap().len() == 2);
    until("the monitor's events", || {
        monitor.store.lock().unwrap().events().len() == 2
    });
    {
        let store = monitor.store.lock().unwrap();
        let events: Vec<_> = store.events().copied().collect();
        assert!(events
            .iter()
            .all(|e| e.port == watch.port && e.dir == Dir::In));
        assert_eq!(
            events[0].msg,
            Msg::Voice {
                status: 0xB0,
                d1: 7,
                d2: 100
            }
        );
        assert!(matches!(
            events[1].msg,
            Msg::SysEx {
                len: 3002,
                end: SysExEnd::Complete,
                ..
            }
        ));
        assert_eq!(
            store.bytes(events[1].msg.blob().unwrap()).unwrap(),
            dump.as_slice()
        );
    }
    // The watched conversation was not disturbed.
    assert_eq!(*got.lock().unwrap(), [vec![0xB0, 7, 100], dump]);

    // A program sends to the monitor's virtual input.
    let virt = monitor
        .virtual_input(Backend::Jack, "lb-monitor-test")
        .unwrap();
    let target = Port {
        name: "lb-monitor-test:in".into(),
        display: "lb-monitor-test:in".into(),
        dir: ctrl_midi::Dir::Out,
    };
    let mut program =
        ctrl_midi::open_output(Backend::Jack, &target, SysexPolicy::default()).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    program.send(&[0x91, 64, 90]).unwrap();
    until("the virtual input's event", || {
        monitor
            .store
            .lock()
            .unwrap()
            .events()
            .any(|e| e.port == virt.port)
    });
    let store = monitor.store.lock().unwrap();
    let e = store
        .events()
        .find(|e| e.port == virt.port)
        .copied()
        .unwrap();
    assert_eq!(
        e.msg,
        Msg::Voice {
            status: 0x91,
            d1: 64,
            d2: 90
        }
    );
    eprintln!("watched 2 messages between two clients; 1 via the virtual input");
}
