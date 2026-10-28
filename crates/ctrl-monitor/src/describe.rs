//! Generic names for MIDI 1.0 messages (docs/MONITOR.md §4.2, step 3):
//! message types as MIDI Monitor names them, notes, controllers,
//! manufacturers, universal SysEx. Descriptor and device describers come
//! first and are layered on top (M2).

use std::fmt::Write as _;

use crate::model::{Msg, SysExEnd};
use crate::tables::{CONTROLLERS, MANUFACTURERS};

/// The 19 message types of the type filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Class {
    NoteOn,
    NoteOff,
    PolyPressure,
    Control,
    Program,
    ChannelPressure,
    PitchBend,
    TimeCode,
    SongPosition,
    SongSelect,
    TuneRequest,
    Clock,
    Start,
    Continue,
    Stop,
    ActiveSense,
    Reset,
    SysEx,
    Invalid,
}

/// Where a class belongs, for colour and the filter's groups.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Group {
    Voice,
    Common,
    Realtime,
    SysEx,
    Invalid,
}

impl Class {
    pub const ALL: [Class; 19] = [
        Class::NoteOn,
        Class::NoteOff,
        Class::PolyPressure,
        Class::Control,
        Class::Program,
        Class::ChannelPressure,
        Class::PitchBend,
        Class::TimeCode,
        Class::SongPosition,
        Class::SongSelect,
        Class::TuneRequest,
        Class::Clock,
        Class::Start,
        Class::Continue,
        Class::Stop,
        Class::ActiveSense,
        Class::Reset,
        Class::SysEx,
        Class::Invalid,
    ];

    /// The class of `msg`; a Note On with velocity 0 is a Note Off unless
    /// `raw`. UMP and HID count as `Invalid` here: they have describers of
    /// their own.
    pub fn of(msg: &Msg, raw: bool) -> Class {
        match *msg {
            Msg::Voice { status, d2, .. } => match status & 0xF0 {
                0x90 if d2 == 0 && !raw => Class::NoteOff,
                0x90 => Class::NoteOn,
                0x80 => Class::NoteOff,
                0xA0 => Class::PolyPressure,
                0xB0 => Class::Control,
                0xC0 => Class::Program,
                0xD0 => Class::ChannelPressure,
                _ => Class::PitchBend,
            },
            Msg::Common { status, .. } => match status {
                0xF1 => Class::TimeCode,
                0xF2 => Class::SongPosition,
                0xF3 => Class::SongSelect,
                _ => Class::TuneRequest,
            },
            Msg::Realtime(status) => match status {
                0xF8 => Class::Clock,
                0xFA => Class::Start,
                0xFB => Class::Continue,
                0xFC => Class::Stop,
                0xFE => Class::ActiveSense,
                _ => Class::Reset,
            },
            Msg::SysEx { .. } => Class::SysEx,
            Msg::Ump(..) | Msg::Hid { .. } | Msg::Invalid { .. } => Class::Invalid,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Class::NoteOn => "Note On",
            Class::NoteOff => "Note Off",
            Class::PolyPressure => "Aftertouch",
            Class::Control => "Control",
            Class::Program => "Program",
            Class::ChannelPressure => "Channel Pressure",
            Class::PitchBend => "Pitch Wheel",
            Class::TimeCode => "MTC Quarter Frame",
            Class::SongPosition => "Song Position Pointer",
            Class::SongSelect => "Song Select",
            Class::TuneRequest => "Tune Request",
            Class::Clock => "Clock",
            Class::Start => "Start",
            Class::Continue => "Continue",
            Class::Stop => "Stop",
            Class::ActiveSense => "Active Sense",
            Class::Reset => "Reset",
            Class::SysEx => "SysEx",
            Class::Invalid => "Invalid",
        }
    }

    pub fn group(self) -> Group {
        match self {
            Class::NoteOn
            | Class::NoteOff
            | Class::PolyPressure
            | Class::Control
            | Class::Program
            | Class::ChannelPressure
            | Class::PitchBend => Group::Voice,
            Class::TimeCode | Class::SongPosition | Class::SongSelect | Class::TuneRequest => {
                Group::Common
            }
            Class::SysEx => Group::SysEx,
            Class::Invalid => Group::Invalid,
            _ => Group::Realtime,
        }
    }

    fn bit(self) -> u32 {
        1 << self as u32
    }
}

/// A set of classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TypeMask(u32);

impl TypeMask {
    pub const ALL: TypeMask = TypeMask((1 << 19) - 1);
    pub const NONE: TypeMask = TypeMask(0);

    pub fn contains(self, class: Class) -> bool {
        self.0 & class.bit() != 0
    }

    pub fn with(self, class: Class, on: bool) -> TypeMask {
        if on {
            TypeMask(self.0 | class.bit())
        } else {
            TypeMask(self.0 & !class.bit())
        }
    }
}

impl Default for TypeMask {
    fn default() -> Self {
        TypeMask::ALL
    }
}

/// A set of channels 1..=16. Messages without a channel always pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChannelMask(u16);

impl ChannelMask {
    pub const ALL: ChannelMask = ChannelMask(0xFFFF);

    pub fn only(channel: u8) -> ChannelMask {
        ChannelMask(1 << (channel.clamp(1, 16) - 1))
    }

    pub fn contains(self, channel: u8) -> bool {
        (1..=16).contains(&channel) && self.0 & (1 << (channel - 1)) != 0
    }

    pub fn with(self, channel: u8, on: bool) -> ChannelMask {
        if !(1..=16).contains(&channel) {
            return self;
        }
        let bit = 1 << (channel - 1);
        ChannelMask(if on { self.0 | bit } else { self.0 & !bit })
    }

    pub fn passes(self, msg: &Msg) -> bool {
        msg.channel().is_none_or(|c| self.contains(c))
    }
}

impl Default for ChannelMask {
    fn default() -> Self {
        ChannelMask::ALL
    }
}

/// How notes are named.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NoteStyle {
    /// Middle C (60) is C4 (Roland, and the KORG editor).
    #[default]
    C4,
    /// Middle C is C3 (Yamaha).
    C3,
    /// The number only.
    Number,
}

/// How values are written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Radix {
    #[default]
    Decimal,
    /// `$3F`, as MIDI Monitor writes hex.
    Hex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Style {
    pub notes: NoteStyle,
    pub values: Radix,
    /// Programs count from 1 (as on most panels) or 0.
    pub program_base: u8,
    /// Show messages as sent: Note On velocity 0 stays Note On.
    pub raw: bool,
}

impl Default for Style {
    fn default() -> Self {
        Style {
            notes: NoteStyle::C4,
            values: Radix::Decimal,
            program_base: 1,
            raw: false,
        }
    }
}

/// What the list shows for one message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Description {
    pub class: Class,
    /// 1..=16.
    pub channel: Option<u8>,
    /// "C4 vel 100", "7 Channel Volume (coarse) = 100", "Roland, 11 bytes".
    pub value: String,
}

const SHARPS: [&str; 12] = [
    "C", "C♯", "D", "D♯", "E", "F", "F♯", "G", "G♯", "A", "A♯", "B",
];

pub fn note_name(note: u8, style: NoteStyle) -> String {
    let base = match style {
        NoteStyle::C4 => -1,
        NoteStyle::C3 => -2,
        NoteStyle::Number => return note.to_string(),
    };
    format!(
        "{}{}",
        SHARPS[(note % 12) as usize],
        note as i32 / 12 + base
    )
}

/// "Volume (coarse)", or `None` when the table has no name.
pub fn controller_name(cc: u8) -> Option<&'static str> {
    CONTROLLERS.get(cc as usize).copied().flatten()
}

/// The manufacturer of a SysEx message (bytes from F0), by its 1- or 3-byte id.
pub fn manufacturer(sysex: &[u8]) -> Option<&'static str> {
    let key = match sysex {
        [0xF0, 0x00, b1, b2, ..] => 0x10000 | (*b1 as u32) << 8 | *b2 as u32,
        [0xF0, id, ..] if *id < 0x80 => *id as u32,
        _ => return None,
    };
    MANUFACTURERS
        .binary_search_by_key(&key, |(k, _)| *k)
        .ok()
        .map(|i| MANUFACTURERS[i].1)
}

/// Universal SysEx messages by their sub-ids.
fn universal(sysex: &[u8]) -> Option<&'static str> {
    let (rt, sub1, sub2) = match sysex {
        [0xF0, rt @ (0x7E | 0x7F), _, sub1, sub2, ..] => (*rt == 0x7F, *sub1, Some(*sub2)),
        [0xF0, rt @ (0x7E | 0x7F), _, sub1, ..] => (*rt == 0x7F, *sub1, None),
        _ => return None,
    };
    Some(match (rt, sub1, sub2) {
        (false, 0x06, Some(0x01)) => "Identity Request",
        (false, 0x06, Some(0x02)) => "Identity Reply",
        (false, 0x01, _) => "Sample Dump Header",
        (false, 0x02, _) => "Sample Data Packet",
        (false, 0x07, _) => "File Dump",
        (false, 0x08, _) => "MIDI Tuning",
        (false, 0x09, _) => "General MIDI",
        (false, 0x7B, _) => "End of File",
        (false, 0x7C, _) => "Wait",
        (false, 0x7D, _) => "Cancel",
        (false, 0x7E, _) => "NAK",
        (false, 0x7F, _) => "ACK",
        (true, 0x01, Some(0x01)) => "MTC Full Frame",
        (true, 0x04, Some(0x01)) => "Master Volume",
        (true, 0x04, Some(0x02)) => "Master Balance",
        (true, 0x06, _) => "MMC Command",
        (true, 0x07, _) => "MMC Response",
        (true, 0x08, _) => "MIDI Tuning",
        _ => return None,
    })
}

fn value(v: u32, radix: Radix) -> String {
    match radix {
        Radix::Decimal => v.to_string(),
        Radix::Hex => format!("${v:02X}"),
    }
}

/// `bytes` is the payload of a SysEx or invalid message, if still kept.
pub fn describe(msg: &Msg, bytes: Option<&[u8]>, style: &Style) -> Description {
    let class = Class::of(msg, style.raw);
    let v = |n: u8| value(n as u32, style.values);
    let value = match *msg {
        Msg::Voice { d1, d2, .. } => match class {
            Class::NoteOn | Class::NoteOff => {
                format!("{} vel {}", note_name(d1, style.notes), v(d2))
            }
            Class::PolyPressure => format!("{} {}", note_name(d1, style.notes), v(d2)),
            Class::Control => match controller_name(d1) {
                Some(name) => format!("{} {} = {}", v(d1), name, v(d2)),
                None => format!("{} = {}", v(d1), v(d2)),
            },
            Class::Program => value(d1 as u32 + style.program_base as u32, style.values),
            Class::ChannelPressure => v(d1),
            _ => {
                let bend = (d1 as i32 | (d2 as i32) << 7) - 0x2000;
                match style.values {
                    Radix::Decimal => format!("{bend:+}"),
                    Radix::Hex => format!("${:04X}", bend + 0x2000),
                }
            }
        },
        Msg::Common { status, d1, d2 } => match status {
            0xF1 => format!("piece {} = {}", d1 >> 4, v(d1 & 0x0F)),
            0xF2 => format!("beat {}", d1 as u32 | (d2 as u32) << 7),
            0xF3 => v(d1),
            _ => String::new(),
        },
        Msg::Realtime(_) => String::new(),
        Msg::SysEx { len, end, .. } => {
            let mut s = String::new();
            if let Some(b) = bytes {
                if let Some(name) = universal(b) {
                    s.push_str(name);
                } else if let Some(name) = manufacturer(b) {
                    s.push_str(name);
                }
            }
            if !s.is_empty() {
                s.push_str(", ");
            }
            let _ = write!(s, "{len} bytes");
            match end {
                SysExEnd::Complete => {}
                SysExEnd::Cut(_) => s.push_str(", no F7"),
                SysExEnd::Timeout => s.push_str(", timed out"),
                SysExEnd::Truncated => s.push_str(", truncated"),
            }
            if bytes.is_none() {
                s.push_str(" (bytes no longer kept)");
            }
            s
        }
        Msg::Ump(_, words) => format!("UMP, {words} words"),
        Msg::Hid { len, .. } => format!("HID report, {len} bytes"),
        Msg::Invalid { len, .. } => format!("{len} bytes"),
    };
    Description {
        class,
        channel: msg.channel(),
        value,
    }
}

/// Bytes as `F0 42 4D …`, at most `max` of them.
pub fn hex(bytes: &[u8], max: usize) -> String {
    let mut s = String::with_capacity(bytes.len().min(max) * 3 + 16);
    for (i, b) in bytes.iter().take(max).enumerate() {
        if i > 0 {
            s.push(' ');
        }
        let _ = write!(s, "{b:02X}");
    }
    if bytes.len() > max {
        let _ = write!(s, " … ({} bytes)", bytes.len());
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::BlobIdx;

    fn d(msg: Msg) -> String {
        describe(&msg, None, &Style::default()).value
    }

    #[test]
    fn notes_and_their_styles() {
        assert_eq!(note_name(60, NoteStyle::C4), "C4");
        assert_eq!(note_name(60, NoteStyle::C3), "C3");
        assert_eq!(note_name(0, NoteStyle::C4), "C-1");
        assert_eq!(note_name(127, NoteStyle::C4), "G9");
        assert_eq!(note_name(61, NoteStyle::C4), "C♯4");
        assert_eq!(note_name(61, NoteStyle::Number), "61");
        assert_eq!(
            d(Msg::Voice {
                status: 0x91,
                d1: 60,
                d2: 100
            }),
            "C4 vel 100"
        );
    }

    #[test]
    fn note_on_with_velocity_zero_is_note_off_unless_raw() {
        let msg = Msg::Voice {
            status: 0x90,
            d1: 60,
            d2: 0,
        };
        assert_eq!(
            describe(&msg, None, &Style::default()).class,
            Class::NoteOff
        );
        let raw = Style {
            raw: true,
            ..Style::default()
        };
        assert_eq!(describe(&msg, None, &raw).class, Class::NoteOn);
    }

    #[test]
    fn controllers_programs_bend_pressure() {
        assert_eq!(
            d(Msg::Voice {
                status: 0xB0,
                d1: 7,
                d2: 100
            }),
            "7 Channel Volume (coarse) = 100"
        );
        assert_eq!(
            d(Msg::Voice {
                status: 0xB0,
                d1: 3,
                d2: 1
            }),
            "3 = 1"
        );
        assert_eq!(
            d(Msg::Voice {
                status: 0xC0,
                d1: 0,
                d2: 0
            }),
            "1"
        );
        assert_eq!(
            d(Msg::Voice {
                status: 0xE0,
                d1: 0,
                d2: 0x40
            }),
            "+0"
        );
        assert_eq!(
            d(Msg::Voice {
                status: 0xE0,
                d1: 0x7F,
                d2: 0x7F
            }),
            "+8191"
        );
        assert_eq!(
            d(Msg::Voice {
                status: 0xE0,
                d1: 0,
                d2: 0
            }),
            "-8192"
        );
        assert_eq!(
            d(Msg::Voice {
                status: 0xA0,
                d1: 60,
                d2: 9
            }),
            "C4 9"
        );
        let hex = Style {
            values: Radix::Hex,
            ..Style::default()
        };
        assert_eq!(
            describe(
                &Msg::Voice {
                    status: 0xB0,
                    d1: 7,
                    d2: 100
                },
                None,
                &hex
            )
            .value,
            "$07 Channel Volume (coarse) = $64"
        );
    }

    #[test]
    fn sysex_names_its_manufacturer_or_universal_type() {
        let msg = |len| Msg::SysEx {
            blob: BlobIdx(0),
            len,
            end: SysExEnd::Complete,
        };
        let style = Style::default();
        let roland = [0xF0, 0x41, 0x10, 0x42, 0x12, 0xF7];
        assert_eq!(
            describe(&msg(6), Some(&roland), &style).value,
            "Roland, 6 bytes"
        );
        let ni = [0xF0, 0x00, 0x21, 0x09, 0xF7];
        assert_eq!(
            describe(&msg(5), Some(&ni), &style).value,
            "Native Instruments, 5 bytes"
        );
        let id = [0xF0, 0x7E, 0x7F, 0x06, 0x01, 0xF7];
        assert_eq!(
            describe(&msg(6), Some(&id), &style).value,
            "Identity Request, 6 bytes"
        );
        assert_eq!(
            describe(&msg(6), None, &style).value,
            "6 bytes (bytes no longer kept)"
        );
        let cut = Msg::SysEx {
            blob: BlobIdx(0),
            len: 3,
            end: SysExEnd::Cut(0x90),
        };
        assert_eq!(
            describe(&cut, Some(&[0xF0, 0x42, 0x30]), &style).value,
            "Korg, 3 bytes, no F7"
        );
    }

    #[test]
    fn system_messages() {
        assert_eq!(
            d(Msg::Common {
                status: 0xF2,
                d1: 0x10,
                d2: 0x01
            }),
            "beat 144"
        );
        assert_eq!(
            d(Msg::Common {
                status: 0xF1,
                d1: 0x35,
                d2: 0
            }),
            "piece 3 = 5"
        );
        let clock = describe(&Msg::Realtime(0xF8), None, &Style::default());
        assert_eq!(
            (clock.class, clock.class.group()),
            (Class::Clock, Group::Realtime)
        );
    }

    #[test]
    fn masks() {
        let types = TypeMask::ALL.with(Class::ActiveSense, false);
        assert!(!types.contains(Class::ActiveSense) && types.contains(Class::Clock));
        assert!(Class::ALL.iter().all(|c| TypeMask::ALL.contains(*c)));
        let ch = ChannelMask::only(10);
        assert!(ch.passes(&Msg::Voice {
            status: 0x99,
            d1: 36,
            d2: 1
        }));
        assert!(!ch.passes(&Msg::Voice {
            status: 0x90,
            d1: 36,
            d2: 1
        }));
        assert!(ch.passes(&Msg::Realtime(0xF8)));
    }

    #[test]
    fn hex_is_cut_at_max() {
        assert_eq!(hex(&[0xF0, 0x42, 0xF7], 8), "F0 42 F7");
        assert_eq!(hex(&[0; 5], 2), "00 00 … (5 bytes)");
    }
}
