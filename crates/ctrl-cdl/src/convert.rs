//! Merged raw nodes into the IR. Checks the shape of every node: names,
//! argument counts, property names and value types, templates, paths and
//! ranges. Cross-references are the validator's job.

use serde::de::{DeserializeOwned, IntoDeserializer};

use crate::raw::{error, RawNode, RawValue};
use crate::template::is_ident;
use crate::*;

/// The CDL versions this crate reads.
pub const SUPPORTED_VERSIONS: &[&str] = &["0.1"];

/// Properties every node may carry (§4.1).
const COMMON: &[&str] = &["status", "evidence", "note"];

pub(crate) fn convert(
    files: Vec<String>,
    nodes: Vec<RawNode>,
    errors: Vec<Error>,
) -> Result<Descriptor, Errors> {
    let mut cx = Cx {
        files: &files,
        errors,
        patterns: Vec::new(),
    };
    let mut b = Builder::default();
    match nodes.first() {
        Some(first) if first.name == "cdl" => {}
        Some(first) => cx.err(first.pos, "a descriptor starts with `cdl \"0.1\"`"),
        None if cx.errors.is_empty() => cx.err(
            Pos {
                file: 0,
                line: 1,
                col: 1,
            },
            "empty descriptor",
        ),
        None => {}
    }
    // Patterns first, so that a message may use one defined later or in
    // another file.
    for n in nodes.iter().filter(|n| n.name == "pattern") {
        cx.shape(n, 2..=2, Some(&[]), false);
        let name = cx.name(n);
        if let Some(template) = cx.arg_with(n, 1, Template::parse) {
            let pattern = Pattern {
                meta: cx.meta(n),
                name,
                template,
            };
            cx.patterns.push(pattern);
        }
    }
    for n in &nodes {
        b.top(&mut cx, n);
    }
    let device = b.device.take();
    if device.is_none() && !nodes.is_empty() {
        cx.err(nodes[0].pos, "missing `device` node");
    }
    b.check_duplicates(&mut cx);
    match (device, cx.errors.is_empty()) {
        (Some(device), true) => Ok(Descriptor {
            version: b.version.unwrap_or_default(),
            files: files.clone(),
            device,
            identify: b.identify,
            transports: b.transports,
            types: b.types,
            blocks: b.blocks,
            params: b.params,
            panel: b.panel,
            midi_maps: b.midi_maps,
            frames: b.frames,
            messages: b.messages,
            procedures: b.procedures,
            firmware: b.firmware,
            lbmt: b.lbmt,
            drivers: b.drivers,
            test_vectors: b.test_vectors,
            consts: b.consts,
            labels: b.labels,
            patterns: cx.patterns.clone(),
        }),
        _ => Err(Errors(cx.errors)),
    }
}

struct Cx<'f> {
    files: &'f [String],
    errors: Vec<Error>,
    /// Named patterns, collected before any message is read (§6.2).
    patterns: Vec<Pattern>,
}

impl Cx<'_> {
    fn err(&mut self, pos: Pos, message: impl Into<String>) {
        self.errors.push(error(self.files, pos, message));
    }

    fn where_(&self, pos: Pos) -> String {
        let file = self.files.get(pos.file as usize).map_or("", |f| f);
        format!("{file}:{}:{}", pos.line, pos.col)
    }

    /// Checks argument count, property names and whether children are allowed.
    /// `props` of `None` allows any property (arguments of `exchange`).
    fn shape(
        &mut self,
        n: &RawNode,
        args: std::ops::RangeInclusive<usize>,
        props: Option<&[&str]>,
        children: bool,
    ) {
        if !args.contains(&n.args.len()) {
            let wanted = match (args.start(), args.end()) {
                (a, b) if a == b => format!("{a}"),
                (a, &usize::MAX) => format!("at least {a}"),
                (a, b) => format!("{a} to {b}"),
            };
            self.err(
                n.pos,
                format!(
                    "`{}` takes {wanted} argument(s), found {}",
                    n.name,
                    n.args.len()
                ),
            );
        }
        if let Some(allowed) = props {
            for (key, value) in &n.props {
                if !allowed.contains(&key.as_str()) && !COMMON.contains(&key.as_str()) {
                    let mut known: Vec<_> = allowed.iter().chain(COMMON).copied().collect();
                    known.sort_unstable();
                    self.err(
                        value.pos,
                        format!(
                            "unknown property `{key}` on `{}` (known: {})",
                            n.name,
                            known.join(", ")
                        ),
                    );
                }
            }
        }
        if !children {
            if let Some(child) = n.children.first() {
                self.err(child.pos, format!("`{}` takes no children", n.name));
            }
        }
    }

    fn unknown_child(&mut self, parent: &str, n: &RawNode, known: &[&str]) {
        self.err(
            n.pos,
            format!(
                "unknown node `{}` in `{parent}` (known: {})",
                n.name,
                known.join(", ")
            ),
        );
    }

    fn meta(&mut self, n: &RawNode) -> Meta {
        Meta {
            pos: n.pos,
            status: self.prop_with(n, "status", keyword),
            evidence: self.prop_str(n, "evidence"),
            note: self.prop_str(n, "note"),
        }
    }

    fn string(&mut self, what: &str, v: &RawValue) -> Option<String> {
        match &v.value {
            Value::String(s) => Some(s.clone()),
            other => {
                self.err(v.pos, format!("{what}: expected a string, found {other:?}"));
                None
            }
        }
    }

    fn int(&mut self, what: &str, v: &RawValue) -> Option<i64> {
        match &v.value {
            Value::Int(i) => Some(*i),
            other => {
                self.err(
                    v.pos,
                    format!("{what}: expected an integer, found {other:?}"),
                );
                None
            }
        }
    }

    fn arg_str(&mut self, n: &RawNode, i: usize) -> Option<String> {
        let v = n.args.get(i)?;
        self.string(&format!("`{}` argument {}", n.name, i + 1), v)
    }

    fn arg_int(&mut self, n: &RawNode, i: usize) -> Option<i64> {
        let v = n.args.get(i)?;
        self.int(&format!("`{}` argument {}", n.name, i + 1), v)
    }

    /// The node's name argument; an empty string after an arity error.
    fn name(&mut self, n: &RawNode) -> String {
        self.arg_str(n, 0).unwrap_or_default()
    }

    fn prop_str(&mut self, n: &RawNode, key: &str) -> Option<String> {
        let v = n.prop(key)?;
        self.string(&format!("`{key}`"), v)
    }

    fn prop_i64(&mut self, n: &RawNode, key: &str) -> Option<i64> {
        let v = n.prop(key)?;
        self.int(&format!("`{key}`"), v)
    }

    fn prop_u32(&mut self, n: &RawNode, key: &str) -> Option<u32> {
        let v = n.prop(key)?;
        let i = self.int(&format!("`{key}`"), v)?;
        match u32::try_from(i) {
            Ok(u) => Some(u),
            Err(_) => {
                self.err(v.pos, format!("`{key}`: {i} is out of range"));
                None
            }
        }
    }

    fn prop_bool(&mut self, n: &RawNode, key: &str) -> bool {
        let Some(v) = n.prop(key) else {
            return false;
        };
        match v.value {
            Value::Bool(b) => b,
            ref other => {
                self.err(
                    v.pos,
                    format!("`{key}`: expected #true or #false, found {other:?}"),
                );
                false
            }
        }
    }

    /// A string property parsed by `f`; errors point at the value.
    fn prop_with<T>(
        &mut self,
        n: &RawNode,
        key: &str,
        f: impl FnOnce(&str) -> Result<T, String>,
    ) -> Option<T> {
        let s = self.prop_str(n, key)?;
        self.parsed(n.prop(key)?.pos, key, f(&s))
    }

    fn arg_with<T>(
        &mut self,
        n: &RawNode,
        i: usize,
        f: impl FnOnce(&str) -> Result<T, String>,
    ) -> Option<T> {
        let s = self.arg_str(n, i)?;
        self.parsed(n.args[i].pos, &n.name, f(&s))
    }

    fn parsed<T>(&mut self, pos: Pos, what: &str, r: Result<T, String>) -> Option<T> {
        match r {
            Ok(t) => Some(t),
            Err(e) => {
                self.err(pos, format!("`{what}`: {e}"));
                None
            }
        }
    }

    fn required<T>(&mut self, n: &RawNode, key: &str, v: Option<T>) -> Option<T> {
        if v.is_none() && n.prop(key).is_none() {
            self.err(n.pos, format!("`{}` needs `{key}=`", n.name));
        }
        v
    }

    /// A send-side template: no `..` or `{name*}`.
    fn send_template(&mut self, pos: Pos, what: &str, s: &str) -> Option<Template> {
        let t = self.parsed(pos, what, Template::parse(s))?;
        if let Some(p) = t.pieces.iter().find(|p| p.is_pattern_only()) {
            self.err(
                pos,
                format!("`{what}`: `{p}` matches replies only; it cannot be sent"),
            );
        }
        Some(t)
    }

    /// Properties other than `reserved` and the common ones, as arguments.
    fn args(&mut self, n: &RawNode, reserved: &[&str]) -> Vec<Arg> {
        n.props
            .iter()
            .filter(|(k, _)| !reserved.contains(&k.as_str()) && !COMMON.contains(&k.as_str()))
            .map(|(k, v)| Arg {
                name: k.clone(),
                value: v.value.clone(),
            })
            .collect()
    }
}

/// A kebab-case keyword into one of the IR enums, with serde's
/// "expected one of" message.
fn keyword<T: DeserializeOwned>(s: &str) -> Result<T, String> {
    let de: serde::de::value::StrDeserializer<'_, serde::de::value::Error> = s.into_deserializer();
    T::deserialize(de).map_err(|e| e.to_string())
}

fn parse_int(s: &str) -> Result<i64, String> {
    let t = s.trim();
    let (negative, digits) = match t.strip_prefix('-') {
        Some(d) => (true, d),
        None => (false, t),
    };
    let value = match digits
        .strip_prefix("0x")
        .or_else(|| digits.strip_prefix("0X"))
    {
        Some(hex) => i64::from_str_radix(hex, 16),
        None => digits.parse(),
    }
    .map_err(|_| format!("`{s}` is not an integer"))?;
    Ok(if negative { -value } else { value })
}

/// `lo..=hi`, inclusive.
fn parse_range(s: &str) -> Result<IntRange, String> {
    let (lo, hi) = s
        .split_once("..=")
        .ok_or_else(|| format!("`{s}`: ranges are inclusive, `lo..=hi`"))?;
    let range = IntRange {
        lo: parse_int(lo)?,
        hi: parse_int(hi)?,
    };
    if range.lo > range.hi {
        return Err(format!("`{s}`: the range is empty"));
    }
    Ok(range)
}

fn parse_hex_bytes(s: &str) -> Result<Vec<u8>, String> {
    s.split_whitespace()
        .map(|t| {
            if t.len() == 2 {
                u8::from_str_radix(t, 16).map_err(|_| format!("`{t}` is not a hex byte"))
            } else {
                Err(format!("`{t}` is not a hex byte (two digits)"))
            }
        })
        .collect()
}

fn parse_type_ref(s: &str) -> Result<TypeRef, String> {
    match Storage::parse(s) {
        Ok(storage) => Ok(TypeRef::Storage(storage)),
        Err(_) if is_ident(s) => Ok(TypeRef::Named(s.into())),
        Err(e) => Err(e),
    }
}

fn parse_size(s: &str) -> Result<Size, String> {
    let bad = || format!("`{s}`: expected WIDTHxHEIGHT, e.g. 550x329");
    let (w, h) = s.split_once('x').ok_or_else(bad)?;
    Ok(Size {
        width: w.trim().parse().map_err(|_| bad())?,
        height: h.trim().parse().map_err(|_| bad())?,
    })
}

fn parse_box(s: &str) -> Result<[Coord; 4], String> {
    let parts: Vec<&str> = s.split(',').map(str::trim).collect();
    if parts.len() != 4 {
        return Err(format!("`{s}`: a box is x,y,w,h"));
    }
    let coord = |p: &str| -> Result<Coord, String> {
        if let Ok(n) = p.parse::<f64>() {
            return Ok(Coord::Num(n));
        }
        match p.strip_prefix('{').and_then(|p| p.strip_suffix('}')) {
            Some(expr) if !expr.trim().is_empty() => Ok(Coord::Expr(expr.trim().into())),
            _ => Err(format!("`{p}` is not a number or a `{{expr}}`")),
        }
    };
    Ok([
        coord(parts[0])?,
        coord(parts[1])?,
        coord(parts[2])?,
        coord(parts[3])?,
    ])
}

/// `16=Disable 17=Off`.
fn parse_special(s: &str) -> Result<Vec<Special>, String> {
    s.split_whitespace()
        .map(|item| {
            let (value, label) = item
                .split_once('=')
                .ok_or_else(|| format!("`{item}`: expected VALUE=Label"))?;
            Ok(Special {
                value: parse_int(value)?,
                label: label.into(),
            })
        })
        .collect()
}

fn words(s: &str) -> Result<Vec<String>, String> {
    Ok(s.split_whitespace().map(String::from).collect())
}

#[derive(Default)]
struct Builder {
    version: Option<String>,
    device: Option<Device>,
    identify: Vec<Matcher>,
    transports: Vec<Transport>,
    types: Vec<TypeDef>,
    blocks: Vec<Block>,
    params: Vec<Param>,
    panel: Option<Panel>,
    midi_maps: Vec<MidiMap>,
    frames: Vec<SysexFrame>,
    messages: Vec<Message>,
    procedures: Vec<Procedure>,
    firmware: Option<Firmware>,
    lbmt: Vec<Lbmt>,
    drivers: Vec<Driver>,
    test_vectors: Vec<TestVector>,
    consts: Vec<Const>,
    labels: Vec<Labels>,
}

const TOP: &[&str] = &[
    "cdl",
    "include",
    "device",
    "identify",
    "transport",
    "types",
    "block",
    "param",
    "panel",
    "midi-map",
    "sysex-frame",
    "message",
    "procedure",
    "firmware",
    "lbmt",
    "driver",
    "test-vector",
    "const",
    "labels",
    "pattern",
];

impl Builder {
    fn top(&mut self, cx: &mut Cx, n: &RawNode) {
        match n.name.as_str() {
            "cdl" => self.version(cx, n),
            "device" => {
                let device = device(cx, n);
                once(cx, n, &mut self.device, device);
            }
            "identify" => {
                cx.shape(n, 0..=0, Some(&[]), true);
                for c in &n.children {
                    if let Some(m) = matcher(cx, c) {
                        self.identify.push(m);
                    }
                }
            }
            "transport" => self.transports.push(transport(cx, n)),
            "types" => {
                cx.shape(n, 0..=0, Some(&[]), true);
                for c in &n.children {
                    if let Some(t) = type_def(cx, c) {
                        self.types.push(t);
                    }
                }
            }
            "block" => self.blocks.push(block(cx, n)),
            "param" => self.params.push(param(cx, n)),
            "panel" => {
                let panel = panel(cx, n);
                once(cx, n, &mut self.panel, panel);
            }
            "midi-map" => self.midi_maps.push(midi_map(cx, n)),
            "sysex-frame" => self.frames.push(frame(cx, n)),
            "message" => self.messages.push(message(cx, n)),
            "procedure" => self.procedures.push(procedure(cx, n)),
            "firmware" => {
                let firmware = firmware(cx, n);
                once(cx, n, &mut self.firmware, firmware);
            }
            "lbmt" => self.lbmt.push(lbmt(cx, n)),
            "driver" => self.drivers.push(driver(cx, n)),
            "test-vector" => self.test_vectors.push(test_vector(cx, n)),
            "labels" => self.labels.push(labels(cx, n)),
            // Read before the other nodes, in `convert`.
            "pattern" => {}
            "const" => {
                cx.shape(n, 2..=2, Some(&[]), false);
                let name = cx.name(n);
                if let Some(value) = cx.arg_int(n, 1) {
                    self.consts.push(Const {
                        meta: cx.meta(n),
                        name,
                        value,
                    });
                }
            }
            // Resolved by the loader; a bad one was reported there.
            "include" => {}
            _ => cx.unknown_child("the descriptor", n, TOP),
        }
    }

    fn version(&mut self, cx: &mut Cx, n: &RawNode) {
        cx.shape(n, 1..=1, Some(&[]), false);
        if self.version.is_some() {
            cx.err(n.pos, "`cdl` appears twice");
            return;
        }
        let version = match n.args.first().map(|v| &v.value) {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Float(f)) => f.to_string(),
            _ => return,
        };
        if !SUPPORTED_VERSIONS.contains(&version.as_str()) {
            cx.err(
                n.pos,
                format!(
                    "CDL {version} is not supported (supported: {})",
                    SUPPORTED_VERSIONS.join(", ")
                ),
            );
        }
        self.version = Some(version);
    }

    fn check_duplicates(&self, cx: &mut Cx) {
        fn dups<'a>(cx: &mut Cx, what: &str, items: impl Iterator<Item = (&'a str, Pos)>) {
            let mut seen: Vec<(&str, Pos)> = Vec::new();
            for (name, pos) in items {
                match seen.iter().find(|(n, _)| *n == name) {
                    Some((_, first)) => {
                        let first = cx.where_(*first);
                        cx.err(pos, format!("duplicate {what} `{name}` (first at {first})"));
                    }
                    None => seen.push((name, pos)),
                }
            }
        }
        dups(
            cx,
            "transport",
            self.transports
                .iter()
                .map(|t| (t.name.as_str(), t.meta.pos)),
        );
        dups(
            cx,
            "type",
            self.types.iter().map(|t| (t.name(), t.meta().pos)),
        );
        dups(
            cx,
            "block",
            self.blocks.iter().map(|b| (b.name.as_str(), b.meta.pos)),
        );
        dups(
            cx,
            "sysex-frame",
            self.frames.iter().map(|f| (f.name.as_str(), f.meta.pos)),
        );
        dups(
            cx,
            "message",
            self.messages.iter().map(|m| (m.name.as_str(), m.meta.pos)),
        );
        dups(
            cx,
            "procedure",
            self.procedures
                .iter()
                .map(|p| (p.name.as_str(), p.meta.pos)),
        );
        dups(
            cx,
            "lbmt",
            self.lbmt.iter().map(|l| (l.name.as_str(), l.meta.pos)),
        );
        dups(
            cx,
            "driver",
            self.drivers.iter().map(|d| (d.name.as_str(), d.meta.pos)),
        );
        dups(
            cx,
            "test-vector",
            self.test_vectors
                .iter()
                .map(|t| (t.name.as_str(), t.meta.pos)),
        );
        dups(
            cx,
            "const",
            self.consts.iter().map(|c| (c.name.as_str(), c.meta.pos)),
        );
        let patterns = cx.patterns.clone();
        dups(
            cx,
            "pattern",
            patterns.iter().map(|p| (p.name.as_str(), p.meta.pos)),
        );
        dups(
            cx,
            "labels",
            self.labels
                .iter()
                .map(|l| (l.language.as_str(), l.meta.pos)),
        );
    }
}

fn once<T>(cx: &mut Cx, n: &RawNode, slot: &mut Option<T>, value: T) {
    if slot.is_some() {
        cx.err(n.pos, format!("`{}` appears twice", n.name));
    } else {
        *slot = Some(value);
    }
}

fn device(cx: &mut Cx, n: &RawNode) -> Device {
    cx.shape(n, 1..=1, Some(&[]), true);
    let mut d = Device {
        meta: cx.meta(n),
        id: cx.name(n),
        vendor: None,
        model: None,
        class: None,
        file_key: None,
        aka: Vec::new(),
        manual: None,
        replaces: None,
        images: Vec::new(),
    };
    for c in &n.children {
        if c.name == "image" {
            cx.shape(c, 1..=1, Some(&["role"]), false);
            let path = cx.arg_with(c, 0, asset_path);
            d.images.push(Picture {
                meta: cx.meta(c),
                path: path.unwrap_or_default(),
                role: cx.prop_str(c, "role"),
            });
            continue;
        }
        cx.shape(c, 1..=1, Some(&[]), false);
        let value = cx.arg_str(c, 0);
        let slot = match c.name.as_str() {
            "vendor" => &mut d.vendor,
            "model" => &mut d.model,
            "class" => &mut d.class,
            "file-key" => &mut d.file_key,
            "manual" => &mut d.manual,
            "replaces" => &mut d.replaces,
            "aka" => {
                d.aka.extend(value);
                continue;
            }
            _ => {
                cx.unknown_child(
                    "device",
                    c,
                    &[
                        "vendor", "model", "class", "file-key", "aka", "manual", "replaces",
                        "image",
                    ],
                );
                continue;
            }
        };
        once(cx, c, slot, value.unwrap_or_default());
    }
    d
}

fn matcher(cx: &mut Cx, n: &RawNode) -> Option<Matcher> {
    let m = match n.name.as_str() {
        "usb" => {
            cx.shape(n, 0..=0, Some(&["vid", "pid", "bcd"]), false);
            Matcher::Usb {
                meta: cx.meta(n),
                vid: cx.prop_u32(n, "vid"),
                pid: cx.prop_u32(n, "pid"),
                bcd: cx.prop_u32(n, "bcd"),
            }
        }
        "midi-port" => {
            cx.shape(n, 0..=0, Some(&["match", "direction"]), false);
            let pattern = cx.prop_str(n, "match");
            Matcher::MidiPort {
                meta: cx.meta(n),
                pattern: cx.required(n, "match", pattern).unwrap_or_default(),
                direction: cx
                    .prop_with(n, "direction", keyword)
                    .unwrap_or(Direction::Both),
            }
        }
        "sysex-identity" => {
            cx.shape(n, 0..=0, Some(&["reply", "manufacturer", "model"]), false);
            Matcher::SysexIdentity {
                meta: cx.meta(n),
                reply: cx.prop_with(n, "reply", Template::parse),
                manufacturer: cx.prop_u32(n, "manufacturer"),
                model: cx.prop_u32(n, "model"),
            }
        }
        "hid" => {
            cx.shape(n, 0..=0, Some(&["usage-page", "report-size"]), false);
            Matcher::Hid {
                meta: cx.meta(n),
                usage_page: cx.prop_u32(n, "usage-page"),
                report_size: cx.prop_u32(n, "report-size"),
            }
        }
        "exchange" => {
            cx.shape(n, 1..=1, Some(&["ok-when"]), false);
            Matcher::Exchange {
                meta: cx.meta(n),
                message: cx.name(n),
                ok_when: cx.prop_str(n, "ok-when"),
            }
        }
        "key" => {
            cx.shape(n, 1..=1, Some(&[]), false);
            Matcher::Key {
                meta: cx.meta(n),
                source: cx.name(n),
            }
        }
        "listen" => {
            cx.shape(n, 1..=1, Some(&[]), false);
            let pattern = cx.arg_with(n, 0, Template::parse)?;
            Matcher::Listen {
                meta: cx.meta(n),
                pattern,
            }
        }
        _ => {
            cx.unknown_child(
                "identify",
                n,
                &[
                    "usb",
                    "midi-port",
                    "sysex-identity",
                    "hid",
                    "exchange",
                    "key",
                    "listen",
                ],
            );
            return None;
        }
    };
    Some(m)
}

fn transport(cx: &mut Cx, n: &RawNode) -> Transport {
    cx.shape(
        n,
        1..=1,
        Some(&[
            "kind",
            "port-match",
            "port-match-in",
            "port-match-out",
            "role",
            "preferred",
            "usage-page",
            "frame",
            "report-id",
            "seize",
            "interface",
            "ep-out",
            "ep-in",
            "scan",
        ]),
        true,
    );
    let kind = cx.prop_with(n, "kind", keyword);
    let both = cx.prop_str(n, "port-match");
    let input = cx.prop_str(n, "port-match-in");
    let output = cx.prop_str(n, "port-match-out");
    let ports = match (both, input, output) {
        (None, None, None) => None,
        (Some(both), None, None) => Some(PortMatch::Both(both)),
        (None, Some(input), Some(output)) => Some(PortMatch::Split { input, output }),
        _ => {
            cx.err(
                n.pos,
                "a transport has either `port-match` or both `port-match-in` and `port-match-out`",
            );
            None
        }
    };
    let mut captures = Vec::new();
    for c in &n.children {
        if c.name != "capture" {
            cx.unknown_child("transport", c, &["capture"]);
            continue;
        }
        cx.shape(c, 1..=1, Some(&["from"]), false);
        let from = cx.prop_str(c, "from");
        captures.push(Capture {
            meta: cx.meta(c),
            name: cx.name(c),
            from: cx.required(c, "from", from).unwrap_or_default(),
        });
    }
    Transport {
        meta: cx.meta(n),
        name: cx.name(n),
        kind: cx.required(n, "kind", kind).unwrap_or(TransportKind::Midi),
        ports,
        role: cx.prop_with(n, "role", keyword),
        preferred: cx.prop_bool(n, "preferred"),
        usage_page: cx.prop_u32(n, "usage-page"),
        frame: cx.prop_u32(n, "frame"),
        report_id: cx.prop_u32(n, "report-id"),
        seize: cx.prop_bool(n, "seize"),
        interface: cx.prop_u32(n, "interface"),
        ep_out: cx.prop_u32(n, "ep-out"),
        ep_in: cx.prop_u32(n, "ep-in"),
        scan: cx.prop_str(n, "scan"),
        captures,
    }
}

fn type_def(cx: &mut Cx, n: &RawNode) -> Option<TypeDef> {
    match n.name.as_str() {
        "enum" | "errors" => {
            cx.shape(n, 1..=1, Some(&[]), true);
            let mut values = Vec::new();
            for c in &n.children {
                if c.name != "value" {
                    cx.unknown_child(&n.name, c, &["value"]);
                    continue;
                }
                cx.shape(c, 2..=2, Some(&[]), false);
                if let (Some(value), Some(label)) = (cx.arg_int(c, 0), cx.arg_str(c, 1)) {
                    values.push(EnumValue {
                        meta: cx.meta(c),
                        value,
                        label,
                    });
                }
            }
            let t = EnumType {
                meta: cx.meta(n),
                name: cx.name(n),
                values,
            };
            Some(if n.name == "enum" {
                TypeDef::Enum(t)
            } else {
                TypeDef::Errors(t)
            })
        }
        "int" => {
            cx.shape(
                n,
                1..=1,
                Some(&["storage", "range", "display", "parse", "special"]),
                false,
            );
            let storage = cx.prop_with(n, "storage", Storage::parse);
            Some(TypeDef::Int(IntType {
                meta: cx.meta(n),
                name: cx.name(n),
                storage: cx.required(n, "storage", storage).unwrap_or(Storage::U7),
                range: cx.prop_with(n, "range", parse_range),
                display: cx.prop_str(n, "display"),
                parse: cx.prop_str(n, "parse"),
                special: cx
                    .prop_with(n, "special", parse_special)
                    .unwrap_or_default(),
            }))
        }
        _ => {
            cx.unknown_child("types", n, &["enum", "int", "errors"]);
            None
        }
    }
}

fn block(cx: &mut Cx, n: &RawNode) -> Block {
    cx.shape(n, 1..=1, Some(&["size", "count", "names"]), true);
    Block {
        meta: cx.meta(n),
        name: cx.name(n),
        size: cx.prop_u32(n, "size"),
        count: cx.prop_u32(n, "count"),
        names: cx.prop_with(n, "names", words).unwrap_or_default(),
        items: items(cx, &n.name, &n.children),
    }
}

fn items(cx: &mut Cx, parent: &str, nodes: &[RawNode]) -> Vec<Item> {
    let mut out = Vec::new();
    for n in nodes {
        match n.name.as_str() {
            "field" => {
                cx.shape(
                    n,
                    1..=1,
                    Some(&["offset", "type", "range", "visible-when", "fixed"]),
                    false,
                );
                let ty = cx.prop_with(n, "type", parse_type_ref);
                let ty = cx.required(n, "type", ty);
                out.push(Item::Field(Field {
                    meta: cx.meta(n),
                    name: cx.name(n),
                    offset: cx.prop_u32(n, "offset"),
                    ty: ty.unwrap_or(TypeRef::Storage(Storage::U7)),
                    range: cx.prop_with(n, "range", parse_range),
                    visible_when: cx.prop_str(n, "visible-when"),
                    fixed: cx.prop_i64(n, "fixed"),
                }));
            }
            "group" => {
                cx.shape(n, 1..=1, Some(&["offset"]), true);
                out.push(Item::Group(Group {
                    meta: cx.meta(n),
                    name: cx.name(n),
                    offset: cx.prop_u32(n, "offset"),
                    items: items(cx, "group", &n.children),
                }));
            }
            "array" => {
                cx.shape(
                    n,
                    1..=1,
                    Some(&["count", "offset", "stride", "type", "driver"]),
                    true,
                );
                let count = cx.prop_u32(n, "count");
                out.push(Item::Array(Array {
                    meta: cx.meta(n),
                    name: cx.name(n),
                    count: cx.required(n, "count", count).unwrap_or(0),
                    offset: cx.prop_u32(n, "offset"),
                    stride: cx.prop_u32(n, "stride"),
                    ty: cx.prop_with(n, "type", parse_type_ref),
                    driver: cx.prop_str(n, "driver"),
                    items: items(cx, "array", &n.children),
                }));
            }
            _ => cx.unknown_child(parent, n, &["field", "group", "array"]),
        }
    }
    out
}

fn param(cx: &mut Cx, n: &RawNode) -> Param {
    cx.shape(
        n,
        1..=1,
        Some(&["label", "group", "ui", "min-firmware"]),
        false,
    );
    Param {
        meta: cx.meta(n),
        path: cx.name(n),
        label: cx.prop_str(n, "label"),
        group: cx.prop_str(n, "group"),
        ui: cx.prop_str(n, "ui"),
        min_firmware: cx.prop_str(n, "min-firmware"),
    }
}

fn panel(cx: &mut Cx, n: &RawNode) -> Panel {
    cx.shape(n, 0..=0, Some(&["picture", "image", "source"]), true);
    Panel {
        meta: cx.meta(n),
        picture: cx.prop_with(n, "picture", parse_size),
        image: cx.prop_with(n, "image", asset_path),
        source: cx.prop_str(n, "source"),
        items: panel_items(cx, &n.children),
    }
}

/// A file next to the descriptor: relative, inside its directory.
fn asset_path(s: &str) -> Result<String, String> {
    let path = std::path::Path::new(s);
    let escapes = path
        .components()
        .any(|c| !matches!(c, std::path::Component::Normal(_)));
    if s.is_empty() || escapes {
        return Err(format!(
            "`{s}`: a relative path below the descriptor, without `..`"
        ));
    }
    Ok(s.to_string())
}

fn labels(cx: &mut Cx, n: &RawNode) -> Labels {
    cx.shape(n, 1..=1, Some(&["source"]), true);
    let mut lists = Vec::new();
    for c in &n.children {
        if c.name != "list" {
            cx.unknown_child("labels", c, &["list"]);
            continue;
        }
        cx.shape(c, 1..=1, Some(&[]), true);
        let mut items = Vec::new();
        for i in &c.children {
            if i.name != "item" {
                cx.unknown_child("list", i, &["item"]);
                continue;
            }
            cx.shape(i, 2..=2, Some(&[]), false);
            if let (Some(value), Some(label)) = (cx.arg_int(i, 0), cx.arg_str(i, 1)) {
                if items.iter().any(|s: &Special| s.value == value) {
                    cx.err(i.pos, format!("item {value} appears twice"));
                }
                items.push(Special { value, label });
            }
        }
        lists.push(LabelList {
            meta: cx.meta(c),
            name: cx.name(c),
            items,
        });
    }
    Labels {
        meta: cx.meta(n),
        language: cx.name(n),
        source: cx.prop_str(n, "source"),
        lists,
    }
}

fn panel_items(cx: &mut Cx, nodes: &[RawNode]) -> Vec<PanelItem> {
    let mut out = Vec::new();
    for n in nodes {
        match n.name.as_str() {
            "control" => {
                cx.shape(
                    n,
                    1..=1,
                    Some(&["kind", "label", "group", "box", "bind", "live"]),
                    false,
                );
                let kind = cx.prop_with(n, "kind", keyword);
                let Some(kind) = cx.required(n, "kind", kind) else {
                    continue;
                };
                out.push(PanelItem::Control(Control {
                    meta: cx.meta(n),
                    name: cx.name(n),
                    kind,
                    label: cx.prop_str(n, "label"),
                    group: cx.prop_str(n, "group"),
                    bbox: cx.prop_with(n, "box", parse_box),
                    bind: cx.prop_with(n, "bind", Path::parse),
                    live: cx.prop_str(n, "live"),
                }));
            }
            "repeat" => {
                cx.shape(n, 2..=2, Some(&[]), true);
                let var = cx.name(n);
                let Some(range) = cx.arg_with(n, 1, parse_range) else {
                    continue;
                };
                out.push(PanelItem::Repeat {
                    meta: cx.meta(n),
                    var,
                    range,
                    items: panel_items(cx, &n.children),
                });
            }
            _ => cx.unknown_child("panel", n, &["control", "repeat"]),
        }
    }
    out
}

fn midi_map(cx: &mut Cx, n: &RawNode) -> MidiMap {
    cx.shape(n, 0..=0, Some(&["transport"]), true);
    let mut endpoints = Vec::new();
    for c in &n.children {
        let direction = match c.name.as_str() {
            "source" => EndpointDirection::Source,
            "sink" => EndpointDirection::Sink,
            _ => {
                cx.unknown_child("midi-map", c, &["source", "sink"]);
                continue;
            }
        };
        cx.shape(
            c,
            1..=1,
            Some(&["count", "kind", "cc", "note", "channel", "message", "range"]),
            false,
        );
        let kind = cx.prop_with(c, "kind", keyword);
        let Some(kind) = cx.required(c, "kind", kind) else {
            continue;
        };
        endpoints.push(Endpoint {
            meta: cx.meta(c),
            direction,
            name: cx.name(c),
            count: cx.prop_u32(c, "count"),
            kind,
            cc: cx.prop_str(c, "cc"),
            note: cx.prop_str(c, "note"),
            channel: cx.prop_str(c, "channel"),
            message: cx.prop_str(c, "message"),
            range: cx.prop_with(c, "range", parse_range),
        });
    }
    MidiMap {
        meta: cx.meta(n),
        transport: cx.prop_str(n, "transport"),
        endpoints,
    }
}

fn frame(cx: &mut Cx, n: &RawNode) -> SysexFrame {
    cx.shape(n, 1..=1, Some(&[]), true);
    let mut f = SysexFrame {
        meta: cx.meta(n),
        name: cx.name(n),
        prefix: None,
        suffix: None,
    };
    for c in &n.children {
        let slot = match c.name.as_str() {
            "prefix" => &mut f.prefix,
            "suffix" => &mut f.suffix,
            _ => {
                cx.unknown_child("sysex-frame", c, &["prefix", "suffix"]);
                continue;
            }
        };
        cx.shape(c, 1..=1, Some(&[]), false);
        let Some(s) = cx.arg_str(c, 0) else {
            continue;
        };
        if let Some(t) = cx.send_template(c.args[0].pos, &c.name, &s) {
            once(cx, c, slot, t);
        }
    }
    f
}

fn message(cx: &mut Cx, n: &RawNode) -> Message {
    cx.shape(n, 1..=1, Some(&["transport", "frame", "kind"]), true);
    let mut m = Message {
        meta: cx.meta(n),
        name: cx.name(n),
        transport: cx.prop_str(n, "transport"),
        frame: cx.prop_str(n, "frame"),
        kind: cx.prop_with(n, "kind", keyword),
        send: None,
        replies: Vec::new(),
        ok_when: None,
    };
    for c in &n.children {
        match c.name.as_str() {
            "send" => {
                cx.shape(c, 0..=1, Some(&["pattern"]), false);
                if let Some((t, _)) = template_or_pattern(cx, c) {
                    if let Some(p) = t.pieces.iter().find(|p| p.is_pattern_only()) {
                        cx.err(c.pos, format!("`send`: `{p}` matches replies only"));
                    }
                    once(cx, c, &mut m.send, t);
                }
            }
            "reply" => {
                cx.shape(
                    c,
                    0..=1,
                    Some(&["timeout", "variant", "pattern", "ok-when"]),
                    false,
                );
                if let Some((pattern, pattern_name)) = template_or_pattern(cx, c) {
                    m.replies.push(Reply {
                        meta: cx.meta(c),
                        variant: cx.prop_str(c, "variant"),
                        pattern,
                        pattern_name,
                        timeout: cx.prop_u32(c, "timeout"),
                        ok_when: cx.prop_str(c, "ok-when"),
                    });
                }
            }
            "ok-when" => {
                cx.shape(c, 1..=1, Some(&[]), false);
                if let Some(expr) = cx.arg_str(c, 0) {
                    once(cx, c, &mut m.ok_when, expr);
                }
            }
            _ => cx.unknown_child("message", c, &["send", "reply", "ok-when"]),
        }
    }
    // Several replies are variants: each is named, once.
    if m.replies.len() > 1 {
        for (i, r) in m.replies.iter().enumerate() {
            match &r.variant {
                None => cx.err(
                    r.meta.pos,
                    "a message with several replies names each with `variant=`",
                ),
                Some(v) if m.replies[..i].iter().any(|o| o.variant.as_ref() == Some(v)) => {
                    cx.err(r.meta.pos, format!("variant `{v}` appears twice"))
                }
                Some(_) => {}
            }
        }
    }
    m
}

/// The template of a `send` or `reply`: its argument, or a named `pattern=`.
fn template_or_pattern(cx: &mut Cx, n: &RawNode) -> Option<(Template, Option<String>)> {
    let named = cx.prop_str(n, "pattern");
    match (n.args.is_empty(), named) {
        (false, None) => cx.arg_with(n, 0, Template::parse).map(|t| (t, None)),
        (true, Some(name)) => match cx.patterns.iter().find(|p| p.name == name) {
            Some(p) => Some((p.template.clone(), Some(name))),
            None => {
                cx.err(n.pos, format!("unknown pattern `{name}`"));
                None
            }
        },
        (false, Some(_)) => {
            cx.err(
                n.pos,
                format!("`{}` takes a template or `pattern=`, not both", n.name),
            );
            None
        }
        (true, None) => {
            cx.err(
                n.pos,
                format!("`{}` needs a template or `pattern=`", n.name),
            );
            None
        }
    }
}

fn procedure(cx: &mut Cx, n: &RawNode) -> Procedure {
    cx.shape(n, 1..=1, Some(&["kind", "transport"]), true);
    Procedure {
        meta: cx.meta(n),
        name: cx.name(n),
        kind: cx.prop_with(n, "kind", keyword),
        transport: cx.prop_str(n, "transport"),
        steps: steps(cx, "procedure", &n.children),
    }
}

const STEPS: &[&str] = &[
    "send",
    "exchange",
    "repeat",
    "read-into",
    "delay",
    "verify",
    "gate",
    "restore-point",
    "require",
    "call",
    "driver",
    "expect-identity",
    "ok-when",
];

fn steps(cx: &mut Cx, parent: &str, nodes: &[RawNode]) -> Vec<Step> {
    nodes.iter().filter_map(|n| step(cx, parent, n)).collect()
}

fn step(cx: &mut Cx, parent: &str, n: &RawNode) -> Option<Step> {
    let meta = cx.meta(n);
    let step = match n.name.as_str() {
        "send" | "exchange" => {
            // Every property but these is a message argument.
            const RESERVED: &[&str] = &["into", "from", "timeout"];
            cx.shape(n, 1..=1, None, false);
            let s = MessageStep {
                message: cx.name(n),
                args: cx.args(n, RESERVED),
                into: cx.prop_with(n, "into", Path::parse),
                from: cx.prop_with(n, "from", Path::parse),
                timeout: cx.prop_u32(n, "timeout"),
                meta,
            };
            if n.name == "send" {
                Step::Send(s)
            } else {
                Step::Exchange(s)
            }
        }
        "repeat" => {
            cx.shape(n, 2..=2, Some(&["size"]), true);
            let var = cx.name(n);
            let over = cx.arg_with(n, 1, |s| {
                if s.trim_start()
                    .starts_with(|c: char| c.is_ascii_digit() || c == '-')
                {
                    parse_range(s).map(RepeatOver::Range)
                } else if is_ident(s) {
                    Ok(RepeatOver::Source(s.into()))
                } else {
                    Err(format!(
                        "`{s}`: repeat over a range `lo..=hi` or a named sequence"
                    ))
                }
            })?;
            Step::Repeat {
                var,
                over,
                size: cx.prop_u32(n, "size"),
                steps: steps(cx, "repeat", &n.children),
                meta,
            }
        }
        "read-into" => {
            cx.shape(n, 1..=1, Some(&["from"]), false);
            let target = cx.arg_with(n, 0, Path::parse)?;
            let from = cx.prop_str(n, "from");
            Step::ReadInto {
                target,
                from: cx.required(n, "from", from)?,
                meta,
            }
        }
        "delay" => {
            cx.shape(n, 1..=1, Some(&[]), false);
            let ms = cx.arg_int(n, 0)?;
            let Ok(ms) = u32::try_from(ms) else {
                cx.err(n.args[0].pos, format!("`delay`: {ms} ms is out of range"));
                return None;
            };
            Step::Delay { ms, meta }
        }
        "verify" => {
            cx.shape(n, 1..=1, Some(&["compare"]), false);
            Step::Verify {
                procedure: cx.name(n),
                compare: cx
                    .prop_with(n, "compare", Path::parse_list)
                    .unwrap_or_default(),
                meta,
            }
        }
        "gate" => {
            cx.shape(n, 1..=1, Some(&["text"]), false);
            Step::Gate {
                gate: cx.arg_with(n, 0, keyword)?,
                text: cx.prop_str(n, "text"),
                meta,
            }
        }
        "restore-point" => {
            cx.shape(n, 1..=usize::MAX, Some(&[]), false);
            let paths = (0..n.args.len())
                .filter_map(|i| cx.arg_with(n, i, Path::parse))
                .collect();
            Step::RestorePoint { paths, meta }
        }
        "require" | "ok-when" => {
            cx.shape(n, 1..=1, Some(&[]), false);
            let expr = cx.arg_str(n, 0)?;
            if n.name == "require" {
                Step::Require { expr, meta }
            } else {
                Step::OkWhen { expr, meta }
            }
        }
        "call" => {
            cx.shape(n, 1..=1, Some(&[]), false);
            Step::Call {
                procedure: cx.name(n),
                meta,
            }
        }
        "driver" => {
            cx.shape(n, 1..=1, Some(&["step"]), false);
            let step = cx.prop_str(n, "step");
            Step::Driver {
                driver: cx.name(n),
                step: cx.required(n, "step", step)?,
                meta,
            }
        }
        "expect-identity" => {
            cx.shape(n, 0..=0, Some(&["product", "timeout"]), false);
            Step::ExpectIdentity {
                product: cx.prop_i64(n, "product"),
                timeout: cx.prop_u32(n, "timeout"),
                meta,
            }
        }
        _ => {
            cx.unknown_child(parent, n, STEPS);
            return None;
        }
    };
    Some(step)
}

fn firmware(cx: &mut Cx, n: &RawNode) -> Firmware {
    cx.shape(n, 0..=0, Some(&[]), true);
    let mut f = Firmware {
        meta: cx.meta(n),
        image: None,
        version_from: None,
        requires: Vec::new(),
        stages: Vec::new(),
        recovery: None,
    };
    for c in &n.children {
        match c.name.as_str() {
            "image" => {
                cx.shape(c, 0..=0, Some(&["format", "magic", "driver"]), false);
                let format = cx.prop_str(c, "format");
                let image = Image {
                    meta: cx.meta(c),
                    format: cx.required(c, "format", format).unwrap_or_default(),
                    magic: cx.prop_with(c, "magic", parse_hex_bytes),
                    driver: cx.prop_str(c, "driver"),
                };
                once(cx, c, &mut f.image, image);
            }
            "version-from" => {
                cx.shape(c, 1..=1, Some(&["field"]), false);
                let field = cx.prop_str(c, "field");
                let v = VersionFrom {
                    meta: cx.meta(c),
                    message: cx.name(c),
                    field: cx.required(c, "field", field).unwrap_or_default(),
                };
                once(cx, c, &mut f.version_from, v);
            }
            "require" => {
                cx.shape(c, 1..=1, Some(&[]), false);
                f.requires.extend(cx.arg_str(c, 0));
            }
            "stage" => {
                cx.shape(c, 1..=1, Some(&["kind"]), true);
                f.stages.push(Stage {
                    meta: cx.meta(c),
                    name: cx.name(c),
                    kind: cx.prop_with(c, "kind", keyword),
                    steps: steps(cx, "stage", &c.children),
                });
            }
            "recovery" => {
                cx.shape(c, 1..=1, Some(&[]), false);
                if let Some(text) = cx.arg_str(c, 0) {
                    once(cx, c, &mut f.recovery, text);
                }
            }
            _ => cx.unknown_child(
                "firmware",
                c,
                &["image", "version-from", "require", "stage", "recovery"],
            ),
        }
    }
    f
}

fn lbmt(cx: &mut Cx, n: &RawNode) -> Lbmt {
    cx.shape(n, 2..=2, Some(&["from", "to", "enabled"]), false);
    Lbmt {
        meta: cx.meta(n),
        name: cx.name(n),
        from: cx.prop_str(n, "from"),
        to: cx.prop_str(n, "to"),
        enabled: cx.prop_bool(n, "enabled"),
        source: cx.arg_str(n, 1).unwrap_or_default(),
    }
}

fn driver(cx: &mut Cx, n: &RawNode) -> Driver {
    cx.shape(n, 1..=1, Some(&["provides", "from"]), true);
    let mut owns = Vec::new();
    for c in &n.children {
        if c.name != "owns" {
            cx.unknown_child("driver", c, &["owns"]);
            continue;
        }
        cx.shape(c, 1..=1, Some(&["bytes", "frame"]), false);
        let bytes = match cx.prop_str(c, "bytes") {
            Some(s) => cx.send_template(c.prop("bytes").map_or(c.pos, |v| v.pos), "bytes", &s),
            None => None,
        };
        owns.push(Owned {
            meta: cx.meta(c),
            name: cx.name(c),
            bytes,
            frame: cx.prop_str(c, "frame"),
        });
    }
    Driver {
        meta: cx.meta(n),
        name: cx.name(n),
        provides: cx.prop_with(n, "provides", words).unwrap_or_default(),
        from: cx.prop_str(n, "from"),
        owns,
    }
}

fn test_vector(cx: &mut Cx, n: &RawNode) -> TestVector {
    cx.shape(
        n,
        1..=1,
        Some(&["message", "block", "file", "sha256", "reply", "variant"]),
        true,
    );
    let mut t = TestVector {
        meta: cx.meta(n),
        name: cx.name(n),
        message: cx.prop_str(n, "message"),
        block: cx.prop_str(n, "block"),
        file: cx.prop_str(n, "file"),
        sha256: cx.prop_str(n, "sha256"),
        reply: cx.prop_bool(n, "reply"),
        variant: cx.prop_str(n, "variant"),
        args: Vec::new(),
        data: Vec::new(),
        bytes: None,
    };
    for c in &n.children {
        match c.name.as_str() {
            "args" => {
                cx.shape(c, 0..=0, None, false);
                t.args.extend(cx.args(c, &[]));
            }
            "bytes" => {
                cx.shape(c, 1..=1, Some(&[]), false);
                if let Some(bytes) = cx.arg_with(c, 0, parse_hex_bytes) {
                    once(cx, c, &mut t.bytes, bytes);
                }
            }
            "data" => {
                cx.shape(c, 2..=2, Some(&[]), false);
                let name = cx.name(c);
                if let Some(bytes) = cx.arg_with(c, 1, parse_hex_bytes) {
                    t.data.push(ByteArg { name, bytes });
                }
            }
            _ => cx.unknown_child("test-vector", c, &["args", "data", "bytes"]),
        }
    }
    t
}
