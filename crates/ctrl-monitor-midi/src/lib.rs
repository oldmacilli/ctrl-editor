//! The monitor's producers on `ctrl-midi` ports (docs/MONITOR.md §6.1,
//! §6.2): watch any port without taking part in its traffic, offer a
//! virtual input other programs send to, and record what an editor sends.
//!
//! Each producer has its own [`Tap`]; the callbacks only push into it, so
//! they neither block nor allocate. One [`Monitor`] holds the store, the
//! clock and the running collector.
//!
//! ```no_run
//! use ctrl_midi::Backend;
//! use ctrl_monitor::StoreConfig;
//! use ctrl_monitor_midi::Monitor;
//!
//! let monitor = Monitor::start(StoreConfig::default(), Vec::new());
//! let backend = Backend::native();
//! let mut watches = Vec::new();
//! for port in ctrl_midi::ports(backend).unwrap() {
//!     if port.dir == ctrl_midi::Dir::In {
//!         watches.push(monitor.watch(backend, &port).unwrap());
//!     }
//! }
//! // monitor.store is what the views draw.
//! ```

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ctrl_midi::{Backend, Error, Output, Port};
use ctrl_monitor::{
    tap, Clock, Collector, Describer, Dir, ParserConfig, PortIdx, PortKind, PortRef, Running,
    Store, StoreConfig, Tap, TapConfig,
};

/// How often the collector drains the taps.
pub const DRAIN: Duration = Duration::from_millis(5);

/// The store's port kind for a backend; `None` where watching is not
/// supported (WinMM).
pub fn port_kind(backend: Backend) -> Option<PortKind> {
    match backend {
        Backend::Alsa => Some(PortKind::Alsa),
        Backend::Jack => Some(PortKind::Jack),
        Backend::CoreMidi => Some(PortKind::CoreMidi),
        Backend::WinMm => None,
    }
}

/// The name people read: JACK's display carries aliases after ` | `.
fn readable(port: &Port) -> String {
    port.display
        .split(" | ")
        .next()
        .unwrap_or(&port.display)
        .to_string()
}

/// A watched port or a virtual input; dropping it closes the port.
pub struct Watch {
    pub port: PortIdx,
    _input: ctrl_midi::Input,
}

/// The store, its clock and the collector filling it.
pub struct Monitor {
    pub store: Arc<Mutex<Store>>,
    pub clock: Clock,
    running: Running,
    tap: TapConfig,
}

impl Monitor {
    /// Starts the collector with these describers (e.g. a
    /// `ctrl_monitor::CdlDescriber`).
    pub fn start(config: StoreConfig, describers: Vec<Box<dyn Describer>>) -> Monitor {
        let store = Arc::new(Mutex::new(Store::new(config)));
        let clock = Clock::new();
        let mut collector = Collector::new(ParserConfig::default());
        for d in describers {
            collector.describe_with(d);
        }
        let running = collector.spawn(store.clone(), clock, DRAIN);
        Monitor {
            store,
            clock,
            running,
            tap: TapConfig::default(),
        }
    }

    fn port(&self, kind: PortKind, id: &str, name: String) -> PortIdx {
        let mut store = self.store.lock().unwrap_or_else(|e| e.into_inner());
        store.port(PortRef::new(kind, id, name))
    }

    fn tap(&self) -> Tap {
        let (tx, rx) = tap(self.tap);
        self.running.add(rx);
        tx
    }

    /// A callback that stamps and pushes what `port` delivers, as `dir`.
    fn callback(&self, port: PortIdx, dir: Dir) -> ctrl_midi::RawCallback {
        let mut tap = self.tap();
        let clock = self.clock;
        let opened = Instant::now();
        Box::new(move |micros, bytes| {
            let at = clock.at(opened + Duration::from_micros(micros));
            tap.push(at, port, dir, bytes);
        })
    }

    /// Reads `port`, an input as `ctrl_midi` lists it (a device's output or
    /// another program's), beside whoever else reads it (§6.1).
    pub fn watch(&self, backend: Backend, port: &Port) -> Result<Watch, Error> {
        let kind = port_kind(backend).ok_or_else(|| {
            Error::Unavailable(backend, "watching needs Windows MIDI Services".into())
        })?;
        let idx = self.port(kind, &port.name, readable(port));
        let input = ctrl_midi::watch(backend, port, self.callback(idx, Dir::In))?;
        Ok(Watch {
            port: idx,
            _input: input,
        })
    }

    /// A port named `name` other programs send to (§6.2).
    pub fn virtual_input(&self, backend: Backend, name: &str) -> Result<Watch, Error> {
        let idx = self.port(PortKind::Virtual, name, name.to_string());
        let input = ctrl_midi::virtual_input(backend, name, self.callback(idx, Dir::In))?;
        Ok(Watch {
            port: idx,
            _input: input,
        })
    }

    /// Wraps an editor's output to `port`: what it sends is recorded as
    /// going to that port, from the editor.
    pub fn recorded(&self, port: &Port, output: Output) -> RecordedOutput {
        let idx = self.port(PortKind::Editor, &port.name, readable(port));
        RecordedOutput {
            output,
            tap: self.tap(),
            port: idx,
            clock: self.clock,
        }
    }
}

/// An output whose messages also go to the monitor, as `Dir::Out` on a port
/// of kind `Editor`.
pub struct RecordedOutput {
    output: Output,
    tap: Tap,
    port: PortIdx,
    clock: Clock,
}

impl RecordedOutput {
    pub fn port(&self) -> PortIdx {
        self.port
    }

    pub fn send(&mut self, msg: &[u8]) -> Result<(), Error> {
        self.tap.push(self.clock.now(), self.port, Dir::Out, msg);
        self.output.send(msg)
    }

    /// The pieces of a long SysEx (CDL §8.2): the monitor's parser joins
    /// them into one message again.
    pub fn send_writes(&mut self, writes: &[ctrl_codec::Write]) -> Result<(), Error> {
        for w in writes {
            self.tap
                .push(self.clock.now(), self.port, Dir::Out, &w.bytes);
        }
        self.output.send_writes(writes)
    }

    pub fn into_inner(self) -> Output {
        self.output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backends_map_to_port_kinds() {
        assert_eq!(port_kind(Backend::Alsa), Some(PortKind::Alsa));
        assert_eq!(port_kind(Backend::Jack), Some(PortKind::Jack));
        assert_eq!(port_kind(Backend::CoreMidi), Some(PortKind::CoreMidi));
        assert_eq!(port_kind(Backend::WinMm), None);
    }

    #[test]
    fn the_readable_name_drops_jack_aliases() {
        let p = Port {
            name: "a2j:nanoKEY2 [24] (capture): nanoKEY2 MIDI 1".into(),
            display: "a2j:nanoKEY2 [24] (capture): nanoKEY2 MIDI 1 | alsa_pcm:nanoKEY2".into(),
            dir: ctrl_midi::Dir::In,
        };
        assert_eq!(readable(&p), "a2j:nanoKEY2 [24] (capture): nanoKEY2 MIDI 1");
    }

    #[test]
    fn callbacks_stamp_and_push_into_the_store() {
        let monitor = Monitor::start(StoreConfig::default(), Vec::new());
        let port = monitor.port(PortKind::Alsa, "x:0", "X".into());
        let mut cb = monitor.callback(port, Dir::In);
        cb(1_500, &[0x90, 60]);
        cb(2_000, &[100]);
        for _ in 0..1000 {
            if monitor.store.lock().unwrap().events().len() == 1 {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        let store = monitor.store.lock().unwrap();
        let e = store.events().next().copied().expect("one note");
        assert_eq!(
            e.msg,
            ctrl_monitor::Msg::Voice {
                status: 0x90,
                d1: 60,
                d2: 100
            }
        );
        assert_eq!(store.port_ref(e.port).unwrap().name, "X");
    }
}
