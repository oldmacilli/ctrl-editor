//! What MIDI message a control sends in a scene: kind, number and channel.
//!
//! Both front ends colour the control list by `MessageKind` and show
//! `details` for the selected control.  Only decoded layouts are read; any
//! other control returns `None`.

use crate::panel::Control;
use korg_model::{JogAcceleration, JogConfig, JogType, ParameterKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageKind {
    Note,
    ControlChange,
    ProgramChange,
    NoAssign,
    /// The assign byte holds a value whose meaning is not decoded.
    Undecoded(u8),
}

impl MessageKind {
    pub fn name(self) -> String {
        match self {
            MessageKind::Note => "Note".into(),
            MessageKind::ControlChange => "Control Change".into(),
            MessageKind::ProgramChange => "Program Change".into(),
            MessageKind::NoAssign => "No Assign".into(),
            MessageKind::Undecoded(raw) => format!("assign type {raw} (not decoded)"),
        }
    }

    /// The list colour, shared so the GUI and the TUI agree.
    pub fn rgb(self) -> (u8, u8, u8) {
        match self {
            MessageKind::Note => (0x7c, 0xd9, 0x8a),
            MessageKind::ControlChange => (0x7f, 0xb8, 0xff),
            MessageKind::ProgramChange => (0xff, 0xb0, 0x5c),
            MessageKind::NoAssign => (0x80, 0x80, 0x80),
            MessageKind::Undecoded(_) => (0xd8, 0xd8, 0xd8),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlMessage {
    pub kind: MessageKind,
    pub number: Option<u8>,
    /// Zero-based MIDI channel; 16 means the global channel.
    pub channel: Option<u8>,
    /// A second CC: counterclockwise for a jog wheel in Inc/Dec mode.
    pub second: Option<u8>,
}

impl ControlMessage {
    #[cfg(test)]
    fn cc(number: u8) -> Self {
        Self {
            kind: MessageKind::ControlChange,
            number: Some(number),
            channel: None,
            second: None,
        }
    }

    /// Short form for badges: `CC13`, `G#3`, `PC5`, `—`.
    pub fn badge(&self) -> String {
        match (self.kind, self.number) {
            (MessageKind::Note, Some(note)) => note_name(note),
            (MessageKind::ControlChange, Some(cw)) if self.second.is_some() => {
                format!("CC{cw}/{}", self.second.unwrap_or_default())
            }
            (MessageKind::ControlChange, Some(number)) => format!("CC{number}"),
            (MessageKind::ProgramChange, Some(number)) => format!("PC{number}"),
            (MessageKind::NoAssign, _) => "—".into(),
            (_, Some(number)) => format!("#{number}"),
            (_, None) => String::new(),
        }
    }

    pub fn channel_text(&self) -> Option<String> {
        self.channel.map(|channel| {
            if channel >= 16 {
                "global channel".to_owned()
            } else {
                format!("ch {}", channel + 1)
            }
        })
    }
}

/// MIDI note name with middle C = C4 (60).
pub fn note_name(note: u8) -> String {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    let octave = i32::from(note) / 12 - 1;
    format!("{}{}", NAMES[usize::from(note) % 12], octave)
}

/// The note a name stands for, by `note_name`'s rule: `C4` = 60, `C#4` or
/// `Db4` = 61, `C-1` = 0, `G9` = 127.  Letters ignore case.
pub fn parse_note(text: &str) -> Option<u8> {
    let mut chars = text.trim().chars();
    let base: i32 = match chars.next()?.to_ascii_uppercase() {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => return None,
    };
    let rest = chars.as_str();
    let (shift, octave) = if let Some(octave) = rest.strip_prefix('#') {
        (1, octave)
    } else if let Some(octave) = rest.strip_prefix('b') {
        (-1, octave)
    } else {
        (0, rest)
    };
    let octave: i32 = octave.parse().ok()?;
    u8::try_from((octave + 1) * 12 + base + shift)
        .ok()
        .filter(|note| *note < 128)
}

/// The 1-based number at the end of a label: "Knob 3" -> 3.
pub(crate) fn ordinal(label: &str) -> Option<usize> {
    label
        .rsplit(' ')
        .next()?
        .parse::<usize>()
        .ok()
        .filter(|n| *n >= 1)
}

/// A control's short name in a group list: its number when it has one,
/// else the label.  "Mute 3" -> "3", "Marker ◀" stays.
pub fn short_label(label: &str) -> String {
    ordinal(label).map_or_else(|| label.to_owned(), |n| n.to_string())
}

/// One 8-byte nanoKONTROL Studio record: channel, assign, number, …
/// Measured 2026-10-21 (`docs/midi-sysex.md` § nanoKONTROL Studio records).
#[cfg(test)]
fn studio_record(scene: &[u8], offset: usize, button: bool) -> Option<ControlMessage> {
    let record = scene.get(offset..offset + 3)?;
    if !button {
        // Knobs and sliders: byte 1 is "Knob/Slider Enable", 0 = Enable.
        return Some(ControlMessage {
            kind: if record[1] == 0 {
                MessageKind::ControlChange
            } else {
                MessageKind::NoAssign
            },
            number: Some(record[2]),
            channel: Some(record[0]),
            second: None,
        });
    }
    // Buttons: Control Change, Note, No Assign (0 and 1 measured live, the
    // order read from the official editor's Assign Type list).
    let kind = match record[1] {
        0 => MessageKind::ControlChange,
        1 => MessageKind::Note,
        2 => MessageKind::NoAssign,
        raw => MessageKind::Undecoded(raw),
    };
    Some(ControlMessage {
        kind,
        number: Some(record[2]),
        channel: Some(record[0]),
        second: None,
    })
}

/// The message `control` sends with this scene, where the layout is decoded.
pub fn control_message(class: &str, control: &Control, scene: &[u8]) -> Option<ControlMessage> {
    let assignments = korg_devices::control_assignments(class)?;
    let item = assignments
        .assignments
        .iter()
        .find(|item| item.group == control.group && item.label == control.label)?;
    if item.meta.status == Some(korg_devices::DescriptorStatus::Unknown) {
        return None;
    }
    if item.codec.as_deref() == Some("jog-config") {
        let jog = jog_at(item.codec_offset?, scene)?;
        let (kind, number, second) = match jog.kind {
            JogType::NoAssign => (MessageKind::NoAssign, None, None),
            JogType::IncDec1 | JogType::IncDec2 => (
                MessageKind::ControlChange,
                Some(jog.cw_cc),
                Some(jog.ccw_cc),
            ),
            JogType::Continuous => (MessageKind::ControlChange, Some(jog.continuous_cc), None),
            JogType::SignMagnitude | JogType::Encoder017F | JogType::Encoder3F41 => (
                MessageKind::ControlChange,
                Some(jog.sign_magnitude_cc),
                None,
            ),
        };
        return Some(ControlMessage {
            kind,
            number,
            channel: Some(jog.channel),
            second,
        });
    }
    let byte = |offset: Option<u32>| offset.and_then(|at| scene.get(at as usize)).copied();
    let raw = item.kind_offset.and_then(|offset| byte(Some(offset)));
    if item.kind_offset.is_some() && raw.is_none() {
        return None;
    }
    let mapping = raw.and_then(|raw| item.values.iter().find(|mapping| mapping.value == raw));
    if mapping
        .is_some_and(|mapping| mapping.meta.status == Some(korg_devices::DescriptorStatus::Unknown))
    {
        return None;
    }
    let kind_name = item
        .kind
        .as_deref()
        .or_else(|| mapping.map(|mapping| mapping.kind.as_str()))
        .or(item.fallback.as_deref())?;
    let kind = match kind_name {
        "note" => MessageKind::Note,
        "control-change" => MessageKind::ControlChange,
        "program-change" => MessageKind::ProgramChange,
        "no-assign" => MessageKind::NoAssign,
        "undecoded" => MessageKind::Undecoded(raw?),
        _ => return None,
    };
    let number = item.number.or_else(|| byte(item.number_offset));
    let channel = byte(item.channel_offset);
    if (item.number_required && number.is_none()) || (item.channel_required && channel.is_none()) {
        return None;
    }
    Some(ControlMessage {
        kind,
        number,
        channel,
        second: None,
    })
}

#[cfg(test)]
fn control_message_baseline(
    class: &str,
    control: &Control,
    scene: &[u8],
) -> Option<ControlMessage> {
    let n = ordinal(&control.label);
    let byte = |offset: usize| scene.get(offset).copied();
    match (class, control.group.as_str()) {
        // The Scene button only switches scenes: no note or CC, just the
        // 0x4F announcement (measured 2026-10-21); no per-scene assignment
        // in the editor.
        ("X14802", "Scene") => Some(ControlMessage {
            kind: MessageKind::NoAssign,
            number: None,
            channel: None,
            second: None,
        }),
        ("X14802", "Jog Wheel") => {
            let jog = studio_jog(scene)?;
            let (kind, number, second) = match jog.kind {
                JogType::NoAssign => (MessageKind::NoAssign, None, None),
                JogType::IncDec1 | JogType::IncDec2 => (
                    MessageKind::ControlChange,
                    Some(jog.cw_cc),
                    Some(jog.ccw_cc),
                ),
                JogType::Continuous => (MessageKind::ControlChange, Some(jog.continuous_cc), None),
                JogType::SignMagnitude | JogType::Encoder017F | JogType::Encoder3F41 => (
                    MessageKind::ControlChange,
                    Some(jog.sign_magnitude_cc),
                    None,
                ),
            };
            Some(ControlMessage {
                kind,
                number,
                channel: Some(jog.channel),
                second,
            })
        }
        ("X14802", group) => {
            let base = studio_base(group, &control.label)?;
            studio_record(scene, base, !matches!(group, "Knob" | "Slider"))
        }
        ("X14800", "Knob") => {
            let n = n?;
            Some(ControlMessage {
                channel: byte(104 + 8 * (n - 1)),
                ..ControlMessage::cc(byte(106 + 8 * (n - 1))?)
            })
        }
        ("X14800", "Trigger Pad") => {
            let at = 32 + 8 * (n? - 1);
            let kind = match byte(at + 1)? {
                0 => MessageKind::Note,
                1 => MessageKind::ControlChange,
                2 => MessageKind::ProgramChange,
                3 => MessageKind::NoAssign,
                raw => MessageKind::Undecoded(raw),
            };
            Some(ControlMessage {
                kind,
                number: byte(at + 2),
                channel: byte(at),
                second: None,
            })
        }
        ("X14800", "X-Y Pad") => Some(ControlMessage {
            channel: byte(168),
            ..ControlMessage::cc(byte(174)?)
        }),
        ("X14800", _) => match control.label.as_str() {
            "Sustain" => Some(ControlMessage {
                channel: byte(200),
                ..ControlMessage::cc(64)
            }),
            "Touch Scale" => Some(ControlMessage {
                channel: byte(192),
                ..ControlMessage::cc(19)
            }),
            "Pitch/Mod" => Some(ControlMessage::cc(1)),
            _ => None,
        },
        ("X10820", _) => match control.label.as_str() {
            "Sustain Button" => Some(ControlMessage::cc(64)),
            "Mod Button" => Some(ControlMessage::cc(1)),
            _ => None,
        },
        _ => None,
    }
}

/// Transport buttons in record order (offset 400, stride 8), measured
/// 2026-10-21 on scene 2, whose twelve numbers are all different.
#[cfg(test)]
const TRANSPORT_RECORDS: [&str; 12] = [
    "Play",
    "Stop",
    "Rec",
    "Rewind",
    "Fast Fwd",
    "Cycle",
    "Return",
    "Set",
    "Marker ◀",
    "Marker ▶",
    "Track ◀",
    "Track ▶",
];

/// Record offset of a nanoKONTROL Studio control.  Groups of eight records,
/// stride 8; filled in from the 2026-10-21 capture.
#[cfg(test)]
fn studio_base(group: &str, label: &str) -> Option<usize> {
    let n = ordinal(label);
    let eight = |start: usize| n.filter(|n| *n <= 8).map(|n| start + 8 * (n - 1));
    match group {
        "Knob" => eight(16),
        "Slider" => eight(80),
        "Track Solo" => eight(144),
        "Track Mute" => eight(208),
        "Track Rec" => eight(272),
        "Track Select" => eight(336),
        "Transport" => {
            // Record order is not panel order.
            let record = TRANSPORT_RECORDS.iter().position(|name| *name == label)?;
            Some(400 + 8 * record)
        }
        _ => None,
    }
}

/// The nanoKONTROL Studio jog block at scene offset 512 (`docs/midi-sysex.md`).
fn jog_at(offset: u32, scene: &[u8]) -> Option<JogConfig> {
    let start = usize::try_from(offset).ok()?;
    let end = start.checked_add(JogConfig::LEN)?;
    JogConfig::decode(scene.get(start..end)?).ok()
}

#[cfg(test)]
fn studio_jog(scene: &[u8]) -> Option<JogConfig> {
    JogConfig::decode(scene.get(512..512 + JogConfig::LEN)?).ok()
}

/// The jog wheel in words.  Codes 0, 4 and 6 come from the option order,
/// not from traffic.
fn jog_details(jog: &JogConfig) -> String {
    let (mode, inferred) = match jog.kind {
        JogType::IncDec1 => ("Inc/Dec 1", true),
        JogType::IncDec2 => ("Inc/Dec 2", false),
        JogType::Continuous => ("Continuous", false),
        JogType::SignMagnitude => ("Sign Magnitude", false),
        JogType::NoAssign => ("No Assign", true),
        JogType::Encoder017F => ("Encoder 01/7F", false),
        JogType::Encoder3F41 => ("Encoder 3F/41", true),
    };
    let mut parts = vec![format!(
        "Jog Wheel · {mode}{}",
        if inferred { " (inferred)" } else { "" }
    )];
    match jog.kind {
        JogType::NoAssign => {}
        JogType::IncDec1 | JogType::IncDec2 => {
            parts.push(format!("CW CC {} / CCW CC {}", jog.cw_cc, jog.ccw_cc))
        }
        JogType::Continuous => parts.push(format!(
            "CC {} ({}..{})",
            jog.continuous_cc, jog.minimum, jog.maximum
        )),
        _ => parts.push(format!("CC {}", jog.sign_magnitude_cc)),
    }
    parts.push(format!(
        "acceleration {}",
        match jog.acceleration {
            JogAcceleration::One => "1",
            JogAcceleration::Two => "2",
            JogAcceleration::Const => "Const",
        }
    ));
    parts.push(if jog.channel >= JogConfig::GLOBAL_CHANNEL {
        "global channel".to_owned()
    } else {
        format!("ch {}", jog.channel + 1)
    });
    parts.join(" · ")
}

/// How one record byte is edited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    /// 1..16 then Global (stored 0..15, 16).
    Channel,
    /// 1..16 only (stored 0..15), for schema channels whose maximum is 15.
    ChannelOnly,
    /// Choices in stored-value order.
    Choice(&'static [&'static str]),
    /// Choices owned by a cached external descriptor.
    ExternalChoice(&'static [String]),
    Number {
        min: u8,
        max: u8,
    },
    /// A number shown `min..max` and stored plus `bias` (nanoKEY2 Transpose).
    Biased {
        min: i8,
        max: i8,
        bias: u8,
    },
    /// A number, or an empty slot when the stored byte is 128 or more
    /// (nanoKEY Studio pad note slots).
    OptionalNumber {
        min: u8,
        max: u8,
    },
    /// ASCII text of `width` bytes starting at the field's offset, padded
    /// with spaces (`studio_store_text`).
    Text {
        width: u8,
    },
}

impl FieldKind {
    /// The choice labels, channels included; empty for a number or text.
    pub fn labels(self) -> Vec<String> {
        match self {
            FieldKind::Channel => (1..=16)
                .map(|n| n.to_string())
                .chain(std::iter::once("Global".to_owned()))
                .collect(),
            FieldKind::ChannelOnly => (1..=16).map(|n| n.to_string()).collect(),
            FieldKind::Choice(labels) => labels.iter().map(|l| (*l).to_owned()).collect(),
            FieldKind::ExternalChoice(labels) => labels.to_vec(),
            FieldKind::Number { .. }
            | FieldKind::Biased { .. }
            | FieldKind::OptionalNumber { .. }
            | FieldKind::Text { .. } => Vec::new(),
        }
    }

    /// The range hint beside a field: numbers as `min..max`, channels by
    /// their labels, choices by count.
    pub fn range_hint(self) -> String {
        match self {
            FieldKind::Channel => "1..16, Global".to_owned(),
            FieldKind::ChannelOnly => "1..16".to_owned(),
            FieldKind::Choice(labels) => format!("{} choices", labels.len()),
            FieldKind::ExternalChoice(labels) => format!("{} choices", labels.len()),
            FieldKind::Number { min, max } => format!("{min}..{max}"),
            FieldKind::Biased { min, max, .. } => format!("{min}..{max}"),
            FieldKind::OptionalNumber { min, max } => format!("{min}..{max} or empty"),
            FieldKind::Text { width } => format!("up to {width} characters"),
        }
    }
}

/// The channel field a numeric schema parameter holds, when its key is
/// `MidiCh`.  Enums carry their own labels and need no conversion.
pub fn channel_field(key: &str, kind: ParameterKind, max: i32) -> Option<FieldKind> {
    (key == "MidiCh" && kind == ParameterKind::Number).then_some(if max >= 16 {
        FieldKind::Channel
    } else {
        FieldKind::ChannelOnly
    })
}

/// One editable byte of a control's record, labelled as the official editor
/// labels it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordField {
    /// Absolute scene offset.
    pub offset: usize,
    pub label: &'static str,
    pub kind: FieldKind,
}

/// The editable fields of a nanoKONTROL Studio control, read from the
/// official editor's pages (2026-10-21) and the measured record layout.
/// Knob/Slider Enable 0 = Enable is observed; 1 = Disable follows.
#[cfg(test)]
pub fn studio_record_fields(control: &Control) -> Vec<RecordField> {
    if control.group == "Scene" {
        // The official editor's Common page: the "SceneName" text editor
        // stores the scene name over
        // bytes 0..11, and "LED Mode" in byte 12; 0 = Internal, 1 = External
        // inferred from its item order.
        return vec![
            RecordField {
                offset: 0,
                label: "Scene Name",
                kind: FieldKind::Text {
                    width: STUDIO_NAME_WIDTH as u8,
                },
            },
            RecordField {
                offset: 12,
                label: "LED Mode",
                kind: FieldKind::Choice(&["Internal", "External"]),
            },
        ];
    }
    if control.group == "Jog Wheel" {
        return JOG_FIELDS
            .iter()
            .map(|&(offset, label, kind)| RecordField {
                offset: JOG_OFFSET + offset,
                label,
                kind,
            })
            .collect();
    }
    let Some(base) = studio_base(&control.group, &control.label) else {
        return Vec::new();
    };
    let field = |offset: usize, label: &'static str, kind: FieldKind| RecordField {
        offset: base + offset,
        label,
        kind,
    };
    let value = FieldKind::Number { min: 0, max: 127 };
    if matches!(control.group.as_str(), "Knob" | "Slider") {
        vec![
            field(0, "MIDI Channel", FieldKind::Channel),
            field(
                1,
                "Knob/Slider Enable",
                FieldKind::Choice(&["Enable", "Disable"]),
            ),
            field(2, "CC Number", value),
            field(3, "Left/Lower Value", value),
            field(4, "Right/Upper Value", value),
        ]
    } else {
        vec![
            field(0, "MIDI Channel", FieldKind::Channel),
            field(
                1,
                "Assign Type",
                FieldKind::Choice(&["Control Change", "Note", "No Assign"]),
            ),
            field(2, "Note/CC Number", value),
            field(3, "Off Value", value),
            field(4, "On Value", value),
            field(
                5,
                "Button Behavior",
                FieldKind::Choice(&["Momentary", "Toggle"]),
            ),
        ]
    }
}

/// The scene name's width: byte 12 is the LED Mode.
#[cfg(test)]
pub const STUDIO_NAME_WIDTH: usize = 12;

/// The text of a `FieldKind::Text` field, trailing padding removed.  The
/// device pads with spaces ("TRAKTOR     ") and the factory set with zeros.
pub fn studio_text(scene: &[u8], field: &RecordField) -> String {
    let FieldKind::Text { width } = field.kind else {
        return String::new();
    };
    let bytes = scene
        .get(field.offset..field.offset + usize::from(width))
        .unwrap_or_default();
    let end = bytes
        .iter()
        .position(|byte| !(0x20..0x7F).contains(byte))
        .unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).trim_end().to_owned()
}

/// Store `text` in a `FieldKind::Text` field: printable ASCII only, cut to
/// the width and padded with spaces as the device pads.  Returns false when
/// the field is not text or lies outside `scene`.
pub fn studio_store_text(scene: &mut [u8], field: &RecordField, text: &str) -> bool {
    let FieldKind::Text { width } = field.kind else {
        return false;
    };
    let Some(slot) = scene.get_mut(field.offset..field.offset + usize::from(width)) else {
        return false;
    };
    let mut chars = text.bytes().filter(|byte| (0x20..0x7F).contains(byte));
    for byte in slot.iter_mut() {
        *byte = chars.next().unwrap_or(b' ');
    }
    true
}

/// The jog block's scene offset (`docs/midi-sysex.md`, jog wheel).
#[cfg(test)]
const JOG_OFFSET: usize = 512;

/// The jog types in stored-value order.  0, 4 and 6 follow the option order.
#[cfg(test)]
pub const JOG_TYPES: &[&str] = &[
    "Inc/Dec 1",
    "Inc/Dec 2",
    "Continuous",
    "Sign Magnitude",
    "No Assign",
    "Encoder 01/7F",
    "Encoder 3F/41",
];

/// Every byte of the jog block, labelled as the official editor labels it.
#[cfg(test)]
const JOG_FIELDS: &[(usize, &str, FieldKind)] = &[
    (1, "Type", FieldKind::Choice(JOG_TYPES)),
    (2, "Acceleration", FieldKind::Choice(&["1", "2", "Const"])),
    // 1..16 and Global, as the editor and the owner's
    // manual p. 14 list it; the schema allows 16 at offset 512.
    (0, "MIDI Channel", FieldKind::Channel),
    (3, "CC Number", FieldKind::Number { min: 0, max: 127 }),
    (4, "CW CC Number", FieldKind::Number { min: 0, max: 127 }),
    (5, "CCW CC Number", FieldKind::Number { min: 0, max: 127 }),
    (6, "CC Number", FieldKind::Number { min: 0, max: 127 }),
    (7, "Min Value", FieldKind::Number { min: 0, max: 127 }),
    (8, "Max Value", FieldKind::Number { min: 0, max: 127 }),
];

/// The fields the official editor shows for the control in `scene`.  The jog
/// wheel shows only the bytes its type uses; every other control shows all of
/// `studio_record_fields`.
#[cfg(test)]
pub fn studio_fields(control: &Control, scene: &[u8]) -> Vec<RecordField> {
    let fields = studio_record_fields(control);
    if control.group != "Jog Wheel" {
        return fields;
    }
    let used: &[usize] = match scene
        .get(JOG_OFFSET + 1)
        .copied()
        .and_then(JogType::from_byte)
    {
        Some(JogType::NoAssign) => &[1, 0],
        Some(JogType::IncDec1 | JogType::IncDec2) => &[1, 2, 0, 4, 5],
        Some(JogType::Continuous) => &[1, 2, 0, 6, 7, 8],
        Some(JogType::SignMagnitude | JogType::Encoder017F | JogType::Encoder3F41) => &[1, 2, 0, 3],
        // An unknown type: show everything so the bytes stay reachable.
        None => return fields,
    };
    fields
        .into_iter()
        .filter(|field| used.contains(&(field.offset - JOG_OFFSET)))
        .collect()
}

/// Store `value` at a nanoKONTROL Studio scene offset.  The value is clamped
/// to the field's range, and the jog minimum never passes the maximum, so the
/// block stays decodable.  Returns false when the offset is outside `scene`.
#[cfg(test)]
pub fn studio_store(
    panel: &crate::panel::Panel,
    scene: &mut [u8],
    offset: usize,
    value: i32,
) -> bool {
    let limit = match studio_field_at(panel, offset).map(|field| field.kind) {
        Some(FieldKind::Number { min, max }) => (i32::from(min), i32::from(max)),
        // Text goes through studio_store_text.
        Some(FieldKind::Text { .. }) => return false,
        Some(kind) => (0, kind.labels().len() as i32 - 1),
        None => (0, 255),
    };
    let (low, high) = match offset.checked_sub(JOG_OFFSET) {
        Some(7) => (
            limit.0,
            scene.get(JOG_OFFSET + 8).map_or(limit.1, |m| i32::from(*m)),
        ),
        Some(8) => (
            scene.get(JOG_OFFSET + 7).map_or(limit.0, |m| i32::from(*m)),
            limit.1,
        ),
        _ => limit,
    };
    match scene.get_mut(offset) {
        Some(slot) => {
            *slot = value.clamp(low, high.max(low)) as u8;
            true
        }
        None => false,
    }
}

/// The nanoKONTROL Studio field at an absolute scene offset, for writers that
/// only know the offset.
#[cfg(test)]
pub fn studio_field_at(panel: &crate::panel::Panel, offset: usize) -> Option<RecordField> {
    panel
        .controls
        .iter()
        .flat_map(studio_record_fields)
        .find(|field| field.offset == offset)
}

/// The one-line description for the status bar under the schematic.
pub fn details(class: &str, control: &Control, scene: &[u8]) -> String {
    let assignment = korg_devices::control_assignments(class)
        .and_then(|all| {
            all.assignments
                .iter()
                .find(|item| item.group == control.group && item.label == control.label)
        })
        .filter(|item| item.meta.status != Some(korg_devices::DescriptorStatus::Unknown));
    if let Some((item, description)) = assignment.and_then(|item| {
        item.description
            .as_ref()
            .map(|description| (item, description))
    }) {
        let suffix = item
            .detail_offset
            .and_then(|offset| scene.get(offset as usize))
            .and_then(|value| {
                item.details.iter().find(|detail| {
                    detail.value == *value
                        && detail.meta.status != Some(korg_devices::DescriptorStatus::Unknown)
                })
            });
        return suffix.map_or_else(
            || description.to_owned(),
            |detail| format!("{description} · {}", detail.text),
        );
    }
    let described_jog = assignment
        .filter(|item| item.codec.as_deref() == Some("jog-config"))
        .and_then(|item| jog_at(item.codec_offset?, scene));
    if let Some(jog) = described_jog {
        return jog_details(&jog);
    }
    let mut parts = vec![control.label.clone()];
    match control_message(class, control, scene) {
        Some(message) => {
            let what = match (message.kind, message.number) {
                (MessageKind::Note, Some(note)) => format!("Note {} ({note})", note_name(note)),
                (MessageKind::NoAssign, _) => "No Assign".to_owned(),
                (kind, Some(number)) => format!("{} {number}", kind.name()),
                (kind, None) => kind.name(),
            };
            parts.push(what);
            parts.extend(message.channel_text());
        }
        None if control.group != control.label => parts.push(control.group.to_owned()),
        None => {}
    }
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panel::panel;

    fn control<'a>(panel: &'a crate::panel::Panel, label: &str) -> &'a Control {
        panel.controls.iter().find(|c| c.label == label).unwrap()
    }

    /// Scene 1 of the nanoKONTROL Studio as received on 2026-10-21, records only.
    fn studio_scene() -> Vec<u8> {
        let mut scene = vec![0xff; 560];
        let knobs = [13, 14, 15, 16, 17, 18, 19, 20];
        let sliders = [2, 3, 4, 5, 6, 8, 9, 12];
        for (i, cc) in knobs.iter().chain(sliders.iter()).enumerate() {
            scene[16 + 8 * i..16 + 8 * i + 5].copy_from_slice(&[0, 0, *cc, 0, 0x7f]);
        }
        scene
    }

    #[test]
    fn external_control_assignments_match_the_previous_decoder() {
        let expected = |class: &str, control: &Control, scene: &[u8]| {
            let mut message = control_message_baseline(class, control, scene)?;
            if class == "X14802" && matches!(control.group.as_str(), "Knob" | "Slider") {
                let raw = *scene.get(studio_base(&control.group, &control.label)? + 1)?;
                if raw != 0 {
                    message.kind = MessageKind::Undecoded(raw);
                }
            }
            Some(message)
        };
        for class in ["X10820", "X14800", "X14802"] {
            let panel = panel(class).unwrap();
            for byte in 0..=255u8 {
                let scene = vec![byte; 560];
                for control in &panel.controls {
                    assert_eq!(
                        control_message(class, control, &scene),
                        expected(class, control, &scene),
                        "{class} {} byte={byte}",
                        control.label
                    );
                }
            }
            for len in [0, 1, 2, 16, 34, 168, 174, 200, 512, 520] {
                let scene = vec![1; len];
                for control in &panel.controls {
                    assert_eq!(
                        control_message(class, control, &scene),
                        expected(class, control, &scene),
                        "{class} {} length={len}",
                        control.label
                    );
                }
            }
        }
    }

    #[test]
    fn external_jog_assignment_keeps_the_validated_codec_for_all_types() {
        let panel = panel("X14802").unwrap();
        let jog = control(&panel, "Jog Wheel");
        let mut scene = vec![0; 560];
        scene[512] = 5;
        scene[514] = 1;
        scene[515..521].copy_from_slice(&[12, 13, 14, 15, 4, 120]);
        for kind in 0..=6 {
            scene[513] = kind;
            assert_eq!(
                control_message("X14802", jog, &scene),
                control_message_baseline("X14802", jog, &scene),
                "jog kind {kind}"
            );
            assert!(control_message("X14802", jog, &scene).is_some());
        }
        scene[513] = 255;
        assert!(control_message("X14802", jog, &scene).is_none());
        assert!(control_message("X14802", jog, &scene[..520]).is_none());
    }

    #[test]
    fn scene_description_is_external_and_matches_previous_display() {
        let panel = panel("X14802").unwrap();
        let scene_control = control(&panel, "Scene");
        let assignment = korg_devices::control_assignments("X14802")
            .unwrap()
            .assignments
            .iter()
            .find(|a| a.label == "Scene")
            .unwrap();
        assert_eq!(assignment.detail_offset, Some(12));
        assert!(assignment
            .details
            .iter()
            .all(|detail| detail.meta.status == Some(korg_devices::DescriptorStatus::Inferred)));
        let prefix = "Scene · switches scenes, sends no note or CC";
        assert_eq!(details("X14802", scene_control, &[]), prefix);
        for value in 0..=255u8 {
            let mut scene = vec![0; 13];
            scene[12] = value;
            let suffix = match value {
                0 => " · LED Mode Internal",
                1 => " · LED Mode External (host drives the LEDs)",
                _ => "",
            };
            assert_eq!(
                details("X14802", scene_control, &scene),
                format!("{prefix}{suffix}")
            );
        }
    }

    #[test]
    fn unobserved_knob_enable_values_are_not_reported_as_known_assignments() {
        let panel = panel("X14802").unwrap();
        let knob = control(&panel, "Knob 1");
        let mut scene = vec![0; 560];
        for raw in [1, 2, 255] {
            scene[17] = raw;
            assert_eq!(
                control_message("X14802", knob, &scene).unwrap().kind,
                MessageKind::Undecoded(raw)
            );
        }
    }

    #[test]
    fn note_names_round_trip() {
        for note in 0..128u8 {
            assert_eq!(parse_note(&note_name(note)), Some(note), "{note}");
        }
        assert_eq!(parse_note("db4"), Some(61));
        assert_eq!(parse_note("C-1"), Some(0));
        assert_eq!(parse_note("G#9"), None);
        assert_eq!(parse_note("H3"), None);
    }

    #[test]
    fn studio_knobs_and_sliders_read_stride_eight() {
        let panel = panel("X14802").unwrap();
        let scene = studio_scene();
        let knob = control_message("X14802", control(&panel, "Knob 2"), &scene).unwrap();
        assert_eq!(knob.badge(), "CC14");
        assert_eq!(knob.channel_text().as_deref(), Some("ch 1"));
        let slider = control_message("X14802", control(&panel, "Slider 8"), &scene).unwrap();
        assert_eq!(slider.badge(), "CC12");
        assert_eq!(
            details("X14802", control(&panel, "Knob 2"), &scene),
            "Knob 2 · Control Change 14 · ch 1"
        );
    }

    #[test]
    fn nanokey_pads_follow_their_assign_byte() {
        let panel = panel("X14800").unwrap();
        let mut scene = vec![0u8; 240];
        scene[32..35].copy_from_slice(&[16, 0, 43]);
        scene[40..43].copy_from_slice(&[8, 1, 101]);
        scene[48..51].copy_from_slice(&[8, 2, 5]);
        scene[56..59].copy_from_slice(&[8, 3, 0]);
        let pad = |label| control_message("X14800", control(&panel, label), &scene).unwrap();
        assert_eq!(pad("Pad 1").kind, MessageKind::Note);
        assert_eq!(pad("Pad 1").badge(), "G2");
        assert_eq!(
            pad("Pad 1").channel_text().as_deref(),
            Some("global channel")
        );
        assert_eq!(pad("Pad 2").badge(), "CC101");
        assert_eq!(pad("Pad 3").badge(), "PC5");
        assert_eq!(pad("Pad 4").kind, MessageKind::NoAssign);
    }

    #[test]
    fn the_studio_jog_wheel_reads_the_measured_blocks() {
        let panel = panel("X14802").unwrap();
        let jog = control(&panel, "Jog Wheel");
        let mut scene = vec![0u8; 560];
        // KLARA: ch5, Inc/Dec 2, acceleration 2.
        scene[512..521].copy_from_slice(&[0x04, 0x01, 0x01, 0x52, 0x53, 0x55, 0x56, 0x00, 0x7f]);
        let message = control_message("X14802", jog, &scene).unwrap();
        assert_eq!(message.badge(), "CC83/85");
        assert_eq!(
            details("X14802", jog, &scene),
            "Jog Wheel · Inc/Dec 2 · CW CC 83 / CCW CC 85 · acceleration 2 · ch 5"
        );
        // GONZO: ch7, Encoder 01/7F, Const.
        scene[512..521].copy_from_slice(&[0x06, 0x05, 0x02, 0x53, 0x54, 0x56, 0x57, 0x00, 0x7f]);
        assert_eq!(
            details("X14802", jog, &scene),
            "Jog Wheel · Encoder 01/7F · CC 83 · acceleration Const · ch 7"
        );
    }

    #[test]
    fn every_studio_control_has_a_message() {
        let panel = panel("X14802").unwrap();
        let scene = vec![0u8; 560];
        let missing: Vec<&str> = panel
            .controls
            .iter()
            .filter(|c| control_message("X14802", c, &scene).is_none())
            .map(|c| c.label.as_str())
            .collect();
        assert!(missing.is_empty(), "{missing:?}");
        let scene_button = control(&panel, "Scene");
        let mut traktor = scene.clone();
        traktor[12] = 1;
        assert_eq!(
            details("X14802", scene_button, &traktor),
            "Scene · switches scenes, sends no note or CC · LED Mode External (host drives the LEDs)"
        );
    }

    #[test]
    fn studio_fields_follow_the_official_editor() {
        let panel = panel("X14802").unwrap();
        let knob = studio_record_fields(control(&panel, "Knob 3"));
        let labels: Vec<&str> = knob.iter().map(|f| f.label).collect();
        assert_eq!(
            labels,
            [
                "MIDI Channel",
                "Knob/Slider Enable",
                "CC Number",
                "Left/Lower Value",
                "Right/Upper Value"
            ]
        );
        assert_eq!(knob[0].offset, 32);
        let mute = studio_record_fields(control(&panel, "Mute 6"));
        assert_eq!(mute[2].offset, 208 + 5 * 8 + 2);
        assert_eq!(mute[5].label, "Button Behavior");
        assert_eq!(
            FieldKind::Channel.labels().last().map(String::as_str),
            Some("Global")
        );
        assert_eq!(
            studio_field_at(&panel, 33).unwrap().label,
            "Knob/Slider Enable"
        );
        // The nonzero enable value has not been independently observed.
        let mut scene = vec![0u8; 560];
        scene[17] = 1;
        let message = control_message("X14802", control(&panel, "Knob 1"), &scene).unwrap();
        assert_eq!(message.kind, MessageKind::Undecoded(1));
    }

    #[test]
    fn studio_jog_fields_follow_the_type() {
        let panel = panel("X14802").unwrap();
        let jog = control(&panel, "Jog Wheel");
        let mut scene = vec![0u8; 560];
        let labels = |scene: &[u8]| -> Vec<&str> {
            studio_fields(jog, scene).iter().map(|f| f.label).collect()
        };
        scene[513] = 1;
        assert_eq!(
            labels(&scene),
            [
                "Type",
                "Acceleration",
                "MIDI Channel",
                "CW CC Number",
                "CCW CC Number"
            ]
        );
        scene[513] = 2;
        assert_eq!(
            labels(&scene),
            [
                "Type",
                "Acceleration",
                "MIDI Channel",
                "CC Number",
                "Min Value",
                "Max Value"
            ]
        );
        assert_eq!(studio_fields(jog, &scene)[3].offset, 518);
        scene[513] = 4;
        assert_eq!(labels(&scene), ["Type", "MIDI Channel"]);
        scene[513] = 6;
        assert_eq!(studio_fields(jog, &scene)[3].offset, 515);
        scene[513] = 9;
        assert_eq!(studio_fields(jog, &scene).len(), 9);
        // The jog channel offers Global, like every other control.
        assert_eq!(
            studio_field_at(&panel, 512).unwrap().kind,
            FieldKind::Channel
        );
    }

    #[test]
    fn studio_store_keeps_the_jog_block_decodable() {
        let panel = panel("X14802").unwrap();
        let mut scene = vec![0u8; 560];
        scene[512..521].copy_from_slice(&[0, 2, 0, 0, 0, 0, 86, 10, 100]);
        assert!(studio_store(&panel, &mut scene, 519, 120));
        assert_eq!(scene[519], 100);
        assert!(studio_store(&panel, &mut scene, 520, 3));
        assert_eq!(scene[520], 100);
        assert!(studio_store(&panel, &mut scene, 512, 17));
        assert_eq!(scene[512], 16, "Global is the highest channel");
        assert!(studio_store(&panel, &mut scene, 513, -1));
        assert_eq!(scene[513], 0);
        assert!(JogConfig::decode(&scene[512..521]).is_ok());
        assert!(!studio_store(&panel, &mut scene, 600, 1));
    }

    #[test]
    fn studio_scene_name_is_twelve_space_padded_bytes() {
        let panel = panel("X14802").unwrap();
        let scene_control = panel
            .controls
            .iter()
            .find(|control| control.group == "Scene")
            .unwrap();
        let mut scene = vec![0u8; 560];
        scene[..7].copy_from_slice(b"Scene 2");
        scene[12] = 1;
        let fields = studio_fields(scene_control, &scene);
        let name = fields[0];
        assert_eq!(name.label, "Scene Name");
        assert_eq!(fields[1].label, "LED Mode");
        assert_eq!(studio_text(&scene, &name), "Scene 2");
        assert!(studio_store_text(&mut scene, &name, "TRAKTOR"));
        assert_eq!(&scene[..12], b"TRAKTOR     ");
        assert_eq!(studio_text(&scene, &name), "TRAKTOR");
        // Cut to 12, non-ASCII dropped, LED Mode untouched.
        assert!(studio_store_text(
            &mut scene,
            &name,
            "Ä very long scene name"
        ));
        assert_eq!(&scene[..12], b" very long s");
        assert_eq!(scene[12], 1);
        // Byte studio_store never touches the name.
        assert!(!studio_store(&panel, &mut scene, 0, 5));
        assert_eq!(scene[0], b' ');
    }

    #[test]
    fn short_labels_keep_the_number() {
        assert_eq!(short_label("Mute 3"), "3");
        assert_eq!(short_label("Marker ◀"), "Marker ◀");
        assert_eq!(note_name(60), "C4");
    }

    #[test]
    fn schema_channels_use_the_field_labels() {
        assert_eq!(
            channel_field("MidiCh", ParameterKind::Number, 16),
            Some(FieldKind::Channel)
        );
        assert_eq!(
            channel_field("MidiCh", ParameterKind::Number, 15),
            Some(FieldKind::ChannelOnly)
        );
        // Enums carry their own labels.
        assert_eq!(channel_field("MidiCh", ParameterKind::Enum, 16), None);
        assert_eq!(channel_field("Assign", ParameterKind::Number, 16), None);
        assert_eq!(FieldKind::Channel.range_hint(), "1..16, Global");
        assert_eq!(FieldKind::ChannelOnly.range_hint(), "1..16");
        assert_eq!(FieldKind::Channel.labels()[16], "Global");
    }
}
