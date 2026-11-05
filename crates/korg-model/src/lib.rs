//! Platform-independent domain types for the KORG editor rewrite.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// File model key: four or five ASCII digits, for example `0820` or `14810`.
///
/// The key keeps its original width.  `"0820"` prints back as `0820`, never as
/// `820` or `00820`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FileModelKey(String);

impl FileModelKey {
    pub fn new(text: impl Into<String>) -> Result<Self, ParseModelIdError> {
        let text = text.into();
        if !(4..=5).contains(&text.len()) || !text.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(ParseModelIdError(text));
        }
        Ok(Self(text))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Five-digit keys use the compact `SnS`/`SnD` suffix.
    pub fn compact(&self) -> bool {
        self.0.len() == 5
    }
}

impl fmt::Display for FileModelKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for FileModelKey {
    type Err = ParseModelIdError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::new(text)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseModelIdError(pub String);

impl fmt::Display for ParseModelIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid file model key: {}", self.0)
    }
}

impl std::error::Error for ParseModelIdError {}

/// Implementation class. This is the device identity
/// in the UI and in the parameter tables.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ClassId(pub String);

impl ClassId {
    pub fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ClassId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// SysEx model id.  This is a third namespace: it is neither the file key nor
/// the class id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SysexModelId(pub u32);

impl SysexModelId {
    /// The measured generator form carries 14 bits in three 7-bit bytes.
    pub const fn fits_measured_form(self) -> bool {
        self.0 <= 0x3FFF
    }
}

/// SysEx descriptor for one device.  Unknown values stay `None`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SysexDescriptor {
    #[serde(default)]
    pub model_id: Option<SysexModelId>,
    /// `short_or_long` or `fixed3`.
    #[serde(default)]
    pub length_style: Option<String>,
    /// Command byte per operation.  `None` means not recovered.
    #[serde(default)]
    pub commands: std::collections::BTreeMap<String, Option<u8>>,
    /// Optional origin information.
    #[serde(default)]
    pub source: Option<String>,
}

impl SysexDescriptor {
    pub fn complete(&self) -> bool {
        self.model_id.is_some()
            && !self.commands.is_empty()
            && self.commands.values().all(Option::is_some)
    }
}

/// Chunk kinds used by the product data files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ChunkKind {
    SceneSet,
    SceneData,
    GlobalData,
}

impl ChunkKind {
    /// Four character suffix used by the four-digit model keys.
    pub const fn suffix(self) -> &'static str {
        match self {
            ChunkKind::SceneSet => "ScnS",
            ChunkKind::SceneData => "ScnD",
            ChunkKind::GlobalData => "GlbD",
        }
    }

    /// Three character suffix used by the five-digit model keys.
    pub const fn compact_suffix(self) -> &'static str {
        match self {
            ChunkKind::SceneSet => "SnS",
            ChunkKind::SceneData => "SnD",
            ChunkKind::GlobalData => "GlbD",
        }
    }

    pub fn from_suffix(suffix: &str) -> Option<Self> {
        match suffix {
            "ScnS" | "SnS" => Some(ChunkKind::SceneSet),
            "ScnD" | "SnD" => Some(ChunkKind::SceneData),
            "GlbD" => Some(ChunkKind::GlobalData),
            _ => None,
        }
    }

    pub const fn is_scene(self) -> bool {
        matches!(self, ChunkKind::SceneSet | ChunkKind::SceneData)
    }
}

/// One scene: an optional name and the raw parameter block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scene {
    pub name: Option<String>,
    pub data: Vec<u8>,
}

/// The name stored at the start of a scene, for the models that store one:
/// printable ASCII up to the first other byte, at most 16 bytes, trimmed.
/// `None` for other models and for fewer than three characters.
pub fn scene_name(model: &FileModelKey, data: &[u8]) -> Option<String> {
    let named = matches!(
        model.as_str(),
        "1480" | "1482" | "1850" | "2087" | "2088" | "14810" | "14823"
    );
    if !named || data.len() < 16 {
        return None;
    }
    let end = data[..16]
        .iter()
        .position(|byte| *byte == 0 || *byte < 0x20 || *byte >= 0x7F)
        .unwrap_or(16);
    if end < 3 {
        return None;
    }
    Some(String::from_utf8_lossy(&data[..end]).trim().to_owned())
}

/// A collection of scenes for one device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneSet {
    pub model: FileModelKey,
    pub element_size: u16,
    pub scenes: Vec<Scene>,
}

/// Global data for one device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlobalData {
    pub model: FileModelKey,
    pub data: Vec<u8>,
}

/// Device metadata from the generated catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub device: String,
    pub class: String,
    #[serde(default)]
    pub localized_name: Option<String>,
    #[serde(default)]
    pub file_model_key: Option<String>,
    #[serde(default)]
    pub extensions: Vec<String>,
    #[serde(default)]
    pub ostypes: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub scenes: Option<u8>,
    #[serde(default)]
    pub scene_payload_size: Option<u16>,
    #[serde(default)]
    pub global_payload_size: Option<u16>,
    #[serde(default)]
    pub files: Vec<String>,
}

impl DeviceInfo {
    pub fn class_id(&self) -> ClassId {
        ClassId::new(self.class.clone())
    }

    /// File key for this device, when the catalog has one.
    pub fn file_key(&self) -> Option<FileModelKey> {
        self.file_model_key
            .as_deref()
            .and_then(|key| key.parse().ok())
    }
}

/// One editable parameter inside a scene or global payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParameterDef {
    pub id: u16,
    /// Stable ASCII key from the descriptor, for example `VelocityCurve`.
    pub key: String,
    pub offset: u16,
    /// Byte width covered by `mask`.
    pub width: u8,
    pub shift: u8,
    pub mask: u32,
    pub min: i32,
    pub max: i32,
    /// Stored byte = logical value + bias.
    pub bias: i32,
    pub kind: ParameterKind,
    #[serde(default)]
    pub labels: Vec<String>,
    /// Optional editor presentation, independent of the stored encoding.
    #[serde(default)]
    pub display_label: Option<String>,
    #[serde(default)]
    pub display_choices: Vec<String>,
    #[serde(default)]
    pub display_choices_inferred: bool,
    #[serde(default)]
    pub default: Option<i32>,
    #[serde(default)]
    pub default_source: DefaultSource,
    /// Optional note. Never used for rendering.
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParameterKind {
    Number,
    Enum,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DefaultSource {
    Factory,
    #[default]
    Unknown,
}

/// Parameter schema for one model and one payload kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParameterSchema {
    #[serde(default = "default_schema_version")]
    pub schema_version: u8,
    /// Implementation class.
    pub model: String,
    /// Parameter table class.
    pub table: String,
    /// `scene`, `global`, or a named page.
    pub payload: String,
    pub payload_size: u16,
    /// Number of repeated elements (pads, controls).  Default 1.
    #[serde(default = "one")]
    pub elements: u16,
    /// Bytes per repeated element.  Default 1.
    #[serde(default = "one")]
    pub stride: u16,
    /// Optional source information.
    #[serde(default)]
    pub source: std::collections::BTreeMap<String, String>,
    pub parameters: Vec<ParameterDef>,
}

const fn one() -> u16 {
    1
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaError {
    OffsetOutOfRange {
        key: String,
    },
    InvalidWidth {
        key: String,
    },
    MaskTooWide {
        key: String,
    },
    InvalidRange {
        key: String,
    },
    DefaultOutOfRange {
        key: String,
    },
    LabelCount {
        key: String,
    },
    PayloadSize {
        expected: usize,
        actual: usize,
    },
    FactoryValueOutOfRange {
        key: String,
        element: usize,
        value: i32,
    },
}

impl std::fmt::Display for SchemaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SchemaError::OffsetOutOfRange { key } => {
                write!(f, "{key}: offset is outside the payload")
            }
            SchemaError::InvalidWidth { key } => write!(f, "{key}: width must be 1..=4"),
            SchemaError::MaskTooWide { key } => write!(f, "{key}: mask does not fit the width"),
            SchemaError::InvalidRange { key } => write!(f, "{key}: min is greater than max"),
            SchemaError::DefaultOutOfRange { key } => {
                write!(f, "{key}: default is outside min..max")
            }
            SchemaError::LabelCount { key } => {
                write!(f, "{key}: label count does not match the range")
            }
            SchemaError::PayloadSize { expected, actual } => {
                write!(f, "payload is {actual} bytes, expected {expected}")
            }
            SchemaError::FactoryValueOutOfRange {
                key,
                element,
                value,
            } => {
                write!(
                    f,
                    "{key}: factory value {value} is outside the range at element {element}"
                )
            }
        }
    }
}

impl std::error::Error for SchemaError {}

impl ParameterDef {
    /// Read the raw masked field from one payload.
    pub fn read_raw(&self, payload: &[u8]) -> Option<u32> {
        if self.width == 0 || self.width > 4 {
            return None;
        }
        let start = self.offset as usize;
        let end = start.checked_add(self.width as usize)?;
        if end > payload.len() {
            return None;
        }
        let mut raw = 0u32;
        for (index, byte) in payload[start..end].iter().enumerate() {
            raw |= (*byte as u32) << (8 * index);
        }
        Some((raw >> self.shift) & self.mask)
    }

    /// Decode the logical value from one payload.
    pub fn decode(&self, payload: &[u8]) -> Option<i32> {
        self.read_raw(payload).map(|raw| raw as i32 - self.bias)
    }

    /// Write a logical value into one payload.  Other bits stay unchanged.
    pub fn write(&self, payload: &mut [u8], value: i32) -> bool {
        let Some(encoded) = self.encode(value) else {
            return false;
        };
        let width = self.width as usize;
        let start = self.offset as usize;
        if self.width == 0 || self.width > 4 || start + width > payload.len() {
            return false;
        }
        let mut raw = 0u32;
        for (index, byte) in payload[start..start + width].iter().enumerate() {
            raw |= (*byte as u32) << (8 * index);
        }
        let shifted_mask = self.mask << self.shift;
        let updated = (raw & !shifted_mask) | (encoded & shifted_mask);
        for (index, byte) in payload[start..start + width].iter_mut().enumerate() {
            *byte = ((updated >> (8 * index)) & 0xFF) as u8;
        }
        true
    }

    /// Encode a logical value.  Returns `None` when the value is out of range
    /// or does not fit the field.
    pub fn encode(&self, value: i32) -> Option<u32> {
        if value < self.min || value > self.max {
            return None;
        }
        let logical = value.checked_add(self.bias)? as u32;
        if logical & !self.mask != 0 {
            return None;
        }
        Some(logical << self.shift)
    }

    pub fn validate(&self, payload_size: u16) -> Result<(), SchemaError> {
        if self.width == 0 || self.width > 4 {
            return Err(SchemaError::InvalidWidth {
                key: self.key.clone(),
            });
        }
        if self.offset as usize + self.width as usize > payload_size as usize {
            return Err(SchemaError::OffsetOutOfRange {
                key: self.key.clone(),
            });
        }
        let bits = self.width as u32 * 8;
        if u32::from(self.shift) >= bits
            || u64::from(self.mask) > ((1u64 << bits) - 1) >> self.shift
        {
            return Err(SchemaError::MaskTooWide {
                key: self.key.clone(),
            });
        }
        if self.min > self.max {
            return Err(SchemaError::InvalidRange {
                key: self.key.clone(),
            });
        }
        if let Some(default) = self.default {
            if default < self.min || default > self.max {
                return Err(SchemaError::DefaultOutOfRange {
                    key: self.key.clone(),
                });
            }
        }
        if self.kind == ParameterKind::Enum && !self.labels.is_empty() {
            let expected = (self.max - self.min + 1) as usize;
            if self.labels.len() != expected {
                return Err(SchemaError::LabelCount {
                    key: self.key.clone(),
                });
            }
        }
        Ok(())
    }
}

impl ParameterSchema {
    pub fn validate(&self) -> Result<(), SchemaError> {
        for parameter in &self.parameters {
            parameter.validate(self.payload_size)?;
            let last_base = usize::from(self.elements.saturating_sub(1)) * usize::from(self.stride);
            if last_base + usize::from(parameter.offset) + usize::from(parameter.width)
                > usize::from(self.payload_size)
            {
                return Err(SchemaError::OffsetOutOfRange {
                    key: parameter.key.clone(),
                });
            }
        }
        Ok(())
    }

    /// Validate every parameter against each factory element.
    pub fn validate_factory(&self, payload: &[u8]) -> Result<(), SchemaError> {
        if payload.len() != self.payload_size as usize {
            return Err(SchemaError::PayloadSize {
                expected: self.payload_size as usize,
                actual: payload.len(),
            });
        }
        for element in 0..self.elements as usize {
            let base = element * self.stride as usize;
            let Some(slice) = payload.get(base..) else {
                return Err(SchemaError::PayloadSize {
                    expected: base + 1,
                    actual: payload.len(),
                });
            };
            for parameter in &self.parameters {
                let Some(value) = parameter.decode(slice) else {
                    return Err(SchemaError::OffsetOutOfRange {
                        key: parameter.key.clone(),
                    });
                };
                if value < parameter.min || value > parameter.max {
                    return Err(SchemaError::FactoryValueOutOfRange {
                        key: parameter.key.clone(),
                        element,
                        value,
                    });
                }
            }
        }
        Ok(())
    }
}

const fn default_schema_version() -> u8 {
    1
}

/// Jog wheel type. Codes are device measurements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum JogType {
    IncDec1,
    IncDec2,
    Continuous,
    SignMagnitude,
    NoAssign,
    Encoder017F,
    Encoder3F41,
}

impl JogType {
    pub const fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0 => Some(Self::IncDec1),
            1 => Some(Self::IncDec2),
            2 => Some(Self::Continuous),
            3 => Some(Self::SignMagnitude),
            4 => Some(Self::NoAssign),
            5 => Some(Self::Encoder017F),
            6 => Some(Self::Encoder3F41),
            _ => None,
        }
    }

    pub const fn byte(self) -> u8 {
        match self {
            Self::IncDec1 => 0,
            Self::IncDec2 => 1,
            Self::Continuous => 2,
            Self::SignMagnitude => 3,
            Self::NoAssign => 4,
            Self::Encoder017F => 5,
            Self::Encoder3F41 => 6,
        }
    }
}

/// Jog wheel acceleration.  Stored as 0, 1, 2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum JogAcceleration {
    One,
    Two,
    Const,
}

impl JogAcceleration {
    pub const fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0 => Some(Self::One),
            1 => Some(Self::Two),
            2 => Some(Self::Const),
            _ => None,
        }
    }

    pub const fn byte(self) -> u8 {
        match self {
            Self::One => 0,
            Self::Two => 1,
            Self::Const => 2,
        }
    }
}

/// The nine-byte jog block in a supported scene payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct JogConfig {
    /// MIDI channel, zero-based; 16 is the global channel.
    pub channel: u8,
    pub kind: JogType,
    pub acceleration: JogAcceleration,
    pub sign_magnitude_cc: u8,
    pub cw_cc: u8,
    pub ccw_cc: u8,
    pub continuous_cc: u8,
    pub minimum: u8,
    pub maximum: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JogError {
    Short(usize),
    Type(u8),
    Acceleration(u8),
    Channel(u8),
    Range(u8, u8),
}

impl fmt::Display for JogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Short(len) => write!(formatter, "jog block is {len} bytes, needs 9"),
            Self::Type(byte) => write!(formatter, "unknown jog type {byte}"),
            Self::Acceleration(byte) => write!(formatter, "unknown jog acceleration {byte}"),
            Self::Channel(channel) => write!(formatter, "jog channel {channel} is outside 0..16"),
            Self::Range(min, max) => write!(formatter, "jog range {min}..{max} is reversed"),
        }
    }
}

impl std::error::Error for JogError {}

impl JogConfig {
    pub const LEN: usize = 9;
    /// The stored channel that means "use the global channel".
    pub const GLOBAL_CHANNEL: u8 = 16;

    pub fn decode(bytes: &[u8]) -> Result<Self, JogError> {
        if bytes.len() < Self::LEN {
            return Err(JogError::Short(bytes.len()));
        }
        let channel = bytes[0];
        if channel > Self::GLOBAL_CHANNEL {
            return Err(JogError::Channel(channel));
        }
        let kind = JogType::from_byte(bytes[1]).ok_or(JogError::Type(bytes[1]))?;
        let acceleration =
            JogAcceleration::from_byte(bytes[2]).ok_or(JogError::Acceleration(bytes[2]))?;
        if bytes[7] > bytes[8] {
            return Err(JogError::Range(bytes[7], bytes[8]));
        }
        Ok(Self {
            channel,
            kind,
            acceleration,
            sign_magnitude_cc: bytes[3],
            cw_cc: bytes[4],
            ccw_cc: bytes[5],
            continuous_cc: bytes[6],
            minimum: bytes[7],
            maximum: bytes[8],
        })
    }

    pub fn encode(&self) -> Result<[u8; Self::LEN], JogError> {
        if self.channel > Self::GLOBAL_CHANNEL {
            return Err(JogError::Channel(self.channel));
        }
        if self.minimum > self.maximum {
            return Err(JogError::Range(self.minimum, self.maximum));
        }
        Ok([
            self.channel,
            self.kind.byte(),
            self.acceleration.byte(),
            self.sign_magnitude_cc,
            self.cw_cc,
            self.ccw_cc,
            self.continuous_cc,
            self.minimum,
            self.maximum,
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_key_keeps_width() {
        let key: FileModelKey = "0820".parse().unwrap();
        assert_eq!(key.as_str(), "0820");
        assert_eq!(key.to_string(), "0820");
        assert!(!key.compact());

        let compact: FileModelKey = "14810".parse().unwrap();
        assert_eq!(compact.to_string(), "14810");
        assert!(compact.compact());
    }

    #[test]
    fn rejects_bad_file_key() {
        assert!("123".parse::<FileModelKey>().is_err());
        assert!("12a4".parse::<FileModelKey>().is_err());
        assert!("123456".parse::<FileModelKey>().is_err());
    }

    #[test]
    fn chunk_suffix_round_trips() {
        for kind in [
            ChunkKind::SceneSet,
            ChunkKind::SceneData,
            ChunkKind::GlobalData,
        ] {
            assert_eq!(ChunkKind::from_suffix(kind.suffix()), Some(kind));
            assert_eq!(ChunkKind::from_suffix(kind.compact_suffix()), Some(kind));
        }
    }

    #[test]
    fn writes_bitfield_and_preserves_neighbors() {
        let parameter = ParameterDef {
            id: 0,
            key: "AssignType".into(),
            offset: 0,
            width: 1,
            shift: 5,
            mask: 7,
            min: 0,
            max: 3,
            bias: 0,
            kind: ParameterKind::Enum,
            labels: Vec::new(),
            display_label: None,
            display_choices: Vec::new(),
            display_choices_inferred: false,
            default: None,
            default_source: DefaultSource::Unknown,
            notes: None,
        };
        let mut payload = [0x48u8];
        assert!(parameter.write(&mut payload, 1));
        assert_eq!(payload[0], 0x28);
        assert_eq!(parameter.decode(&payload), Some(1));
        assert!(!parameter.write(&mut payload, 4));
    }

    #[test]
    fn rejects_a_mask_or_shift_that_cannot_fit_the_storage_width() {
        let mut parameter = ParameterDef {
            id: 0,
            key: "field".into(),
            offset: 0,
            width: 1,
            shift: 5,
            mask: 0xff,
            min: 0,
            max: 3,
            bias: 0,
            kind: ParameterKind::Number,
            labels: Vec::new(),
            display_label: None,
            display_choices: Vec::new(),
            display_choices_inferred: false,
            default: None,
            default_source: DefaultSource::Unknown,
            notes: None,
        };
        assert!(matches!(
            parameter.validate(1),
            Err(SchemaError::MaskTooWide { .. })
        ));
        parameter.shift = 32;
        assert!(matches!(
            parameter.validate(1),
            Err(SchemaError::MaskTooWide { .. })
        ));
        parameter.width = 4;
        assert!(matches!(
            parameter.validate(4),
            Err(SchemaError::MaskTooWide { .. })
        ));
        parameter.shift = 31;
        parameter.mask = 1;
        assert!(parameter.validate(4).is_ok());
    }

    #[test]
    fn schema_rejects_a_repeated_field_extending_beyond_its_payload() {
        let parameter = ParameterDef {
            id: 0,
            key: "field".into(),
            offset: 3,
            width: 1,
            shift: 0,
            mask: 0xff,
            min: 0,
            max: 127,
            bias: 0,
            kind: ParameterKind::Number,
            labels: Vec::new(),
            display_label: None,
            display_choices: Vec::new(),
            display_choices_inferred: false,
            default: None,
            default_source: DefaultSource::Unknown,
            notes: None,
        };
        let mut schema = ParameterSchema {
            schema_version: 1,
            model: "test".into(),
            table: String::new(),
            payload: "scene".into(),
            payload_size: 9,
            elements: 2,
            stride: 6,
            source: Default::default(),
            parameters: vec![parameter],
        };
        assert!(matches!(
            schema.validate(),
            Err(SchemaError::OffsetOutOfRange { .. })
        ));
        schema.payload_size = 10;
        assert!(schema.validate().is_ok());
    }

    #[test]
    fn sysex_id_range() {
        assert!(SysexModelId(0x112).fits_measured_form());
        assert!(!SysexModelId(0x4000).fits_measured_form());
    }

    #[test]
    fn jog_blocks_round_trip() {
        let gonzo = [0x06, 0x05, 0x02, 0x53, 0x54, 0x56, 0x57, 0x00, 0x7F];
        let config = JogConfig::decode(&gonzo).unwrap();
        assert_eq!(config.kind, JogType::Encoder017F);
        assert_eq!(config.acceleration, JogAcceleration::Const);
        assert_eq!(config.channel, 6);
        assert_eq!(config.encode().unwrap(), gonzo);

        let klara = [0x04, 0x01, 0x01, 0x52, 0x53, 0x55, 0x56, 0x00, 0x7F];
        let config = JogConfig::decode(&klara).unwrap();
        assert_eq!(config.kind, JogType::IncDec2);
        assert_eq!(config.acceleration, JogAcceleration::Two);
        assert_eq!(config.encode().unwrap(), klara);
    }

    #[test]
    fn jog_rejects_out_of_range_values() {
        assert_eq!(JogConfig::decode(&[0; 8]), Err(JogError::Short(8)));
        let mut bad = [0x06, 0x05, 0x02, 0x53, 0x54, 0x56, 0x57, 0x00, 0x7F];
        bad[1] = 7;
        assert_eq!(JogConfig::decode(&bad), Err(JogError::Type(7)));
        bad[1] = 5;
        bad[2] = 3;
        assert_eq!(JogConfig::decode(&bad), Err(JogError::Acceleration(3)));
        bad[2] = 2;
        bad[0] = 16;
        assert_eq!(JogConfig::decode(&bad).map(|jog| jog.channel), Ok(16));
        bad[0] = 17;
        assert_eq!(JogConfig::decode(&bad), Err(JogError::Channel(17)));
        bad[0] = 6;
        bad[7] = 100;
        bad[8] = 50;
        assert_eq!(JogConfig::decode(&bad), Err(JogError::Range(100, 50)));
    }
}
