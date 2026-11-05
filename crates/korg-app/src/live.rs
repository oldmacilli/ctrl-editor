//! What the device sends while it is played, for the MIDI monitor.
//!
//! `LiveParser` splits the incoming byte stream into messages (running
//! status, SysEx split over packets, real-time bytes inside SysEx).
//! `lit_controls` finds the panel controls that send a message with the
//! current scene, so both front ends light the same controls.

use crate::assign::{self, ControlMessage, MessageKind};
use crate::monitor::parse_scene_change;
use crate::panel::{Control, Panel};

/// One complete incoming message.  Channels are 0-based.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveMessage {
    NoteOn {
        channel: u8,
        note: u8,
        velocity: u8,
    },
    NoteOff {
        channel: u8,
        note: u8,
        velocity: u8,
    },
    PolyPressure {
        channel: u8,
        note: u8,
        value: u8,
    },
    ControlChange {
        channel: u8,
        number: u8,
        value: u8,
    },
    ProgramChange {
        channel: u8,
        program: u8,
    },
    ChannelPressure {
        channel: u8,
        value: u8,
    },
    /// 14-bit, centre 8192.
    PitchBend {
        channel: u8,
        value: u16,
    },
    /// A complete frame, `F0` to `F7`.
    SysEx(Vec<u8>),
    /// System common or real-time bytes.
    System(Vec<u8>),
}

/// Longest SysEx the monitor keeps; a scene dump is under 1 kB.
const MAX_SYSEX: usize = 4096;

impl LiveMessage {
    /// The raw bytes, as received (running status expanded).
    pub fn bytes(&self) -> Vec<u8> {
        match *self {
            LiveMessage::NoteOn {
                channel,
                note,
                velocity,
            } => vec![0x90 | channel, note, velocity],
            LiveMessage::NoteOff {
                channel,
                note,
                velocity,
            } => vec![0x80 | channel, note, velocity],
            LiveMessage::PolyPressure {
                channel,
                note,
                value,
            } => vec![0xA0 | channel, note, value],
            LiveMessage::ControlChange {
                channel,
                number,
                value,
            } => vec![0xB0 | channel, number, value],
            LiveMessage::ProgramChange { channel, program } => vec![0xC0 | channel, program],
            LiveMessage::ChannelPressure { channel, value } => vec![0xD0 | channel, value],
            LiveMessage::PitchBend { channel, value } => {
                vec![0xE0 | channel, (value & 0x7F) as u8, (value >> 7) as u8]
            }
            LiveMessage::SysEx(ref bytes) | LiveMessage::System(ref bytes) => bytes.clone(),
        }
    }

    /// Hex bytes; a long frame shows its first 24 bytes and its length.
    pub fn hex(&self) -> String {
        let bytes = self.bytes();
        let shown: Vec<String> = bytes.iter().take(24).map(|b| format!("{b:02X}")).collect();
        if bytes.len() > 24 {
            format!("{} … ({} bytes)", shown.join(" "), bytes.len())
        } else {
            shown.join(" ")
        }
    }

    pub fn channel(&self) -> Option<u8> {
        match *self {
            LiveMessage::NoteOn { channel, .. }
            | LiveMessage::NoteOff { channel, .. }
            | LiveMessage::PolyPressure { channel, .. }
            | LiveMessage::ControlChange { channel, .. }
            | LiveMessage::ProgramChange { channel, .. }
            | LiveMessage::ChannelPressure { channel, .. }
            | LiveMessage::PitchBend { channel, .. } => Some(channel),
            LiveMessage::SysEx(_) | LiveMessage::System(_) => None,
        }
    }

    /// The colour class of the message, as the control list uses it.
    pub fn kind(&self) -> Option<MessageKind> {
        match self {
            LiveMessage::NoteOn { .. } | LiveMessage::NoteOff { .. } => Some(MessageKind::Note),
            LiveMessage::ControlChange { .. } => Some(MessageKind::ControlChange),
            LiveMessage::ProgramChange { .. } => Some(MessageKind::ProgramChange),
            _ => None,
        }
    }

    /// The message in words: `CC 51 = 127 · ch 1`, `Note On C4 (60) vel 100 · ch 1`.
    pub fn describe(&self) -> String {
        let on = |channel: u8| format!(" · ch {}", channel + 1);
        match *self {
            LiveMessage::NoteOn {
                channel,
                note,
                velocity,
            } => format!(
                "Note On {} ({note}) vel {velocity}{}",
                assign::note_name(note),
                on(channel)
            ),
            LiveMessage::NoteOff {
                channel,
                note,
                velocity,
            } => format!(
                "Note Off {} ({note}) vel {velocity}{}",
                assign::note_name(note),
                on(channel)
            ),
            LiveMessage::PolyPressure {
                channel,
                note,
                value,
            } => format!(
                "Poly Pressure {} = {value}{}",
                assign::note_name(note),
                on(channel)
            ),
            LiveMessage::ControlChange {
                channel,
                number,
                value,
            } => format!("CC {number} = {value}{}", on(channel)),
            LiveMessage::ProgramChange { channel, program } => {
                format!("Program {program}{}", on(channel))
            }
            LiveMessage::ChannelPressure { channel, value } => {
                format!("Channel Pressure {value}{}", on(channel))
            }
            LiveMessage::PitchBend { channel, value } => {
                format!("Pitch Bend {:+}{}", i32::from(value) - 8192, on(channel))
            }
            LiveMessage::SysEx(ref bytes) => match parse_scene_change(bytes) {
                Some(scene) => format!("Scene change to {}", scene + 1),
                None if korg_sysex::parse_identity(bytes).is_ok() => "Identity reply".into(),
                None if korg_sysex::parse_search_reply(bytes).is_ok() => "KORG search reply".into(),
                None if bytes.get(1) == Some(&0x42) => format!("KORG SysEx, {} bytes", bytes.len()),
                None => format!("SysEx, {} bytes", bytes.len()),
            },
            LiveMessage::System(ref bytes) => match bytes.first() {
                Some(0xF8) => "Clock".into(),
                Some(0xFA) => "Start".into(),
                Some(0xFB) => "Continue".into(),
                Some(0xFC) => "Stop".into(),
                Some(0xFE) => "Active Sensing".into(),
                _ => "System".into(),
            },
        }
    }
}

/// Splits a MIDI byte stream into messages.
#[derive(Debug, Default)]
pub struct LiveParser {
    running: Option<u8>,
    data: Vec<u8>,
    sysex: Option<Vec<u8>>,
}

impl LiveParser {
    pub fn push(&mut self, bytes: &[u8]) -> Vec<LiveMessage> {
        let mut out = Vec::new();
        for &byte in bytes {
            if byte >= 0xF8 {
                // Real-time bytes may sit inside anything.
                out.push(LiveMessage::System(vec![byte]));
                continue;
            }
            if let Some(frame) = self.sysex.as_mut() {
                if byte == 0xF7 {
                    frame.push(byte);
                    out.push(LiveMessage::SysEx(self.sysex.take().unwrap_or_default()));
                    continue;
                }
                if byte < 0x80 {
                    if frame.len() < MAX_SYSEX {
                        frame.push(byte);
                    }
                    continue;
                }
                // A status byte ends an unterminated frame; drop it.
                self.sysex = None;
            }
            match byte {
                0xF0 => {
                    self.sysex = Some(vec![0xF0]);
                    self.running = None;
                }
                0xF1..=0xF7 => {
                    self.running = None;
                    self.data.clear();
                    out.push(LiveMessage::System(vec![byte]));
                }
                0x80..=0xEF => {
                    self.running = Some(byte);
                    self.data.clear();
                }
                _ => {
                    let Some(status) = self.running else {
                        continue;
                    };
                    self.data.push(byte);
                    let needed = if matches!(status & 0xF0, 0xC0 | 0xD0) {
                        1
                    } else {
                        2
                    };
                    if self.data.len() == needed {
                        out.push(channel_message(status, &self.data));
                        self.data.clear();
                    }
                }
            }
        }
        out
    }
}

fn channel_message(status: u8, data: &[u8]) -> LiveMessage {
    let channel = status & 0x0F;
    let (a, b) = (data[0], data.get(1).copied().unwrap_or(0));
    match status & 0xF0 {
        // Note On with velocity 0 is a Note Off.
        0x90 if b == 0 => LiveMessage::NoteOff {
            channel,
            note: a,
            velocity: 0,
        },
        0x90 => LiveMessage::NoteOn {
            channel,
            note: a,
            velocity: b,
        },
        0x80 => LiveMessage::NoteOff {
            channel,
            note: a,
            velocity: b,
        },
        0xA0 => LiveMessage::PolyPressure {
            channel,
            note: a,
            value: b,
        },
        0xB0 => LiveMessage::ControlChange {
            channel,
            number: a,
            value: b,
        },
        0xC0 => LiveMessage::ProgramChange {
            channel,
            program: a,
        },
        0xD0 => LiveMessage::ChannelPressure { channel, value: a },
        _ => LiveMessage::PitchBend {
            channel,
            value: u16::from(a) | (u16::from(b) << 7),
        },
    }
}

/// The global MIDI channel at the externally described offset, if known.
pub fn global_channel(class: &str, global: Option<&[u8]>) -> Option<u8> {
    let offset = korg_devices::control_assignments(class)?.global_channel_offset?;
    global?.get(offset as usize).copied().filter(|c| *c < 16)
}

fn channel_matches(control: Option<u8>, sent: u8, global: Option<u8>) -> bool {
    match control {
        Some(channel) if channel < 16 => channel == sent,
        // The global channel, or unknown: accept any.
        Some(_) => global.is_none_or(|global| global == sent),
        None => true,
    }
}

/// The numbers a control sends, including externally described additional slots.
fn numbers(class: &str, control: &Control, scene: &[u8], message: &ControlMessage) -> Vec<u8> {
    let mut numbers: Vec<u8> = message.number.into_iter().chain(message.second).collect();
    if let Some(item) = korg_devices::control_assignments(class).and_then(|assignments| {
        assignments.assignments.iter().find(|item| {
            item.group == control.group
                && item.label == control.label
                && item.meta.status != Some(korg_devices::DescriptorStatus::Unknown)
        })
    }) {
        for route in &item.live_numbers {
            if let Some(offset) = route.if_offset {
                if scene
                    .get(offset as usize)
                    .is_none_or(|value| Some(*value) == route.if_not_value)
                {
                    continue;
                }
            }
            let start = route.offset as usize;
            if let Some(values) = start
                .checked_add(route.count as usize)
                .and_then(|end| scene.get(start..end))
            {
                numbers.extend(values.iter().copied().filter(|value| *value <= route.max));
            }
        }
    }
    numbers
}

/// Indices into `panel.controls` of the controls that send `message` with
/// this scene.  A note no control claims lights the keyboard; a pitch bend
/// lights the pitch control; a scene announcement lights the Scene button.
pub fn lit_controls(
    class: &str,
    panel: &Panel,
    scene: &[u8],
    message: &LiveMessage,
    global: Option<u8>,
) -> Vec<usize> {
    let wanted = match *message {
        LiveMessage::NoteOn { channel, note, .. } | LiveMessage::NoteOff { channel, note, .. } => {
            Some((MessageKind::Note, channel, note))
        }
        LiveMessage::ControlChange {
            channel, number, ..
        } => Some((MessageKind::ControlChange, channel, number)),
        LiveMessage::ProgramChange { channel, program } => {
            Some((MessageKind::ProgramChange, channel, program))
        }
        _ => None,
    };
    let mut lit: Vec<usize> = match wanted {
        Some((kind, channel, number)) => panel
            .controls
            .iter()
            .enumerate()
            .filter(|(_, control)| {
                assign::control_message(class, control, scene).is_some_and(|sent| {
                    sent.kind == kind
                        && channel_matches(sent.channel, channel, global)
                        && numbers(class, control, scene, &sent).contains(&number)
                })
            })
            .map(|(index, _)| index)
            .collect(),
        None => Vec::new(),
    };
    if lit.is_empty() {
        let by = |test: &dyn Fn(&Control) -> bool| -> Vec<usize> {
            panel
                .controls
                .iter()
                .enumerate()
                .filter(|(_, control)| test(control))
                .map(|(index, _)| index)
                .collect()
        };
        lit = match message {
            LiveMessage::NoteOn { .. }
            | LiveMessage::NoteOff { .. }
            | LiveMessage::PolyPressure { .. }
            | LiveMessage::ChannelPressure { .. } => {
                by(&|control| control.group == "Keyboard" || control.label == "Keyboard")
            }
            LiveMessage::PitchBend { .. } => by(&|control| control.label.starts_with("Pitch")),
            LiveMessage::SysEx(bytes) if parse_scene_change(bytes).is_some() => {
                by(&|control| control.group == "Scene")
            }
            _ => Vec::new(),
        };
    }
    lit
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panel::panel;

    #[test]
    fn splits_running_status_and_split_sysex() {
        let mut parser = LiveParser::default();
        let out = parser.push(&[0xB0, 0x33, 0x7F, 0x33, 0x00, 0xF0, 0x42, 0x40]);
        assert_eq!(
            out,
            [
                LiveMessage::ControlChange {
                    channel: 0,
                    number: 0x33,
                    value: 0x7F
                },
                LiveMessage::ControlChange {
                    channel: 0,
                    number: 0x33,
                    value: 0
                },
            ]
        );
        let out = parser.push(&[0xF8, 0x00, 0xF7, 0x91, 0x3C, 0x00]);
        assert_eq!(out[0], LiveMessage::System(vec![0xF8]));
        assert_eq!(
            out[1],
            LiveMessage::SysEx(vec![0xF0, 0x42, 0x40, 0x00, 0xF7])
        );
        assert_eq!(
            out[2],
            LiveMessage::NoteOff {
                channel: 1,
                note: 60,
                velocity: 0
            }
        );
        assert_eq!(out[2].describe(), "Note Off C4 (60) vel 0 · ch 2");
        assert_eq!(out[2].hex(), "81 3C 00");
    }

    #[test]
    fn describes_messages() {
        let cc = LiveMessage::ControlChange {
            channel: 0,
            number: 51,
            value: 127,
        };
        assert_eq!(cc.describe(), "CC 51 = 127 · ch 1");
        assert_eq!(cc.hex(), "B0 33 7F");
        let bend = LiveMessage::PitchBend {
            channel: 0,
            value: 8192 + 100,
        };
        assert_eq!(bend.describe(), "Pitch Bend +100 · ch 1");
        let scene = LiveMessage::SysEx(vec![
            0xF0, 0x42, 0x40, 0x00, 0x01, 0x37, 0x02, 0x00, 0x00, 0x4F, 0x02, 0xF7,
        ]);
        assert_eq!(scene.describe(), "Scene change to 3");
        let identity = LiveMessage::SysEx(vec![
            0xF0, 0x7E, 0x00, 0x06, 0x02, 0x42, 0x37, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x00,
            0xF7,
        ]);
        assert_eq!(identity.describe(), "Identity reply");
        let long = LiveMessage::SysEx([vec![0xF0], vec![0; 40], vec![0xF7]].concat());
        assert!(long.hex().ends_with("… (42 bytes)"));
    }

    /// A nanoKONTROL Studio scene with Track Select 6 on CC 51, channel 1,
    /// and Play as a note.
    fn studio_scene() -> Vec<u8> {
        let mut scene = vec![0u8; 576 + 16];
        let select6 = 336 + 8 * 5;
        scene[select6..select6 + 3].copy_from_slice(&[0, 0, 51]);
        // Play: channel Global, Note 60.
        scene[400..403].copy_from_slice(&[16, 1, 60]);
        scene
    }

    #[test]
    fn external_live_routes_match_previous_studio_routing() {
        for class in ["X14800", "X14802"] {
            for value in 0..=255 {
                assert_eq!(
                    global_channel(class, Some(&[value])),
                    (value < 16).then_some(value)
                );
            }
            assert_eq!(global_channel(class, Some(&[])), None);
        }
        assert_eq!(global_channel("X10820", Some(&[0])), None);
        let panel = panel("X14800").unwrap();
        for control in &panel.controls {
            for byte in 0..=255 {
                let full = vec![byte; 256];
                for length in [0, 33, 34, 37, 38, 169, 170, 171, 178, 179, 256] {
                    let scene = &full[..length];
                    let Some(message) = assign::control_message("X14800", control, scene) else {
                        continue;
                    };
                    let mut expected: Vec<u8> =
                        message.number.into_iter().chain(message.second).collect();
                    if control.group == "Trigger Pad" {
                        if let Some(n) = assign::ordinal(&control.label).filter(|n| *n <= 8) {
                            let base = 32 + 8 * (n - 1);
                            expected.extend(
                                scene
                                    .get(base + 2..base + 6)
                                    .unwrap_or_default()
                                    .iter()
                                    .copied()
                                    .filter(|v| *v < 128),
                            );
                        }
                    }
                    if control.group == "X-Y Pad" {
                        // Out-of-range data bytes cannot be MIDI numbers.
                        expected.extend(scene.get(178).copied().filter(|v| *v < 128));
                        if scene.get(169) != Some(&1) {
                            expected.extend(scene.get(170).copied().filter(|v| *v < 128));
                        }
                    }
                    assert_eq!(
                        numbers("X14800", control, scene, &message),
                        expected,
                        "{} byte={byte} len={length}",
                        control.label
                    );
                }
            }
        }
    }

    #[test]
    fn lights_the_control_that_sends_the_message() {
        let panel = panel("X14802").unwrap();
        let scene = studio_scene();
        let label = |indices: Vec<usize>| -> Vec<String> {
            indices
                .into_iter()
                .map(|i| panel.controls[i].label.clone())
                .collect()
        };
        let cc = LiveMessage::ControlChange {
            channel: 0,
            number: 51,
            value: 127,
        };
        assert_eq!(
            label(lit_controls("X14802", &panel, &scene, &cc, None)),
            ["Select 6"]
        );
        let other_channel = LiveMessage::ControlChange {
            channel: 3,
            number: 51,
            value: 127,
        };
        assert!(lit_controls("X14802", &panel, &scene, &other_channel, None).is_empty());
        // Play sends on the global channel: any channel while it is unknown,
        // only the global one once the global block is loaded.
        let play = LiveMessage::NoteOn {
            channel: 4,
            note: 60,
            velocity: 127,
        };
        assert_eq!(
            label(lit_controls("X14802", &panel, &scene, &play, None)),
            ["Play"]
        );
        assert!(lit_controls("X14802", &panel, &scene, &play, Some(0)).is_empty());
        let announce = LiveMessage::SysEx(vec![
            0xF0, 0x42, 0x40, 0x00, 0x01, 0x37, 0x02, 0x00, 0x00, 0x4F, 0x02, 0xF7,
        ]);
        assert_eq!(
            lit_controls("X14802", &panel, &scene, &announce, None).len(),
            1
        );
    }

    #[test]
    fn the_jog_lights_for_both_directions() {
        // The TRAKTOR scene's jog, measured 2026-10-28: Inc/Dec 1 on CC 84
        // (clockwise) and CC 85 (anticlockwise), channel 1.
        let panel = panel("X14802").unwrap();
        let mut scene = studio_scene();
        let jog = korg_model::JogConfig {
            channel: 0,
            kind: korg_model::JogType::IncDec1,
            acceleration: korg_model::JogAcceleration::Const,
            sign_magnitude_cc: 0,
            cw_cc: 84,
            ccw_cc: 85,
            continuous_cc: 0,
            minimum: 0,
            maximum: 127,
        };
        scene[512..512 + korg_model::JogConfig::LEN].copy_from_slice(&jog.encode().unwrap());
        for number in [84, 85] {
            let turn = LiveMessage::ControlChange {
                channel: 0,
                number,
                value: 1,
            };
            let lit = lit_controls("X14802", &panel, &scene, &turn, None);
            assert_eq!(lit.len(), 1, "CC {number}");
            assert_eq!(panel.controls[lit[0]].group, "Jog Wheel");
        }
    }

    #[test]
    fn a_keyboard_note_lights_the_keyboard() {
        let panel = panel("X14800").unwrap();
        let scene = vec![0u8; 256];
        let note = LiveMessage::NoteOn {
            channel: 0,
            note: 64,
            velocity: 90,
        };
        let lit = lit_controls("X14800", &panel, &scene, &note, None);
        assert!(!lit.is_empty());
        assert!(lit.iter().all(|i| panel.controls[*i].group == "Keyboard"
            || panel.controls[*i].label == "Keyboard"
            || panel.controls[*i].group == "Trigger Pad"));
    }
}
