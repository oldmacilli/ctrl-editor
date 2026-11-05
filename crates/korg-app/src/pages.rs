//! The parameter pages of the original editor, per device: which fields a
//! panel control shows, how they are labelled, and how a shown value is
//! stored.  Both front ends render these rows, so a page cannot differ
//! between the GUI and the TUI.
//!
//! Devices without a page here (`fields` returns `None`) show the generic
//! schema rows of `korg_core::ViewModel::params`.

use crate::assign::{self, FieldKind, RecordField};
use crate::panel::{Control, Panel};

/// One row of a page: a section heading or an editable field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageRow {
    Heading(&'static str),
    Field(PageField),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageField {
    /// Absolute scene offset.
    pub offset: usize,
    pub label: &'static str,
    pub kind: FieldKind,
    /// False when another field makes this one meaningless (the nanoKEY2
    /// Const Velocity Value outside the Const curve, the nanoKEY Studio
    /// touch values with touch disabled).  Shown, not editable.
    pub enabled: bool,
    /// The number is a note with the current assign type, so it is shown
    /// with its name ("60 · C4") and accepts one as input.
    pub note: bool,
    pub min_from_offset: Option<usize>,
    pub max_from_offset: Option<usize>,
}

impl PageField {
    fn new(offset: usize, label: &'static str, kind: FieldKind) -> Self {
        Self {
            offset,
            label,
            kind,
            enabled: true,
            note: false,
            min_from_offset: None,
            max_from_offset: None,
        }
    }

    /// The note name of a note field's value, `None` for any other field
    /// or an empty slot.
    pub fn note_name(&self, scene: &[u8]) -> Option<String> {
        let raw = *scene.get(self.offset)?;
        (self.note && raw < 128).then(|| assign::note_name(raw))
    }

    /// A typed value: a number in range, or a note name in a note field.
    pub fn parse_number(&self, text: &str) -> Option<i32> {
        let (min, max) = self.range();
        let text = text.trim();
        text.parse::<i32>()
            .ok()
            .or_else(|| {
                self.note
                    .then(|| assign::parse_note(text))
                    .flatten()
                    .map(i32::from)
            })
            .filter(|value| (min..=max).contains(value))
    }

    /// The value as the original shows it: a choice label, a channel, a
    /// number with its bias removed, text, or "—" for an empty slot.
    pub fn display(&self, scene: &[u8]) -> String {
        let raw = scene.get(self.offset).copied();
        match self.kind {
            FieldKind::Text { .. } => assign::studio_text(scene, &self.record()),
            FieldKind::Number { .. } | FieldKind::OptionalNumber { .. }
                if self.note_name(scene).is_some() =>
            {
                format!(
                    "{} · {}",
                    raw.unwrap_or_default(),
                    self.note_name(scene).unwrap_or_default()
                )
            }
            FieldKind::Number { .. } => raw.map_or_else(String::new, |raw| raw.to_string()),
            FieldKind::Biased { bias, .. } => raw
                .map(|raw| (i32::from(raw) - i32::from(bias)).to_string())
                .unwrap_or_default(),
            FieldKind::OptionalNumber { .. } => match raw {
                Some(raw) if raw < 128 => raw.to_string(),
                _ => "—".to_owned(),
            },
            FieldKind::Channel
            | FieldKind::ChannelOnly
            | FieldKind::Choice(_)
            | FieldKind::ExternalChoice(_) => {
                let raw = raw.unwrap_or(0);
                self.kind
                    .labels()
                    .get(usize::from(raw))
                    .cloned()
                    .unwrap_or_else(|| raw.to_string())
            }
        }
    }

    /// The value a spinner or a choice list starts from: the shown number,
    /// or the choice index; `None` for text and empty slots.
    pub fn shown(&self, scene: &[u8]) -> Option<i32> {
        let raw = i32::from(*scene.get(self.offset)?);
        match self.kind {
            FieldKind::Text { .. } => None,
            FieldKind::Biased { bias, .. } => Some(raw - i32::from(bias)),
            FieldKind::OptionalNumber { .. } if raw >= 128 => None,
            _ => Some(raw),
        }
    }

    /// The shown range: `min..max` of a number, `0..len-1` of a choice.
    pub fn range(&self) -> (i32, i32) {
        match self.kind {
            FieldKind::Number { min, max } | FieldKind::OptionalNumber { min, max } => {
                (i32::from(min), i32::from(max))
            }
            FieldKind::Biased { min, max, .. } => (i32::from(min), i32::from(max)),
            FieldKind::Text { .. } => (0, 0),
            kind => (0, kind.labels().len() as i32 - 1),
        }
    }

    pub(crate) fn record(&self) -> RecordField {
        RecordField {
            offset: self.offset,
            label: self.label,
            kind: self.kind,
        }
    }
}

/// Labels of choices that are shown but never written.
const UNDECODED: &str = "not decoded";

/// The page `control` opens on `class`, or the device's default page when
/// no control is selected.  `None` for devices without decoded pages.
pub fn fields(
    class: &str,
    _panel: &Panel,
    control: Option<&Control>,
    scene: &[u8],
) -> Option<Vec<PageRow>> {
    if let Some(pages) = korg_devices::editor_pages(class) {
        let name = match control {
            None => Some(pages.default_page.as_str()),
            Some(control) => pages
                .selectors
                .iter()
                .find(|selector| {
                    selector
                        .group
                        .as_ref()
                        .is_none_or(|group| group == &control.group)
                        && selector
                            .label
                            .as_ref()
                            .is_none_or(|label| label == &control.label)
                })
                .map(|selector| selector.page.as_str()),
        };
        let Some(page) = name.and_then(|name| pages.pages.iter().find(|page| page.name == name))
        else {
            return Some(Vec::new());
        };
        return page
            .fields
            .iter()
            .filter(|field| match field.hidden_if_offset {
                Some(offset) => scene
                    .get(offset as usize)
                    .is_some_and(|byte| !field.hidden_if_values.contains(&u32::from(*byte))),
                None => true,
            })
            .map(|field| {
                if field.kind == "heading" {
                    return Some(PageRow::Heading(&field.label));
                }
                let kind = match field.kind.as_str() {
                    "channel" => FieldKind::Channel,
                    "channel-only" => FieldKind::ChannelOnly,
                    "choice" => FieldKind::ExternalChoice(&field.choices),
                    "number" => FieldKind::Number {
                        min: u8::try_from(field.min?).ok()?,
                        max: u8::try_from(field.max?).ok()?,
                    },
                    "optional-number" => FieldKind::OptionalNumber {
                        min: u8::try_from(field.min?).ok()?,
                        max: u8::try_from(field.max?).ok()?,
                    },
                    "text" => FieldKind::Text {
                        width: u8::try_from(field.width?).ok()?,
                    },
                    "biased" => FieldKind::Biased {
                        min: i8::try_from(field.min?).ok()?,
                        max: i8::try_from(field.max?).ok()?,
                        bias: u8::try_from(field.bias?).ok()?,
                    },
                    _ => return None,
                };
                let enabled = field.meta.status != Some(korg_devices::DescriptorStatus::Unknown)
                    && match (
                        field.enabled_if_offset,
                        field.enabled_if_value,
                        field.enabled_if_not_value,
                    ) {
                        (None, None, None) => true,
                        (Some(offset), Some(value), None) => {
                            scene.get(offset as usize).map(|byte| u32::from(*byte)) == Some(value)
                        }
                        (Some(offset), None, Some(value)) => scene
                            .get(offset as usize)
                            .is_some_and(|byte| u32::from(*byte) != value),
                        _ => return None,
                    };
                let note = match (field.note_if_offset, field.note_if_value) {
                    (None, None) => false,
                    (Some(offset), Some(value)) => {
                        scene.get(offset as usize).map(|byte| u32::from(*byte)) == Some(value)
                    }
                    _ => return None,
                };
                Some(PageRow::Field(PageField {
                    enabled,
                    note,
                    min_from_offset: field.min_from_offset.map(|offset| offset as usize),
                    max_from_offset: field.max_from_offset.map(|offset| offset as usize),
                    ..PageField::new(field.offset as usize, &field.label, kind)
                }))
            })
            .collect();
    }
    None
}

/// The field at `offset` on the page `control` opens.
pub fn field_at(
    class: &str,
    panel: &Panel,
    control: Option<&Control>,
    scene: &[u8],
    offset: usize,
) -> Option<PageField> {
    fields(class, panel, control, scene)?
        .into_iter()
        .find_map(|row| match row {
            PageRow::Field(field) if field.offset == offset => Some(field),
            _ => None,
        })
}

/// Store a shown value (a number, or a choice index) in `field`.  The value
/// is clamped to the field's range and encoded as the device stores it; the
/// nanoKONTROL Studio jog keeps its minimum below its maximum.  Returns false
/// when nothing may be written: text, a disabled field, an undecoded choice,
/// or an offset outside `scene`.
pub fn store(
    _class: &str,
    _panel: &Panel,
    scene: &mut [u8],
    field: &PageField,
    value: i32,
) -> bool {
    if !field.enabled || matches!(field.kind, FieldKind::Text { .. }) {
        return false;
    }
    let (min, max) = field.range();
    let choice_value = value.clamp(min, max.max(min));
    if let FieldKind::Choice(labels) = field.kind {
        if labels.get(choice_value as usize) == Some(&UNDECODED) {
            return false;
        }
    }
    if let FieldKind::ExternalChoice(labels) = field.kind {
        if labels.get(choice_value as usize).map(String::as_str) == Some(UNDECODED) {
            return false;
        }
    }
    let min = match field.min_from_offset {
        Some(offset) => match scene.get(offset) {
            Some(byte) => min.max(i32::from(*byte)),
            None => return false,
        },
        None => min,
    };
    let max = match field.max_from_offset {
        Some(offset) => match scene.get(offset) {
            Some(byte) => max.min(i32::from(*byte)),
            None => return false,
        },
        None => max,
    };
    if min > max {
        return false;
    }
    let value = value.clamp(min, max);
    let raw = match field.kind {
        FieldKind::Biased { bias, .. } => value + i32::from(bias),
        _ => value,
    };
    match scene.get_mut(field.offset) {
        Some(slot) => {
            *slot = raw.clamp(0, 255) as u8;
            true
        }
        None => false,
    }
}

/// Store text in a text field; see `assign::studio_store_text`.
pub fn store_text(scene: &mut [u8], field: &PageField, text: &str) -> bool {
    field.enabled && assign::studio_store_text(scene, &field.record(), text)
}

/// The choice index of `label` in a choice or channel field.
pub fn choice_index(field: &PageField, label: &str) -> Option<i32> {
    field
        .kind
        .labels()
        .iter()
        .position(|choice| choice.eq_ignore_ascii_case(label))
        .map(|index| index as i32)
}

// nanoKEY Studio (X14800) ---------------------------------------------------

/// The X14800 page a panel control opens.
#[cfg(test)]
fn x14800_page(control: &Control) -> Option<&'static str> {
    match (control.group.as_str(), control.label.as_str()) {
        (_, "Keyboard") | ("Keyboard", _) => Some("Keyboard"),
        (_, "Chord Pad") => Some("Chord"),
        (_, "X-Y Pad") => Some("XY"),
        (_, "Pitch/Mod") => Some("PitchMod"),
        (_, "Touch Scale") => Some("TouchScale"),
        (_, "Sustain") => Some("Sustain"),
        ("Knob", _) => Some("Knob"),
        ("Trigger Pad", _) => Some("Pad"),
        _ => None,
    }
}

#[cfg(test)]
const CHANNEL: FieldKind = FieldKind::Channel;
#[cfg(test)]
const VALUE: FieldKind = FieldKind::Number { min: 0, max: 127 };
#[cfg(test)]
const TOUCH_ASSIGN: FieldKind = FieldKind::Choice(&["Control Change", "No Assign"]);
#[cfg(test)]
const ENABLE: FieldKind = FieldKind::Choice(&["Enable", "Disable"]);
#[cfg(test)]
const BEHAVIOR: FieldKind = FieldKind::Choice(&["Momentary", "Toggle"]);

#[cfg(test)]
fn x14800(page: Option<&'static str>, scene: &[u8]) -> Vec<PageRow> {
    let f = |offset, label, kind| PageRow::Field(PageField::new(offset, label, kind));
    let byte = |offset: usize| scene.get(offset).copied().unwrap_or(0);
    let Some(page) = page else {
        return Vec::new();
    };
    match page {
        "Keyboard" => vec![f(16, "MIDI Channel", CHANNEL)],
        "Chord" => vec![f(96, "MIDI Channel", CHANNEL)],
        "XY" => {
            // Touch values mean nothing while touch is set to No Assign.
            let touch = byte(169) != 1;
            let touch_field = |offset, label| {
                PageRow::Field(PageField {
                    enabled: touch,
                    ..PageField::new(offset, label, VALUE)
                })
            };
            vec![
                PageRow::Heading("Common"),
                f(168, "MIDI Channel", CHANNEL),
                PageRow::Heading("Touch"),
                f(169, "Enable", TOUCH_ASSIGN),
                touch_field(170, "CC Number"),
                touch_field(171, "Off Value"),
                touch_field(172, "On Value"),
                PageRow::Heading("X-axis"),
                f(173, "Assign Type", TOUCH_ASSIGN),
                f(174, "CC Number", VALUE),
                f(175, "Left Value", VALUE),
                f(176, "Right Value", VALUE),
                PageRow::Heading("Y-axis"),
                f(177, "Assign Type", TOUCH_ASSIGN),
                f(178, "CC Number", VALUE),
                f(179, "Lower Value", VALUE),
                f(180, "Upper Value", VALUE),
            ]
        }
        "PitchMod" => vec![
            PageRow::Heading("Mod"),
            f(184, "Enable", ENABLE),
            f(185, "CC Number", VALUE),
            f(186, "Lower Value", VALUE),
            f(187, "Upper Value", VALUE),
        ],
        "TouchScale" => vec![
            PageRow::Heading("Common"),
            f(192, "MIDI Channel", CHANNEL),
            PageRow::Heading("Y-axis"),
            f(193, "Enable", ENABLE),
            f(194, "CC Number", VALUE),
            f(195, "Lower Value", VALUE),
            f(196, "Upper Value", VALUE),
        ],
        "Sustain" => vec![
            f(200, "MIDI Channel", CHANNEL),
            f(
                201,
                "Assign Type",
                FieldKind::Choice(&["Control Change", UNDECODED]),
            ),
            f(202, "CC Number", VALUE),
            f(203, "Off Value", VALUE),
            f(204, "On Value", VALUE),
            f(205, "Button Behavior", BEHAVIOR),
        ],
        _ => Vec::new(),
    }
}

/// X14800 knob and pad pages need the control's number.
#[cfg(test)]
fn x14800_numbered(control: &Control) -> Option<(usize, &'static str)> {
    let n = assign::ordinal(&control.label).filter(|n| *n <= 8)?;
    match control.group.as_str() {
        "Knob" => Some((104 + 8 * (n - 1), "Knob")),
        "Trigger Pad" => Some((32 + 8 * (n - 1), "Pad")),
        _ => None,
    }
}

#[cfg(test)]
const NOTE_SLOTS: [&str; 4] = [
    "Note/CC/Prog Number 1",
    "Note/CC/Prog Number 2",
    "Note/CC/Prog Number 3",
    "Note/CC/Prog Number 4",
];

#[cfg(test)]
fn x14800_record(base: usize, page: &str, scene: &[u8]) -> Vec<PageRow> {
    let f = |offset, label, kind| PageRow::Field(PageField::new(offset, label, kind));
    match page {
        "Knob" => vec![
            f(base, "MIDI Channel", CHANNEL),
            f(base + 1, "Enable", ENABLE),
            f(base + 2, "CC Number", VALUE),
            f(base + 3, "Left Value", VALUE),
            f(base + 4, "Right Value", VALUE),
        ],
        _ => {
            let mut rows = vec![
                f(base, "MIDI Channel", CHANNEL),
                f(
                    base + 1,
                    "Assign Type",
                    FieldKind::Choice(&["Note", "Control Change", "Program Change", "No Assign"]),
                ),
            ];
            // Assign Type 0 = Note.
            let note = scene.get(base + 1) == Some(&0);
            for (slot, label) in NOTE_SLOTS.iter().enumerate() {
                rows.push(PageRow::Field(PageField {
                    note,
                    ..PageField::new(
                        base + 2 + slot,
                        label,
                        FieldKind::OptionalNumber { min: 0, max: 127 },
                    )
                }));
            }
            rows.push(f(base + 6, "Pad Behavior", BEHAVIOR));
            rows
        }
    }
}

// nanoKEY2 (X10820) ---------------------------------------------------------

/// The page of the original's nanoKEY2 editor a panel control opens.  Its
/// `ControllerNames` are Common, Keyboard, Mod Button, Sustain Button and
/// Pitch Button: both pitch buttons share one page, and the octave buttons
/// have none, so they show Common, which the panel has no control for.
#[cfg(test)]
fn x10820_page(control: &Control) -> &str {
    match control.group.as_str() {
        "Octave" => "Common",
        "Pitch Button" => "Pitch Button",
        _ => control.label.as_str(),
    }
}

/// One nanoKEY2 page, labelled as in the original editor.
#[cfg(test)]
fn x10820(page: &str, scene: &[u8]) -> Vec<PageRow> {
    let f = |offset, label, kind| PageRow::Field(PageField::new(offset, label, kind));
    let button = |base: usize| {
        vec![
            f(
                base,
                "Button Enable",
                FieldKind::Choice(&["Disable", "Enable"]),
            ),
            f(base + 2, "Button Behavior", BEHAVIOR),
            f(base + 1, "CC Number", VALUE),
            f(base + 4, "On Value", VALUE),
            f(base + 3, "Off Value", VALUE),
            f(
                base + 5,
                "Switch Speed",
                FieldKind::Choice(&["Immediate", "Fast", "Normal"]),
            ),
        ]
    };
    match page {
        // Shown 1..16, stored 0..15.
        "Common" => vec![f(0, "Key Mode MIDI Channel", FieldKind::ChannelOnly)],
        "Keyboard" => vec![
            // Stored with the schema's bias of 64.
            f(
                8,
                "Transpose",
                FieldKind::Biased {
                    min: -12,
                    max: 12,
                    bias: 64,
                },
            ),
            f(
                9,
                "Velocity Curve",
                FieldKind::Choice(&["Light", "Normal", "Heavy", "Const"]),
            ),
            PageRow::Field(PageField {
                enabled: scene.get(9) == Some(&3),
                ..PageField::new(10, "Const Velocity Value", VALUE)
            }),
        ],
        "Pitch Button" => vec![f(
            2,
            "Pitch Bend Speed",
            FieldKind::Choice(&["No", "Fast", "Normal", "Slow"]),
        )],
        "Sustain Button" => button(24),
        "Mod Button" => button(16),
        _ => Vec::new(),
    }
}

/// The rows of an X14800 page, knobs and pads by their number.
#[cfg(test)]
fn x14800_rows(control: Option<&Control>, scene: &[u8]) -> Vec<PageRow> {
    match control.and_then(x14800_numbered) {
        Some((base, page)) => x14800_record(base, page, scene),
        None => x14800(control.map_or(Some("Keyboard"), x14800_page), scene),
    }
}

// Generic schema rows ------------------------------------------------------

/// The label both front ends show for a schema row of a device without a
/// decoded page: the schema key split into words ("VelocityCurve" ->
/// "Velocity Curve", "MidiCh" -> "MIDI Channel", "3 SwType" -> "3 Switch
/// Type").  Words already apart ("Global MIDI Channel") stay as they are.
pub fn schema_label(label: &str) -> String {
    label
        .split(' ')
        .map(|word| {
            let mut words: Vec<String> = Vec::new();
            for c in word.chars() {
                let split = c.is_uppercase()
                    && words.last().is_some_and(|last| {
                        last.chars().last().is_some_and(|prev| !prev.is_uppercase())
                    });
                if split || words.is_empty() {
                    words.push(String::new());
                }
                words.last_mut().unwrap().push(c);
            }
            words
                .into_iter()
                .map(|word| match word.as_str() {
                    "Midi" => "MIDI".to_owned(),
                    "Ch" => "Channel".to_owned(),
                    "Sw" => "Switch".to_owned(),
                    "Verocity" => "Velocity".to_owned(),
                    _ => word,
                })
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join(" ")
        .replace("On Off", "On/Off")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panel::panel;

    fn page(class: &str, label: &str, scene: &[u8]) -> Vec<PageRow> {
        let panel = panel(class).unwrap();
        let control = panel.controls.iter().find(|c| c.label == label).unwrap();
        fields(class, &panel, Some(control), scene).unwrap()
    }

    fn labels(rows: &[PageRow]) -> Vec<&'static str> {
        rows.iter()
            .map(|row| match row {
                PageRow::Heading(label) => *label,
                PageRow::Field(field) => field.label,
            })
            .collect()
    }

    #[test]
    fn external_nano_key2_pages_match_the_previous_implementation() {
        let panel = panel("X10820").unwrap();
        for velocity in 0..=255u8 {
            let mut scene = vec![0; 560];
            scene[9] = velocity;
            for control in std::iter::once(None).chain(panel.controls.iter().map(Some)) {
                let expected = x10820(control.map_or("Common", x10820_page), &scene);
                let actual = fields("X10820", &panel, control, &scene).unwrap();
                assert_eq!(actual.len(), expected.len());
                for (a, b) in actual.iter().zip(&expected) {
                    match (a, b) {
                        (PageRow::Heading(a), PageRow::Heading(b)) => assert_eq!(a, b),
                        (PageRow::Field(a), PageRow::Field(b)) => {
                            assert_eq!(
                                (a.offset, a.label, a.enabled, a.note),
                                (b.offset, b.label, b.enabled, b.note)
                            );
                            assert_eq!(a.range(), b.range());
                            assert_eq!(a.kind.labels(), b.kind.labels());
                            assert_eq!(a.display(&scene), b.display(&scene));
                        }
                        _ => panic!("row type changed"),
                    }
                }
            }
        }
    }

    #[test]
    fn every_control_of_a_paged_device_opens_a_page() {
        for class in ["X10820", "X14800", "X14802"] {
            let panel = panel(class).unwrap();
            let scene = vec![0u8; 560];
            for control in &panel.controls {
                let rows = fields(class, &panel, Some(control), &scene).unwrap();
                assert!(
                    !rows.is_empty(),
                    "{class} {} opens an empty page",
                    control.label
                );
            }
        }
    }

    #[test]
    fn studio_button_numbers_show_note_names_for_note_buttons() {
        let mut scene = vec![0u8; 560];
        // Play (record 400): Assign Type Note, number 60.
        scene[401] = 1;
        scene[402] = 60;
        let play = page("X14802", "Play", &scene);
        let number = play
            .iter()
            .find_map(|row| match row {
                PageRow::Field(field) if field.label == "Note/CC Number" => Some(field.clone()),
                _ => None,
            })
            .unwrap();
        assert_eq!(number.display(&scene), "60 · C4");
        scene[401] = 0;
        let stop = page("X14802", "Play", &scene);
        assert!(stop.iter().all(|row| match row {
            PageRow::Field(field) => !field.note,
            PageRow::Heading(_) => true,
        }));
    }

    #[test]
    fn nanokey2_pages_follow_the_original() {
        let mut scene = vec![0u8; 64];
        scene[8] = 61;
        scene[9] = 1;
        assert_eq!(
            labels(&page("X10820", "Octave Down", &scene)),
            ["Key Mode MIDI Channel"]
        );
        assert_eq!(
            labels(&page("X10820", "Pitch Up", &scene)),
            ["Pitch Bend Speed"]
        );
        let keyboard = page("X10820", "Keyboard", &scene);
        let PageRow::Field(transpose) = &keyboard[0] else {
            panic!()
        };
        assert_eq!(transpose.display(&scene), "-3");
        let PageRow::Field(constant) = &keyboard[2] else {
            panic!()
        };
        assert!(
            !constant.enabled,
            "Const Velocity Value needs the Const curve"
        );
        let panel = panel("X10820").unwrap();
        assert!(store("X10820", &panel, &mut scene, transpose, 5));
        assert_eq!(scene[8], 69);
        assert!(!store("X10820", &panel, &mut scene, constant, 50));
        let channel = field_at("X10820", &panel, None, &scene, 0).unwrap();
        assert_eq!(channel.display(&scene), "1");
        assert!(store("X10820", &panel, &mut scene, &channel, 15));
        assert_eq!(channel.display(&scene), "16");
    }

    #[test]
    fn nanokey_studio_pads_show_empty_slots_and_never_write_undecoded() {
        let mut scene = vec![0u8; 256];
        scene[34] = 36;
        scene[35] = 0xFF;
        let pad = page("X14800", "Pad 1", &scene);
        let slot = |index: usize| match &pad[index] {
            PageRow::Field(field) => field.display(&scene),
            PageRow::Heading(_) => panic!(),
        };
        // Assign Type 0 = Note: the slots show and accept note names.
        assert_eq!(slot(2), "36 · C2");
        assert_eq!(slot(3), "—");
        let PageRow::Field(first) = &pad[2] else {
            panic!()
        };
        assert_eq!(first.parse_number("C#2"), Some(37));
        assert_eq!(first.parse_number("40"), Some(40));
        let mut cc = scene.clone();
        cc[33] = 1;
        let pad = page("X14800", "Pad 1", &cc);
        let PageRow::Field(first) = &pad[2] else {
            panic!()
        };
        assert_eq!(first.display(&cc), "36");
        assert_eq!(first.parse_number("C#2"), None, "a CC number takes no note");
        let panel = panel("X14800").unwrap();
        let sustain = field_at(
            "X14800",
            &panel,
            panel.controls.iter().find(|c| c.label == "Sustain"),
            &scene,
            201,
        )
        .unwrap();
        assert!(!store("X14800", &panel, &mut scene, &sustain, 1));
        let xy = page("X14800", "X-Y Pad", &scene);
        assert_eq!(xy[0], PageRow::Heading("Common"));
    }

    #[test]
    fn external_nano_key_studio_pages_match_the_previous_implementation() {
        assert!(korg_devices::editor_pages("X14800").is_some());
        let panel = panel("X14800").unwrap();
        for state in 0..=255u8 {
            let scene = vec![state; 560];
            for control in std::iter::once(None).chain(panel.controls.iter().map(Some)) {
                let expected = x14800_rows(control, &scene);
                let actual = fields("X14800", &panel, control, &scene).unwrap();
                assert_eq!(
                    actual.len(),
                    expected.len(),
                    "control {control:?}, state {state}"
                );
                for (a, b) in actual.iter().zip(&expected) {
                    match (a, b) {
                        (PageRow::Heading(a), PageRow::Heading(b)) => assert_eq!(a, b),
                        (PageRow::Field(a), PageRow::Field(b)) => {
                            assert_eq!(
                                (a.offset, a.label, a.enabled, a.note),
                                (b.offset, b.label, b.enabled, b.note)
                            );
                            assert_eq!(a.range(), b.range());
                            assert_eq!(a.kind.labels(), b.kind.labels());
                            assert_eq!(a.display(&scene), b.display(&scene));
                            for value in [-1, 0, 1, 127, 255] {
                                let mut actual_scene = scene.clone();
                                let mut expected_scene = scene.clone();
                                assert_eq!(
                                    store("X14800", &panel, &mut actual_scene, a, value),
                                    store("X14800", &panel, &mut expected_scene, b, value)
                                );
                                assert_eq!(actual_scene, expected_scene);
                            }
                        }
                        _ => panic!("row type changed"),
                    }
                }
            }
        }
    }

    #[test]
    fn external_nano_kontrol_studio_pages_match_the_previous_implementation() {
        assert!(korg_devices::editor_pages("X14802").is_some());
        let panel = panel("X14802").unwrap();
        for state in 0..=255u8 {
            let scene = vec![state; 560];
            for control in std::iter::once(None).chain(panel.controls.iter().map(Some)) {
                let selected = control
                    .or_else(|| panel.controls.iter().find(|c| c.group == "Scene"))
                    .unwrap();
                let expected = assign::studio_fields(selected, &scene);
                let actual = fields("X14802", &panel, control, &scene).unwrap();
                assert_eq!(
                    actual.len(),
                    expected.len(),
                    "control {control:?}, state {state}"
                );
                for (a, b) in actual.iter().zip(&expected) {
                    let PageRow::Field(a) = a else {
                        panic!("unexpected heading")
                    };
                    let expected_note = b.label == "Note/CC Number"
                        && b.offset.checked_sub(1).and_then(|at| scene.get(at)) == Some(&1);
                    assert_eq!(
                        (a.offset, a.label, a.note),
                        (b.offset, b.label, expected_note)
                    );
                    assert_eq!(a.kind.labels(), b.kind.labels());
                    if matches!(b.kind, FieldKind::Text { .. }) {
                        for text in ["", "TRAKTOR", "0123456789abcdef", "x\u{00e4}z"] {
                            let mut actual_scene = scene.clone();
                            let mut expected_scene = scene.clone();
                            assert_eq!(
                                store_text(&mut actual_scene, a, text),
                                assign::studio_store_text(&mut expected_scene, b, text)
                            );
                            assert_eq!(actual_scene, expected_scene);
                        }
                    } else {
                        // Malformed dynamic bounds outside the declared byte range
                        // are deliberately handled more safely than the baseline.
                        if state > 127
                            && (a.min_from_offset.is_some() || a.max_from_offset.is_some())
                        {
                            continue;
                        }
                        for value in [-1, 0, 1, 64, 127, 255] {
                            let mut actual_scene = scene.clone();
                            let mut expected_scene = scene.clone();
                            assert_eq!(
                                store("X14802", &panel, &mut actual_scene, a, value),
                                assign::studio_store(&panel, &mut expected_scene, b.offset, value)
                            );
                            assert_eq!(actual_scene, expected_scene);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn undecoded_choices_remain_unwritable_after_clamping() {
        let panel = panel("X14800").unwrap();
        let external = Box::leak(
            vec![String::from("Control Change"), String::from(UNDECODED)].into_boxed_slice(),
        );
        for kind in [
            FieldKind::Choice(&["Control Change", UNDECODED]),
            FieldKind::ExternalChoice(external),
        ] {
            let field = PageField::new(0, "Assign Type", kind);
            for value in [1, 2, 255] {
                let mut scene = [0];
                assert!(!store("X14800", &panel, &mut scene, &field, value));
                assert_eq!(scene, [0]);
            }
        }
    }

    #[test]
    fn missing_condition_byte_disables_the_field() {
        let panel = panel("X14800").unwrap();
        let control = panel.controls.iter().find(|c| c.label == "X-Y Pad");
        let rows = fields("X14800", &panel, control, &[]).unwrap();
        let field = rows
            .iter()
            .find_map(|row| match row {
                PageRow::Field(field) if field.offset == 170 => Some(field),
                _ => None,
            })
            .unwrap();
        assert!(!field.enabled);
    }

    #[test]
    fn schema_keys_read_as_words() {
        assert_eq!(schema_label("MidiCh"), "MIDI Channel");
        assert_eq!(schema_label("VelocityCurve"), "Velocity Curve");
        assert_eq!(schema_label("3 SwType"), "3 Switch Type");
        assert_eq!(schema_label("OnOff"), "On/Off");
        assert_eq!(schema_label("Value"), "Value");
        assert_eq!(schema_label("Global MIDI Channel"), "Global MIDI Channel");
        assert_eq!(schema_label("VerocityCurve"), "Velocity Curve");
    }

    #[test]
    fn other_devices_have_no_page() {
        let panel = panel("X10830").unwrap();
        assert!(fields("X10830", &panel, panel.controls.first(), &[0; 64]).is_none());
    }
}
