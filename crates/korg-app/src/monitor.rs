//! Follow the device's active scene.
//!
//! No request returns the active scene; the official editor never knows it
//! (the `0x4F` notification is parsed and dropped by the official editor).
//! The device announces every change on all its ports with
//! `F0 42 4g 00 01 37 02 00 00 4F <scene> F7`, scene 0-based (measured
//! 2026-10-21 on the nanoKONTROL Studio, USB and Bluetooth).  The monitor
//! keeps a connection open and records the last announcement.
//!
//! The same connection feeds the MIDI monitor: every message the device
//! sends is decoded (`crate::live`), kept in `log`, and handed out once by
//! `take_live` so a front end can light the control that sent it.
//!
//! A worker thread owns the transport: `list_ports` and `open` can take tens
//! of milliseconds (longer over Bluetooth), and they must not stall the UI
//! that calls `connect`.  `poll` only drains the worker's messages and is
//! non-blocking.  The transport is created *on* the worker, so a platform
//! transport that is not `Send` (midir's CoreMIDI client) never crosses
//! threads.

use crate::device::find_port;
use crate::live::{LiveMessage, LiveParser};
use korg_midi::{Direction, MidiTransport, TransportEvent};
use std::collections::VecDeque;
use std::marker::PhantomData;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

/// How long the monitor waits between attempts to reopen a lost link.
const RETRY_INTERVAL: Duration = Duration::from_secs(2);
/// How often the worker reads the transport when no command arrives.
const POLL_INTERVAL: Duration = Duration::from_millis(10);
/// Messages the monitor log keeps.
pub const LOG_LEN: usize = 200;

/// One message the device sent, stamped in milliseconds since the monitor
/// started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveEvent {
    pub at_ms: u64,
    pub message: LiveMessage,
}

impl LiveEvent {
    /// `12.345` seconds, for a log line.
    pub fn time_text(&self) -> String {
        format!("{}.{:03}", self.at_ms / 1000, self.at_ms % 1000)
    }
}

/// The scene of a scene-change announcement.
pub fn parse_scene_change(frame: &[u8]) -> Option<u8> {
    match frame {
        [0xF0, 0x42, channel, 0x00, 0x01, _, 0x02, 0x00, 0x00, 0x4F, scene, 0xF7]
            if channel & 0xF0 == 0x40 && *scene < 0x80 =>
        {
            Some(*scene)
        }
        _ => None,
    }
}

/// What the worker tells the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
struct State {
    ports: Option<(String, String)>,
    active: Option<u8>,
}

/// What the worker sends: link state, or messages the device sent.
enum Report {
    State(State),
    Live(Vec<LiveEvent>),
}

/// What the UI asks the worker to do.
enum Command {
    /// Listen on this pair, or stop listening (`None`).
    Want(Option<(String, String)>),
    /// The editor itself selected a scene.
    Active(u8),
}

pub struct SceneMonitor<T: MidiTransport> {
    commands: Sender<Command>,
    messages: Receiver<Report>,
    /// The pair `connect` asked for, for the same-pair early return.
    wanted: Option<(String, String)>,
    /// The port pair the worker has open, once it confirmed the open.
    pub ports: Option<(String, String)>,
    /// The last announced scene, 0-based; `None` until the device changes
    /// scene while the monitor listens.
    pub active: Option<u8>,
    /// The last `LOG_LEN` messages, oldest first.
    pub log: VecDeque<LiveEvent>,
    /// Messages not yet handed out by `take_live`.
    fresh: Vec<LiveEvent>,
    /// `T` lives on the worker; the marker keeps the type in the API.
    transport: PhantomData<fn() -> T>,
}

impl<T: MidiTransport + 'static> SceneMonitor<T> {
    /// Start the worker.  The transport is built there, so it never has to be
    /// `Send`; pass a constructor, not a value.
    pub fn new<F>(make_transport: F) -> Self
    where
        F: FnOnce() -> T + Send + 'static,
    {
        let (commands, command_rx) = mpsc::channel();
        let (message_tx, messages) = mpsc::channel();
        let _ = thread::Builder::new()
            .name("korg-scene-monitor".into())
            .spawn(move || Worker::new(make_transport(), command_rx, message_tx).run());
        Self {
            commands,
            messages,
            wanted: None,
            ports: None,
            active: None,
            log: VecDeque::new(),
            fresh: Vec::new(),
            transport: PhantomData,
        }
    }
}

impl<T: MidiTransport> SceneMonitor<T> {
    /// Listen on this port pair.  Same pair: nothing to do.  A new pair
    /// forgets the scene, which belongs to the old connection.  The open
    /// happens on the worker; `poll` reports it.
    pub fn connect(&mut self, input: &str, output: &str) -> Result<(), String> {
        let pair = (input.to_owned(), output.to_owned());
        if self.wanted.as_ref() == Some(&pair) {
            return Ok(());
        }
        // States queued for the old pair must not be applied after this one.
        self.drain();
        self.wanted = Some(pair.clone());
        self.ports = None;
        self.active = None;
        self.commands
            .send(Command::Want(Some(pair)))
            .map_err(|_| "the scene monitor thread is not running".to_owned())
    }

    pub fn disconnect(&mut self) {
        self.wanted = None;
        self.ports = None;
        self.active = None;
        self.drain();
        let _ = self.commands.send(Command::Want(None));
    }

    /// Read what the worker reported.  True when the active scene changed.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        let reports: Vec<Report> = self.messages.try_iter().collect();
        for report in reports {
            match report {
                Report::State(state) => {
                    self.ports = state.ports;
                    if state.active != self.active {
                        self.active = state.active;
                        changed = true;
                    }
                }
                Report::Live(events) => {
                    for event in events {
                        if self.log.len() == LOG_LEN {
                            self.log.pop_front();
                        }
                        self.log.push_back(event.clone());
                        if self.fresh.len() == LOG_LEN {
                            self.fresh.remove(0);
                        }
                        self.fresh.push(event);
                    }
                }
            }
        }
        changed
    }

    /// The messages `poll` read since the last call, oldest first.
    pub fn take_live(&mut self) -> Vec<LiveEvent> {
        std::mem::take(&mut self.fresh)
    }

    pub fn clear_log(&mut self) {
        self.log.clear();
        self.fresh.clear();
    }

    /// Record a scene the editor itself selected (after a job restores it).
    /// The device does not announce a host-selected scene.
    pub fn set_active(&mut self, scene: u8) {
        self.drain();
        self.active = Some(scene);
        let _ = self.commands.send(Command::Active(scene));
    }

    fn drain(&mut self) {
        for _ in self.messages.try_iter() {}
    }
}

/// Open `input` and `output` by name on `transport`.
fn open_pair<T: MidiTransport>(transport: &mut T, input: &str, output: &str) -> Result<(), String> {
    let ports = transport.list_ports().map_err(|e| e.to_string())?;
    let input_id = find_port(&ports, input, Direction::Input)?;
    let output_id = find_port(&ports, output, Direction::Output)?;
    transport
        .open(&input_id, &output_id)
        .map_err(|e| e.to_string())
}

/// Owns the transport for the whole lifetime of the monitor.
struct Worker<T: MidiTransport> {
    transport: T,
    commands: Receiver<Command>,
    messages: Sender<Report>,
    wanted: Option<(String, String)>,
    ports: Option<(String, String)>,
    active: Option<u8>,
    parser: LiveParser,
    retried_at: Option<Instant>,
    started: Instant,
}

impl<T: MidiTransport> Worker<T> {
    fn new(transport: T, commands: Receiver<Command>, messages: Sender<Report>) -> Self {
        Self {
            transport,
            commands,
            messages,
            wanted: None,
            ports: None,
            active: None,
            parser: LiveParser::default(),
            retried_at: None,
            started: Instant::now(),
        }
    }

    fn run(mut self) {
        loop {
            loop {
                match self.commands.try_recv() {
                    Ok(command) => self.apply(command),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return,
                }
            }
            if self.ports.is_none() {
                self.retry();
            }
            if self.ports.is_some() {
                self.read_events();
            }
            match self.commands.recv_timeout(POLL_INTERVAL) {
                Ok(command) => self.apply(command),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }

    fn apply(&mut self, command: Command) {
        match command {
            Command::Want(wanted) => self.set_wanted(wanted),
            Command::Active(scene) => {
                self.active = Some(scene);
                self.publish();
            }
        }
    }

    fn set_wanted(&mut self, wanted: Option<(String, String)>) {
        if self.wanted == wanted && (wanted.is_none() || self.ports.is_some()) {
            return;
        }
        self.wanted = wanted;
        self.close_link();
        self.publish();
    }

    /// Close the connection but keep `wanted`, so the retry reopens it.
    fn close_link(&mut self) {
        if self.ports.take().is_some() {
            self.transport.close();
        }
        self.parser = LiveParser::default();
        self.active = None;
        self.retried_at = None;
    }

    /// Reopen the wanted pair after a lost link, at most once per
    /// `RETRY_INTERVAL` after a failed attempt.  A Bluetooth device comes back
    /// under the same names.
    fn retry(&mut self) {
        let Some((input, output)) = self.wanted.clone() else {
            return;
        };
        if self
            .retried_at
            .is_some_and(|at| at.elapsed() < RETRY_INTERVAL)
        {
            return;
        }
        match open_pair(&mut self.transport, &input, &output) {
            Ok(()) => {
                self.ports = Some((input, output));
                self.retried_at = None;
                self.publish();
            }
            Err(_) => {
                self.transport.close();
                self.retried_at = Some(Instant::now());
            }
        }
    }

    fn read_events(&mut self) {
        let mut events = Vec::new();
        self.transport.poll(&mut events);
        let before = self.active;
        let mut changed = false;
        let mut live = Vec::new();
        let at_ms = self.started.elapsed().as_millis() as u64;
        for event in events {
            match event {
                TransportEvent::Packet(packet) => {
                    for message in self.parser.push(&packet.bytes) {
                        if let LiveMessage::SysEx(frame) = &message {
                            if let Some(scene) = parse_scene_change(frame) {
                                self.active = Some(scene);
                            }
                        }
                        live.push(LiveEvent { at_ms, message });
                    }
                }
                TransportEvent::Disconnected { .. } => {
                    self.close_link();
                    changed = true;
                }
                TransportEvent::PortsChanged => {}
            }
        }
        if !live.is_empty() {
            let _ = self.messages.send(Report::Live(live));
        }
        if changed || self.active != before {
            self.publish();
        }
    }

    fn publish(&mut self) {
        let _ = self.messages.send(Report::State(State {
            ports: self.ports.clone(),
            active: self.active,
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use korg_midi::{MockTransport, PortId, PortInfo};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn ports() -> Vec<PortInfo> {
        [Direction::Input, Direction::Output]
            .into_iter()
            .map(|direction| PortInfo {
                id: PortId::new("nanoKONTROL Studio CTRL#0"),
                name: "nanoKONTROL Studio CTRL".into(),
                direction,
            })
            .collect()
    }

    /// Poll until `done` holds; the worker publishes asynchronously.
    fn wait<T: MidiTransport, F>(monitor: &mut SceneMonitor<T>, mut done: F)
    where
        F: FnMut(&SceneMonitor<T>) -> bool,
    {
        for _ in 0..2_000 {
            monitor.poll();
            if done(monitor) {
                return;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        panic!("the scene monitor did not reach the expected state");
    }

    #[test]
    fn parses_the_measured_announcement() {
        let frame = [
            0xF0, 0x42, 0x40, 0x00, 0x01, 0x37, 0x02, 0x00, 0x00, 0x4F, 0x04, 0xF7,
        ];
        assert_eq!(parse_scene_change(&frame), Some(4));
        let mut ack = frame;
        ack[9] = 0x23;
        assert_eq!(parse_scene_change(&ack), None);
        assert_eq!(parse_scene_change(&[0xB1, 0x15, 0x7F]), None);
    }

    #[test]
    fn follows_announcements_and_ignores_other_messages() {
        let mut transport = MockTransport::new(ports());
        transport.push_packet(vec![0xB1, 0x15, 0x7F]);
        transport.push_packet(vec![
            0xF0, 0x42, 0x40, 0x00, 0x01, 0x37, 0x02, 0x00, 0x00, 0x4F, 0x02, 0xF7,
        ]);
        let mut monitor = SceneMonitor::new(move || transport);
        assert!(!monitor.poll(), "not connected yet");
        monitor
            .connect("nanoKONTROL Studio CTRL", "nanoKONTROL Studio CTRL")
            .unwrap();
        wait(&mut monitor, |monitor| monitor.active == Some(2));
        assert_eq!(monitor.active, Some(2));
        assert!(!monitor.poll());
        // Both messages reach the MIDI monitor, the CC first.
        let live = monitor.take_live();
        assert_eq!(live.len(), 2);
        assert_eq!(live[0].message.describe(), "CC 21 = 127 · ch 2");
        assert_eq!(monitor.log.len(), 2);
        assert!(monitor.take_live().is_empty(), "handed out once");
        monitor.disconnect();
        assert_eq!(monitor.active, None);
    }

    /// A `MockTransport` that counts `open` calls, so the test can see the
    /// reopen after a disconnect without catching the transient closed state.
    struct CountingTransport {
        inner: MockTransport,
        opens: Arc<AtomicUsize>,
    }

    impl MidiTransport for CountingTransport {
        fn list_ports(&mut self) -> Result<Vec<PortInfo>, korg_midi::TransportError> {
            self.inner.list_ports()
        }
        fn open(
            &mut self,
            input: &PortId,
            output: &PortId,
        ) -> Result<(), korg_midi::TransportError> {
            self.opens.fetch_add(1, Ordering::Relaxed);
            self.inner.open(input, output)
        }
        fn close(&mut self) {
            self.inner.close();
        }
        fn is_open(&self) -> bool {
            self.inner.is_open()
        }
        fn send(&mut self, message: &[u8]) -> Result<(), korg_midi::TransportError> {
            self.inner.send(message)
        }
        fn poll(&mut self, out: &mut Vec<TransportEvent>) -> usize {
            self.inner.poll(out)
        }
        fn wait(&mut self, max_ms: u64) {
            self.inner.wait(max_ms);
        }
    }

    #[test]
    fn reopens_the_link_after_a_disconnect() {
        let mut inner = MockTransport::new(ports());
        inner.push_packet(vec![
            0xF0, 0x42, 0x40, 0x00, 0x01, 0x37, 0x02, 0x00, 0x00, 0x4F, 0x01, 0xF7,
        ]);
        inner.push_event(TransportEvent::Disconnected {
            port: PortId::new("nanoKONTROL Studio CTRL#0"),
        });
        let opens = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&opens);
        let mut monitor = SceneMonitor::new(move || CountingTransport {
            inner,
            opens: counter,
        });
        monitor
            .connect("nanoKONTROL Studio CTRL", "nanoKONTROL Studio CTRL")
            .unwrap();
        wait(&mut monitor, |monitor| monitor.ports.is_some());
        wait(&mut monitor, |_| opens.load(Ordering::Relaxed) >= 2);
        assert_eq!(monitor.active, None, "the lost link was closed");
        monitor.disconnect();
        monitor.poll();
        assert_eq!(monitor.ports, None, "an explicit disconnect stays off");
    }

    #[test]
    fn connect_returns_before_the_open_finishes() {
        struct SlowTransport {
            ports: Vec<PortInfo>,
            open: bool,
        }
        impl MidiTransport for SlowTransport {
            fn list_ports(&mut self) -> Result<Vec<PortInfo>, korg_midi::TransportError> {
                Ok(self.ports.clone())
            }
            fn open(
                &mut self,
                _input: &PortId,
                _output: &PortId,
            ) -> Result<(), korg_midi::TransportError> {
                std::thread::sleep(Duration::from_millis(300));
                self.open = true;
                Ok(())
            }
            fn close(&mut self) {
                self.open = false;
            }
            fn is_open(&self) -> bool {
                self.open
            }
            fn send(&mut self, _message: &[u8]) -> Result<(), korg_midi::TransportError> {
                Ok(())
            }
            fn poll(&mut self, _out: &mut Vec<TransportEvent>) -> usize {
                0
            }
        }

        let ports = ports();
        let mut monitor = SceneMonitor::new(move || SlowTransport { ports, open: false });
        let started = Instant::now();
        monitor
            .connect("nanoKONTROL Studio CTRL", "nanoKONTROL Studio CTRL")
            .unwrap();
        assert!(
            started.elapsed() < Duration::from_millis(100),
            "connect blocked for {:?}",
            started.elapsed()
        );
        wait(&mut monitor, |monitor| monitor.ports.is_some());
    }
}
