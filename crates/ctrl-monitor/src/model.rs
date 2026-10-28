//! What the monitor records: ports, time stamps, messages, events
//! (docs/MONITOR.md §3).

use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// Where a port lives.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PortKind {
    Alsa,
    Jack,
    CoreMidi,
    WinMidiServices,
    Hid,
    UsbCapture,
    /// A port the monitor created for other programs.
    Virtual,
    /// A virtual port pair that forwards to a device (§6.3).
    Thru,
    /// A network peer: "rtp-midi", "bome", "mesh", …
    Network {
        protocol: String,
    },
    /// What an editor sends itself.
    Editor,
    /// Read from a file.
    File,
}

/// One port, as the views show it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PortRef {
    pub kind: PortKind,
    /// Stable across runs: ALSA client:port name, JACK full name, CoreMIDI
    /// unique id, peer address.
    pub id: String,
    /// What people read.
    pub name: String,
}

impl PortRef {
    pub fn new(kind: PortKind, id: impl Into<String>, name: impl Into<String>) -> Self {
        PortRef {
            kind,
            id: id.into(),
            name: name.into(),
        }
    }
}

/// Index into the store's port table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct PortIdx(pub u16);

/// The message's direction, seen from the port.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Dir {
    /// Received from the port.
    In,
    /// Sent to the port.
    Out,
    /// Forwarded by a thru port.
    Thru,
}

/// What a time stamp's `domain` counts in, beside host time.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Domain {
    #[default]
    Host,
    /// A sample position (plug-in hosts).
    Sample(u64),
    /// A beat position on the Link grid (the LightBow mesh).
    LinkBeat(f64),
    /// The sender asked for "now" (time stamp zero); `host_ns` is the arrival.
    DeviceZero,
}

/// When a message happened.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Stamp {
    /// Monotonic nanoseconds since the [`Clock`] started.
    pub host_ns: u64,
    /// Wall clock, milliseconds since the Unix epoch.
    pub wall_ms: i64,
    pub domain: Domain,
}

/// Turns `Instant`s into [`Stamp`]s with one origin, so every producer of a
/// store shares a time base. Cheap to copy; `now` does not allocate.
#[derive(Debug, Clone, Copy)]
pub struct Clock {
    origin: Instant,
    origin_wall_ms: i64,
}

impl Clock {
    pub fn new() -> Self {
        let wall = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        Clock {
            origin: Instant::now(),
            origin_wall_ms: wall,
        }
    }

    pub fn now(&self) -> Stamp {
        self.at(Instant::now())
    }

    /// A stamp for a backend's own packet time.
    pub fn at(&self, when: Instant) -> Stamp {
        let host_ns = when.saturating_duration_since(self.origin).as_nanos() as u64;
        Stamp {
            host_ns,
            wall_ms: self.origin_wall_ms + (host_ns / 1_000_000) as i64,
            domain: Domain::Host,
        }
    }
}

impl Default for Clock {
    fn default() -> Self {
        Clock::new()
    }
}

/// Index of a byte payload in the store's arena.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlobIdx(pub u64);

/// How a SysEx message ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SysExEnd {
    /// With F7.
    Complete,
    /// A status byte arrived before F7; it is the next message.
    Cut(u8),
    /// No byte arrived for the parser's timeout.
    Timeout,
    /// Longer than the parser's limit; the rest was not kept.
    Truncated,
}

/// One message. `Copy`: long payloads live in the store's arena.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Msg {
    /// 80..EF, running status resolved; `d2` is 0 for two-byte messages.
    Voice { status: u8, d1: u8, d2: u8 },
    /// F1 F2 F3 F6.
    Common { status: u8, d1: u8, d2: u8 },
    /// F8 FA FB FC FE FF.
    Realtime(u8),
    /// F0 … (F7 when complete), as received.
    SysEx {
        blob: BlobIdx,
        len: u32,
        end: SysExEnd,
    },
    /// MIDI 2.0: the packet and its word count.
    Ump([u32; 4], u8),
    /// A HID report.
    Hid { blob: BlobIdx, len: u32 },
    /// Bytes that are no message: data without a status, F4 F5 F7 F9 FD.
    Invalid { blob: BlobIdx, len: u32 },
}

impl Msg {
    /// 1..=16 for voice messages.
    pub fn channel(&self) -> Option<u8> {
        match *self {
            Msg::Voice { status, .. } => Some((status & 0x0F) + 1),
            _ => None,
        }
    }

    pub fn blob(&self) -> Option<BlobIdx> {
        match *self {
            Msg::SysEx { blob, .. } | Msg::Hid { blob, .. } | Msg::Invalid { blob, .. } => {
                Some(blob)
            }
            _ => None,
        }
    }
}

/// One recorded message.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Event {
    /// Per store, monotonic from 0; a gap means events were evicted.
    pub seq: u64,
    pub at: Stamp,
    pub port: PortIdx,
    pub dir: Dir,
    pub msg: Msg,
    /// The store holds annotations for this event ([`crate::Store::notes`]).
    pub annotated: bool,
}

/// An explanation attached to an event (§3.4).
#[derive(Debug, Clone, PartialEq)]
pub enum Annotation {
    /// Matched a descriptor message.
    Described {
        name: String,
        fields: Vec<(String, String)>,
    },
    /// The panel control that sent it.
    Control { label: String, id: String },
    /// A device state it changed or reports: "scene 3".
    State(String),
    /// A MeshGate / LBMT rule saw it.
    Rule {
        gate: String,
        action: String,
        rule: Option<String>,
    },
    /// The way it took through a network.
    Path {
        tier: String,
        hops: Vec<Hop>,
        latency_us: Option<u64>,
    },
    /// It arrived in this many pieces and was joined.
    Joined { pieces: u32 },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Hop {
    pub label: String,
    pub location: String,
    pub dt_us: Option<u64>,
}
