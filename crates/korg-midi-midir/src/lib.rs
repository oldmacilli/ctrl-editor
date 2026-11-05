//! Cross-platform MIDI adapter over `ctrl-midi`.
//!
//! `ctrl-midi` (the shared platform, ctrl-editor `docs/LINUX.md`) opens ports
//! on a backend chosen at run time: the ALSA sequencer or JACK (jackd2, or
//! PipeWire's libjack) on Linux, CoreMIDI or JACK on macOS, WinMM on
//! Windows.  This crate is the only place where a platform MIDI API reaches
//! the rewrite.
//!
//! The backend is process-wide ([`backend`], [`set_backend`]): every
//! [`MidirTransport`] lists and opens ports on it.  A transport whose
//! connection was opened on another backend closes it on the next `poll` and
//! reports `Disconnected`, so the editor reopens the device on the new one.

pub use ctrl_midi::Backend;
use ctrl_midi::{Dir, Port, SysexPolicy};
use korg_midi::{
    BackendInfo, Direction, MidiPacket, MidiTransport, PortId, PortInfo, TransportError,
    TransportEvent,
};
use midir::{MidiInput, MidiOutput};
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Every backend `ctrl-midi` knows, in a fixed order for [`BACKEND`].
const BACKENDS: [Backend; 4] = [
    Backend::Alsa,
    Backend::Jack,
    Backend::CoreMidi,
    Backend::WinMm,
];

/// Index into [`BACKENDS`]; `u8::MAX` until a front end chooses.
static BACKEND: AtomicU8 = AtomicU8::new(u8::MAX);

/// The backend every transport uses: the one [`set_backend`] chose, else
/// the platform's own.
pub fn backend() -> Backend {
    BACKENDS
        .get(usize::from(BACKEND.load(Ordering::Relaxed)))
        .copied()
        .unwrap_or_else(Backend::native)
}

/// Switch every transport to `backend`.  Open connections close on their
/// next `poll`.
pub fn set_backend(backend: Backend) {
    if let Some(index) = BACKENDS.iter().position(|known| *known == backend) {
        BACKEND.store(index as u8, Ordering::Relaxed);
    }
}

/// The backends this build can switch between, the platform's own first.
pub fn backends() -> Vec<Backend> {
    Backend::compiled()
}

/// Pick the backend at start: the command-line flag, then
/// `CTRL_MIDI_BACKEND`, then the saved preference, then the platform's own.
/// Only an unknown flag value is an error; an unknown environment or saved
/// value is skipped, so a preference from another build cannot lock the
/// editor out.
pub fn choose_backend(
    flag: Option<&str>,
    env: Option<&str>,
    saved: Option<&str>,
) -> Result<Backend, String> {
    if let Some(flag) = flag {
        return Backend::parse(flag).ok_or_else(|| {
            let known: Vec<&str> = backends().iter().map(|backend| backend.id()).collect();
            format!(
                "unknown MIDI backend {flag:?}; this build has: {}",
                known.join(", ")
            )
        });
    }
    Ok(env
        .and_then(Backend::parse)
        .or_else(|| saved.and_then(Backend::parse))
        .unwrap_or_else(Backend::native))
}

/// [`choose_backend`] with `CTRL_MIDI_BACKEND` from the environment, then
/// [`set_backend`].  Front ends call this once, before the first transport.
pub fn init_backend(flag: Option<&str>, saved: &str) -> Result<Backend, String> {
    let env = std::env::var("CTRL_MIDI_BACKEND").ok();
    let saved = Some(saved).filter(|saved| !saved.trim().is_empty());
    let chosen = choose_backend(flag, env.as_deref(), saved)?;
    set_backend(chosen);
    Ok(chosen)
}

/// What the front ends show about `backend`.  JACK is probed (a short-lived
/// client, never a server start), so `problem` says why it cannot run; the
/// native backend is not probed, its port scan reports errors.
pub fn backend_info(backend: Backend) -> BackendInfo {
    let problem = if backend == Backend::native() {
        None
    } else {
        backend.probe().err().map(|error| error.to_string())
    };
    BackendInfo {
        id: backend.id().to_owned(),
        label: backend.label().to_owned(),
        problem,
    }
}

/// Switch to the next compiled backend and describe it.
pub fn switch_to_next_backend() -> BackendInfo {
    let next = backend().next();
    set_backend(next);
    backend_info(next)
}

/// How often `poll` checks that the open input port still exists.
const LIVENESS_INTERVAL: Duration = Duration::from_secs(1);

/// Packets buffered between the MIDI callback and `poll`.  One push is
/// one complete message (ctrl-midi joins SysEx the host delivers in pieces), so a full queue drops a whole message, never
/// half of one; the SysEx assembler resynchronises on the next `F0`.
const QUEUE_DEPTH: usize = 256;

/// Queue one callback packet.  When the queue is full the newest packet is
/// dropped and counted; dropping the oldest would need a receiver here.
fn enqueue(sender: &SyncSender<MidiPacket>, dropped: &AtomicUsize, packet: MidiPacket) {
    if let Err(TrySendError::Full(_)) = sender.try_send(packet) {
        dropped.fetch_add(1, Ordering::Relaxed);
    }
}

/// SysEx start byte.  A chunked message keeps this byte on the first chunk.
const SYSEX_START: u8 = 0xf0;

/// SysEx end byte.  A chunked message keeps this byte on the last chunk.
const SYSEX_END: u8 = 0xf7;

/// Bytes per chunk when a SysEx is longer than this on the ALSA sequencer
/// backend.  midir 0.10.4
/// encodes a whole message into one ALSA sequencer event, and the kernel
/// `snd_seq_midi` port copies each event into its `output_buffer_size`
/// buffer (one page, 4096 bytes).  The port drops the tail of a larger
/// event and reports no error.  ALSA's own encoder already splits a long
/// SysEx at its encoder buffer size, so a chunk that starts mid-message is
/// still one continuous SysEx on the wire.
const SYSEX_CHUNK: usize = 256;

/// Pause between SysEx chunks on the ALSA sequencer backend, so the port buffer refills while the
/// hardware drains.  256 bytes per 5 ms is 51 KB/s, below a USB MIDI bulk
/// endpoint that moves one 64-byte packet per millisecond.
const SYSEX_CHUNK_PAUSE: Duration = Duration::from_millis(5);

/// Split one message for transmission.  A complete SysEx longer than
/// [`SYSEX_CHUNK`] becomes consecutive slices of at most that many bytes.
/// Every other message stays one slice.  Only the ALSA sequencer backend
/// uses this: CoreMIDI and WinMM keep one event per message, and JACK
/// carries every message as one event, never split.
fn transmission_chunks(message: &[u8]) -> Vec<&[u8]> {
    let split = message.len() > SYSEX_CHUNK
        && message.first() == Some(&SYSEX_START)
        && message.last() == Some(&SYSEX_END);
    if split {
        message.chunks(SYSEX_CHUNK).collect()
    } else {
        vec![message]
    }
}

/// What `send` hands to ctrl-midi.  The measured ALSA chunking stays in this
/// crate ([`transmission_chunks`]): ctrl-midi's own policy would split any
/// message that starts with `F0`, the measured rule only a complete SysEx.
/// For a complete SysEx both give the same pieces (tested below), so the
/// policy can move to ctrl-midi once the descriptors carry `sysex-limit`.
const SEND_POLICY: SysexPolicy = SysexPolicy {
    limit: None,
    delay_ms: 0,
};

/// One listed port: the name the backend opens it by, and its direction.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Listed {
    name: String,
    direction: Direction,
}

impl Listed {
    fn port(&self) -> Port {
        Port {
            name: self.name.clone(),
            display: ctrl_midi::display_name(&self.name),
            dir: match self.direction {
                Direction::Input => Dir::In,
                Direction::Output => Dir::Out,
            },
        }
    }
}

fn listed(ports: Vec<Port>) -> Vec<Listed> {
    ports
        .into_iter()
        .map(|port| Listed {
            name: port.name,
            direction: match port.dir {
                Dir::In => Direction::Input,
                Dir::Out => Direction::Output,
            },
        })
        .collect()
}

/// Port ids carry the index within their direction (`name#index`), so two
/// ports with one name stay apart in the list.
fn port_infos(ports: &[Listed]) -> Vec<PortInfo> {
    let mut infos = Vec::with_capacity(ports.len());
    for direction in [Direction::Input, Direction::Output] {
        for (index, port) in ports
            .iter()
            .filter(|port| port.direction == direction)
            .enumerate()
        {
            infos.push(PortInfo {
                id: PortId::new(format!("{}#{index}", port.name)),
                name: port.name.clone(),
                direction,
            });
        }
    }
    infos
}

fn unavailable(error: ctrl_midi::Error) -> TransportError {
    TransportError::BackendUnavailable(error.to_string())
}

pub struct MidirTransport {
    input: Option<ctrl_midi::Input>,
    output: Option<ctrl_midi::Output>,
    /// The backend the open connection runs on.
    bound: Option<Backend>,
    receiver: Receiver<MidiPacket>,
    sender: SyncSender<MidiPacket>,
    /// Packets the callback dropped because the queue was full.
    dropped: Arc<AtomicUsize>,
    /// A packet `wait` took off the channel; `poll` returns it first.
    pending: Option<MidiPacket>,
    open: bool,
    ports: Vec<PortInfo>,
    /// The open input port's name, for the liveness check.
    input_name: Option<String>,
    /// Long-lived clients that list input and output ports on the native
    /// backend.
    probe: Option<MidiInput>,
    lister: Option<MidiOutput>,
    checked_at: Instant,
}
#[cfg(target_os = "macos")]
mod coremidi {
    use std::ffi::c_void;

    type MidiObjectRef = u32;
    type OsStatus = i32;
    type CfStringRef = *const c_void;

    #[link(name = "CoreMIDI", kind = "framework")]
    extern "C" {
        fn MIDIGetDestination(index: usize) -> MidiObjectRef;
        fn MIDIObjectSetIntegerProperty(
            object: MidiObjectRef,
            property: CfStringRef,
            value: i32,
        ) -> OsStatus;
        static kMIDIPropertyIsEmbeddedEntity: CfStringRef;
        static kMIDIPropertyMaxSysExSpeed: CfStringRef;
    }

    /// Match the original editor: mark the destination embedded and raise the
    /// SysEx speed limit.  Failure is not fatal; the connection still works.
    pub fn prepare_destination(index: usize) {
        // SAFETY: the two property symbols are CoreMIDI CFStringRef globals;
        // the object comes from the matching index.  Return codes are ignored
        // because these are hints, not a connection precondition.
        unsafe {
            let object = MIDIGetDestination(index);
            if object == 0 {
                return;
            }
            let _ = MIDIObjectSetIntegerProperty(object, kMIDIPropertyIsEmbeddedEntity, 1);
            let _ = MIDIObjectSetIntegerProperty(object, kMIDIPropertyMaxSysExSpeed, 0x7A12);
        }
    }
}

impl MidirTransport {
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::sync_channel(QUEUE_DEPTH);
        Self {
            input: None,
            output: None,
            bound: None,
            receiver,
            sender,
            dropped: Arc::new(AtomicUsize::new(0)),
            pending: None,
            open: false,
            ports: Vec::new(),
            input_name: None,
            probe: None,
            lister: None,
            checked_at: Instant::now(),
        }
    }

    /// Packets the MIDI callback dropped because the queue was full.
    pub fn dropped_packets(&self) -> usize {
        self.dropped.load(Ordering::Relaxed)
    }

    /// The backend the open connection runs on, or the current one.
    pub fn backend(&self) -> Backend {
        self.bound.unwrap_or_else(backend)
    }

    /// List the ports of `backend`.  On the native backend this goes through
    /// two long-lived clients; creating CoreMIDI clients on every scan costs
    /// the UI thread each second.  A port that leaves while it is listed is
    /// skipped, not an error.  JACK lists through ctrl-midi.
    fn list(&mut self, backend: Backend) -> Result<Vec<Listed>, TransportError> {
        if backend != Backend::native() {
            return ctrl_midi::ports(backend).map(listed).map_err(unavailable);
        }
        if self.probe.is_none() {
            self.probe = Some(
                MidiInput::new("KORG Kontrol Editor probe")
                    .map_err(|error| TransportError::BackendUnavailable(error.to_string()))?,
            );
        }
        if self.lister.is_none() {
            self.lister = Some(
                MidiOutput::new("KORG Kontrol Editor probe")
                    .map_err(|error| TransportError::BackendUnavailable(error.to_string()))?,
            );
        }
        let (Some(input), Some(output)) = (self.probe.as_ref(), self.lister.as_ref()) else {
            return Err(TransportError::BackendUnavailable("no MIDI client".into()));
        };
        let mut ports = Vec::new();
        for port in input.ports() {
            if let Ok(name) = input.port_name(&port) {
                ports.push(Listed {
                    name,
                    direction: Direction::Input,
                });
            }
        }
        for port in output.ports() {
            if let Ok(name) = output.port_name(&port) {
                ports.push(Listed {
                    name,
                    direction: Direction::Output,
                });
            }
        }
        Ok(ports)
    }

    /// True when the open input port has left the system, as a Bluetooth
    /// port does when its link drops.  No backend reports such an event, so
    /// this lists the ports at most once per `LIVENESS_INTERVAL`.
    fn input_gone(&mut self) -> bool {
        let (Some(name), Some(bound)) = (self.input_name.clone(), self.bound) else {
            return false;
        };
        if self.checked_at.elapsed() < LIVENESS_INTERVAL {
            return false;
        }
        self.checked_at = Instant::now();
        let Ok(ports) = self.list(bound) else {
            return false;
        };
        !ports
            .iter()
            .any(|port| port.direction == Direction::Input && port.name == name)
    }

    /// Send one message on the ALSA sequencer: a long SysEx in measured
    /// chunks.  The call blocks for the chunks, up to a few hundred
    /// milliseconds for a scene set; ALSA gives no way to wait for the port
    /// buffer to drain, so the pause is a fixed delay between events.
    fn send_chunked(output: &mut ctrl_midi::Output, message: &[u8]) -> Result<(), TransportError> {
        for (index, chunk) in transmission_chunks(message).iter().enumerate() {
            if index > 0 {
                std::thread::sleep(SYSEX_CHUNK_PAUSE);
            }
            output
                .send(chunk)
                .map_err(|error| TransportError::SendFailed(error.to_string()))?;
        }
        Ok(())
    }
}

impl Default for MidirTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl MidiTransport for MidirTransport {
    fn list_ports(&mut self) -> Result<Vec<PortInfo>, TransportError> {
        let ports = port_infos(&self.list(backend())?);
        self.ports = ports.clone();
        Ok(ports)
    }

    fn open(&mut self, input: &PortId, output: &PortId) -> Result<(), TransportError> {
        self.close();
        let bound = backend();
        // Resolve against a fresh listing.  A Bluetooth port that comes or
        // goes shifts every index after it, so an older listing could name a
        // different port.
        let ports = ctrl_midi::ports(bound).map(listed).map_err(unavailable)?;
        let names = |direction| -> Vec<&Listed> {
            ports
                .iter()
                .filter(|port| port.direction == direction)
                .collect()
        };
        let inputs = names(Direction::Input);
        let outputs = names(Direction::Output);
        let in_names: Vec<String> = inputs.iter().map(|port| port.name.clone()).collect();
        let out_names: Vec<String> = outputs.iter().map(|port| port.name.clone()).collect();
        let input_index =
            resolve(&in_names, input).ok_or_else(|| TransportError::PortNotFound(input.clone()))?;
        let output_index = resolve(&out_names, output)
            .ok_or_else(|| TransportError::PortNotFound(output.clone()))?;

        let sender = self.sender.clone();
        let dropped = Arc::clone(&self.dropped);
        let connection = ctrl_midi::open_input(
            bound,
            &inputs[input_index].port(),
            Box::new(move |micros, message| {
                enqueue(
                    &sender,
                    &dropped,
                    MidiPacket {
                        bytes: message.to_vec(),
                        received_at_ms: micros / 1_000,
                    },
                );
            }),
        )
        .map_err(|error| match error {
            ctrl_midi::Error::Unavailable(..) => unavailable(error),
            _ => TransportError::PortOpenFailed(input.clone()),
        })?;

        #[cfg(target_os = "macos")]
        if bound == Backend::CoreMidi {
            coremidi::prepare_destination(output_index);
        }
        let output_connection =
            ctrl_midi::open_output(bound, &outputs[output_index].port(), SEND_POLICY).map_err(
                |error| match error {
                    ctrl_midi::Error::Unavailable(..) => unavailable(error),
                    _ => TransportError::PortOpenFailed(output.clone()),
                },
            )?;

        self.input = Some(connection);
        self.output = Some(output_connection);
        self.bound = Some(bound);
        self.open = true;
        self.input_name = Some(in_names[input_index].clone());
        self.checked_at = Instant::now();
        Ok(())
    }

    fn close(&mut self) {
        self.input = None;
        self.output = None;
        self.bound = None;
        self.open = false;
        self.input_name = None;
        // Replies still queued from this connection must not answer a
        // request on the next one.
        self.pending = None;
        while self.receiver.try_recv().is_ok() {}
    }

    fn is_open(&self) -> bool {
        self.open
    }

    fn send(&mut self, message: &[u8]) -> Result<(), TransportError> {
        let (Some(output), Some(bound)) = (self.output.as_mut(), self.bound) else {
            return Err(TransportError::NotOpen);
        };
        if bound == Backend::Alsa {
            return Self::send_chunked(output, message);
        }
        output
            .send(message)
            .map_err(|error| TransportError::SendFailed(error.to_string()))
    }

    fn poll(&mut self, out: &mut Vec<TransportEvent>) -> usize {
        let mut count = 0;
        for packet in self
            .pending
            .take()
            .into_iter()
            .chain(self.receiver.try_iter())
        {
            out.push(TransportEvent::Packet(packet));
            count += 1;
        }
        // The backend was switched: this connection belongs to the old one.
        let switched = self.bound.is_some_and(|bound| bound != backend());
        if switched || self.input_gone() {
            let port = PortId::new(self.input_name.clone().unwrap_or_default());
            self.close();
            out.push(TransportEvent::Disconnected { port });
            count += 1;
        }
        count
    }

    /// Sleep on the callback channel instead of spinning.  A Bluetooth reply
    /// takes tens of milliseconds; a spin loop burns a core for all of it.
    fn wait(&mut self, max_ms: u64) {
        if self.pending.is_some() {
            return;
        }
        if let Ok(packet) = self.receiver.recv_timeout(Duration::from_millis(max_ms)) {
            self.pending = Some(packet);
        }
    }
}

/// The index of the port `id` names in `names`: the listed index when the
/// name still sits there, else the first port with that name.  An id from an
/// older listing stays usable after other ports come or go.
fn resolve(names: &[String], id: &PortId) -> Option<usize> {
    let (name, index) = match id.0.rsplit_once('#') {
        Some((name, index)) => (name, index.parse::<usize>().ok()),
        None => (id.0.as_str(), None),
    };
    index
        .filter(|index| names.get(*index).is_some_and(|listed| listed == name))
        .or_else(|| names.iter().position(|listed| listed == name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_queue_drops_the_newest_packet_and_counts_it() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let dropped = AtomicUsize::new(0);
        let packet = |byte: u8| MidiPacket {
            bytes: vec![byte],
            received_at_ms: 0,
        };
        enqueue(&sender, &dropped, packet(1));
        enqueue(&sender, &dropped, packet(2));
        assert_eq!(dropped.load(Ordering::Relaxed), 1);
        assert_eq!(receiver.try_recv().unwrap().bytes, vec![1]);
        assert!(
            receiver.try_recv().is_err(),
            "the newest packet was dropped"
        );
    }

    #[test]
    fn a_port_id_survives_a_shifted_listing() {
        let names = |list: &[&str]| list.iter().map(|n| (*n).to_owned()).collect::<Vec<_>>();
        let id = PortId::new("nanoKONTROL Studio Bluetooth#11");
        let shifted = names(&["nanoKEY Studio Bluetooth", "nanoKONTROL Studio Bluetooth"]);
        assert_eq!(resolve(&shifted, &id), Some(1));
        let mut same = vec![String::new(); 11];
        same.push("nanoKONTROL Studio Bluetooth".into());
        assert_eq!(resolve(&same, &id), Some(11));
        assert_eq!(resolve(&names(&["UMC404HD 192k"]), &id), None);
        // Two ports with one name: the listed index wins.
        let twins = names(&["Port", "Port"]);
        assert_eq!(resolve(&twins, &PortId::new("Port#1")), Some(1));
    }

    /// A complete SysEx of `length` bytes: `F0`, payload, `F7`.
    fn sysex(length: usize) -> Vec<u8> {
        let mut message = vec![0u8; length];
        for (index, byte) in message.iter_mut().enumerate() {
            *byte = (index % 0x80) as u8;
        }
        message[0] = SYSEX_START;
        message[length - 1] = SYSEX_END;
        message
    }

    #[test]
    fn a_long_sysex_splits_into_chunks_that_reassemble() {
        let message = sysex(9_100);
        let chunks = transmission_chunks(&message);
        assert_eq!(chunks.len(), 9_100_usize.div_ceil(SYSEX_CHUNK));
        assert_eq!(chunks[0].first(), Some(&SYSEX_START));
        assert_eq!(chunks.last().unwrap().last(), Some(&SYSEX_END));
        assert!(
            chunks[..chunks.len() - 1]
                .iter()
                .all(|chunk| chunk.len() == SYSEX_CHUNK),
            "every chunk except the last is full"
        );
        let joined: Vec<u8> = chunks.concat();
        assert_eq!(joined, message);
    }

    #[test]
    fn a_sysex_that_fills_whole_chunks_ends_with_f7() {
        let message = sysex(SYSEX_CHUNK * 2);
        let chunks = transmission_chunks(&message);
        assert_eq!(chunks.len(), 2);
        assert!(chunks.iter().all(|chunk| chunk.len() == SYSEX_CHUNK));
        assert_eq!(chunks[1].last(), Some(&SYSEX_END));
        assert_eq!(chunks.concat(), message);
    }

    #[test]
    fn a_sysex_within_the_chunk_size_stays_in_one_piece() {
        for length in [2, 32, SYSEX_CHUNK] {
            let message = sysex(length);
            assert_eq!(transmission_chunks(&message), vec![&message[..]]);
        }
    }

    /// CDL conformance: the shared descriptors (ctrl-editor, pinned in the
    /// workspace `Cargo.toml`) say `sysex-limit=256 segment-delay=5` on every
    /// KORG transport. `Codec::plan` must cut a whole scene write, and the
    /// global write, into the same pieces this adapter writes on Linux, with
    /// the same pause between them.
    #[test]
    fn chunks_match_the_cdl_plan_for_studio_writes() {
        use ctrl_codec::{Bindings, Codec, Val};

        let mut long = 0;
        for (path, model, device_id) in [
            ("korg/nanokontrol-studio.ctrl.kdl", 0x137, 0x00),
            ("korg/nanokey-studio.ctrl.kdl", 0x136, 0x0F),
        ] {
            let descriptor = ctrl_descriptors::load(path).unwrap();
            let codec = Codec::new(&descriptor);
            let layout = korg_devices::measured_layout(model).unwrap();
            for (name, header, size) in [
                (
                    "scene-write",
                    layout.write_header.unwrap(),
                    layout.scene_len,
                ),
                (
                    "global-write",
                    layout.global_write_header.unwrap(),
                    layout.global_len,
                ),
            ] {
                let payload: Vec<u8> = (0..size).map(|i| (i * 37 + 11) as u8).collect();
                let frame =
                    korg_sysex::build_write_frame(device_id, model, header, &payload).unwrap();
                let args: Bindings = [
                    ("device-id".to_string(), Val::Int(device_id.into())),
                    ("data".to_string(), Val::Bytes(payload)),
                ]
                .into_iter()
                .collect();
                let plan = codec.plan(name, &args).unwrap();
                assert_eq!(plan.len(), 1, "{path} {name}: one message");
                assert_eq!(plan[0].frame, frame, "{path} {name}");
                let ours = transmission_chunks(&frame);
                let theirs: Vec<&[u8]> = plan[0].writes.iter().map(|w| &w.bytes[..]).collect();
                assert_eq!(ours, theirs, "{path} {name}");
                if ours.len() > 1 {
                    long += 1;
                    for write in &plan[0].writes {
                        assert_eq!(
                            u128::from(write.delay_ms),
                            SYSEX_CHUNK_PAUSE.as_millis(),
                            "{path} {name}"
                        );
                    }
                }
                assert_eq!(
                    codec.sysex_limit(name),
                    Some((SYSEX_CHUNK, SYSEX_CHUNK_PAUSE.as_millis() as u32)),
                    "{path} {name}"
                );
            }
        }
        // 651 bytes (256 + 256 + 139) and 286 bytes (256 + 30) scene writes;
        // the 158-byte global writes stay whole.
        assert_eq!(long, 2);
    }

    #[test]
    fn a_message_without_the_sysex_frame_stays_in_one_piece() {
        let note = vec![0x90, 60, 100];
        assert_eq!(transmission_chunks(&note), vec![&note[..]]);

        let mut unterminated = sysex(600);
        unterminated.pop();
        assert_eq!(
            transmission_chunks(&unterminated),
            vec![&unterminated[..]],
            "an F0 without a closing F7 is not split"
        );

        let mut unstarted = sysex(600);
        unstarted[0] = 0x01;
        assert_eq!(
            transmission_chunks(&unstarted),
            vec![&unstarted[..]],
            "an F7 without an opening F0 is not split"
        );
    }

    #[test]
    fn a_complete_sysex_splits_as_ctrl_midis_policy_would() {
        // The measured rule and ctrl-midi's `sysex-limit` give the same
        // pieces for every complete SysEx, so the policy can move there.
        for length in [2, SYSEX_CHUNK, SYSEX_CHUNK + 1, SYSEX_CHUNK * 2, 9_100] {
            let message = sysex(length);
            let ours: Vec<Vec<u8>> = transmission_chunks(&message)
                .into_iter()
                .map(<[u8]>::to_vec)
                .collect();
            assert_eq!(ours, ctrl_codec::split_sysex(&message, SYSEX_CHUNK));
        }
        assert_eq!(SYSEX_CHUNK_PAUSE, Duration::from_millis(5));
        assert_eq!(SEND_POLICY.limit, None, "ctrl-midi must not split again");
    }

    #[test]
    fn the_flag_wins_then_the_environment_then_the_saved_choice() {
        let native = Backend::native();
        assert_eq!(choose_backend(None, None, None), Ok(native));
        assert_eq!(
            choose_backend(Some(native.id()), Some("jack"), Some("jack")),
            Ok(native)
        );
        assert!(choose_backend(Some("bogus"), None, None)
            .unwrap_err()
            .contains(native.id()));
        // Unknown environment and saved values fall through.
        assert_eq!(
            choose_backend(None, Some("bogus"), Some("bogus")),
            Ok(native)
        );
        if backends().contains(&Backend::Jack) {
            assert_eq!(
                choose_backend(None, Some("pipewire"), Some(native.id())),
                Ok(Backend::Jack)
            );
            assert_eq!(choose_backend(None, None, Some("jack")), Ok(Backend::Jack));
            assert_eq!(
                choose_backend(None, Some(native.id()), Some("jack")),
                Ok(native)
            );
        }
    }

    #[test]
    fn port_ids_count_within_their_direction() {
        let port = |name: &str, direction| Listed {
            name: name.into(),
            direction,
        };
        let infos = port_infos(&[
            port("A", Direction::Input),
            port("B", Direction::Output),
            port("A", Direction::Input),
            port("C", Direction::Output),
        ]);
        let ids: Vec<&str> = infos.iter().map(|info| info.id.0.as_str()).collect();
        assert_eq!(ids, ["A#0", "A#1", "B#0", "C#1"]);
        assert_eq!(
            resolve(&["A".into(), "A".into()], &infos[1].id),
            Some(1),
            "a listed id resolves to its own port"
        );
    }

    #[test]
    fn a_closed_transport_reports_no_switch_and_refuses_to_send() {
        let mut transport = MidirTransport::new();
        let mut events = Vec::new();
        assert_eq!(transport.poll(&mut events), 0);
        assert_eq!(transport.send(&[0x90, 60, 1]), Err(TransportError::NotOpen));
    }
}
