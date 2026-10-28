//! JACK: one client per open port, a lock-free ring buffer between the
//! process callback and the editor. libjack is loaded at run time, so the
//! same binary uses jackd2 or PipeWire's libjack (`pw-jack`, or the
//! distribution's pipewire-jack package) and fails cleanly without either.
//!
//! The process callback allocates nothing: messages cross the ring buffer
//! as `[len: u32 le][bytes]` records, copied into buffers made before the
//! client is activated.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use jack::{
    Client, ClientOptions, Control, MidiIn, MidiOut, PortFlags, ProcessHandler, ProcessScope,
    RawMidi, RingBuffer, RingBufferReader, RingBufferWriter,
};

use crate::{Backend, Deliver, Dir, Error, Port};

const RING: usize = 1 << 20;
/// The largest single message the editor moves through JACK.
const SCRATCH: usize = 64 * 1024;
const MIDI_TYPE: &str = "8 bit raw midi";

fn unavailable(e: impl std::fmt::Display) -> Error {
    Error::Unavailable(Backend::Jack, e.to_string())
}

fn client(name: &str) -> Result<Client, Error> {
    // never start a server behind the user's back
    Client::new(name, ClientOptions::NO_START_SERVER)
        .map(|(c, _)| c)
        .map_err(|e| match e {
            jack::Error::LibraryError(_) => {
                unavailable("libjack is not installed (jackd2, or PipeWire's pipewire-jack)")
            }
            jack::Error::ClientError(_) => unavailable(
                "no JACK server answers (start jackd, or run under PipeWire with pw-jack)",
            ),
            e => unavailable(e),
        })
}

pub fn ports() -> Result<Vec<Port>, Error> {
    let c = client("ctrl-editor-scan")?;
    let own = |n: &str| n.starts_with("ctrl-editor");
    let mut out = Vec::new();
    // a port that outputs MIDI is one the editor reads from
    for (flags, dir) in [
        (PortFlags::IS_OUTPUT, Dir::In),
        (PortFlags::IS_INPUT, Dir::Out),
    ] {
        for name in c.ports(None, Some(MIDI_TYPE), flags) {
            if own(&name) {
                continue;
            }
            // PipeWire and a2jmidid put the device name in the port name;
            // aliases carry the ALSA names, so matching sees both
            let aliases = c
                .port_by_name(&name)
                .and_then(|p| p.aliases().ok())
                .unwrap_or_default();
            let display = std::iter::once(name.clone())
                .chain(aliases)
                .collect::<Vec<_>>()
                .join(" | ");
            out.push(Port { name, display, dir });
        }
    }
    Ok(out)
}

struct InHandler {
    port: jack::Port<MidiIn>,
    ring: RingBufferWriter,
    lost: Arc<AtomicUsize>,
}

impl ProcessHandler for InHandler {
    fn process(&mut self, _: &Client, ps: &ProcessScope) -> Control {
        for e in self.port.iter(ps) {
            let len = e.bytes.len();
            if self.ring.space() < 4 + len {
                self.lost.fetch_add(1, Ordering::Relaxed);
                continue;
            }
            self.ring.write_buffer(&(len as u32).to_le_bytes());
            self.ring.write_buffer(e.bytes);
        }
        Control::Continue
    }
}

pub struct JackInput {
    _client: jack::AsyncClient<(), InHandler>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for JackInput {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

pub fn open_input(target: &str, deliver: Arc<Mutex<Deliver>>) -> Result<JackInput, Error> {
    let c = client("ctrl-editor-in")?;
    let port = c
        .register_port("in", MidiIn::default())
        .map_err(unavailable)?;
    let own = port.name().map_err(unavailable)?;
    let (mut reader, writer) = RingBuffer::new(RING)
        .map_err(unavailable)?
        .into_reader_writer();
    let handler = InHandler {
        port,
        ring: writer,
        lost: Arc::new(AtomicUsize::new(0)),
    };
    let active = c.activate_async((), handler).map_err(unavailable)?;
    active
        .as_client()
        .connect_ports_by_name(target, &own)
        .map_err(|e| Error::NoPort(format!("{target}: {e}")))?;
    let stop = Arc::new(AtomicBool::new(false));
    let s = stop.clone();
    let thread = std::thread::Builder::new()
        .name("ctrl-midi-jack-in".into())
        .spawn(move || {
            let mut buf = vec![0u8; SCRATCH];
            while !s.load(Ordering::Relaxed) {
                if !read_record(&mut reader, &mut buf, |m| {
                    if let Ok(mut d) = deliver.lock() {
                        d.push(m)
                    }
                }) {
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
        })
        .map_err(|e| Error::Backend(e.to_string()))?;
    Ok(JackInput {
        _client: active,
        stop,
        thread: Some(thread),
    })
}

/// Takes one `[len][bytes]` record if it is complete; false when none is.
fn read_record(r: &mut RingBufferReader, buf: &mut [u8], mut f: impl FnMut(&[u8])) -> bool {
    let mut head = [0u8; 4];
    if r.space() < 4 || r.peek(&mut head) < 4 {
        return false;
    }
    let len = u32::from_le_bytes(head) as usize;
    if r.space() < 4 + len {
        return false;
    }
    r.advance(4);
    if len > buf.len() {
        r.advance(len); // too long to keep: drop it whole
        return true;
    }
    let n = r.read_buffer(&mut buf[..len]);
    f(&buf[..n]);
    true
}

struct OutHandler {
    port: jack::Port<MidiOut>,
    ring: RingBufferReader,
    scratch: Vec<u8>,
    max_event: Arc<AtomicUsize>,
    too_long: Arc<AtomicUsize>,
}

impl ProcessHandler for OutHandler {
    fn process(&mut self, _: &Client, ps: &ProcessScope) -> Control {
        let mut w = self.port.writer(ps);
        self.max_event.store(w.max_event_size(), Ordering::Relaxed);
        loop {
            let mut head = [0u8; 4];
            if self.ring.space() < 4 || self.ring.peek(&mut head) < 4 {
                break;
            }
            let len = u32::from_le_bytes(head) as usize;
            if self.ring.space() < 4 + len || 4 + len > self.scratch.len() {
                break;
            }
            self.ring.peek(&mut self.scratch[..4 + len]);
            let msg = &self.scratch[4..4 + len];
            if len > w.max_event_size() && w.max_event_size() > 0 {
                // can never fit this port's buffer: drop it and say so
                self.too_long.store(len, Ordering::Relaxed);
                self.ring.advance(4 + len);
                continue;
            }
            match w.write(&RawMidi {
                time: 0,
                bytes: msg,
            }) {
                Ok(()) => self.ring.advance(4 + len),
                Err(_) => break, // no room in this cycle; next one
            }
        }
        Control::Continue
    }
}

pub struct JackOutput {
    _client: jack::AsyncClient<(), OutHandler>,
    ring: RingBufferWriter,
    max_event: Arc<AtomicUsize>,
    too_long: Arc<AtomicUsize>,
}

pub fn open_output(target: &str) -> Result<JackOutput, Error> {
    let c = client("ctrl-editor-out")?;
    let port = c
        .register_port("out", MidiOut::default())
        .map_err(unavailable)?;
    let own = port.name().map_err(unavailable)?;
    let (reader, writer) = RingBuffer::new(RING)
        .map_err(unavailable)?
        .into_reader_writer();
    let max_event = Arc::new(AtomicUsize::new(0));
    let too_long = Arc::new(AtomicUsize::new(0));
    let handler = OutHandler {
        port,
        ring: reader,
        scratch: vec![0u8; 4 + SCRATCH],
        max_event: max_event.clone(),
        too_long: too_long.clone(),
    };
    let active = c.activate_async((), handler).map_err(unavailable)?;
    active
        .as_client()
        .connect_ports_by_name(&own, target)
        .map_err(|e| Error::NoPort(format!("{target}: {e}")))?;
    Ok(JackOutput {
        _client: active,
        ring: writer,
        max_event,
        too_long,
    })
}

impl JackOutput {
    /// Queues one whole message for the next process cycle. JACK MIDI
    /// events are whole messages, so a SysEx is never split here.
    pub fn send(&mut self, msg: &[u8]) -> Result<(), Error> {
        let dropped = self.too_long.swap(0, Ordering::Relaxed);
        let max = self.max_event.load(Ordering::Relaxed);
        if dropped > 0 {
            return Err(Error::TooLong { len: dropped, max });
        }
        if msg.len() > SCRATCH || (max > 0 && msg.len() > max) {
            return Err(Error::TooLong {
                len: msg.len(),
                max: if max > 0 { max } else { SCRATCH },
            });
        }
        // wait for room rather than drop: the editor's traffic is small
        for _ in 0..1000 {
            if self.ring.space() >= 4 + msg.len() {
                self.ring.write_buffer(&(msg.len() as u32).to_le_bytes());
                self.ring.write_buffer(msg);
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        Err(Error::Backend(
            "JACK output queue full (is the server running?)".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    //! Runs only with a JACK server: `CTRL_MIDI_JACK_TEST=1 cargo test -p
    //! ctrl-midi -- --ignored` (jackd -d dummy, or PipeWire with pw-jack).
    use super::*;
    use std::sync::mpsc;

    #[test]
    #[ignore = "needs a running JACK server (jackd or PipeWire)"]
    fn loopback_carries_cc_and_long_sysex() {
        // a loopback client: its MIDI output is wired to its own input
        let c = client("ctrl-editor-loop").unwrap();
        let li = c.register_port("in", MidiIn::default()).unwrap();
        let mut lo = c.register_port("out", MidiOut::default()).unwrap();
        let (li_name, lo_name) = (li.name().unwrap(), lo.name().unwrap());
        let cb = jack::contrib::ClosureProcessHandler::new(move |_: &Client, ps: &ProcessScope| {
            let mut w = lo.writer(ps);
            for e in li.iter(ps) {
                let _ = w.write(&e);
            }
            Control::Continue
        });
        let _active = c.activate_async((), cb).unwrap();

        let (tx, rx) = mpsc::channel();
        let deliver = Arc::new(Mutex::new(Deliver::new(Box::new(move |_, m| {
            let _ = tx.send(m.to_vec());
        }))));
        let _input = open_input(&lo_name, deliver).unwrap();
        let mut out = open_output(&li_name).unwrap();

        out.send(&[0xB0, 7, 100]).unwrap();
        let sysex: Vec<u8> = std::iter::once(0xF0)
            .chain((0..6000).map(|i| (i % 128) as u8))
            .chain(std::iter::once(0xF7))
            .collect();
        out.send(&sysex).unwrap();
        let a = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let b = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(a, vec![0xB0, 7, 100]);
        assert_eq!(b, sysex, "a 6002-byte SysEx crosses JACK whole");
    }
}
