//! `ctrl-engine`: runs CDL procedures (CDL §11) against a [`Port`] and an
//! [`Operator`], with no I/O of its own.
//!
//! Two documents take part. The **editor document** holds what the user
//! wants on the device: block data loaded from a file and edited with
//! [`Document::set`]. The **device document** holds what the device sent
//! during this run (`await`, `exchange`). A `restore-point` snapshots the
//! device document and hands the raw messages to the operator to keep; a
//! `send` with `records-from` takes its data from the editor document; a
//! `verify` runs a read procedure and compares the device document with the
//! editor document on the given paths.
//!
//! The write policy (§11) is the descriptor's: the validator refuses a write
//! without a gate and a restore point, or one that sends bytes not measured.
//! The engine adds: nothing is written unless the run is `apply`, every gate
//! asks the operator, and a write stops when no restore point was kept.

mod document;

use std::fmt;

use ctrl_cdl::{Descriptor, Expr, Gate, Index, MessageStep, Path, RepeatOver, Step, Value};
use ctrl_codec::{Bindings, Codec, CodecError, Planned, Val};

pub use document::Document;

/// The device side: sends planned writes, returns whole messages.
pub trait Port {
    fn send(&mut self, planned: &Planned) -> Result<(), String>;
    /// The next whole message from the device, or None after `timeout_ms`.
    fn recv(&mut self, timeout_ms: u32) -> Result<Option<Vec<u8>>, String>;
}

/// The person at the editor.
pub trait Operator {
    /// Shows a gate's text; true to go on.
    fn gate(&mut self, gate: Gate, text: &str) -> bool;
    /// Progress and dry-run output.
    fn note(&mut self, text: &str);
    /// Keeps a restore point: the raw messages the device sent for it.
    /// An error stops the run before anything is written.
    fn restore_point(&mut self, procedure: &str, messages: &[Vec<u8>]) -> Result<(), String>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineError {
    UnknownProcedure(String),
    /// The operator said no at a gate; nothing after it ran.
    Declined(String),
    Timeout(String),
    Codec(String),
    Port(String),
    /// A check failed: ok-when, verify, a missing restore point.
    Check(String),
    Unsupported(String),
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownProcedure(p) => write!(f, "unknown procedure `{p}`"),
            Self::Declined(t) => write!(f, "stopped at a gate: {t}"),
            Self::Timeout(m) => write!(f, "no `{m}` arrived in time"),
            Self::Codec(e) | Self::Port(e) | Self::Check(e) | Self::Unsupported(e) => {
                f.write_str(e)
            }
        }
    }
}

impl std::error::Error for EngineError {}

impl From<CodecError> for EngineError {
    fn from(e: CodecError) -> Self {
        Self::Codec(e.to_string())
    }
}

/// What a run did, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Gate(String),
    Received(String, usize),
    RestorePoint(String),
    /// A message the run sent (or, in a dry run, would send).
    Sent {
        message: String,
        bytes: Vec<u8>,
        applied: bool,
    },
    Verified(String),
}

pub struct Run<'d> {
    d: &'d Descriptor,
    codec: Codec<'d>,
    /// What the editor wants on the device.
    pub doc: Document,
    /// What the device sent during this run.
    pub device: Document,
    /// Procedure variables (`setup=15`) and reply captures.
    pub vars: Bindings,
    /// Without it nothing is written; sends are shown instead.
    pub apply: bool,
    pub events: Vec<Event>,
    /// Raw messages the device sent in this run, oldest first.
    received: Vec<Vec<u8>>,
    restore_kept: bool,
}

/// Fallback for `await` and `exchange` without a `timeout`.
const DEFAULT_TIMEOUT_MS: u32 = 2000;

impl<'d> Run<'d> {
    pub fn new(d: &'d Descriptor) -> Self {
        Self {
            d,
            codec: Codec::new(d),
            doc: Document::default(),
            device: Document::default(),
            vars: Bindings::new(),
            apply: false,
            events: Vec::new(),
            received: Vec::new(),
            restore_kept: false,
        }
    }

    pub fn codec(&self) -> &Codec<'d> {
        &self.codec
    }

    /// Loads a message the device sent (a dump file) into the editor
    /// document: its records fill the addressed blocks.
    pub fn load(&mut self, message: &str, bytes: &[u8]) -> Result<Bindings, EngineError> {
        let mut doc = std::mem::take(&mut self.doc);
        let r = doc.load(&self.codec, self.d, message, bytes);
        self.doc = doc;
        r
    }

    /// Sets a field of the editor document: `names[15].group[0].char[0]`.
    pub fn set(&mut self, path: &str, value: i64) -> Result<(), EngineError> {
        let path = Path::parse(path).map_err(EngineError::Check)?;
        let vars = self.vars.clone();
        self.doc
            .set(&self.codec, self.d, &path, value, &|n| lookup(&vars, n))
    }

    pub fn run(
        &mut self,
        procedure: &str,
        port: &mut dyn Port,
        op: &mut dyn Operator,
    ) -> Result<(), EngineError> {
        let p = self
            .d
            .procedure(procedure)
            .ok_or_else(|| EngineError::UnknownProcedure(procedure.into()))?;
        let write = matches!(
            p.kind,
            Some(ctrl_cdl::Access::Write | ctrl_cdl::Access::Commit)
        );
        if write {
            self.check_sources(&p.steps)?;
        }
        self.steps(procedure, write, &p.steps, port, op)
    }

    /// Refuses a write whose `records-from` blocks are not in the editor
    /// document before any step runs, so the operator is not asked to arm
    /// the device for a send that cannot happen. A path whose index needs a
    /// value the run sets later is left to the send itself.
    fn check_sources(&self, steps: &'d [Step]) -> Result<(), EngineError> {
        for step in steps {
            match step {
                Step::Send(s) | Step::Exchange(s) => {
                    for path in &s.records_from {
                        let Ok((block, index)) = self.element(path) else {
                            continue;
                        };
                        if self.doc.block_bytes(&self.codec, &block, index).is_none() {
                            return Err(EngineError::Check(format!(
                                "the document has no `{path}`: load the data to write first"
                            )));
                        }
                    }
                }
                Step::Call { procedure: sub, .. } => {
                    if let Some(p) = self.d.procedure(sub) {
                        self.check_sources(&p.steps)?;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn steps(
        &mut self,
        procedure: &str,
        write: bool,
        steps: &'d [Step],
        port: &mut dyn Port,
        op: &mut dyn Operator,
    ) -> Result<(), EngineError> {
        for step in steps {
            match step {
                Step::Gate { gate, text, .. } => {
                    let text = self.text(text.as_deref().unwrap_or("Go on?"));
                    self.events.push(Event::Gate(text.clone()));
                    if !op.gate(*gate, &text) {
                        return Err(EngineError::Declined(text));
                    }
                }
                Step::Await(s) => self.await_step(s, port, op)?,
                Step::Send(s) | Step::Exchange(s) => {
                    if write && !self.restore_kept {
                        return Err(EngineError::Check(format!(
                            "`{procedure}` would write `{}` without a kept restore point",
                            s.message
                        )));
                    }
                    self.send_step(s, port, op)?;
                    if matches!(step, Step::Exchange(_)) {
                        self.await_step(s, port, op)?;
                    }
                }
                Step::RestorePoint { paths, .. } => {
                    for path in paths.iter().flat_map(|p| self.elements(p)) {
                        if self.device_bytes(&path)?.is_none() {
                            return Err(EngineError::Check(format!(
                                "restore point: the device has not sent `{path}` in this run"
                            )));
                        }
                    }
                    op.restore_point(procedure, &self.received)
                        .map_err(EngineError::Check)?;
                    self.restore_kept = true;
                    self.events.push(Event::RestorePoint(procedure.into()));
                }
                Step::Verify {
                    procedure: read,
                    compare,
                    ..
                } => {
                    let p = self
                        .d
                        .procedure(read)
                        .ok_or_else(|| EngineError::UnknownProcedure(read.clone()))?;
                    self.device = Document::default();
                    self.steps(read, false, &p.steps, port, op)?;
                    for path in compare {
                        let want = self.doc_bytes(path)?;
                        let got = self.device_bytes(path)?;
                        if want.is_none() || want != got {
                            return Err(EngineError::Check(format!(
                                "verify: `{path}` on the device differs from what was written"
                            )));
                        }
                    }
                    self.events.push(Event::Verified(read.clone()));
                    op.note(&format!("verified with `{read}`"));
                }
                Step::Call { procedure: sub, .. } => {
                    let p = self
                        .d
                        .procedure(sub)
                        .ok_or_else(|| EngineError::UnknownProcedure(sub.clone()))?;
                    self.steps(sub, write, &p.steps, port, op)?;
                }
                Step::OkWhen { expr, .. } | Step::Require { expr, .. } => {
                    if self.eval(expr)? == 0 {
                        return Err(EngineError::Check(format!("`{expr}` does not hold")));
                    }
                }
                Step::Delay { ms, .. } => {
                    if self.apply {
                        std::thread::sleep(std::time::Duration::from_millis(u64::from(*ms)));
                    }
                }
                Step::Repeat {
                    var, over, steps, ..
                } => {
                    let RepeatOver::Range(r) = over else {
                        return Err(EngineError::Unsupported(format!(
                            "repeat over `{var}`: only ranges run here"
                        )));
                    };
                    for i in r.lo..=r.hi {
                        self.vars.insert(var.clone(), Val::Int(i));
                        self.steps(procedure, write, steps, port, op)?;
                    }
                }
                other => {
                    return Err(EngineError::Unsupported(format!(
                        "step {other:?} does not run in this engine yet"
                    )))
                }
            }
        }
        Ok(())
    }

    fn await_step(
        &mut self,
        s: &MessageStep,
        port: &mut dyn Port,
        op: &mut dyn Operator,
    ) -> Result<(), EngineError> {
        let mut want = Bindings::new();
        for a in &s.args {
            want.insert(a.name.clone(), Val::Int(self.arg_value(&a.value)?));
        }
        let timeout = s.timeout.unwrap_or(DEFAULT_TIMEOUT_MS);
        let start = std::time::Instant::now();
        loop {
            let left = timeout.saturating_sub(start.elapsed().as_millis() as u32);
            let Some(bytes) = port.recv(left).map_err(EngineError::Port)? else {
                return Err(EngineError::Timeout(s.message.clone()));
            };
            // other traffic (controls, clock) is skipped
            let Ok(values) = self.codec.decode_reply(&s.message, &bytes, &want) else {
                continue;
            };
            let mut device = std::mem::take(&mut self.device);
            let loaded = device.load(&self.codec, self.d, &s.message, &bytes);
            self.device = device;
            loaded?;
            for (k, v) in values {
                if let Val::Int(_) = v {
                    self.vars.insert(k, v);
                }
            }
            op.note(&format!("received `{}`, {} bytes", s.message, bytes.len()));
            self.events
                .push(Event::Received(s.message.clone(), bytes.len()));
            self.received.push(bytes);
            return Ok(());
        }
    }

    fn send_step(
        &mut self,
        s: &MessageStep,
        port: &mut dyn Port,
        op: &mut dyn Operator,
    ) -> Result<(), EngineError> {
        let m = self
            .d
            .message(&s.message)
            .ok_or_else(|| EngineError::Codec(format!("unknown message `{}`", s.message)))?;
        let mut args = Bindings::new();
        for a in &s.args {
            args.insert(a.name.clone(), Val::Int(self.arg_value(&a.value)?));
        }
        if !s.records_from.is_empty() {
            let r = m
                .records
                .first()
                .ok_or_else(|| EngineError::Check(format!("`{}` has no records", m.name)))?;
            let mut pieces = Vec::new();
            for path in &s.records_from {
                let (block, index) = self.element(path)?;
                let data = self
                    .doc
                    .block_bytes(&self.codec, &block, index)
                    .ok_or_else(|| EngineError::Check(format!("the document has no `{path}`")))?;
                pieces.extend(self.codec.block_pieces(&block, index, &data)?);
            }
            let records = self.codec.records_for(&m.name, &r.name, &pieces)?;
            let bytes = self.codec.encode_records(&m.name, &r.name, &records)?;
            args.insert(r.name.clone(), Val::Bytes(bytes));
        }
        for planned in self.codec.plan(&m.name, &args)? {
            if self.apply {
                port.send(&planned).map_err(EngineError::Port)?;
                op.note(&format!(
                    "sent `{}`, {} bytes in {} writes",
                    planned.message,
                    planned.frame.len(),
                    planned.writes.len()
                ));
            } else {
                op.note(&format!(
                    "dry run: would send `{}`, {} bytes in {} writes",
                    planned.message,
                    planned.frame.len(),
                    planned.writes.len()
                ));
            }
            self.events.push(Event::Sent {
                message: planned.message.clone(),
                bytes: planned.frame.clone(),
                applied: self.apply,
            });
        }
        Ok(())
    }

    /// A path without an index on a block with `count` means every element.
    fn elements(&self, path: &Path) -> Vec<Path> {
        let seg = &path.segments[0];
        let count = self.d.block(&seg.name).and_then(|b| b.count);
        match (count, &seg.index, path.segments.len()) {
            (Some(n), None, 1) => (0..n)
                .map(|i| Path {
                    segments: vec![ctrl_cdl::Segment {
                        name: seg.name.clone(),
                        index: Some(Index::At(i.to_string())),
                    }],
                })
                .collect(),
            _ => vec![path.clone()],
        }
    }

    /// `block[index]`, the index evaluated with the run's variables.
    fn element(&self, path: &Path) -> Result<(String, u32), EngineError> {
        let seg = &path.segments[0];
        let index = match &seg.index {
            None => 0,
            Some(Index::At(e)) => u32::try_from(self.eval(e)?)
                .map_err(|_| EngineError::Check(format!("`{path}`: negative index")))?,
            Some(Index::Range { .. }) => {
                return Err(EngineError::Unsupported(format!("`{path}`: a range")))
            }
        };
        if path.segments.len() != 1 {
            return Err(EngineError::Unsupported(format!(
                "`{path}`: only whole block elements here"
            )));
        }
        Ok((seg.name.clone(), index))
    }

    fn doc_bytes(&self, path: &Path) -> Result<Option<Vec<u8>>, EngineError> {
        let (block, index) = self.element(path)?;
        Ok(self.doc.block_bytes(&self.codec, &block, index))
    }

    fn device_bytes(&self, path: &Path) -> Result<Option<Vec<u8>>, EngineError> {
        let (block, index) = self.element(path)?;
        Ok(self.device.block_bytes(&self.codec, &block, index))
    }

    fn eval(&self, expr: &str) -> Result<i64, EngineError> {
        let e = Expr::parse(expr).map_err(EngineError::Check)?;
        e.eval(&|n| lookup(&self.vars, n))
            .map_err(|e| EngineError::Check(format!("`{expr}`: {e}")))
    }

    fn arg_value(&self, v: &Value) -> Result<i64, EngineError> {
        match v {
            Value::Int(i) => Ok(*i),
            Value::String(s) => match s.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
                Some(e) => self.eval(e),
                None => Err(EngineError::Check(format!(
                    "argument `{s}` is not a number"
                ))),
            },
            other => Err(EngineError::Check(format!(
                "argument {other:?} is not a number"
            ))),
        }
    }

    /// Gate texts may hold `{expression}`s over the run's variables.
    fn text(&self, t: &str) -> String {
        let mut out = String::new();
        let mut rest = t;
        while let Some(open) = rest.find('{') {
            let Some(close) = rest[open..].find('}') else {
                break;
            };
            out.push_str(&rest[..open]);
            let e = &rest[open + 1..open + close];
            match self.eval(e) {
                Ok(v) => out.push_str(&v.to_string()),
                Err(_) => out.push_str(&rest[open..=open + close]),
            }
            rest = &rest[open + close + 1..];
        }
        out.push_str(rest);
        out
    }
}

fn lookup(vars: &Bindings, name: &str) -> Option<i64> {
    match vars.get(name) {
        Some(Val::Int(i)) => Some(*i),
        _ => None,
    }
}
