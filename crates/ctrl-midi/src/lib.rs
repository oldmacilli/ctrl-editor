//! MIDI ports for the editors, with the backend chosen at run time.
//!
//! | Backend | Platform | Through |
//! |---|---|---|
//! | `alsa` | Linux (default) | the kernel ALSA sequencer, via midir |
//! | `jack` | Linux, macOS | libjack loaded at run time: jackd2, or PipeWire's libjack |
//! | `coremidi` | macOS (default) | CoreMIDI, via midir |
//! | `winmm` | Windows (default) | WinMM, via midir |
//!
//! Choose with [`Backend::from_env`] (`CTRL_MIDI_BACKEND=alsa|jack`), a
//! command-line flag, or the editor's preference; `docs/LINUX.md` explains
//! the consequences. Both directions deliver whole messages: SysEx that the
//! host hands over in pieces is joined first ([`ctrl_codec::SysexJoiner`]).
//!
//! Sending follows the descriptor's `sysex-limit` (CDL §8.2) where the host
//! needs it: on ALSA a long SysEx goes out in pieces with a pause between
//! them, because the kernel's `snd_seq_midi` drops the tail of an event
//! over 4096 bytes. JACK carries every message as one event, never split;
//! the limit of a JACK-to-ALSA bridge is noted in `docs/LINUX.md`.

use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ctrl_codec::{Joined, SysexJoiner};

#[cfg(feature = "jack")]
mod jack_backend;

/// Largest SysEx the joiner accepts on input.
pub const MAX_SYSEX: usize = 1 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Backend {
    Alsa,
    Jack,
    CoreMidi,
    WinMm,
}

impl Backend {
    /// What midir uses on this platform.
    pub fn native() -> Self {
        if cfg!(target_os = "linux") {
            Backend::Alsa
        } else if cfg!(target_os = "macos") {
            Backend::CoreMidi
        } else {
            Backend::WinMm
        }
    }

    /// The backends this build can use, native first. Whether JACK works
    /// depends on a running server: see [`Backend::probe`].
    pub fn compiled() -> Vec<Backend> {
        let mut v = vec![Backend::native()];
        if cfg!(feature = "jack") && !cfg!(target_os = "windows") {
            v.push(Backend::Jack);
        }
        v
    }

    pub fn id(self) -> &'static str {
        match self {
            Backend::Alsa => "alsa",
            Backend::Jack => "jack",
            Backend::CoreMidi => "coremidi",
            Backend::WinMm => "winmm",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Backend::Alsa => "ALSA sequencer",
            Backend::Jack => "JACK",
            Backend::CoreMidi => "CoreMIDI",
            Backend::WinMm => "Windows MIDI",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "alsa" | "alsa-seq" => Some(Backend::Alsa),
            "jack" | "pipewire" | "pw-jack" => Some(Backend::Jack),
            "coremidi" => Some(Backend::CoreMidi),
            "winmm" | "windows" => Some(Backend::WinMm),
            _ => None,
        }
        .filter(|b| Self::compiled().contains(b))
    }

    /// `CTRL_MIDI_BACKEND`, when it names a compiled backend.
    pub fn from_env() -> Option<Self> {
        std::env::var("CTRL_MIDI_BACKEND")
            .ok()
            .and_then(|v| Self::parse(&v))
    }

    /// The next compiled backend, for a "switch" command.
    pub fn next(self) -> Self {
        let all = Self::compiled();
        let i = all.iter().position(|b| *b == self).unwrap_or(0);
        all[(i + 1) % all.len()]
    }

    /// Can this backend run now? JACK needs libjack and a server.
    pub fn probe(self) -> Result<(), Error> {
        ports(self).map(|_| ())
    }
}

impl fmt::Display for Backend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    /// The device sends, the editor receives.
    In,
    /// The editor sends.
    Out,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Port {
    /// The name the backend uses to open it.
    pub name: String,
    /// The name without backend decoration (ALSA's ` 24:0` address), for
    /// matching against a descriptor's `port-match`.
    pub display: String,
    pub dir: Dir,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The backend cannot run: no JACK server, libjack missing.
    Unavailable(Backend, String),
    NoPort(String),
    Backend(String),
    /// A message the backend cannot carry in one piece.
    TooLong {
        len: usize,
        max: usize,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Unavailable(b, why) => write!(f, "{b} is not available: {why}"),
            Error::NoPort(p) => write!(f, "no MIDI port {p:?}"),
            Error::Backend(e) => f.write_str(e),
            Error::TooLong { len, max } => {
                write!(
                    f,
                    "a {len}-byte message does not fit the port's {max}-byte events"
                )
            }
        }
    }
}

impl std::error::Error for Error {}

/// How long SysEx is written (CDL §8.2, from the transport's `sysex-limit`
/// and `segment-delay`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SysexPolicy {
    pub limit: Option<usize>,
    pub delay_ms: u32,
}

/// Removes ALSA's sequencer address (`… MIDI 1 24:0`) from a midir name.
pub fn display_name(name: &str) -> String {
    let t = name.trim_end();
    if let Some((head, addr)) = t.rsplit_once(' ') {
        let mut parts = addr.split(':');
        let numeric = |p: Option<&str>| {
            p.is_some_and(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
        };
        // midir names an ALSA port "client:port 24:0"; only then is it an address
        if head.contains(':')
            && numeric(parts.next())
            && numeric(parts.next())
            && parts.next().is_none()
        {
            return head.to_string();
        }
    }
    t.to_string()
}

/// Every port of the backend.
pub fn ports(backend: Backend) -> Result<Vec<Port>, Error> {
    match backend {
        Backend::Jack => jack_ports(),
        b if b == Backend::native() => midir_ports(),
        b => Err(Error::Unavailable(b, "not on this platform".into())),
    }
}

/// The ports whose display name contains `pattern` (a descriptor's
/// `port-match`).
pub fn find(backend: Backend, dir: Dir, pattern: &str) -> Result<Vec<Port>, Error> {
    Ok(ports(backend)?
        .into_iter()
        .filter(|p| p.dir == dir && p.display.contains(pattern))
        .collect())
}

/// Called with microseconds since the input opened and one whole message.
pub type Callback = Box<dyn FnMut(u64, &[u8]) + Send + 'static>;

/// An open input; dropping it closes the port.
pub struct Input {
    _inner: Box<dyn Send>,
}

/// An open output.
pub struct Output {
    inner: OutputInner,
    policy: SysexPolicy,
}

enum OutputInner {
    Midir(midir::MidiOutputConnection),
    #[cfg(feature = "jack")]
    Jack(jack_backend::JackOutput),
}

/// Joins pieces and hands whole messages to the callback.
struct Deliver {
    joiner: SysexJoiner,
    cb: Callback,
    start: Instant,
}

impl Deliver {
    fn new(cb: Callback) -> Self {
        Self {
            joiner: SysexJoiner::new(MAX_SYSEX),
            cb,
            start: Instant::now(),
        }
    }

    fn push(&mut self, bytes: &[u8]) {
        let t = self.start.elapsed().as_micros() as u64;
        for j in self.joiner.push(bytes) {
            match j {
                Joined::Message(m) => (self.cb)(t, &m),
                Joined::RealTime(b) => (self.cb)(t, &[b]),
                Joined::Broken(_) => {}
            }
        }
    }
}

pub fn open_input(backend: Backend, port: &Port, cb: Callback) -> Result<Input, Error> {
    match backend {
        #[cfg(feature = "jack")]
        Backend::Jack => {
            let deliver = Arc::new(Mutex::new(Deliver::new(cb)));
            let inner = jack_backend::open_input(&port.name, deliver)?;
            Ok(Input {
                _inner: Box::new(inner),
            })
        }
        b if b == Backend::native() => {
            let mut mi =
                midir::MidiInput::new("ctrl-editor").map_err(|e| Error::Backend(e.to_string()))?;
            mi.ignore(midir::Ignore::None);
            let p = mi
                .ports()
                .into_iter()
                .find(|p| mi.port_name(p).is_ok_and(|n| n == port.name))
                .ok_or_else(|| Error::NoPort(port.name.clone()))?;
            let mut deliver = Deliver::new(cb);
            let conn = mi
                .connect(
                    &p,
                    "ctrl-editor-in",
                    move |_, bytes, _| deliver.push(bytes),
                    (),
                )
                .map_err(|e| Error::Backend(e.to_string()))?;
            Ok(Input {
                _inner: Box::new(conn),
            })
        }
        b => Err(Error::Unavailable(b, "not on this platform".into())),
    }
}

pub fn open_output(backend: Backend, port: &Port, policy: SysexPolicy) -> Result<Output, Error> {
    let inner = match backend {
        #[cfg(feature = "jack")]
        Backend::Jack => OutputInner::Jack(jack_backend::open_output(&port.name)?),
        b if b == Backend::native() => {
            let mo =
                midir::MidiOutput::new("ctrl-editor").map_err(|e| Error::Backend(e.to_string()))?;
            let p = mo
                .ports()
                .into_iter()
                .find(|p| mo.port_name(p).is_ok_and(|n| n == port.name))
                .ok_or_else(|| Error::NoPort(port.name.clone()))?;
            OutputInner::Midir(
                mo.connect(&p, "ctrl-editor-out")
                    .map_err(|e| Error::Backend(e.to_string()))?,
            )
        }
        b => return Err(Error::Unavailable(b, "not on this platform".into())),
    };
    Ok(Output { inner, policy })
}

impl Output {
    /// Sends one whole message. On midir backends a SysEx longer than the
    /// policy's limit goes out in pieces with the policy's pause between
    /// them; JACK sends it as one event.
    pub fn send(&mut self, msg: &[u8]) -> Result<(), Error> {
        match &mut self.inner {
            OutputInner::Midir(conn) => {
                let pieces = match self.policy.limit {
                    Some(l) if msg.first() == Some(&0xF0) => ctrl_codec::split_sysex(msg, l),
                    _ => vec![msg.to_vec()],
                };
                let n = pieces.len();
                for (i, p) in pieces.iter().enumerate() {
                    conn.send(p).map_err(|e| Error::Backend(e.to_string()))?;
                    if i + 1 < n && self.policy.delay_ms > 0 {
                        std::thread::sleep(Duration::from_millis(u64::from(self.policy.delay_ms)));
                    }
                }
                Ok(())
            }
            #[cfg(feature = "jack")]
            OutputInner::Jack(j) => j.send(msg),
        }
    }
}

fn midir_ports() -> Result<Vec<Port>, Error> {
    let mut out = Vec::new();
    let mi =
        midir::MidiInput::new("ctrl-editor-scan").map_err(|e| Error::Backend(e.to_string()))?;
    for p in mi.ports() {
        if let Ok(name) = mi.port_name(&p) {
            out.push(Port {
                display: display_name(&name),
                name,
                dir: Dir::In,
            });
        }
    }
    let mo =
        midir::MidiOutput::new("ctrl-editor-scan").map_err(|e| Error::Backend(e.to_string()))?;
    for p in mo.ports() {
        if let Ok(name) = mo.port_name(&p) {
            out.push(Port {
                display: display_name(&name),
                name,
                dir: Dir::Out,
            });
        }
    }
    Ok(out)
}

#[cfg(feature = "jack")]
fn jack_ports() -> Result<Vec<Port>, Error> {
    jack_backend::ports()
}

#[cfg(not(feature = "jack"))]
fn jack_ports() -> Result<Vec<Port>, Error> {
    Err(Error::Unavailable(
        Backend::Jack,
        "built without the `jack` feature".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alsa_address_is_not_part_of_the_name() {
        assert_eq!(
            display_name("UC-33 USB MIDI Controller:UC-33 USB MIDI Controller MIDI 1 24:0"),
            "UC-33 USB MIDI Controller:UC-33 USB MIDI Controller MIDI 1"
        );
        assert_eq!(
            display_name("UC-33 USB MIDI Controller Anschluss 1"),
            "UC-33 USB MIDI Controller Anschluss 1"
        );
        assert_eq!(display_name("Port 1:2"), "Port 1:2");
    }

    #[test]
    fn backend_names() {
        assert_eq!(
            Backend::parse("PipeWire"),
            Backend::compiled()
                .contains(&Backend::Jack)
                .then_some(Backend::Jack)
        );
        assert_eq!(
            Backend::parse(Backend::native().id()),
            Some(Backend::native())
        );
        assert_eq!(Backend::native().next().next(), Backend::native());
    }

    #[test]
    fn deliver_joins_split_sysex() {
        let got = Arc::new(Mutex::new(Vec::new()));
        let g = got.clone();
        let mut d = Deliver::new(Box::new(move |_, m| g.lock().unwrap().push(m.to_vec())));
        d.push(&[0xF0, 1, 2]);
        d.push(&[3, 0xF7, 0xB0, 7, 9]);
        assert_eq!(
            *got.lock().unwrap(),
            vec![vec![0xF0, 1, 2, 3, 0xF7], vec![0xB0, 7, 9]]
        );
    }
}
