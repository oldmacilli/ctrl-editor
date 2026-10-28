//! Drains the taps into a store (docs/MONITOR.md §2): one parser per port
//! and direction, HID reports kept as they are, drops added to the store's
//! counters. The only part of the monitor that allocates.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::model::{Annotation, Clock, Dir, Event, PortIdx};
use crate::parser::{Parsed, Parser, ParserConfig};
use crate::store::Store;
use crate::tap::TapReader;

/// Explains events as they are recorded (docs/MONITOR.md §4.2): the CDL
/// describer ([`crate::CdlDescriber`]), an editor's device annotator.
/// Runs on the collector thread, once per event, never per redraw.
pub trait Describer: Send {
    fn describe(&mut self, store: &Store, event: &Event) -> Vec<Annotation>;
}

struct Reader {
    tap: TapReader,
    seen_dropped: u64,
    /// The producer was gone before the last drain: nothing more comes.
    done: bool,
}

pub struct Collector {
    readers: Vec<Reader>,
    parsers: HashMap<(PortIdx, Dir), Parser>,
    config: ParserConfig,
    parsed: Vec<Parsed>,
    describers: Vec<Box<dyn Describer>>,
}

impl Collector {
    pub fn new(config: ParserConfig) -> Self {
        Collector {
            readers: Vec::new(),
            parsers: HashMap::new(),
            config,
            parsed: Vec::new(),
            describers: Vec::new(),
        }
    }

    /// Adds a describer; describers run in the order they were added.
    pub fn describe_with(&mut self, describer: Box<dyn Describer>) {
        self.describers.push(describer);
    }

    pub fn add(&mut self, tap: TapReader) {
        self.readers.push(Reader {
            tap,
            seen_dropped: 0,
            done: false,
        });
    }

    /// Moves everything waiting into `store` and ends SysEx that timed out
    /// by `now_ns`. Returns the number of packets read.
    pub fn drain(&mut self, store: &mut Store, now_ns: u64) -> usize {
        let Collector {
            readers,
            parsers,
            config,
            parsed,
            describers,
        } = self;
        let first = store.next_seq();
        let mut packets = 0;
        for reader in readers.iter_mut() {
            reader.done = reader.tap.is_abandoned();
            packets += reader.tap.drain(|p| {
                if store.is_hid(p.port) {
                    store.record_hid(p.at, p.port, p.dir, p.bytes);
                    return;
                }
                let parser = parsers
                    .entry((p.port, p.dir))
                    .or_insert_with(|| Parser::new(*config));
                parser.push(p.at, p.bytes, parsed);
                for item in parsed.drain(..) {
                    store.record(p.port, p.dir, item);
                }
            });
            let dropped = reader.tap.dropped();
            store.add_dropped(dropped - reader.seen_dropped);
            reader.seen_dropped = dropped;
        }
        for (&(port, dir), parser) in parsers.iter_mut() {
            parser.poll(now_ns, parsed);
            for item in parsed.drain(..) {
                store.record(port, dir, item);
            }
        }
        readers.retain(|r| !r.done);
        if !describers.is_empty() {
            for seq in first.max(store.first_seq())..store.next_seq() {
                let Some(event) = store.get(seq).copied() else {
                    continue;
                };
                let mut notes = Vec::new();
                for d in describers.iter_mut() {
                    notes.extend(d.describe(store, &event));
                }
                for note in notes {
                    store.annotate(seq, note);
                }
            }
        }
        packets
    }

    /// Runs the collector on its own thread, draining every `period` into
    /// the shared store, until the handle is stopped or dropped.
    pub fn spawn(mut self, store: Arc<Mutex<Store>>, clock: Clock, period: Duration) -> Running {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let (taps, new_taps) = std::sync::mpsc::channel::<TapReader>();
        let thread = std::thread::Builder::new()
            .name("ctrl-monitor".into())
            .spawn(move || {
                while !flag.load(Ordering::Relaxed) {
                    for tap in new_taps.try_iter() {
                        self.add(tap);
                    }
                    if let Ok(mut store) = store.lock() {
                        self.drain(&mut store, clock.now().host_ns);
                    }
                    std::thread::sleep(period);
                }
            })
            .expect("spawn the collector thread");
        Running {
            stop,
            taps,
            thread: Some(thread),
        }
    }
}

/// A collector running on its own thread.
pub struct Running {
    stop: Arc<AtomicBool>,
    taps: std::sync::mpsc::Sender<TapReader>,
    thread: Option<JoinHandle<()>>,
}

impl Running {
    /// Adds a tap while the collector runs: a port watched later.
    pub fn add(&self, tap: TapReader) {
        let _ = self.taps.send(tap);
    }

    pub fn stop(mut self) {
        self.halt();
    }

    fn halt(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.halt();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Msg, PortKind, PortRef, Stamp, SysExEnd};
    use crate::store::StoreConfig;
    use crate::tap::{tap, TapConfig};

    fn at(ns: u64) -> Stamp {
        Stamp {
            host_ns: ns,
            ..Stamp::default()
        }
    }

    #[test]
    fn each_port_and_direction_has_its_own_running_status() {
        let mut store = Store::new(StoreConfig::default());
        let a = store.port(PortRef::new(PortKind::Alsa, "a", "A"));
        let b = store.port(PortRef::new(PortKind::Alsa, "b", "B"));
        let (mut tx, rx) = tap(TapConfig::default());
        let mut c = Collector::new(ParserConfig::default());
        c.add(rx);
        tx.push(at(1), a, Dir::In, &[0x90, 60]);
        tx.push(at(2), b, Dir::In, &[0xB0, 7, 100]);
        tx.push(at(3), a, Dir::In, &[1, 61, 2]);
        tx.push(at(4), a, Dir::Out, &[0xF0, 0x7E]);
        assert_eq!(c.drain(&mut store, 5), 4);
        let got: Vec<(PortIdx, Dir, Msg)> =
            store.events().map(|e| (e.port, e.dir, e.msg)).collect();
        assert_eq!(
            got,
            [
                (
                    b,
                    Dir::In,
                    Msg::Voice {
                        status: 0xB0,
                        d1: 7,
                        d2: 100
                    }
                ),
                (
                    a,
                    Dir::In,
                    Msg::Voice {
                        status: 0x90,
                        d1: 60,
                        d2: 1
                    }
                ),
                (
                    a,
                    Dir::In,
                    Msg::Voice {
                        status: 0x90,
                        d1: 61,
                        d2: 2
                    }
                ),
            ]
        );
        // The open SysEx on a's output ends after the timeout.
        c.drain(&mut store, 2_000_000_000);
        let last = store.events().last().unwrap();
        assert_eq!(last.dir, Dir::Out);
        assert!(matches!(
            last.msg,
            Msg::SysEx {
                len: 2,
                end: SysExEnd::Timeout,
                ..
            }
        ));
    }

    #[test]
    fn hid_reports_are_kept_as_they_are_and_drops_are_counted() {
        let mut store = Store::new(StoreConfig::default());
        let hid = store.port(PortRef::new(PortKind::Hid, "tec", "TEC"));
        let (mut tx, rx) = tap(TapConfig {
            slots: 1,
            bytes: 64,
        });
        let mut c = Collector::new(ParserConfig::default());
        c.add(rx);
        tx.push(at(1), hid, Dir::In, &[0x90, 0x01, 0x02]);
        tx.push(at(2), hid, Dir::In, &[0x00]);
        c.drain(&mut store, 3);
        let e = *store.events().next().unwrap();
        assert!(matches!(e.msg, Msg::Hid { len: 3, .. }));
        assert_eq!(
            store.bytes(e.msg.blob().unwrap()).unwrap(),
            [0x90, 0x01, 0x02]
        );
        assert_eq!(store.stats().dropped, 1);
    }

    #[test]
    fn describers_annotate_each_new_event_once() {
        struct Notes;
        impl Describer for Notes {
            fn describe(&mut self, store: &Store, event: &Event) -> Vec<Annotation> {
                let wire = store.wire(&event.msg).unwrap();
                vec![Annotation::State(format!("{} bytes", wire.len()))]
            }
        }
        let mut store = Store::default();
        let port = store.port(PortRef::new(PortKind::Alsa, "a", "A"));
        let (mut tx, rx) = tap(TapConfig::default());
        let mut c = Collector::new(ParserConfig::default());
        c.add(rx);
        c.describe_with(Box::new(Notes));
        tx.push(at(1), port, Dir::In, &[0x90, 60, 1, 0xF8]);
        c.drain(&mut store, 2);
        c.drain(&mut store, 3);
        let notes: Vec<_> = store
            .events()
            .map(|e| store.notes(e.seq).to_vec())
            .collect();
        assert_eq!(
            notes,
            [
                vec![Annotation::State("3 bytes".into())],
                vec![Annotation::State("1 bytes".into())],
            ]
        );
    }

    #[test]
    fn the_thread_collects_until_stopped() {
        let store = Arc::new(Mutex::new(Store::default()));
        let port = store
            .lock()
            .unwrap()
            .port(PortRef::new(PortKind::Editor, "e", "Editor"));
        let clock = Clock::new();
        let (mut tx, rx) = tap(TapConfig::default());
        let mut c = Collector::new(ParserConfig::default());
        c.add(rx);
        let running = c.spawn(store.clone(), clock, Duration::from_millis(1));
        tx.push(clock.now(), port, Dir::Out, &[0xC0, 3]);
        // A tap added while it runs.
        let (mut later, reader) = tap(TapConfig::default());
        running.add(reader);
        later.push(clock.now(), port, Dir::In, &[0xC0, 4]);
        for _ in 0..1000 {
            if store.lock().unwrap().events().len() == 2 {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        running.stop();
        assert_eq!(store.lock().unwrap().events().len(), 2);
    }
}
