//! The template codec (CDL §6): message templates to bytes and back.
//!
//! [`Codec::encode`] builds a message from its frame and `send` template;
//! [`Codec::decode_reply`] matches bytes against the frame and `reply`
//! pattern and returns what the pattern captures. [`Codec::run_vectors`]
//! checks a descriptor's test vectors both ways (§15).
//!
//! Names in a template resolve to the caller's bindings first, then to the
//! descriptor's `const` nodes. On a MIDI transport a placeholder without an
//! encoding is `u7`, on USB `u8`; every SysEx data byte must be 7-bit.

mod pack;

use std::collections::BTreeMap;
use std::fmt;

use ctrl_cdl::{
    ChecksumAlgorithm, Descriptor, Encoding, Expr, Message, Piece, Role, Storage, Template,
    TestVector, TransportKind, Value, SELECTED_TRANSPORT,
};

pub use pack::{pack7, unpack7};

/// A bound value: a number, or bytes for arrays, blocks and text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Val {
    Int(i64),
    Bytes(Vec<u8>),
}

pub type Bindings = BTreeMap<String, Val>;

/// A matched reply: which variant, and its values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    pub variant: Option<String>,
    pub values: Bindings,
}

/// The bytes a reply puts into a step's `into=` target (CDL §6.3): its one
/// byte-array capture, which must be exactly `size` bytes. A short or long
/// frame is a protocol error, not a partial block.
pub fn fill(values: &Bindings, size: usize) -> Result<Vec<u8>, CodecError> {
    let mut arrays = values.iter().filter_map(|(k, v)| match v {
        Val::Bytes(b) => Some((k, b)),
        Val::Int(_) => None,
    });
    let (Some((_, bytes)), None) = (arrays.next(), arrays.next()) else {
        return Err(CodecError::Unsupported(
            "`into=` a block needs exactly one byte-array capture in the reply".into(),
        ));
    };
    if bytes.len() != size {
        return Err(CodecError::Length {
            expected: size as i64,
            found: bytes.len() as i64,
        });
    }
    Ok(bytes.clone())
}

/// Where a message travels; it sets the width of plain placeholders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Medium {
    /// 7-bit data bytes.
    Midi,
    /// 8-bit bytes (HID, USB bulk).
    Usb,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodecError {
    UnknownMessage(String),
    /// The message has no `send` (or no `reply`) template.
    NoTemplate(String, &'static str),
    UnknownName(String),
    /// A value does not fit its encoding.
    Range(String),
    /// The bytes do not match the pattern at this index.
    Mismatch {
        index: usize,
        expected: String,
        found: String,
    },
    Truncated(usize),
    Trailing(usize),
    Checksum {
        index: usize,
        expected: u8,
        found: u8,
    },
    Length {
        expected: i64,
        found: i64,
    },
    Expr(String),
    Unsupported(String),
    /// No reply variant matched; one reason per variant.
    NoVariant(Vec<String>),
}

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownMessage(m) => write!(f, "unknown message `{m}`"),
            Self::NoTemplate(m, what) => write!(f, "message `{m}` has no `{what}`"),
            Self::UnknownName(n) => write!(f, "no value for `{n}`"),
            Self::Range(e) => f.write_str(e),
            Self::Mismatch {
                index,
                expected,
                found,
            } => write!(f, "byte {index}: expected {expected}, found {found}"),
            Self::Truncated(i) => write!(f, "the message ends at byte {i}, too early"),
            Self::Trailing(i) => write!(f, "unexpected bytes from byte {i} on"),
            Self::Checksum {
                index,
                expected,
                found,
            } => write!(
                f,
                "checksum at byte {index}: expected {expected:02X}, found {found:02X}"
            ),
            Self::Length { expected, found } => {
                write!(f, "length field says {found}, the message has {expected}")
            }
            Self::Expr(e) => f.write_str(e),
            Self::Unsupported(e) => f.write_str(e),
            Self::NoVariant(misses) => {
                write!(f, "no reply variant matched ({})", misses.join("; "))
            }
        }
    }
}

impl std::error::Error for CodecError {}

/// A device pack encoding (`{x:vendor-name}`), a pure function with test
/// vectors (CDL §8).
pub trait VendorEncoding: Send + Sync {
    fn encode(&self, value: &Val) -> Result<Vec<u8>, String>;
    /// Decodes from the start of `bytes`; returns the value and the bytes
    /// used.
    fn decode(&self, bytes: &[u8]) -> Result<(Val, usize), String>;
}

/// Where a run of encoded bytes came from: `explain` prints these.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub len: usize,
    /// `prefix`, `body`, `suffix` or `padding`.
    pub region: &'static str,
    /// The template piece, e.g. `{model:midi-id}`, and its value.
    pub label: String,
}

/// Which way a traced frame went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// To the device: matched against `send` templates.
    Out,
    /// From the device: matched against `reply` patterns.
    In,
}

/// A frame that matched a message (see [`Codec::identify`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    pub message: String,
    pub direction: Direction,
    pub variant: Option<String>,
    pub values: Bindings,
    /// A reply's `ok-when`; None for sends or when it cannot be evaluated.
    pub ok: Option<bool>,
    /// Literal bytes in the template that matched. A frame that several
    /// templates accept most likely belongs to the one with the most.
    pub literals: usize,
}

/// One test vector's result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VectorResult {
    pub name: String,
    pub outcome: Outcome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Pass,
    Fail(String),
    /// External or block vectors, which this runner does not read.
    Skipped(String),
}

pub struct Codec<'d> {
    d: &'d Descriptor,
    vendor: BTreeMap<String, Box<dyn VendorEncoding>>,
}

/// Which part of the message a piece belongs to. Checksum and length
/// ranges count within the body (CDL §6.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Region {
    Prefix,
    Body,
    Suffix,
}

enum Chunk {
    Bytes(Vec<u8>),
    Len(Encoding),
    Sum(ctrl_cdl::Checksum),
}

impl<'d> Codec<'d> {
    pub fn new(d: &'d Descriptor) -> Self {
        Self {
            d,
            vendor: BTreeMap::new(),
        }
    }

    pub fn register(&mut self, name: &str, encoding: Box<dyn VendorEncoding>) {
        self.vendor.insert(name.to_string(), encoding);
    }

    fn message(&self, name: &str) -> Result<&'d Message, CodecError> {
        self.d
            .message(name)
            .ok_or_else(|| CodecError::UnknownMessage(name.into()))
    }

    /// MIDI unless the message goes out on HID or USB bulk.
    pub fn medium(&self, m: &Message) -> Medium {
        if m.frame.is_some() {
            return Medium::Midi;
        }
        let kind = match m.transport.as_deref() {
            Some(SELECTED_TRANSPORT) => self
                .d
                .transports
                .iter()
                .find(|t| t.role == Some(Role::Config))
                .map(|t| t.kind),
            Some(name) => self.d.transport(name).map(|t| t.kind),
            None => None,
        };
        match kind {
            Some(TransportKind::Hid | TransportKind::UsbBulk) => Medium::Usb,
            _ => Medium::Midi,
        }
    }

    /// The fixed report size of a HID or USB message's transport (CDL §9).
    fn frame_len(&self, m: &Message) -> Option<usize> {
        let t = self.d.transport(m.transport.as_deref()?)?;
        if t.kind == TransportKind::Midi {
            return None;
        }
        t.frame.map(|f| f as usize)
    }

    /// Frame prefix, the template, frame suffix, each piece with its region.
    fn layout(&self, m: &'d Message, body: &'d Template) -> Vec<(&'d Piece, Region)> {
        let frame = m
            .frame
            .as_ref()
            .and_then(|f| self.d.frames.iter().find(|x| &x.name == f));
        let mut out = Vec::new();
        if let Some(prefix) = frame.and_then(|f| f.prefix.as_ref()) {
            out.extend(prefix.pieces.iter().map(|p| (p, Region::Prefix)));
        }
        out.extend(body.pieces.iter().map(|p| (p, Region::Body)));
        if let Some(suffix) = frame.and_then(|f| f.suffix.as_ref()) {
            out.extend(suffix.pieces.iter().map(|p| (p, Region::Suffix)));
        }
        out
    }

    fn lookup<'a>(&'a self, bound: &'a Bindings) -> impl Fn(&str) -> Option<i64> + 'a {
        move |name| match bound.get(name) {
            Some(Val::Int(i)) => Some(*i),
            Some(Val::Bytes(_)) => None,
            None => self.d.constant(name).map(|c| c.value),
        }
    }

    fn value(&self, name: &str, bound: &Bindings) -> Result<Val, CodecError> {
        bound
            .get(name)
            .cloned()
            .or_else(|| self.d.constant(name).map(|c| Val::Int(c.value)))
            .ok_or_else(|| CodecError::UnknownName(name.into()))
    }

    // ---- encode -----------------------------------------------------------

    /// The bytes of message `name` with `args`.
    pub fn encode(&self, name: &str, args: &Bindings) -> Result<Vec<u8>, CodecError> {
        self.explain(name, args).map(|(bytes, _)| bytes)
    }

    /// The bytes of message `name` with `args`, and which template piece
    /// produced each run of them.
    pub fn explain(&self, name: &str, args: &Bindings) -> Result<(Vec<u8>, Vec<Span>), CodecError> {
        let m = self.message(name)?;
        let send = m
            .send
            .as_ref()
            .ok_or_else(|| CodecError::NoTemplate(name.into(), "send"))?;
        self.encode_template(m, send, args)
    }

    fn encode_template(
        &self,
        m: &'d Message,
        body: &'d Template,
        args: &Bindings,
    ) -> Result<(Vec<u8>, Vec<Span>), CodecError> {
        self.check_args(m, args)?;
        let medium = self.medium(m);
        let layout = self.layout(m, body);
        let mut chunks: Vec<(Chunk, Region, String)> = Vec::new();
        for (piece, region) in &layout {
            let chunk = match piece {
                Piece::Byte { value } => Chunk::Bytes(vec![*value]),
                Piece::Any | Piece::Rest { .. } => {
                    return Err(CodecError::Unsupported(format!(
                        "`{piece}` matches replies only"
                    )))
                }
                Piece::Field { name, encoding } => {
                    let v = self.value(name, args)?;
                    Chunk::Bytes(self.encode_value(name, &v, encoding.as_ref(), medium)?)
                }
                Piece::Slice {
                    name,
                    count,
                    encoding,
                    pad,
                } => Chunk::Bytes(self.encode_slice(
                    name,
                    args,
                    *count,
                    encoding.as_ref(),
                    *pad,
                    medium,
                )?),
                Piece::Expr { expr } => {
                    let e = Expr::parse(expr).map_err(CodecError::Expr)?;
                    let v = e.eval(&self.lookup(args)).map_err(CodecError::Expr)?;
                    Chunk::Bytes(plain(expr, v, medium)?)
                }
                Piece::Checksum(c) => {
                    if *region != Region::Body {
                        return Err(CodecError::Unsupported(
                            "a checksum belongs in the message body".into(),
                        ));
                    }
                    Chunk::Sum(c.clone())
                }
                Piece::Length { encoding } => Chunk::Len(encoding.clone()),
            };
            chunks.push((chunk, *region, self.label(piece, args)));
        }

        // A length counts the body bytes after it, up to the first checksum.
        if let Some(at) = chunks.iter().position(|(c, ..)| matches!(c, Chunk::Len(_))) {
            let mut count = 0usize;
            for (chunk, region, _) in &chunks[at + 1..] {
                match chunk {
                    _ if *region != Region::Body => break,
                    Chunk::Sum(_) => break,
                    Chunk::Bytes(b) => count += b.len(),
                    Chunk::Len(_) => {
                        return Err(CodecError::Unsupported(
                            "a message has at most one length field".into(),
                        ))
                    }
                }
            }
            let Chunk::Len(encoding) = &chunks[at].0 else {
                unreachable!()
            };
            let bytes =
                self.encode_value("len", &Val::Int(count as i64), Some(encoding), medium)?;
            chunks[at].0 = Chunk::Bytes(bytes);
            chunks[at].2 = format!("{} = {count}", chunks[at].2);
        }

        let mut out = Vec::new();
        let mut spans = Vec::new();
        let (mut body_start, mut body_end) = (None, 0);
        let mut sums = Vec::new();
        for (chunk, region, label) in chunks {
            if region == Region::Body && body_start.is_none() {
                body_start = Some(out.len());
            }
            let start = out.len();
            match chunk {
                Chunk::Bytes(b) => out.extend(b),
                Chunk::Sum(c) => {
                    sums.push((out.len(), c));
                    out.push(0);
                }
                Chunk::Len(_) => unreachable!(),
            }
            spans.push(Span {
                start,
                len: out.len() - start,
                region: match region {
                    Region::Prefix => "prefix",
                    Region::Body => "body",
                    Region::Suffix => "suffix",
                },
                label,
            });
            if region == Region::Body {
                body_end = out.len();
            }
        }
        let body_start = body_start.unwrap_or(0);
        for (at, c) in sums {
            out[at] = checksum(&c, &out[body_start..body_end])?;
        }
        if medium == Medium::Midi {
            check_7bit(&out)?;
        }
        if let Some(frame) = self.frame_len(m) {
            if out.len() > frame {
                return Err(CodecError::Range(format!(
                    "{} bytes do not fit the {frame}-byte frame",
                    out.len()
                )));
            }
            if out.len() < frame {
                spans.push(Span {
                    start: out.len(),
                    len: frame - out.len(),
                    region: "padding",
                    label: format!("to the {frame}-byte frame"),
                });
            }
            out.resize(frame, 0);
        }
        Ok((out, spans))
    }

    /// A piece as written, with the value it takes from `args` or a const.
    fn label(&self, piece: &Piece, args: &Bindings) -> String {
        let value = |name: &str| match self.value(name, args) {
            Ok(Val::Int(i)) => format!(" = {i} ({i:#X})"),
            Ok(Val::Bytes(b)) => format!(" = {} bytes", b.len()),
            Err(_) => String::new(),
        };
        match piece {
            Piece::Byte { .. } => "literal".into(),
            Piece::Field { name, .. } => format!("{piece}{}", value(name)),
            Piece::Expr { expr } => {
                let v = Expr::parse(expr)
                    .and_then(|e| e.eval(&self.lookup(args)))
                    .map(|v| format!(" = {v} ({v:#X})"))
                    .unwrap_or_default();
                format!("{piece}{v}")
            }
            _ => piece.to_string(),
        }
    }

    fn encode_slice(
        &self,
        name: &str,
        args: &Bindings,
        count: u16,
        encoding: Option<&Encoding>,
        pad: Option<u8>,
        medium: Medium,
    ) -> Result<Vec<u8>, CodecError> {
        let Val::Bytes(values) = self.value(name, args)? else {
            return Err(CodecError::Range(format!("`{name}` needs an array value")));
        };
        if values.len() > usize::from(count) {
            return Err(CodecError::Range(format!(
                "`{name}`: {} values for a slice of {count}",
                values.len()
            )));
        }
        let mut out = Vec::new();
        for i in 0..usize::from(count) {
            match values.get(i) {
                Some(v) => out.extend(self.encode_value(
                    name,
                    &Val::Int(i64::from(*v)),
                    encoding,
                    medium,
                )?),
                None => match pad {
                    Some(p) => out.push(p),
                    None => {
                        return Err(CodecError::Range(format!(
                            "`{name}`: {} values for a slice of {count} and no `pad`",
                            values.len()
                        )))
                    }
                },
            }
        }
        Ok(out)
    }

    fn encode_value(
        &self,
        name: &str,
        v: &Val,
        encoding: Option<&Encoding>,
        medium: Medium,
    ) -> Result<Vec<u8>, CodecError> {
        let default = match medium {
            Medium::Midi => Storage::U7,
            Medium::Usb => Storage::U8,
        };
        let encoding = encoding.cloned().unwrap_or(Encoding::Storage(default));
        let int = || match v {
            Val::Int(i) => Ok(*i),
            Val::Bytes(_) => Err(CodecError::Range(format!(
                "`{name}` is bytes; {encoding} needs a number"
            ))),
        };
        let bytes = || match v {
            Val::Bytes(b) => Ok(b.clone()),
            Val::Int(_) => Err(CodecError::Range(format!(
                "`{name}` is a number; {encoding} needs bytes"
            ))),
        };
        match &encoding {
            Encoding::Storage(Storage::Ascii(n)) => {
                let mut text = bytes()?;
                if text.len() > usize::from(*n) || text.iter().any(|b| !(0x20..=0x7E).contains(b)) {
                    return Err(CodecError::Range(format!(
                        "`{name}`: ascii({n}) takes up to {n} printable characters"
                    )));
                }
                text.resize(usize::from(*n), b' ');
                Ok(text)
            }
            Encoding::Storage(s) => store(name, *s, int()?),
            Encoding::MidiId => midi_id(name, int()?),
            Encoding::Packed7 => Ok(pack7(&bytes()?)),
            Encoding::Raw => bytes(),
            Encoding::Vendor(vendor) => self
                .vendor
                .get(vendor)
                .ok_or_else(|| {
                    CodecError::Unsupported(format!("no codec registered for `{vendor}`"))
                })?
                .encode(v)
                .map_err(CodecError::Range),
        }
    }

    // ---- decode -----------------------------------------------------------

    /// Matches `bytes` against the reply of message `name`. `known` holds
    /// values that must match (arguments, captures such as `device-id`);
    /// returns `known` plus everything the pattern captured.
    pub fn decode_reply(
        &self,
        name: &str,
        bytes: &[u8],
        known: &Bindings,
    ) -> Result<Bindings, CodecError> {
        self.decode_variant(name, bytes, known).map(|r| r.values)
    }

    /// Tries the reply variants in order (CDL §6.1); the first that matches
    /// wins. Fails with every variant's reason when none matches.
    pub fn decode_variant(
        &self,
        name: &str,
        bytes: &[u8],
        known: &Bindings,
    ) -> Result<Decoded, CodecError> {
        let m = self.message(name)?;
        if m.replies.is_empty() {
            return Err(CodecError::NoTemplate(name.into(), "reply"));
        }
        let mut misses = Vec::new();
        for reply in &m.replies {
            match self.decode_template(m, &reply.pattern, bytes, known) {
                Ok(values) => {
                    return Ok(Decoded {
                        variant: reply.variant.clone(),
                        values,
                    })
                }
                Err(e) if m.replies.len() == 1 => return Err(e),
                Err(e) => misses.push(format!(
                    "{}: {e}",
                    reply.variant.as_deref().unwrap_or("reply")
                )),
            }
        }
        Err(CodecError::NoVariant(misses))
    }

    /// Matches `bytes` against the `send` template: recovers the arguments
    /// of a sent message, as a trace decoder does.
    pub fn decode_send(
        &self,
        name: &str,
        bytes: &[u8],
        known: &Bindings,
    ) -> Result<Bindings, CodecError> {
        let m = self.message(name)?;
        let send = m
            .send
            .as_ref()
            .ok_or_else(|| CodecError::NoTemplate(name.into(), "send"))?;
        self.decode_template(m, send, bytes, known)
    }

    fn decode_template(
        &self,
        m: &'d Message,
        body: &'d Template,
        bytes: &[u8],
        known: &Bindings,
    ) -> Result<Bindings, CodecError> {
        let medium = self.medium(m);
        let layout = self.layout(m, body);
        let mut bound = known.clone();
        let mut at = 0usize;
        let (mut body_start, mut body_end) = (None, 0);
        let mut sums = Vec::new();
        // The length field's value and the byte index after it.
        let mut length: Option<(i64, usize)> = None;

        for (i, (piece, region)) in layout.iter().enumerate() {
            if *region == Region::Body && body_start.is_none() {
                body_start = Some(at);
            }
            let need = |n: usize| {
                if at + n > bytes.len() {
                    Err(CodecError::Truncated(bytes.len()))
                } else {
                    Ok(())
                }
            };
            match piece {
                Piece::Byte { value } => {
                    need(1)?;
                    if bytes[at] != *value {
                        return Err(mismatch(at, format!("{value:02X}"), bytes[at]));
                    }
                    at += 1;
                }
                Piece::Any => {
                    need(1)?;
                    at += 1;
                }
                Piece::Field { name, encoding } => {
                    let tail = self.tail_width(&layout[i + 1..], medium);
                    let (v, used) =
                        self.decode_value(name, encoding.as_ref(), medium, &bytes[at..], tail, at)?;
                    bind(&mut bound, self.d, name, v, at)?;
                    at += used;
                }
                Piece::Rest { name } => {
                    let tail = self
                        .tail_width(&layout[i + 1..], medium)
                        .ok_or_else(|| unsplittable(name))?;
                    let end = bytes.len().checked_sub(tail).filter(|e| *e >= at);
                    let end = end.ok_or(CodecError::Truncated(bytes.len()))?;
                    bind(
                        &mut bound,
                        self.d,
                        name,
                        Val::Bytes(bytes[at..end].to_vec()),
                        at,
                    )?;
                    at = end;
                }
                Piece::Slice {
                    name,
                    count,
                    encoding,
                    ..
                } => {
                    let mut values = Vec::new();
                    for _ in 0..*count {
                        let (v, used) = self.decode_value(
                            name,
                            encoding.as_ref(),
                            medium,
                            &bytes[at..],
                            None,
                            at,
                        )?;
                        match v {
                            Val::Int(i) if (0..=255).contains(&i) => values.push(i as u8),
                            _ => {
                                return Err(CodecError::Unsupported(format!(
                                    "`{name}`: slices decode to bytes"
                                )))
                            }
                        }
                        at += used;
                    }
                    bind(&mut bound, self.d, name, Val::Bytes(values), at)?;
                }
                Piece::Expr { expr } => {
                    need(1)?;
                    let e = Expr::parse(expr).map_err(CodecError::Expr)?;
                    let byte = i64::from(bytes[at]);
                    let lookup = self.lookup(&bound);
                    match e.eval(&lookup) {
                        Ok(v) if v == byte => {}
                        Ok(v) => return Err(mismatch(at, format!("{v:02X} ({expr})"), bytes[at])),
                        Err(_) => match e.solve(byte, &lookup).map_err(CodecError::Expr)? {
                            Some((name, v)) => {
                                drop(lookup);
                                bind(&mut bound, self.d, &name, Val::Int(v), at)?
                            }
                            None => return Err(mismatch(at, format!("`{expr}`"), bytes[at])),
                        },
                    }
                    at += 1;
                }
                Piece::Checksum(c) => {
                    need(1)?;
                    if *region != Region::Body {
                        return Err(CodecError::Unsupported(
                            "a checksum belongs in the message body".into(),
                        ));
                    }
                    sums.push((at, c.clone()));
                    at += 1;
                }
                Piece::Length { encoding } => {
                    let tail = self.tail_width(&layout[i + 1..], medium);
                    let (v, used) =
                        self.decode_value("len", Some(encoding), medium, &bytes[at..], tail, at)?;
                    let Val::Int(v) = v else {
                        return Err(CodecError::Unsupported("a length is a number".into()));
                    };
                    at += used;
                    length = Some((v, at));
                }
            }
            if *region == Region::Body {
                body_end = at;
            }
        }
        // A fixed-size frame is padded with zeros after the template.
        let padding = self.frame_len(m) == Some(bytes.len()) && bytes[at..].iter().all(|b| *b == 0);
        if at < bytes.len() && !padding {
            return Err(CodecError::Trailing(at));
        }
        let body_start = body_start.unwrap_or(0);
        if let Some((found, after)) = length {
            // Up to the first checksum after the length, or the body's end.
            let stop = sums
                .iter()
                .map(|(i, _)| *i)
                .find(|i| *i >= after)
                .unwrap_or(body_end);
            let expected = (stop - after) as i64;
            if expected != found {
                return Err(CodecError::Length { expected, found });
            }
        }
        // Checksums are computed with every checksum byte at 0, as on encode.
        let mut body = bytes[body_start..body_end].to_vec();
        for (index, _) in &sums {
            body[index - body_start] = 0;
        }
        for (index, c) in sums {
            let expected = checksum(&c, &body)?;
            if bytes[index] != expected {
                return Err(CodecError::Checksum {
                    index,
                    expected,
                    found: bytes[index],
                });
            }
        }
        self.check_args(m, &bound)?;
        Ok(bound)
    }

    /// Declared arguments (`arg`) must lie in their range (CDL §6.5).
    fn check_args(&self, m: &Message, bound: &Bindings) -> Result<(), CodecError> {
        for a in &m.args {
            let (Some(v), Some(r)) = (bound.get(&a.name), self.d.arg_range(m, &a.name)) else {
                continue;
            };
            match v {
                Val::Int(i) if (r.lo..=r.hi).contains(i) => {}
                v => {
                    return Err(CodecError::Range(format!(
                        "`{}` = {} is outside {}..={}",
                        a.name,
                        show(v),
                        r.lo,
                        r.hi
                    )))
                }
            }
        }
        Ok(())
    }

    /// The fixed byte width of `pieces`, or `None` when one varies.
    fn tail_width(&self, pieces: &[(&Piece, Region)], medium: Medium) -> Option<usize> {
        pieces.iter().try_fold(0usize, |n, (piece, _)| {
            let w = match piece {
                Piece::Byte { .. } | Piece::Any | Piece::Expr { .. } | Piece::Checksum(_) => 1,
                Piece::Field { encoding, .. } => fixed_width(encoding.as_ref(), medium)?,
                Piece::Length { encoding } => fixed_width(Some(encoding), medium)?,
                Piece::Slice {
                    count, encoding, ..
                } => usize::from(*count) * fixed_width(encoding.as_ref(), medium)?,
                Piece::Rest { .. } => return None,
            };
            Some(n + w)
        })
    }

    /// Decodes one value from the start of `bytes`. `tail` is the fixed
    /// width of what follows, for encodings that run to it.
    fn decode_value(
        &self,
        name: &str,
        encoding: Option<&Encoding>,
        medium: Medium,
        bytes: &[u8],
        tail: Option<usize>,
        at: usize,
    ) -> Result<(Val, usize), CodecError> {
        let default = match medium {
            Medium::Midi => Storage::U7,
            Medium::Usb => Storage::U8,
        };
        let encoding = encoding.cloned().unwrap_or(Encoding::Storage(default));
        let truncated = || CodecError::Truncated(at + bytes.len());
        let region = || -> Result<&[u8], CodecError> {
            let tail = tail.ok_or_else(|| unsplittable(name))?;
            let end = bytes.len().checked_sub(tail).ok_or_else(truncated)?;
            Ok(&bytes[..end])
        };
        match &encoding {
            Encoding::Storage(Storage::Ascii(n)) => {
                let n = usize::from(*n);
                let text = bytes.get(..n).ok_or_else(truncated)?;
                Ok((Val::Bytes(text.to_vec()), n))
            }
            Encoding::Storage(s) => {
                let w = storage_width(*s);
                let raw = bytes.get(..w).ok_or_else(truncated)?;
                Ok((Val::Int(load(name, *s, raw)?), w))
            }
            Encoding::MidiId => {
                let first = *bytes.first().ok_or_else(truncated)?;
                if first != 0 {
                    return Ok((Val::Int(i64::from(first)), 1));
                }
                let raw = bytes.get(..3).ok_or_else(truncated)?;
                Ok((Val::Int(i64::from(raw[1]) << 8 | i64::from(raw[2])), 3))
            }
            Encoding::Packed7 => {
                let packed = region()?;
                Ok((Val::Bytes(unpack7(packed)), packed.len()))
            }
            Encoding::Raw => {
                let raw = region()?;
                Ok((Val::Bytes(raw.to_vec()), raw.len()))
            }
            Encoding::Vendor(vendor) => self
                .vendor
                .get(vendor)
                .ok_or_else(|| {
                    CodecError::Unsupported(format!("no codec registered for `{vendor}`"))
                })?
                .decode(bytes)
                .map_err(CodecError::Range),
        }
    }

    /// `ok-when` of message `name` against decoded values; true without one.
    pub fn ok(&self, name: &str, bound: &Bindings) -> Result<bool, CodecError> {
        self.ok_variant(name, None, bound)
    }

    /// `ok-when` for a decoded variant: the variant's own, else the
    /// message's; true without either.
    pub fn ok_variant(
        &self,
        name: &str,
        variant: Option<&str>,
        bound: &Bindings,
    ) -> Result<bool, CodecError> {
        let m = self.message(name)?;
        let own = m
            .replies
            .iter()
            .find(|r| r.variant.as_deref() == variant)
            .and_then(|r| r.ok_when.as_ref());
        let Some(when) = own.or(m.ok_when.as_ref()) else {
            return Ok(true);
        };
        let e = Expr::parse(when).map_err(CodecError::Expr)?;
        Ok(e.eval(&self.lookup(bound)).map_err(CodecError::Expr)? != 0)
    }

    // ---- trace --------------------------------------------------------------

    /// Every message whose `send` (direction Out) or reply (In) matches
    /// `bytes`, in descriptor order. `None` tries both. `known` holds
    /// values a match must agree with, such as a captured `device-id`.
    pub fn identify(
        &self,
        bytes: &[u8],
        direction: Option<Direction>,
        known: &Bindings,
    ) -> Vec<Match> {
        let mut out = Vec::new();
        for m in &self.d.messages {
            if direction != Some(Direction::In) {
                if let Some(send) = &m.send {
                    if let Ok(values) = self.decode_send(&m.name, bytes, known) {
                        out.push(Match {
                            message: m.name.clone(),
                            direction: Direction::Out,
                            variant: None,
                            values,
                            ok: None,
                            literals: self.literals(m, send),
                        });
                    }
                }
            }
            if direction != Some(Direction::Out) && !m.replies.is_empty() {
                if let Ok(d) = self.decode_variant(&m.name, bytes, known) {
                    let ok = self
                        .ok_variant(&m.name, d.variant.as_deref(), &d.values)
                        .ok();
                    let reply = m
                        .replies
                        .iter()
                        .find(|r| r.variant == d.variant)
                        .unwrap_or(&m.replies[0]);
                    out.push(Match {
                        literals: self.literals(m, &reply.pattern),
                        message: m.name.clone(),
                        direction: Direction::In,
                        variant: d.variant,
                        values: d.values,
                        ok,
                    });
                }
            }
        }
        out
    }

    fn literals(&self, m: &'d Message, body: &'d Template) -> usize {
        self.layout(m, body)
            .iter()
            .filter(|(p, _)| matches!(p, Piece::Byte { .. }))
            .count()
    }

    // ---- test vectors -----------------------------------------------------

    /// Runs every message test vector: encode the arguments and compare
    /// with `bytes`, then decode `bytes` and compare with the arguments. A
    /// vector with `reply=#true` only decodes, against the reply pattern.
    pub fn run_vectors(&self) -> Vec<VectorResult> {
        self.d
            .test_vectors
            .iter()
            .map(|t| VectorResult {
                name: t.name.clone(),
                outcome: self.run_vector(t),
            })
            .collect()
    }

    fn run_vector(&self, t: &TestVector) -> Outcome {
        let Some(message) = &t.message else {
            return Outcome::Skipped("block vectors read external files".into());
        };
        let Some(bytes) = &t.bytes else {
            return Outcome::Skipped("no `bytes`".into());
        };
        let mut args = Bindings::new();
        for a in &t.args {
            let v = match &a.value {
                Value::Int(i) => Val::Int(*i),
                Value::Bool(b) => Val::Int(i64::from(*b)),
                Value::String(s) => Val::Bytes(s.as_bytes().to_vec()),
                other => return Outcome::Fail(format!("argument `{}`: {other:?}", a.name)),
            };
            args.insert(a.name.clone(), v);
        }
        for d in &t.data {
            args.insert(d.name.clone(), Val::Bytes(d.bytes.clone()));
        }
        let decoded = if t.reply {
            match self.decode_variant(message, bytes, &Bindings::new()) {
                Ok(d) if t.variant.is_some() && d.variant != t.variant => {
                    return Outcome::Fail(format!(
                        "matched variant {:?}, expected {:?}",
                        d.variant, t.variant
                    ))
                }
                other => other.map(|d| d.values),
            }
        } else {
            match self.encode(message, &args) {
                Ok(encoded) if &encoded == bytes => {}
                Ok(encoded) => {
                    return Outcome::Fail(format!(
                        "encodes to {}, expected {}",
                        hex(&encoded),
                        hex(bytes)
                    ))
                }
                Err(e) => return Outcome::Fail(format!("encode: {e}")),
            }
            self.decode_send(message, bytes, &Bindings::new())
        };
        let decoded = match decoded {
            Ok(d) => d,
            Err(e) => return Outcome::Fail(format!("decode: {e}")),
        };
        for (name, want) in &args {
            match decoded.get(name) {
                Some(got) if got == want => {}
                got => {
                    return Outcome::Fail(format!("decode: `{name}` is {got:?}, expected {want:?}"))
                }
            }
        }
        Outcome::Pass
    }
}

/// Binds `name`, or checks it when it is already bound or a constant.
fn bind(
    bound: &mut Bindings,
    d: &Descriptor,
    name: &str,
    v: Val,
    at: usize,
) -> Result<(), CodecError> {
    let existing = bound
        .get(name)
        .cloned()
        .or_else(|| d.constant(name).map(|c| Val::Int(c.value)));
    match existing {
        Some(e) if e != v => Err(CodecError::Mismatch {
            index: at,
            expected: format!("`{name}` = {}", show(&e)),
            found: show(&v),
        }),
        Some(_) => Ok(()),
        None => {
            bound.insert(name.to_string(), v);
            Ok(())
        }
    }
}

fn show(v: &Val) -> String {
    match v {
        Val::Int(i) => format!("{i:#X}"),
        Val::Bytes(b) => hex(b),
    }
}

pub fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn mismatch(index: usize, expected: String, found: u8) -> CodecError {
    CodecError::Mismatch {
        index,
        expected,
        found: format!("{found:02X}"),
    }
}

fn unsplittable(name: &str) -> CodecError {
    CodecError::Unsupported(format!(
        "`{name}` has no fixed width and what follows has none either"
    ))
}

/// A computed byte: 0..=7F on MIDI, 0..=FF on USB.
fn plain(what: &str, v: i64, medium: Medium) -> Result<Vec<u8>, CodecError> {
    let max = if medium == Medium::Midi { 0x7F } else { 0xFF };
    if (0..=max).contains(&v) {
        Ok(vec![v as u8])
    } else {
        Err(CodecError::Range(format!(
            "`{what}` = {v} does not fit one byte (0..={max})"
        )))
    }
}

fn check_7bit(bytes: &[u8]) -> Result<(), CodecError> {
    let last = bytes.len().saturating_sub(1);
    for (i, b) in bytes.iter().enumerate() {
        let edge = (i == 0 && *b == 0xF0) || (i == last && *b == 0xF7);
        if *b > 0x7F && !edge {
            return Err(CodecError::Range(format!(
                "byte {i} is {b:02X}; MIDI data bytes are 0..=7F"
            )));
        }
    }
    Ok(())
}

fn fixed_width(encoding: Option<&Encoding>, _medium: Medium) -> Option<usize> {
    match encoding {
        None => Some(1),
        Some(Encoding::Storage(s)) => Some(storage_width(*s)),
        Some(_) => None,
    }
}

fn storage_width(s: Storage) -> usize {
    match s {
        Storage::U7 | Storage::U8 | Storage::S7 | Storage::Bool | Storage::Bits(_) => 1,
        Storage::U14le | Storage::U14be | Storage::U16le | Storage::U16be => 2,
        Storage::Ascii(n) => usize::from(n),
    }
}

fn range_error(name: &str, s: impl fmt::Display, v: i64) -> CodecError {
    CodecError::Range(format!("`{name}` = {v} does not fit {s}"))
}

fn store(name: &str, s: Storage, v: i64) -> Result<Vec<u8>, CodecError> {
    let fits = |lo: i64, hi: i64| {
        if (lo..=hi).contains(&v) {
            Ok(())
        } else {
            Err(range_error(name, s, v))
        }
    };
    Ok(match s {
        Storage::U7 => {
            fits(0, 0x7F)?;
            vec![v as u8]
        }
        Storage::U8 => {
            fits(0, 0xFF)?;
            vec![v as u8]
        }
        Storage::S7 => {
            fits(-64, 63)?;
            vec![(v + 64) as u8]
        }
        Storage::Bool => {
            fits(0, 1)?;
            vec![v as u8]
        }
        Storage::Bits(n) => {
            fits(0, (1 << n) - 1)?;
            vec![v as u8]
        }
        Storage::U14le => {
            fits(0, 0x3FFF)?;
            vec![(v & 0x7F) as u8, (v >> 7) as u8]
        }
        Storage::U14be => {
            fits(0, 0x3FFF)?;
            vec![(v >> 7) as u8, (v & 0x7F) as u8]
        }
        Storage::U16le => {
            fits(0, 0xFFFF)?;
            vec![(v & 0xFF) as u8, (v >> 8) as u8]
        }
        Storage::U16be => {
            fits(0, 0xFFFF)?;
            vec![(v >> 8) as u8, (v & 0xFF) as u8]
        }
        Storage::Ascii(_) => unreachable!("handled by the caller"),
    })
}

fn load(name: &str, s: Storage, raw: &[u8]) -> Result<i64, CodecError> {
    let b = |i: usize| i64::from(raw[i]);
    let v = match s {
        Storage::U7 | Storage::U8 | Storage::Bool | Storage::Bits(_) => b(0),
        Storage::S7 => b(0) - 64,
        Storage::U14le => b(0) | b(1) << 7,
        Storage::U14be => b(0) << 7 | b(1),
        Storage::U16le => b(0) | b(1) << 8,
        Storage::U16be => b(0) << 8 | b(1),
        Storage::Ascii(_) => unreachable!("handled by the caller"),
    };
    // Re-encoding must give the same bytes: a u7 of 0x80 or a bool of 2 is
    // not a value of that type.
    if store(name, s, v)? != raw {
        return Err(range_error(name, s, v));
    }
    Ok(v)
}

/// One byte below 0x80, otherwise `00 hi lo` with 7-bit `hi = v >> 8` and
/// `lo = v & 0xFF` (CDL §8.1).
fn midi_id(name: &str, v: i64) -> Result<Vec<u8>, CodecError> {
    if (0..0x80).contains(&v) {
        return Ok(vec![v as u8]);
    }
    let (hi, lo) = (v >> 8, v & 0xFF);
    if (0x80..=0x7F7F).contains(&v) && hi <= 0x7F && lo <= 0x7F {
        Ok(vec![0, hi as u8, lo as u8])
    } else {
        Err(range_error(name, "midi-id", v))
    }
}

fn checksum(c: &ctrl_cdl::Checksum, body: &[u8]) -> Result<u8, CodecError> {
    let len = body.len() as i64;
    let index = |i: i64| if i < 0 { len + i } else { i };
    let (from, to) = (index(c.from), index(c.to));
    if !(0 <= from && from <= to && to <= len) {
        return Err(CodecError::Range(format!(
            "checksum range {}..{} is outside the {len}-byte body",
            c.from, c.to
        )));
    }
    let bytes = &body[from as usize..to as usize];
    let sum: u32 = bytes.iter().map(|b| u32::from(*b)).sum();
    Ok(match c.algorithm {
        ChecksumAlgorithm::Sum7 | ChecksumAlgorithm::Roland => ((128 - sum % 128) % 128) as u8,
        ChecksumAlgorithm::Xor7 => bytes.iter().fold(0, |x, b| x ^ b) & 0x7F,
        ChecksumAlgorithm::Fnv1a32 => {
            return Err(CodecError::Unsupported(
                "fnv1a32 is for files, not messages".into(),
            ))
        }
    })
}
