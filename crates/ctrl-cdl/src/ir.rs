//! The typed IR of a descriptor. Section numbers refer to docs/CDL.md.
//!
//! LBMT expressions (`ok-when`, `visible-when`, `display`, `require`, index
//! expressions) stay source text here; `ctrl-lbmt` compiles them.

use serde::{Deserialize, Serialize};

use crate::{Error, Path, Storage, Template};

/// A source position: an index into [`Descriptor::files`], line and column
/// (1-based).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pos {
    pub file: u32,
    pub line: u32,
    pub col: u32,
}

/// Evidence level (§4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    Measured,
    Decoded,
    Inferred,
    Unknown,
}

/// What every node may carry: its position, `status`, `evidence` and `note`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Meta {
    pub pos: Pos,
    pub status: Option<Status>,
    pub evidence: Option<String>,
    pub note: Option<String>,
}

/// A KDL value as written, for arguments the engine evaluates later.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
}

/// An inclusive integer range, `0..=15`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntRange {
    pub lo: i64,
    pub hi: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Descriptor {
    /// `cdl "0.1"`.
    pub version: String,
    /// The descriptor first, then its includes, in load order.
    pub files: Vec<String>,
    pub device: Device,
    pub identify: Vec<Matcher>,
    pub transports: Vec<Transport>,
    pub types: Vec<TypeDef>,
    pub blocks: Vec<Block>,
    pub params: Vec<Param>,
    pub panel: Option<Panel>,
    pub midi_maps: Vec<MidiMap>,
    pub frames: Vec<SysexFrame>,
    pub messages: Vec<Message>,
    pub procedures: Vec<Procedure>,
    pub firmware: Option<Firmware>,
    pub lbmt: Vec<Lbmt>,
    pub drivers: Vec<Driver>,
    pub test_vectors: Vec<TestVector>,
    pub consts: Vec<Const>,
    pub labels: Vec<Labels>,
    pub patterns: Vec<Pattern>,
}

impl Descriptor {
    /// An error at `pos`, for checks that run on the IR (the validator).
    pub fn error_at(&self, pos: Pos, message: impl Into<String>) -> Error {
        Error {
            file: self
                .files
                .get(pos.file as usize)
                .cloned()
                .unwrap_or_default(),
            line: pos.line,
            col: pos.col,
            message: message.into(),
        }
    }

    pub fn transport(&self, name: &str) -> Option<&Transport> {
        self.transports.iter().find(|t| t.name == name)
    }

    pub fn block(&self, name: &str) -> Option<&Block> {
        self.blocks.iter().find(|b| b.name == name)
    }

    pub fn message(&self, name: &str) -> Option<&Message> {
        self.messages.iter().find(|m| m.name == name)
    }

    pub fn procedure(&self, name: &str) -> Option<&Procedure> {
        self.procedures.iter().find(|p| p.name == name)
    }

    pub fn driver(&self, name: &str) -> Option<&Driver> {
        self.drivers.iter().find(|d| d.name == name)
    }

    pub fn constant(&self, name: &str) -> Option<&Const> {
        self.consts.iter().find(|c| c.name == name)
    }
}

/// `device "vendor.model" { … }` (§3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Device {
    pub meta: Meta,
    pub id: String,
    pub vendor: Option<String>,
    pub model: Option<String>,
    pub class: Option<String>,
    pub file_key: Option<String>,
    pub aka: Vec<String>,
    pub manual: Option<String>,
    pub replaces: Option<String>,
    /// `image "assets/x.png" role="thumbnail"`, relative to the descriptor.
    pub images: Vec<Picture>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Picture {
    pub meta: Meta,
    pub path: String,
    pub role: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Direction {
    In,
    Out,
    Both,
}

/// One `identify` matcher (§3). Matchers combine with OR.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "matcher", rename_all = "kebab-case")]
pub enum Matcher {
    Usb {
        meta: Meta,
        vid: Option<u32>,
        pid: Option<u32>,
        bcd: Option<u32>,
    },
    MidiPort {
        meta: Meta,
        pattern: String,
        direction: Direction,
    },
    SysexIdentity {
        meta: Meta,
        reply: Option<Template>,
        manufacturer: Option<u32>,
        model: Option<u32>,
    },
    Hid {
        meta: Meta,
        usage_page: Option<u32>,
        report_size: Option<u32>,
    },
    /// Send a message and test its reply.
    Exchange {
        meta: Meta,
        message: String,
        ok_when: Option<String>,
    },
    /// The property that keys the device hub, e.g. `usb-serial`.
    Key { meta: Meta, source: String },
    /// Bytes the device sends by itself, such as Active Sensing (`FE`).
    /// A hint for a port, never an identification on its own.
    Listen { meta: Meta, pattern: Template },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TransportKind {
    Midi,
    Hid,
    UsbBulk,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Role {
    Config,
    Performance,
}

/// `port-match`, or `port-match-in` with `port-match-out` (§5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PortMatch {
    Both(String),
    Split { input: String, output: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Transport {
    pub meta: Meta,
    pub name: String,
    pub kind: TransportKind,
    pub ports: Option<PortMatch>,
    pub role: Option<Role>,
    pub preferred: bool,
    pub usage_page: Option<u32>,
    pub frame: Option<u32>,
    pub report_id: Option<u32>,
    pub seize: bool,
    pub interface: Option<u32>,
    pub ep_out: Option<u32>,
    pub ep_in: Option<u32>,
    pub captures: Vec<Capture>,
    /// A read message the engine sends on each MIDI output to find the
    /// device; the input that answers pairs with that output (§5.1).
    pub scan: Option<String>,
    /// The longest SysEx message the host stack carries in one write, in
    /// bytes (§8.2). Longer messages go out in pieces of at most this many
    /// bytes; the wire still carries one message.
    pub sysex_limit: Option<u32>,
    /// Milliseconds between the pieces of one split SysEx write.
    pub segment_delay: Option<u32>,
    /// The shortest time between two frames, in milliseconds (§9).
    pub pace: Option<u32>,
    /// The HID report descriptor the device reports (§9): the validator
    /// checks `frame` against its input and output reports.
    pub report_descriptor: Option<ReportDescriptor>,
    /// A message the engine repeats for live values (§9).
    pub poll: Option<Poll>,
}

/// `report-descriptor "05 0C …"`: the HID report descriptor, as read from
/// the device.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReportDescriptor {
    pub meta: Meta,
    pub bytes: Vec<u8>,
}

/// `poll "poll" interval=52`: send a read message every `interval` ms
/// while the device is shown.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Poll {
    pub meta: Meta,
    pub message: String,
    pub interval: u32,
}

/// `segment message="write-part" over="data" size=1024`: the device's own
/// multi-message form of a long message (§8.2). When the whole message
/// would be longer than `threshold` bytes (default: the transport's
/// `sysex-limit`), the engine sends `message` once per `size`-byte piece
/// of the byte array `over`, with the arguments `part`, `parts` and
/// `offset`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SysexSegment {
    pub meta: Meta,
    pub threshold: Option<u32>,
    pub message: String,
    pub over: String,
    pub size: u32,
}

/// `capture "device-id" from="identity"`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Capture {
    pub meta: Meta,
    pub name: String,
    pub from: String,
}

/// A field's type: a built-in storage type or a name from `types`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TypeRef {
    Storage(Storage),
    Named(String),
}

/// `types { … }` (§4.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum TypeDef {
    Enum(EnumType),
    Int(IntType),
    /// Status codes and their texts (§6).
    Errors(EnumType),
}

impl TypeDef {
    pub fn name(&self) -> &str {
        match self {
            Self::Enum(t) | Self::Errors(t) => &t.name,
            Self::Int(t) => &t.name,
        }
    }

    pub fn meta(&self) -> &Meta {
        match self {
            Self::Enum(t) | Self::Errors(t) => &t.meta,
            Self::Int(t) => &t.meta,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnumType {
    pub meta: Meta,
    pub name: String,
    pub values: Vec<EnumValue>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnumValue {
    pub meta: Meta,
    pub value: i64,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IntType {
    pub meta: Meta,
    pub name: String,
    pub storage: Storage,
    pub range: Option<IntRange>,
    /// LBMT expression or built-in, stored to shown.
    pub display: Option<String>,
    /// LBMT expression or built-in, shown to stored.
    pub parse: Option<String>,
    /// Values with a name instead of a number, `special="16=Disable"`.
    pub special: Vec<Special>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Special {
    pub value: i64,
    pub label: String,
}

/// `block "scene" size=560 count=5 { … }` (§4.3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Block {
    pub meta: Meta,
    pub name: String,
    pub size: Option<u32>,
    pub count: Option<u32>,
    /// Element names, `names="Breath Bite Nod Tilt"`.
    pub names: Vec<String>,
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "item", rename_all = "kebab-case")]
pub enum Item {
    Field(Field),
    Group(Group),
    Array(Array),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Field {
    pub meta: Meta,
    pub name: String,
    pub offset: Option<u32>,
    pub ty: TypeRef,
    pub range: Option<IntRange>,
    pub visible_when: Option<String>,
    /// A constant the editor writes back unchanged.
    pub fixed: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Group {
    pub meta: Meta,
    pub name: String,
    pub offset: Option<u32>,
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Array {
    pub meta: Meta,
    pub name: String,
    pub count: u32,
    pub offset: Option<u32>,
    pub stride: Option<u32>,
    pub ty: Option<TypeRef>,
    /// The driver that computes the elements (§14).
    pub driver: Option<String>,
    pub items: Vec<Item>,
}

/// `param "drawbar.*.cc" label="CC"` (§4.5).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Param {
    pub meta: Meta,
    pub path: String,
    pub label: Option<String>,
    pub group: Option<String>,
    pub ui: Option<String>,
    pub min_firmware: Option<String>,
}

/// `panel picture="550x329" { … }` (§13).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Panel {
    pub meta: Meta,
    pub picture: Option<Size>,
    /// The panel picture's file, relative to the descriptor.
    pub image: Option<String>,
    pub source: Option<String>,
    pub items: Vec<PanelItem>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "item", rename_all = "kebab-case")]
pub enum PanelItem {
    Control(Control),
    Repeat {
        meta: Meta,
        var: String,
        range: IntRange,
        items: Vec<PanelItem>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ControlKind {
    Knob,
    Fader,
    Button,
    Pad,
    Key,
    Touchpad,
    Jog,
    Meter,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Control {
    pub meta: Meta,
    /// May contain `{n}` inside a `repeat`.
    pub name: String,
    pub kind: ControlKind,
    pub label: Option<String>,
    pub group: Option<String>,
    /// x, y, w, h in picture pixels.
    pub bbox: Option<[Coord; 4]>,
    pub bind: Option<Path>,
    /// A reply capture that feeds a meter, `poll.breath`.
    pub live: Option<String>,
}

/// A box coordinate: a number, or `{expr}` inside a `repeat`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Coord {
    Num(f64),
    Expr(String),
}

/// `midi-map transport="main" { source …; sink … }` (§7).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MidiMap {
    pub meta: Meta,
    pub transport: Option<String>,
    pub endpoints: Vec<Endpoint>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EndpointDirection {
    /// The device sends it.
    Source,
    /// The device takes it.
    Sink,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MidiKind {
    Cc,
    Cc14,
    Nrpn,
    Rpn,
    Note,
    Pc,
    Pb,
    At,
    PolyAt,
    Clock,
    /// The kind is itself a field, named by `message=` (TEC sensors).
    FromField,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Endpoint {
    pub meta: Meta,
    pub direction: EndpointDirection,
    pub name: String,
    pub count: Option<u32>,
    pub kind: MidiKind,
    /// Value references such as `{drawbar.$i.cc}`, kept as text.
    pub cc: Option<String>,
    pub note: Option<String>,
    pub channel: Option<String>,
    pub message: Option<String>,
    pub range: Option<IntRange>,
}

/// `sysex-frame "korg-native" { prefix …; suffix … }` (§8).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SysexFrame {
    pub meta: Meta,
    pub name: String,
    pub prefix: Option<Template>,
    pub suffix: Option<Template>,
}

/// Read, write (RAM or settings) or commit (flash), §11.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Access {
    Read,
    Write,
    Commit,
}

/// `message "set-drawbar" { send …; reply …; ok-when … }` (§6).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub meta: Meta,
    pub name: String,
    pub transport: Option<String>,
    pub frame: Option<String>,
    pub kind: Option<Access>,
    pub send: Option<Template>,
    /// Reply variants, tried in order; the first that matches wins (§6.1).
    /// None: the message is send-only.
    pub replies: Vec<Reply>,
    pub ok_when: Option<String>,
    /// Declared arguments with their allowed values (§6.5).
    pub args: Vec<ArgDecl>,
    /// The device's multi-message form for long sends (§8.2).
    pub segment: Option<SysexSegment>,
}

/// `arg "s" index="sensor"` or `arg "x" range="0..=3"`: the values a
/// message argument may take. The codec refuses others when it encodes and
/// does not match them when it decodes, so `{0x10 + s}` is not any byte.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArgDecl {
    pub meta: Meta,
    pub name: String,
    pub range: Option<IntRange>,
    /// An element index of this block: `0..count`.
    pub index: Option<String>,
}

impl Descriptor {
    /// The inclusive range of a message argument, if it declares one.
    pub fn arg_range(&self, m: &Message, name: &str) -> Option<IntRange> {
        let a = m.args.iter().find(|a| a.name == name)?;
        match (&a.range, &a.index) {
            (Some(r), _) => Some(*r),
            (None, Some(block)) => {
                let count = self.block(block)?.count?;
                Some(IntRange {
                    lo: 0,
                    hi: i64::from(count) - 1,
                })
            }
            (None, None) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reply {
    pub meta: Meta,
    /// The variant's name; required when a message has several replies.
    pub variant: Option<String>,
    pub pattern: Template,
    /// The named `pattern` this reply uses, if any (§6.2).
    pub pattern_name: Option<String>,
    /// Milliseconds.
    pub timeout: Option<u32>,
    /// Overrides the message's `ok-when` for this variant.
    pub ok_when: Option<String>,
}

impl Message {
    /// The first reply variant.
    pub fn reply(&self) -> Option<&Reply> {
        self.replies.first()
    }
}

/// `pattern "korg-ack" "01 00 00 {status}"`: a template that messages reuse
/// with `send pattern=` or `reply pattern=` (§6.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pattern {
    pub meta: Meta,
    pub name: String,
    pub template: Template,
}

/// `procedure "write-all" kind="write" { steps }` (§11).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Procedure {
    pub meta: Meta,
    pub name: String,
    pub kind: Option<Access>,
    pub transport: Option<String>,
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Gate {
    Confirm,
    Arm,
}

/// What a `repeat` counts over.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RepeatOver {
    Range(IntRange),
    /// A sequence the engine provides, e.g. `image-chunks`.
    Source(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum Step {
    Send(MessageStep),
    Exchange(MessageStep),
    Repeat {
        meta: Meta,
        var: String,
        over: RepeatOver,
        size: Option<u32>,
        steps: Vec<Step>,
    },
    ReadInto {
        meta: Meta,
        target: Path,
        from: String,
    },
    Delay {
        meta: Meta,
        ms: u32,
    },
    Verify {
        meta: Meta,
        procedure: String,
        /// Empty: compare everything the procedure wrote.
        compare: Vec<Path>,
    },
    Gate {
        meta: Meta,
        gate: Gate,
        text: Option<String>,
    },
    RestorePoint {
        meta: Meta,
        paths: Vec<Path>,
    },
    Require {
        meta: Meta,
        expr: String,
    },
    Call {
        meta: Meta,
        procedure: String,
    },
    Driver {
        meta: Meta,
        driver: String,
        step: String,
    },
    ExpectIdentity {
        meta: Meta,
        product: Option<i64>,
        timeout: Option<u32>,
    },
    OkWhen {
        meta: Meta,
        expr: String,
    },
}

/// `send` or `exchange`: a message with arguments.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MessageStep {
    pub meta: Meta,
    pub message: String,
    pub args: Vec<Arg>,
    /// Store reply captures here.
    pub into: Option<Path>,
    /// Take template fields from here.
    pub from: Option<Path>,
    pub timeout: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Arg {
    pub name: String,
    pub value: Value,
}

/// `firmware { image …; stage … }` (§12).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Firmware {
    pub meta: Meta,
    pub image: Option<Image>,
    pub version_from: Option<VersionFrom>,
    pub requires: Vec<String>,
    pub stages: Vec<Stage>,
    pub recovery: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Image {
    pub meta: Meta,
    pub format: String,
    pub magic: Option<Vec<u8>>,
    pub driver: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VersionFrom {
    pub meta: Meta,
    pub message: String,
    pub field: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stage {
    pub meta: Meta,
    pub name: String,
    pub kind: Option<Access>,
    pub steps: Vec<Step>,
}

/// `lbmt "name" from=… to=… #"…"#` (§10.3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Lbmt {
    pub meta: Meta,
    pub name: String,
    pub from: Option<String>,
    pub to: Option<String>,
    pub enabled: bool,
    pub source: String,
}

/// `driver "name" provides="…" { owns … }` (§14).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Driver {
    pub meta: Meta,
    pub name: String,
    pub provides: Vec<String>,
    pub from: Option<String>,
    pub owns: Vec<Owned>,
}

/// Bytes a driver sends that are not yet CDL messages.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Owned {
    pub meta: Meta,
    pub name: String,
    pub bytes: Option<Template>,
    pub frame: Option<String>,
}

/// `test-vector "name" message=… { args …; bytes … }` (§15).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TestVector {
    pub meta: Meta,
    pub name: String,
    pub message: Option<String>,
    pub block: Option<String>,
    /// External, opt-in input under the local capture archive.
    pub file: Option<String>,
    pub sha256: Option<String>,
    pub args: Vec<Arg>,
    /// Decode `bytes` with the reply pattern instead of encoding the send.
    pub reply: bool,
    /// With `reply`: the variant that must match.
    pub variant: Option<String>,
    /// Byte-array arguments: `data "payload" "AA 55 80 01"`.
    pub data: Vec<ByteArg>,
    pub bytes: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ByteArg {
    pub name: String,
    pub bytes: Vec<u8>,
}

/// `labels "en" { list "controller-names" { item 0 "Common" } }`: UI texts
/// by language, as numbered lists (§13.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Labels {
    pub meta: Meta,
    pub language: String,
    pub source: Option<String>,
    pub lists: Vec<LabelList>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LabelList {
    pub meta: Meta,
    pub name: String,
    pub items: Vec<Special>,
}

/// `const "model" 0x137`: a named number the device's templates use (§8.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Const {
    pub meta: Meta,
    pub name: String,
    pub value: i64,
}
