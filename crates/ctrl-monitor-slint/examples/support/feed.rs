//! Synthetic MIDI for the examples: notes, CC sweeps, clock at 120 BPM,
//! active sense, SysEx from the editor and the device, one cut SysEx.

use ctrl_monitor::{Dir, PortIdx, PortKind, PortRef, Store};

/// Which port a packet travels.
#[derive(Clone, Copy)]
pub enum Via {
    Device,
    Editor,
}

pub struct Packet {
    pub ms: u64,
    pub via: Via,
    pub dir: Dir,
    pub bytes: Vec<u8>,
}

pub struct Ports {
    pub device: PortIdx,
    pub editor: PortIdx,
}

impl Ports {
    pub fn register(store: &mut Store) -> Ports {
        Ports {
            device: store.port(PortRef::new(
                PortKind::CoreMidi,
                "demo-keys",
                "Keys & Knobs",
            )),
            editor: store.port(PortRef::new(PortKind::Editor, "editor", "Editor")),
        }
    }

    pub fn of(&self, via: Via) -> PortIdx {
        match via {
            Via::Device => self.device,
            Via::Editor => self.editor,
        }
    }
}

/// `seconds` of traffic starting at `start_ms`; `first` adds the cut SysEx.
pub fn script(start_ms: u64, seconds: u64, first: bool) -> Vec<Packet> {
    let mut out = Vec::new();
    let mut at = |ms: u64, via: Via, dir: Dir, bytes: &[u8]| {
        out.push(Packet {
            ms: start_ms + ms,
            via,
            dir,
            bytes: bytes.to_vec(),
        })
    };
    let end = seconds * 1000;
    // Clock at 120 BPM: 24 per quarter note, every 20.83 ms.
    let mut t = 0.0f64;
    while (t as u64) < end {
        at(t as u64, Via::Device, Dir::In, &[0xF8]);
        t += 60_000.0 / 120.0 / 24.0;
    }
    for ms in (0..end).step_by(300) {
        at(ms, Via::Device, Dir::In, &[0xFE]);
    }
    // A melody on channel 1, a bass line on channel 2.
    let melody = [60u8, 64, 67, 72, 71, 67, 64, 62];
    for (i, ms) in (100..end).step_by(500).enumerate() {
        let n = melody[i % melody.len()];
        at(
            ms,
            Via::Device,
            Dir::In,
            &[0x90, n, 70 + (i as u8 * 7) % 50],
        );
        at(ms + 240, Via::Device, Dir::In, &[0x80, n, 0]);
        if i % 4 == 0 {
            at(ms + 5, Via::Device, Dir::In, &[0x91, n - 24, 96]);
            at(ms + 900, Via::Device, Dir::In, &[0x81, n - 24, 0]);
        }
    }
    for s in 0..seconds {
        let base = s * 1000;
        // A volume sweep up and down, then a filter sweep.
        if s % 2 == 0 {
            for k in 0..48u64 {
                let v = if k < 24 { k * 5 } else { (47 - k) * 5 } as u8;
                at(
                    base + 300 + k * 12,
                    Via::Device,
                    Dir::In,
                    &[0xB0, 7, v.min(127)],
                );
            }
        } else {
            for k in 0..32u64 {
                let v = (64.0 + 60.0 * (k as f64 / 5.0).sin()) as u8;
                at(base + 320 + k * 15, Via::Device, Dir::In, &[0xB1, 74, v]);
            }
            at(base + 850, Via::Device, Dir::In, &[0xE0, 0x00, 0x48]);
            at(base + 870, Via::Device, Dir::In, &[0xE0, 0x00, 0x40]);
        }
        if s % 3 == 1 {
            at(base + 50, Via::Device, Dir::In, &[0xC0, (s % 8) as u8]);
            // The editor asks who is there; the device answers.
            at(
                base + 700,
                Via::Editor,
                Dir::Out,
                &[0xF0, 0x7E, 0x7F, 0x06, 0x01, 0xF7],
            );
            at(
                base + 712,
                Via::Device,
                Dir::In,
                &[
                    0xF0, 0x7E, 0x00, 0x06, 0x02, 0x42, 0x13, 0x01, 0x00, 0x00, 0x01, 0x00, 0x02,
                    0x00, 0xF7,
                ],
            );
        }
        if s % 5 == 2 {
            // A 600-byte dump.
            let mut dump = vec![0xF0, 0x42, 0x30, 0x00, 0x01, 0x13, 0x4C];
            dump.extend((0..592u32).map(|i| (i * 7 % 128) as u8));
            dump.push(0xF7);
            at(base + 600, Via::Device, Dir::In, &dump);
        }
    }
    if first && seconds > 2 {
        // A SysEx cut short by a note: the alert strip shows it.
        at(2_450, Via::Device, Dir::In, &[0xF0, 0x42, 0x30, 0x00, 0x01]);
        at(2_451, Via::Device, Dir::In, &[0x90, 48, 80]);
        at(2_700, Via::Device, Dir::In, &[0x80, 48, 0]);
    }
    out.sort_by_key(|p| p.ms);
    out
}

/// Stands in for a device describer: names the identity reply, the dump
/// and the scene changes, so the detail column has something to say.
pub struct Demo;

impl ctrl_monitor::Describer for Demo {
    fn describe(
        &mut self,
        store: &Store,
        event: &ctrl_monitor::Event,
    ) -> Vec<ctrl_monitor::Annotation> {
        use ctrl_monitor::{Annotation, Msg};
        let field = |k: &str, v: String| (k.to_string(), v);
        match event.msg {
            Msg::Voice { status, d1, .. } if status & 0xF0 == 0xC0 => {
                vec![Annotation::State(format!("scene {}", d1 + 1))]
            }
            Msg::SysEx { blob, len, .. } => {
                let Some(b) = store.bytes(blob) else {
                    return Vec::new();
                };
                if b.starts_with(&[0xF0, 0x7E]) && b.get(3..5) == Some(&[0x06, 0x02]) && len >= 14 {
                    vec![Annotation::Described {
                        name: "Identity".into(),
                        fields: vec![
                            field("family", format!("{:02X} {:02X}", b[6], b[7])),
                            field("member", format!("{:02X} {:02X}", b[8], b[9])),
                            field("version", format!("{}.{}.{}", b[10], b[11], b[12])),
                        ],
                    }]
                } else if b.starts_with(&[0xF0, 0x42, 0x30]) && len > 100 {
                    vec![
                        Annotation::Described {
                            name: "Scene dump".into(),
                            fields: vec![
                                field("scene", "1".into()),
                                field("payload", format!("{} bytes", len - 8)),
                            ],
                        },
                        Annotation::Control {
                            label: "Write button".into(),
                            id: "write".into(),
                        },
                    ]
                } else {
                    Vec::new()
                }
            }
            _ => Vec::new(),
        }
    }
}
