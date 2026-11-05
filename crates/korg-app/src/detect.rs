//! Find KORG devices by asking each port, not only by its name.
//!
//! Every output port that has an input of a matching name gets the KORG
//! search request (`korg_sysex::search_request`); other manufacturers ignore
//! it.  A port whose name already matches a catalog device also gets the
//! universal identity request when the search goes unanswered.  The reply
//! gives the model, the firmware and the SysEx device id, so a device whose
//! ports were renamed (an ALSA client name, a CoreMIDI rename) is still
//! found.  `remember` keeps the result; `port_name_matches_device` then
//! accepts those ports, so links, jobs and the monitor use them.
//!
//! Probes run in parallel, one transport per port pair, created on its own
//! thread (a midir CoreMIDI client is not `Send`).

use crate::document::port_name_matches_device;
use korg_devices::Catalog;
use korg_midi::{Direction, MidiTransport, PortId, PortInfo, TransportEvent};
use korg_sysex::{
    parse_identity, parse_search_reply, search_request, Assembler, IdentityReply, IDENTITY_REQUEST,
};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

/// How long one port pair may take to answer.
pub const PROBE_TIMEOUT: Duration = Duration::from_millis(400);

/// A port pair that answered, with what it said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detected {
    pub input: String,
    pub output: String,
    pub reply: IdentityReply,
    /// The catalog class of the reply's model, when the catalog knows it.
    pub class: Option<String>,
}

impl Detected {
    /// One line: device, firmware, device id and ports.
    pub fn summary(&self, catalog: &Catalog) -> String {
        let device = match self.class.as_deref() {
            Some(class) => catalog
                .find_by_class(class)
                .map_or_else(|| class.to_owned(), |d| format!("{} ({class})", d.device)),
            None => format!("unknown KORG model {:#06x}", self.reply.model_id),
        };
        let ports = if self.input == self.output {
            self.input.clone()
        } else {
            format!("{} / {}", self.input, self.output)
        };
        format!(
            "{device} · firmware {} · device id {} · {ports}",
            self.reply.firmware(),
            self.reply.device_id
        )
    }
}

/// The catalog class whose SysEx model id is `model_id`.
pub fn class_of_model(model_id: u32) -> Option<String> {
    korg_devices::sysex_catalog()
        .ok()?
        .devices
        .into_iter()
        .find(|(_, descriptor)| descriptor.model_id.map(|id| id.0) == Some(model_id))
        .map(|(class, _)| class)
}

/// Each output port with the input that most likely belongs to it: the same
/// name, else the input sharing the longest run of leading words, at least
/// two ("nanoKEY Studio CTRL" pairs with "nanoKEY Studio KEYBOARD/CTRL").
/// Outputs without such an input cannot hear a reply and are left out.
pub fn port_pairs(ports: &[PortInfo]) -> Vec<(String, String)> {
    let inputs: Vec<&str> = ports
        .iter()
        .filter(|port| port.direction == Direction::Input)
        .map(|port| port.name.as_str())
        .collect();
    let common = |a: &str, b: &str| {
        a.split_whitespace()
            .zip(b.split_whitespace())
            .take_while(|(x, y)| x == y)
            .count()
    };
    let mut pairs = Vec::new();
    for output in ports
        .iter()
        .filter(|port| port.direction == Direction::Output)
    {
        let name = output.name.as_str();
        let input = inputs.iter().find(|input| **input == name).or_else(|| {
            inputs
                .iter()
                .map(|input| (common(input, name), input))
                .filter(|(words, _)| *words >= 2)
                .max_by_key(|(words, _)| *words)
                .map(|(_, input)| input)
        });
        if let Some(input) = input {
            let pair = ((*input).to_owned(), name.to_owned());
            if !pairs.contains(&pair) {
                pairs.push(pair);
            }
        }
    }
    pairs
}

fn exact_port(ports: &[PortInfo], name: &str, direction: Direction) -> Option<PortId> {
    ports
        .iter()
        .find(|port| port.direction == direction && port.name == name)
        .map(|port| port.id.clone())
}

/// Ask one open transport who it is.  The search request goes first; the
/// identity request follows only with `identity_fallback`.
pub fn probe<T: MidiTransport>(
    transport: &mut T,
    echo: u8,
    timeout: Duration,
    identity_fallback: bool,
) -> Option<IdentityReply> {
    let mut requests = vec![search_request(echo).to_vec()];
    if identity_fallback {
        requests.push(IDENTITY_REQUEST.to_vec());
    }
    for request in requests {
        transport.send(&request).ok()?;
        let mut assembler = Assembler::default();
        let deadline = Instant::now() + timeout;
        let mut events = Vec::new();
        while Instant::now() < deadline {
            transport.wait(10);
            events.clear();
            transport.poll(&mut events);
            for event in &events {
                let TransportEvent::Packet(packet) = event else {
                    continue;
                };
                for frame in assembler.push(&packet.bytes) {
                    if let Ok((got, reply)) = parse_search_reply(&frame) {
                        if got == echo & 0x7F {
                            return Some(reply);
                        }
                    }
                    if let Ok(reply) = parse_identity(&frame) {
                        return Some(reply);
                    }
                }
            }
        }
    }
    None
}

/// Probe every port pair of a fresh transport from `make_transport`.
pub fn detect<T, F>(
    make_transport: F,
    catalog: &Catalog,
    timeout: Duration,
) -> Result<Vec<Detected>, String>
where
    T: MidiTransport,
    F: Fn() -> T + Sync,
{
    let ports = make_transport()
        .list_ports()
        .map_err(|error| error.to_string())?;
    let pairs = port_pairs(&ports);
    let make = &make_transport;
    let found = thread::scope(|scope| {
        let handles: Vec<_> = pairs
            .iter()
            .enumerate()
            .map(|(index, (input, output))| {
                // A port of a catalog device may predate the search request.
                let known = catalog.devices.iter().any(|device| {
                    port_name_matches_device(device, input)
                        || port_name_matches_device(device, output)
                });
                scope.spawn(move || {
                    let mut transport = make();
                    let ports = transport.list_ports().ok()?;
                    let input_id = exact_port(&ports, input, Direction::Input)?;
                    let output_id = exact_port(&ports, output, Direction::Output)?;
                    transport.open(&input_id, &output_id).ok()?;
                    let reply = probe(&mut transport, index as u8, timeout, known);
                    transport.close();
                    reply.map(|reply| Detected {
                        input: input.clone(),
                        output: output.clone(),
                        class: class_of_model(reply.model_id),
                        reply,
                    })
                })
            })
            .collect();
        handles
            .into_iter()
            .filter_map(|handle| handle.join().ok().flatten())
            .collect::<Vec<_>>()
    });
    Ok(found)
}

static DETECTED: Mutex<Vec<Detected>> = Mutex::new(Vec::new());

/// Keep the last detection for `port_class`; replaces the previous one.
pub fn remember(found: &[Detected]) {
    if let Ok(mut detected) = DETECTED.lock() {
        *detected = found.to_vec();
    }
}

/// The last detection.
pub fn detected() -> Vec<Detected> {
    DETECTED.lock().map(|d| d.clone()).unwrap_or_default()
}

/// The class a detected device answered with on `port_name`.
pub fn port_class(port_name: &str) -> Option<String> {
    DETECTED.lock().ok()?.iter().find_map(|found| {
        (found.input == port_name || found.output == port_name)
            .then(|| found.class.clone())
            .flatten()
    })
}

/// What detection found for `class`, if anything.
pub fn detected_class(class: &str) -> Option<Detected> {
    detected()
        .into_iter()
        .find(|found| found.class.as_deref() == Some(class))
}

#[cfg(test)]
mod tests {
    use super::*;
    use korg_midi::{MockTransport, TransportError};
    use std::collections::VecDeque;

    fn port(name: &str, direction: Direction) -> PortInfo {
        PortInfo {
            id: PortId::new(format!("{name}#{direction:?}")),
            name: name.into(),
            direction,
        }
    }

    #[test]
    fn pairs_outputs_with_their_inputs() {
        let ports = vec![
            port("nanoKEY Studio KEYBOARD/CTRL", Direction::Input),
            port("nanoKEY Studio CTRL", Direction::Output),
            port("SSL V-MIDI Port 1 Source", Direction::Input),
            port("SSL V-MIDI Port 10 Source", Direction::Input),
            port("SSL V-MIDI Port 1 Destination", Direction::Output),
            port("Port A", Direction::Input),
            port("Port A", Direction::Output),
            port("Lonely Out", Direction::Output),
        ];
        assert_eq!(
            port_pairs(&ports),
            [
                (
                    "nanoKEY Studio KEYBOARD/CTRL".to_owned(),
                    "nanoKEY Studio CTRL".to_owned()
                ),
                (
                    "SSL V-MIDI Port 1 Source".to_owned(),
                    "SSL V-MIDI Port 1 Destination".to_owned()
                ),
                ("Port A".to_owned(), "Port A".to_owned()),
            ]
        );
    }

    #[test]
    fn maps_models_to_classes() {
        assert_eq!(class_of_model(0x137).as_deref(), Some("X14802"));
        assert_eq!(class_of_model(0x136).as_deref(), Some("X14800"));
        assert_eq!(class_of_model(0x7777), None);
    }

    /// Answers the search request with the measured nanoKONTROL Studio reply.
    struct Answering {
        inner: MockTransport,
        replies: VecDeque<Vec<u8>>,
        sent: Vec<Vec<u8>>,
        answer_search: bool,
    }

    impl MidiTransport for Answering {
        fn list_ports(&mut self) -> Result<Vec<PortInfo>, TransportError> {
            self.inner.list_ports()
        }
        fn open(&mut self, input: &PortId, output: &PortId) -> Result<(), TransportError> {
            self.inner.open(input, output)
        }
        fn close(&mut self) {
            self.inner.close();
        }
        fn is_open(&self) -> bool {
            self.inner.is_open()
        }
        fn send(&mut self, message: &[u8]) -> Result<(), TransportError> {
            self.sent.push(message.to_vec());
            if message[1] == 0x42 && self.answer_search {
                let echo = message[4];
                self.replies.push_back(vec![
                    0xF0, 0x42, 0x50, 0x01, 0x00, echo, 0x37, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01,
                    0x00, 0xF7,
                ]);
            } else if message == IDENTITY_REQUEST {
                self.replies.push_back(vec![
                    0xF0, 0x7E, 0x00, 0x06, 0x02, 0x42, 0x37, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01,
                    0x00, 0xF7,
                ]);
            }
            Ok(())
        }
        fn poll(&mut self, out: &mut Vec<TransportEvent>) -> usize {
            let n = self.replies.len();
            for bytes in self.replies.drain(..) {
                out.push(TransportEvent::Packet(korg_midi::MidiPacket {
                    bytes,
                    received_at_ms: 0,
                }));
            }
            n
        }
    }

    fn answering(answer_search: bool) -> Answering {
        Answering {
            inner: MockTransport::new(Vec::new()),
            replies: VecDeque::new(),
            sent: Vec::new(),
            answer_search,
        }
    }

    #[test]
    fn probe_reads_the_search_reply() {
        let mut transport = answering(true);
        let reply = probe(&mut transport, 3, Duration::from_millis(50), false).unwrap();
        assert_eq!(reply.model_id, 0x137);
        assert_eq!(transport.sent, [search_request(3).to_vec()]);
    }

    #[test]
    fn probe_falls_back_to_the_identity_request_only_when_asked() {
        let mut transport = answering(false);
        assert_eq!(
            probe(&mut transport, 3, Duration::from_millis(20), false),
            None
        );
        assert_eq!(
            transport.sent.len(),
            1,
            "no identity request to unknown ports"
        );
        let reply = probe(&mut transport, 3, Duration::from_millis(20), true).unwrap();
        assert_eq!(reply.firmware(), "1.05");
    }

    #[test]
    fn detect_finds_a_renamed_port_and_remembers_it() {
        let catalog = Catalog::load().unwrap();
        let ports = vec![
            port("Renamed Box 7", Direction::Input),
            port("Renamed Box 7", Direction::Output),
        ];
        let found = detect(
            || Answering {
                inner: MockTransport::new(ports.clone()),
                ..answering(true)
            },
            &catalog,
            Duration::from_millis(50),
        )
        .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].class.as_deref(), Some("X14802"));
        assert!(found[0]
            .summary(&catalog)
            .starts_with("nanoKONTROL Studio (X14802) · firmware 1.05"));
        remember(&found);
        let studio = catalog.find_by_class("X14802").unwrap();
        assert!(port_name_matches_device(studio, "Renamed Box 7"));
        let links = crate::device_links(&ports, studio);
        assert_eq!(links.len(), 1);
        remember(&[]);
        assert!(!port_name_matches_device(studio, "Renamed Box 7"));
    }
}
