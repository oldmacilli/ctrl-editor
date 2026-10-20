//! Checks on a loaded [`Descriptor`]: references, sizes and ranges, the
//! status rules (CDL §4.1), the write policy (§11) and transports (§5).
//!
//! A node marked `unknown` is disabled: it is listed as a note, and it is an
//! error only when a node that is not `unknown` depends on it.

use std::fmt;

use crate::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// The descriptor is wrong, or would send something it must not.
    Error,
    /// Probably a mistake.
    Warning,
    /// Listed by `ctrl-cli check`: unknown and inferred items.
    Note,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub severity: Severity,
    pub error: Error,
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Note => "note",
        };
        write!(
            f,
            "{}:{}:{}: {label}: {}",
            self.error.file, self.error.line, self.error.col, self.error.message
        )
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    pub findings: Vec<Finding>,
}

impl Report {
    pub fn errors(&self) -> impl Iterator<Item = &Finding> {
        self.findings
            .iter()
            .filter(|f| f.severity == Severity::Error)
    }

    pub fn is_ok(&self) -> bool {
        self.errors().next().is_none()
    }
}

/// Runs every check. Findings come in check order; each carries a position.
pub fn validate(d: &Descriptor) -> Report {
    let mut v = V {
        d,
        findings: Vec::new(),
    };
    v.types();
    v.blocks();
    v.transports();
    v.messages();
    v.procedures();
    v.firmware();
    v.identify();
    v.panel();
    v.midi_maps();
    v.lbmt();
    v.drivers();
    v.test_vectors();
    v.unknown_and_inferred();
    Report {
        findings: v.findings,
    }
}

/// The transport name that means "the config transport the user selected".
pub const SELECTED_TRANSPORT: &str = "selected";

struct V<'d> {
    d: &'d Descriptor,
    findings: Vec<Finding>,
}

/// What a path resolves to.
enum Target<'d> {
    Block(&'d Block),
    Group(&'d Group),
    Field(&'d Field),
    Array(&'d Array),
}

impl<'d> V<'d> {
    fn push(&mut self, severity: Severity, pos: Pos, message: impl Into<String>) {
        let error = self.d.error_at(pos, message);
        self.findings.push(Finding { severity, error });
    }

    fn error(&mut self, pos: Pos, message: impl Into<String>) {
        self.push(Severity::Error, pos, message);
    }

    /// Reports an expression that does not parse (CDL §10.1).
    fn expr(&mut self, pos: Pos, what: &str, s: &str) {
        if let Err(e) = Expr::parse(s) {
            self.error(pos, format!("{what}: {e}"));
        }
    }

    fn template_exprs(&mut self, pos: Pos, what: &str, t: &Template) {
        for piece in &t.pieces {
            if let Piece::Expr { expr } = piece {
                self.expr(pos, what, expr);
            }
        }
    }

    fn type_def(&self, name: &str) -> Option<&'d TypeDef> {
        self.d.types.iter().find(|t| t.name() == name)
    }

    // ---- types and blocks -------------------------------------------------

    fn types(&mut self) {
        for t in &self.d.types {
            match t {
                TypeDef::Enum(e) | TypeDef::Errors(e) => {
                    for (i, v) in e.values.iter().enumerate() {
                        if e.values[..i].iter().any(|w| w.value == v.value) {
                            self.error(
                                v.meta.pos,
                                format!("`{}` has value {} twice", e.name, v.value),
                            );
                        }
                        if !(0..=255).contains(&v.value) {
                            self.error(
                                v.meta.pos,
                                format!("`{}`: {} does not fit one byte", e.name, v.value),
                            );
                        }
                    }
                }
                TypeDef::Int(i) => match storage_range(i.storage) {
                    None => self.error(
                        i.meta.pos,
                        format!("int `{}`: {} is not a number type", i.name, i.storage),
                    ),
                    Some(full) => {
                        if let Some(r) = i.range {
                            if r.lo < full.lo || r.hi > full.hi {
                                self.error(
                                    i.meta.pos,
                                    format!(
                                        "int `{}`: range {}..={} does not fit {}",
                                        i.name, r.lo, r.hi, i.storage
                                    ),
                                );
                            }
                        }
                    }
                },
            }
        }
    }

    /// The width in bytes and the value range of a field type.
    fn type_shape(&mut self, pos: Pos, ty: &TypeRef) -> Option<(u32, Option<IntRange>)> {
        match ty {
            TypeRef::Storage(s) => Some((storage_width(*s), storage_range(*s))),
            TypeRef::Named(name) => match self.type_def(name) {
                Some(TypeDef::Int(i)) => Some((
                    storage_width(i.storage),
                    i.range.or(storage_range(i.storage)),
                )),
                Some(TypeDef::Enum(_)) => Some((1, None)),
                Some(TypeDef::Errors(_)) => {
                    self.error(
                        pos,
                        format!("`{name}` is an `errors` table, not a field type"),
                    );
                    None
                }
                None => {
                    self.error(pos, format!("unknown type `{name}`"));
                    None
                }
            },
        }
    }

    fn blocks(&mut self) {
        for b in &self.d.blocks {
            self.items(&b.name, &b.items, 0, b.size);
        }
    }

    /// Checks names, types, ranges and that offsets fit the block size.
    fn items(&mut self, parent: &str, items: &'d [Item], base: u32, size: Option<u32>) {
        for (i, item) in items.iter().enumerate() {
            let (name, meta) = item_name(item);
            if items[..i].iter().any(|o| item_name(o).0 == name) {
                self.error(meta.pos, format!("`{parent}` has two items named `{name}`"));
            }
            match item {
                Item::Field(f) => {
                    if let Some(when) = &f.visible_when {
                        self.expr(f.meta.pos, "visible-when", when);
                    }
                    let Some((width, range)) = self.type_shape(f.meta.pos, &f.ty) else {
                        continue;
                    };
                    if let (Some(offset), Some(size)) = (f.offset, size) {
                        if base + offset + width > size {
                            self.error(
                                f.meta.pos,
                                format!(
                                    "field `{}` ends at byte {}, past the block size {size}",
                                    f.name,
                                    base + offset + width
                                ),
                            );
                        }
                    }
                    if let (Some(own), Some(full)) = (f.range, range) {
                        if own.lo < full.lo || own.hi > full.hi {
                            self.error(
                                f.meta.pos,
                                format!(
                                    "field `{}`: range {}..={} is outside its type",
                                    f.name, own.lo, own.hi
                                ),
                            );
                        }
                    }
                    if let Some(fixed) = f.fixed {
                        let r = f.range.or(range);
                        if r.is_some_and(|r| fixed < r.lo || fixed > r.hi) {
                            self.error(
                                f.meta.pos,
                                format!("field `{}`: fixed {fixed} is outside its range", f.name),
                            );
                        }
                    }
                }
                Item::Group(g) => {
                    let base = base + g.offset.unwrap_or(0);
                    self.items(&g.name, &g.items, base, size);
                }
                Item::Array(a) => {
                    if let Some(ty) = &a.ty {
                        if let (Some(width), Some(offset), Some(size)) =
                            (self.type_shape(a.meta.pos, ty).map(|s| s.0), a.offset, size)
                        {
                            let stride = a.stride.unwrap_or(width);
                            let end = base + offset + stride * a.count.saturating_sub(1) + width;
                            if end > size {
                                self.error(
                                    a.meta.pos,
                                    format!(
                                        "array `{}` ends at byte {end}, past the block size {size}",
                                        a.name
                                    ),
                                );
                            }
                        }
                    }
                    if let Some(driver) = &a.driver {
                        self.driver_ref(a.meta.pos, driver);
                    }
                    self.items(&a.name, &a.items, base, None);
                }
            }
        }
    }

    // ---- paths ------------------------------------------------------------

    fn resolve(&self, path: &Path) -> Result<Target<'d>, String> {
        let first = &path.segments[0];
        let block = self
            .d
            .block(&first.name)
            .ok_or_else(|| format!("`{path}`: unknown block `{}`", first.name))?;
        if first.index.is_some() && block.count.is_none() {
            return Err(format!(
                "`{path}`: block `{}` has no `count`, so it takes no index",
                block.name
            ));
        }
        let mut target = Target::Block(block);
        for segment in &path.segments[1..] {
            let items: &[Item] = match target {
                Target::Block(b) => &b.items,
                Target::Group(g) => &g.items,
                Target::Array(a) => &a.items,
                Target::Field(f) => {
                    return Err(format!("`{path}`: field `{}` has no members", f.name))
                }
            };
            let item = items
                .iter()
                .find(|i| item_name(i).0 == segment.name)
                .ok_or_else(|| format!("`{path}`: no item `{}`", segment.name))?;
            target = match item {
                Item::Field(f) => Target::Field(f),
                Item::Group(g) => Target::Group(g),
                Item::Array(a) => Target::Array(a),
            };
            if segment.index.is_some() && !matches!(target, Target::Array(_)) {
                return Err(format!(
                    "`{path}`: `{}` is not an array, so it takes no index",
                    segment.name
                ));
            }
        }
        Ok(target)
    }

    /// The weakest status along a path: block, then each item.
    fn path_status(&self, path: &Path) -> Option<Status> {
        let block = self.d.block(path.block())?;
        let mut worst = block.meta.status;
        let mut items: &[Item] = &block.items;
        for segment in &path.segments[1..] {
            let Some(item) = items.iter().find(|i| item_name(i).0 == segment.name) else {
                break;
            };
            worst = weaker(worst, item_name(item).1.status);
            items = match item {
                Item::Group(g) => &g.items,
                Item::Array(a) => &a.items,
                Item::Field(_) => &[],
            };
        }
        worst
    }

    fn path(&mut self, pos: Pos, path: &Path) {
        if let Err(e) = self.resolve(path) {
            self.error(pos, e);
        }
        for segment in &path.segments {
            match &segment.index {
                Some(Index::At(e)) => self.expr(pos, "index", e),
                Some(Index::Range { start, end }) => {
                    self.expr(pos, "index", start);
                    self.expr(pos, "index", end);
                }
                None => {}
            }
        }
    }

    // ---- transports -------------------------------------------------------

    fn transport_ref(&mut self, pos: Pos, name: &str) {
        if name == SELECTED_TRANSPORT {
            // §5: resolved at run time to the config transport the user or the
            // `preferred` flag picks; there must be at least one.
            if !self
                .d
                .transports
                .iter()
                .any(|t| t.role == Some(Role::Config))
            {
                self.error(
                    pos,
                    "transport `selected` needs at least one `role=\"config\"` transport",
                );
            }
        } else if self.d.transport(name).is_none() {
            self.error(pos, format!("unknown transport `{name}`"));
        }
    }

    fn transports(&mut self) {
        let preferred: Vec<_> = self.d.transports.iter().filter(|t| t.preferred).collect();
        for extra in preferred.iter().skip(1) {
            self.error(
                extra.meta.pos,
                format!(
                    "transport `{}`: only one transport may be preferred (`{}` is too)",
                    extra.name, preferred[0].name
                ),
            );
        }
        for (i, t) in self.d.transports.iter().enumerate() {
            match t.kind {
                TransportKind::Hid if t.frame.is_none() => self.error(
                    t.meta.pos,
                    format!("hid transport `{}` needs `frame=` (report size)", t.name),
                ),
                TransportKind::Hid | TransportKind::UsbBulk if t.ports.is_some() => self.error(
                    t.meta.pos,
                    format!("transport `{}`: port matches are for MIDI", t.name),
                ),
                _ => {}
            }
            if t.preferred && t.role != Some(Role::Config) {
                self.error(
                    t.meta.pos,
                    format!(
                        "transport `{}`: `preferred` is for config transports",
                        t.name
                    ),
                );
            }
            if let Some(scan) = &t.scan {
                match self.d.message(scan) {
                    None => self.error(t.meta.pos, format!("scan with unknown message `{scan}`")),
                    Some(m) => {
                        if t.kind != TransportKind::Midi {
                            self.error(
                                t.meta.pos,
                                format!("transport `{}`: `scan` is for MIDI", t.name),
                            );
                        }
                        if m.kind.is_some_and(|k| k != Access::Read) {
                            self.error(
                                t.meta.pos,
                                format!("transport `{}`: the scan message `{scan}` must be a read; it goes to every device on every port", t.name),
                            );
                        }
                        if matches!(
                            m.meta.status,
                            None | Some(Status::Unknown | Status::Inferred)
                        ) {
                            self.error(
                                t.meta.pos,
                                format!("transport `{}`: the scan message `{scan}` must be measured or decoded", t.name),
                            );
                        }
                        if m.replies.is_empty() {
                            self.error(
                                t.meta.pos,
                                format!("transport `{}`: the scan message `{scan}` needs a `reply` to match", t.name),
                            );
                        }
                    }
                }
            }
            for c in &t.captures {
                if self.d.message(&c.from).is_none() {
                    self.error(
                        c.meta.pos,
                        format!("capture from unknown message `{}`", c.from),
                    );
                }
            }
            // §5: no port may match two transports, per direction. Patterns
            // are substrings, so one containing the other overlaps.
            for earlier in &self.d.transports[..i] {
                let overlaps: Vec<_> = [
                    ("input", input(earlier), input(t)),
                    ("output", output(earlier), output(t)),
                ]
                .into_iter()
                .filter_map(|(direction, a, b)| match (a, b) {
                    (Some(a), Some(b)) if a.contains(b) || b.contains(a) => Some((direction, a, b)),
                    _ => None,
                })
                .collect();
                let Some((_, a, b)) = overlaps.first() else {
                    continue;
                };
                let directions = if overlaps.len() == 2 {
                    "input and output".to_string()
                } else {
                    overlaps[0].0.to_string()
                };
                self.error(
                    t.meta.pos,
                    format!(
                        "transport `{}`: {directions} match \"{b}\" overlaps \"{a}\" of `{}`; name the ports fully",
                        t.name, earlier.name
                    ),
                );
            }
        }
    }

    /// The kind of transport a message goes out on, if known.
    fn message_kind(&self, m: &Message) -> Option<TransportKind> {
        match m.transport.as_deref()? {
            SELECTED_TRANSPORT => {
                let mut config = self
                    .d
                    .transports
                    .iter()
                    .filter(|t| t.role == Some(Role::Config));
                let first = config.next()?.kind;
                config.all(|t| t.kind == first).then_some(first)
            }
            name => self.d.transport(name).map(|t| t.kind),
        }
    }

    // ---- messages ---------------------------------------------------------

    fn messages(&mut self) {
        // A frame's names come from the device, not from message arguments:
        // each is a `const` or a transport `capture` (CDL §8.1).
        let captured: Vec<&str> = self
            .d
            .transports
            .iter()
            .flat_map(|t| t.captures.iter().map(|c| c.name.as_str()))
            .collect();
        for p in &self.d.patterns {
            self.template_exprs(p.meta.pos, "pattern", &p.template);
        }
        for f in &self.d.frames {
            for t in f.prefix.iter().chain(&f.suffix) {
                self.template_exprs(f.meta.pos, "sysex-frame", t);
                for name in template_names(t) {
                    if self.d.constant(&name).is_none() && !captured.contains(&name.as_str()) {
                        self.error(
                            f.meta.pos,
                            format!(
                                "sysex-frame `{}` uses `{name}`, which is neither a `const` nor a transport `capture`",
                                f.name
                            ),
                        );
                    }
                }
            }
        }
        for m in &self.d.messages {
            if let Some(t) = &m.transport {
                self.transport_ref(m.meta.pos, t);
            }
            if let Some(send) = &m.send {
                self.template_exprs(m.meta.pos, "send", send);
            }
            let sent = m.send.as_ref().map(template_names).unwrap_or_default();
            for a in &m.args {
                if !sent.contains(&a.name) {
                    self.error(
                        a.meta.pos,
                        format!("`arg \"{}\"` does not appear in `send`", a.name),
                    );
                }
                if let Some(block) = &a.index {
                    match self.d.block(block) {
                        None => self.error(a.meta.pos, format!("unknown block `{block}`")),
                        Some(b) if b.count.is_none() => self.error(
                            a.meta.pos,
                            format!("`index=\"{block}\"`: the block has no `count`"),
                        ),
                        Some(_) => {}
                    }
                }
            }
            for reply in &m.replies {
                self.template_exprs(reply.meta.pos, "reply", &reply.pattern);
                if let Some(when) = &reply.ok_when {
                    self.expr(reply.meta.pos, "ok-when", when);
                }
            }
            if let Some(when) = &m.ok_when {
                self.expr(m.meta.pos, "ok-when", when);
            }
            let frame = match &m.frame {
                Some(name) => {
                    let f = self.d.frames.iter().find(|f| &f.name == name);
                    if f.is_none() {
                        self.error(m.meta.pos, format!("unknown sysex-frame `{name}`"));
                    }
                    f
                }
                None => None,
            };
            let kind = if frame.is_some() {
                Some(TransportKind::Midi)
            } else {
                self.message_kind(m)
            };
            let templates = m
                .send
                .iter()
                .map(|t| (m.meta.pos, "send", t))
                .chain(m.replies.iter().map(|r| (r.meta.pos, "reply", &r.pattern)));
            for (pos, what, body) in templates {
                match kind {
                    Some(TransportKind::Midi) => self.midi_bytes(pos, &m.name, what, frame, body),
                    Some(TransportKind::Hid) => {
                        let frame_len = self.hid_frame(m);
                        self.hid_bytes(pos, &m.name, what, frame_len, body);
                    }
                    _ => {}
                }
            }
        }
    }

    fn hid_frame(&self, m: &Message) -> Option<u32> {
        self.d.transport(m.transport.as_deref()?)?.frame
    }

    /// SysEx data bytes are 7-bit: only the first F0 and the last F7 may
    /// have the top bit set.
    fn midi_bytes(
        &mut self,
        pos: Pos,
        name: &str,
        what: &str,
        frame: Option<&SysexFrame>,
        body: &Template,
    ) {
        let prefix = frame.and_then(|f| f.prefix.as_ref());
        let suffix = frame.and_then(|f| f.suffix.as_ref());
        let pieces: Vec<&Piece> = prefix
            .into_iter()
            .chain([body])
            .chain(suffix)
            .flat_map(|t| &t.pieces)
            .collect();
        let last = pieces.len() - 1;
        for (i, piece) in pieces.iter().enumerate() {
            if let Piece::Byte { value } = piece {
                let edge = (i == 0 && *value == 0xF0) || (i == last && *value == 0xF7);
                if *value > 0x7F && !edge {
                    self.error(
                        pos,
                        format!(
                            "message `{name}` {what}: byte {value:02X} inside SysEx; data bytes are 0..=7F"
                        ),
                    );
                }
            }
        }
    }

    /// HID frames are fixed size: a send must fit, a reply must fill it.
    fn hid_bytes(&mut self, pos: Pos, name: &str, what: &str, frame: Option<u32>, t: &Template) {
        let (Some(frame), Some(len)) = (frame, template_len(t, 1)) else {
            return;
        };
        let bad = match what {
            "send" => len > frame,
            _ => len != frame,
        };
        if bad {
            self.error(
                pos,
                format!("message `{name}` {what}: {len} bytes, the HID frame is {frame}"),
            );
        }
    }

    // ---- procedures and the write policy ----------------------------------

    fn procedures(&mut self) {
        for p in &self.d.procedures {
            if let Some(t) = &p.transport {
                self.transport_ref(p.meta.pos, t);
            }
            let what = format!("procedure `{}`", p.name);
            let runnable = p.meta.status != Some(Status::Unknown);
            self.steps(&what, p.kind, runnable, &p.steps);
            if runnable {
                self.policy(&what, p.meta.pos, p.kind, &p.steps);
            }
        }
    }

    fn steps(&mut self, what: &str, kind: Option<Access>, runnable: bool, steps: &'d [Step]) {
        for step in steps {
            match step {
                Step::Send(s) => self.message_step(what, kind, runnable, s, false),
                Step::Exchange(s) => self.message_step(what, kind, runnable, s, true),
                Step::Repeat { steps, .. } => self.steps(what, kind, runnable, steps),
                Step::ReadInto { meta, target, .. } => self.path(meta.pos, target),
                Step::Verify {
                    meta,
                    procedure,
                    compare,
                } => {
                    match self.d.procedure(procedure) {
                        None => self.error(meta.pos, format!("unknown procedure `{procedure}`")),
                        Some(p) if matches!(p.kind, Some(Access::Write | Access::Commit)) => self
                            .error(
                                meta.pos,
                                format!("verify runs `{procedure}`, which is not a read"),
                            ),
                        Some(_) => {}
                    }
                    for path in compare {
                        self.path(meta.pos, path);
                    }
                }
                Step::RestorePoint { meta, paths } => {
                    for path in paths {
                        self.path(meta.pos, path);
                    }
                }
                Step::Call { meta, procedure } => {
                    if self.d.procedure(procedure).is_none() {
                        self.error(meta.pos, format!("unknown procedure `{procedure}`"));
                    }
                }
                Step::Driver { meta, driver, .. } => self.driver_ref(meta.pos, driver),
                Step::Require { meta, expr } => self.expr(meta.pos, "require", expr),
                Step::OkWhen { meta, expr } => self.expr(meta.pos, "ok-when", expr),
                Step::Delay { .. } | Step::Gate { .. } | Step::ExpectIdentity { .. } => {}
            }
        }
    }

    fn message_step(
        &mut self,
        what: &str,
        kind: Option<Access>,
        runnable: bool,
        s: &MessageStep,
        exchange: bool,
    ) {
        for path in s.into.iter().chain(&s.from) {
            self.path(s.meta.pos, path);
        }
        for arg in &s.args {
            if let Value::String(v) = &arg.value {
                if let Some(inner) = v.strip_prefix('{').and_then(|v| v.strip_suffix('}')) {
                    self.expr(s.meta.pos, &format!("argument `{}`", arg.name), inner);
                }
            }
        }
        let Some(m) = self.d.message(&s.message) else {
            self.error(s.meta.pos, format!("unknown message `{}`", s.message));
            return;
        };
        // §6: a message without a reply is send-only. An `unknown` message
        // is a placeholder whose shape is not known yet.
        let placeholder = m.meta.status == Some(Status::Unknown);
        match (exchange, m.replies.is_empty() && !placeholder) {
            (true, true) => self.error(
                s.meta.pos,
                format!(
                    "`exchange` waits for a reply, but message `{}` has none; use `send`",
                    m.name
                ),
            ),
            (false, false) if !m.replies.is_empty() => self.push(
                Severity::Warning,
                s.meta.pos,
                format!(
                    "message `{}` has a reply that `send` ignores; use `exchange` to wait for it",
                    m.name
                ),
            ),
            _ => {}
        }
        if s.into.is_some() && m.replies.is_empty() {
            self.error(
                s.meta.pos,
                format!("`into=` needs a reply, and `{}` has none", m.name),
            );
        }
        if rank(m.kind) > rank(kind) {
            self.error(
                s.meta.pos,
                format!(
                    "{what} is a {} but sends `{}`, a {}",
                    access_name(kind),
                    m.name,
                    access_name(m.kind)
                ),
            );
        }
        if !runnable {
            return;
        }
        // §4.1: what a runnable node sends must be known; writes must be
        // measured.
        let frame = m
            .frame
            .as_ref()
            .and_then(|f| self.d.frames.iter().find(|x| &x.name == f));
        // (what, status, position, is it data rather than bytes on record)
        let mut parts = vec![(
            format!("message `{}`", m.name),
            m.meta.status,
            m.meta.pos,
            false,
        )];
        if let Some(f) = frame {
            parts.push((
                format!("sysex-frame `{}`", f.name),
                f.meta.status,
                f.meta.pos,
                false,
            ));
        }
        if let Some(path) = &s.from {
            parts.push((
                format!("`{path}`"),
                self.path_status(path),
                s.meta.pos,
                true,
            ));
        }
        for (name, status, pos, is_data) in parts {
            match status {
                None if !is_data => {
                    self.error(pos, format!("{name} is sent by {what} but has no `status`"))
                }
                Some(Status::Unknown) => self.error(
                    s.meta.pos,
                    format!("{what} sends {name}, which is unknown; measure it or mark {what} unknown"),
                ),
                Some(Status::Decoded)
                    if !is_data && matches!(kind, Some(Access::Write | Access::Commit)) =>
                {
                    self.error(
                        s.meta.pos,
                        format!("{what} writes with {name}, which is decoded only; writes need measured bytes"),
                    )
                }
                Some(Status::Inferred) => self.push(
                    Severity::Note,
                    s.meta.pos,
                    format!("{what} sends {name}, which is inferred: it needs the user's opt-in"),
                ),
                _ => {}
            }
        }
    }

    /// §11: a write needs a gate and a restore point before it sends; a
    /// commit needs the arm gate.
    fn policy(&mut self, what: &str, pos: Pos, kind: Option<Access>, steps: &[Step]) {
        let (mut gate, mut restore) = (None, false);
        let first_send = flatten(steps).into_iter().find(|s| {
            match s {
                // arm is the stronger gate; keep it once seen
                Step::Gate { gate: g, .. } if gate != Some(Gate::Arm) => gate = Some(*g),
                Step::RestorePoint { .. } => restore = true,
                _ => {}
            }
            matches!(s, Step::Send(_) | Step::Exchange(_) | Step::Driver { .. })
        });
        if first_send.is_none() {
            return;
        }
        match kind {
            Some(Access::Write) => {
                if gate.is_none() {
                    self.error(pos, format!("{what} writes without a `gate` before it"));
                }
                if !restore {
                    self.error(
                        pos,
                        format!("{what} writes without a `restore-point` before it"),
                    );
                }
            }
            Some(Access::Commit) if gate != Some(Gate::Arm) => self.error(
                pos,
                format!("{what} commits without `gate \"arm\"` before it"),
            ),
            _ => {}
        }
    }

    // ---- the rest ---------------------------------------------------------

    fn firmware(&mut self) {
        let Some(f) = &self.d.firmware else {
            return;
        };
        let runnable = f.meta.status != Some(Status::Unknown);
        for r in &f.requires {
            self.expr(f.meta.pos, "require", r);
        }
        if let Some(image) = &f.image {
            if let Some(driver) = &image.driver {
                self.driver_ref(image.meta.pos, driver);
            }
        }
        if let Some(v) = &f.version_from {
            if self.d.message(&v.message).is_none() {
                self.error(v.meta.pos, format!("unknown message `{}`", v.message));
            }
        }
        for stage in &f.stages {
            let what = format!("firmware stage `{}`", stage.name);
            let kind = stage.kind.or(Some(Access::Commit));
            self.steps(&what, kind, runnable, &stage.steps);
        }
        if runnable {
            if f.recovery.is_none() {
                self.error(f.meta.pos, "firmware needs `recovery` text (§12)");
            }
            if f.image.is_none() {
                self.error(f.meta.pos, "firmware needs an `image` (§12)");
            }
        }
    }

    fn identify(&mut self) {
        for m in &self.d.identify {
            if let Matcher::Exchange {
                meta,
                message,
                ok_when,
            } = m
            {
                match self.d.message(message) {
                    None => self.error(meta.pos, format!("unknown message `{message}`")),
                    Some(m) if m.replies.is_empty() => self.error(
                        meta.pos,
                        format!("identify exchanges `{message}`, which has no reply"),
                    ),
                    Some(_) => {}
                }
                if let Some(when) = ok_when {
                    self.expr(meta.pos, "ok-when", when);
                }
            }
        }
    }

    fn panel(&mut self) {
        fn walk<'d>(v: &mut V<'d>, items: &'d [PanelItem]) {
            for item in items {
                match item {
                    PanelItem::Control(c) => {
                        if let Some(bind) = &c.bind {
                            v.path(c.meta.pos, bind);
                        }
                    }
                    PanelItem::Repeat { items, .. } => walk(v, items),
                }
            }
        }
        if let Some(panel) = &self.d.panel {
            walk(self, &panel.items);
        }
    }

    fn midi_maps(&mut self) {
        for map in &self.d.midi_maps {
            if let Some(t) = &map.transport {
                self.transport_ref(map.meta.pos, t);
            }
        }
    }

    fn lbmt(&mut self) {
        for l in &self.d.lbmt {
            for end in l.from.iter().chain(&l.to) {
                if !end.starts_with("virtual:") {
                    self.transport_ref(l.meta.pos, end);
                }
            }
        }
    }

    fn driver_ref(&mut self, pos: Pos, name: &str) {
        if self.d.driver(name).is_none() {
            self.error(
                pos,
                format!("driver `{name}` has no `driver` node; declare it (§14)"),
            );
        }
    }

    fn drivers(&mut self) {
        for driver in &self.d.drivers {
            for p in &driver.provides {
                let pos = driver.meta.pos;
                match p.split_once(':') {
                    Some(("procedure", name)) => {
                        if self.d.procedure(name).is_none() {
                            self.error(pos, format!("provides unknown procedure `{name}`"));
                        }
                    }
                    Some(("block", path)) => match Path::parse(path) {
                        Ok(path) => self.path(pos, &path),
                        Err(e) => self.error(pos, e),
                    },
                    Some(("file", _)) => {}
                    _ => self.error(
                        pos,
                        format!("`{p}`: provides takes procedure:, block: or file: entries"),
                    ),
                }
            }
            for owned in &driver.owns {
                if let Some(bytes) = &owned.bytes {
                    self.template_exprs(owned.meta.pos, "bytes", bytes);
                }
                if owned.meta.status.is_none() {
                    self.error(
                        owned.meta.pos,
                        format!("`owns \"{}\"` needs a `status`", owned.name),
                    );
                }
                if let Some(frame) = &owned.frame {
                    if !self.d.frames.iter().any(|f| &f.name == frame) {
                        self.error(owned.meta.pos, format!("unknown sysex-frame `{frame}`"));
                    }
                }
            }
        }
    }

    fn test_vectors(&mut self) {
        for t in &self.d.test_vectors {
            match (&t.message, &t.block) {
                (Some(m), None) => {
                    if self.d.message(m).is_none() {
                        self.error(t.meta.pos, format!("unknown message `{m}`"));
                    }
                }
                (None, Some(b)) => {
                    if self.d.block(b).is_none() {
                        self.error(t.meta.pos, format!("unknown block `{b}`"));
                    }
                }
                _ => self.error(
                    t.meta.pos,
                    format!(
                        "test-vector `{}` names exactly one `message=` or `block=`",
                        t.name
                    ),
                ),
            }
        }
    }

    /// Notes for `ctrl-cli check`: what is disabled, and what needs opt-in.
    fn unknown_and_inferred(&mut self) {
        let d = self.d;
        let mut list: Vec<(String, &Meta)> = Vec::new();
        list.extend(
            d.messages
                .iter()
                .map(|m| (format!("message `{}`", m.name), &m.meta)),
        );
        list.extend(
            d.procedures
                .iter()
                .map(|p| (format!("procedure `{}`", p.name), &p.meta)),
        );
        list.extend(
            d.blocks
                .iter()
                .map(|b| (format!("block `{}`", b.name), &b.meta)),
        );
        for b in &d.blocks {
            collect_items(&b.name, &b.items, &mut list);
        }
        list.extend(d.firmware.iter().map(|f| ("firmware".to_string(), &f.meta)));
        for (name, meta) in list {
            match meta.status {
                Some(Status::Unknown) => self.push(
                    Severity::Note,
                    meta.pos,
                    format!("{name} is unknown: disabled until measured"),
                ),
                Some(Status::Inferred) => self.push(
                    Severity::Note,
                    meta.pos,
                    format!("{name} is inferred: shown marked, sent only after opt-in"),
                ),
                _ => {}
            }
        }
    }
}

/// The names a template reads: fields, slices and expression names.
fn template_names(t: &Template) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for piece in &t.pieces {
        let names = match piece {
            Piece::Field { name, .. } | Piece::Slice { name, .. } | Piece::Rest { name } => {
                vec![name.clone()]
            }
            Piece::Expr { expr } => Expr::parse(expr)
                .map(|e| e.names().into_iter().map(String::from).collect())
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        for n in names {
            if !out.contains(&n) {
                out.push(n);
            }
        }
    }
    out
}

fn collect_items<'d>(parent: &str, items: &'d [Item], out: &mut Vec<(String, &'d Meta)>) {
    for item in items {
        let (name, meta) = item_name(item);
        let full = format!("{parent}.{name}");
        out.push((format!("`{full}`"), meta));
        match item {
            Item::Group(g) => collect_items(&full, &g.items, out),
            Item::Array(a) => collect_items(&full, &a.items, out),
            Item::Field(_) => {}
        }
    }
}

fn item_name(item: &Item) -> (&str, &Meta) {
    match item {
        Item::Field(f) => (&f.name, &f.meta),
        Item::Group(g) => (&g.name, &g.meta),
        Item::Array(a) => (&a.name, &a.meta),
    }
}

fn input(t: &Transport) -> Option<&str> {
    match t.ports.as_ref()? {
        PortMatch::Both(s) => Some(s),
        PortMatch::Split { input, .. } => Some(input),
    }
}

fn output(t: &Transport) -> Option<&str> {
    match t.ports.as_ref()? {
        PortMatch::Both(s) => Some(s),
        PortMatch::Split { output, .. } => Some(output),
    }
}

/// Steps in order, `repeat` bodies inlined.
fn flatten(steps: &[Step]) -> Vec<&Step> {
    let mut out = Vec::new();
    for s in steps {
        out.push(s);
        if let Step::Repeat { steps, .. } = s {
            out.extend(flatten(steps));
        }
    }
    out
}

fn rank(kind: Option<Access>) -> u8 {
    match kind {
        None | Some(Access::Read) => 0,
        Some(Access::Write) => 1,
        Some(Access::Commit) => 2,
    }
}

fn access_name(kind: Option<Access>) -> &'static str {
    match kind {
        None | Some(Access::Read) => "read",
        Some(Access::Write) => "write",
        Some(Access::Commit) => "commit",
    }
}

/// `unknown` < `inferred` < `decoded` < `measured`; no status counts as
/// nothing known about this level, so the other side wins.
fn weaker(a: Option<Status>, b: Option<Status>) -> Option<Status> {
    fn order(s: Status) -> u8 {
        match s {
            Status::Unknown => 0,
            Status::Inferred => 1,
            Status::Decoded => 2,
            Status::Measured => 3,
        }
    }
    match (a, b) {
        (Some(a), Some(b)) => Some(if order(a) <= order(b) { a } else { b }),
        (a, None) => a,
        (None, b) => b,
    }
}

pub(crate) fn storage_width(s: Storage) -> u32 {
    match s {
        Storage::U7 | Storage::U8 | Storage::S7 | Storage::Bool | Storage::Bits(_) => 1,
        Storage::U14le | Storage::U14be | Storage::U16le | Storage::U16be => 2,
        Storage::Ascii(n) => u32::from(n),
    }
}

fn storage_range(s: Storage) -> Option<IntRange> {
    let (lo, hi) = match s {
        Storage::U7 => (0, 127),
        Storage::U8 => (0, 255),
        Storage::U14le | Storage::U14be => (0, 16383),
        Storage::S7 => (-64, 63),
        Storage::U16le | Storage::U16be => (0, 65535),
        Storage::Bool => (0, 1),
        Storage::Bits(n) => (0, (1i64 << n) - 1),
        Storage::Ascii(_) => return None,
    };
    Some(IntRange { lo, hi })
}

/// The byte length of a template, when every piece has a fixed width.
/// `default` is the width of a placeholder without an encoding.
fn template_len(t: &Template, default: u32) -> Option<u32> {
    let width = |e: &Option<Encoding>| match e {
        None => Some(default),
        Some(Encoding::Storage(s)) => Some(storage_width(*s)),
        Some(_) => None,
    };
    t.pieces.iter().try_fold(0, |n, piece| {
        let w = match piece {
            Piece::Byte { .. } | Piece::Any | Piece::Expr { .. } => 1,
            Piece::Field { encoding, .. } => width(encoding)?,
            Piece::Slice {
                count, encoding, ..
            } => u32::from(*count) * width(encoding)?,
            Piece::Checksum(c) if c.algorithm == ChecksumAlgorithm::Fnv1a32 => 4,
            Piece::Checksum(_) => 1,
            Piece::Length { encoding } => width(&Some(encoding.clone()))?,
            Piece::Rest { .. } => return None,
        };
        Some(n + w)
    })
}
