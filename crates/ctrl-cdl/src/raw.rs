//! KDL documents as plain nodes with resolved positions, and `include`
//! merging (CDL §2).

use std::io;
use std::path::{Path as FsPath, PathBuf};

use kdl::{KdlDocument, KdlNode, KdlValue};

use crate::{Error, Pos, Value};

/// Includes nest at most this deep; it also stops cycles that the
/// spelling of a path hides (`a/../a.kdl`).
const MAX_INCLUDE_DEPTH: usize = 16;

#[derive(Debug, Clone)]
pub(crate) struct RawValue {
    pub value: Value,
    pub pos: Pos,
}

#[derive(Debug, Clone)]
pub(crate) struct RawNode {
    pub name: String,
    pub pos: Pos,
    pub args: Vec<RawValue>,
    pub props: Vec<(String, RawValue)>,
    pub children: Vec<RawNode>,
}

impl RawNode {
    pub fn prop(&self, key: &str) -> Option<&RawValue> {
        self.props.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// The merge key. Named kinds key by kind and name, `value` by its
    /// number; every other kind (`send`, `vendor`, `types`, …) is a slot
    /// keyed by kind alone.
    fn key(&self) -> (&str, Option<String>) {
        let first = self.args.first().map(|a| &a.value);
        let name = match (NAMED.contains(&self.name.as_str()), first) {
            (true, Some(Value::String(s))) => Some(s.clone()),
            (_, Some(Value::Int(i))) if self.name == "value" => Some(i.to_string()),
            _ => None,
        };
        (&self.name, name)
    }
}

/// Node kinds whose first argument is a name.
const NAMED: &[&str] = &[
    "device",
    "transport",
    "capture",
    "enum",
    "int",
    "errors",
    "block",
    "field",
    "group",
    "array",
    "param",
    "control",
    "source",
    "sink",
    "sysex-frame",
    "message",
    "procedure",
    "stage",
    "lbmt",
    "driver",
    "owns",
    "test-vector",
    "const",
    "data",
];

/// Kinds whose children are an ordered step list; a local list replaces the
/// included one as a whole.
const STEP_LISTS: &[&str] = &["procedure", "stage"];

pub(crate) fn error(files: &[String], pos: Pos, message: impl Into<String>) -> Error {
    Error {
        file: files.get(pos.file as usize).cloned().unwrap_or_default(),
        line: pos.line,
        col: pos.col,
        message: message.into(),
    }
}

pub(crate) type Read<'r> = &'r mut dyn FnMut(&FsPath) -> io::Result<String>;

pub(crate) struct Loader<'r> {
    read: Read<'r>,
    pub files: Vec<String>,
    pub errors: Vec<Error>,
    stack: Vec<PathBuf>,
}

impl<'r> Loader<'r> {
    pub fn new(read: Read<'r>) -> Self {
        Self {
            read,
            files: Vec::new(),
            errors: Vec::new(),
            stack: Vec::new(),
        }
    }

    fn error_at(&mut self, at: Option<Pos>, path: &FsPath, message: String) {
        let error = match at {
            Some(pos) => error(&self.files, pos, message),
            None => Error {
                file: path.display().to_string(),
                line: 1,
                col: 1,
                message,
            },
        };
        self.errors.push(error);
    }

    /// Loads `path` and its includes into one merged node list. `at` is the
    /// `include` node that asked for it.
    pub fn load(&mut self, path: &FsPath, at: Option<Pos>) -> Vec<RawNode> {
        if self.stack.iter().any(|p| p == path) || self.stack.len() >= MAX_INCLUDE_DEPTH {
            let chain: Vec<_> = self
                .stack
                .iter()
                .chain(std::iter::once(&path.to_path_buf()))
                .map(|p| p.display().to_string())
                .collect();
            self.error_at(at, path, format!("include cycle: {}", chain.join(" -> ")));
            return Vec::new();
        }
        let text = match (self.read)(path) {
            Ok(text) => text,
            Err(e) => {
                self.error_at(at, path, format!("cannot read {}: {e}", path.display()));
                return Vec::new();
            }
        };
        let file = self.files.len() as u32;
        self.files.push(path.display().to_string());
        let lines = Lines::new(&text);
        let doc = match text.parse::<KdlDocument>() {
            Ok(doc) => doc,
            Err(e) => {
                for d in e.diagnostics {
                    let mut message = d.message.unwrap_or_else(|| "KDL syntax error".into());
                    if let Some(help) = d.help {
                        message = format!("{message} ({help})");
                    }
                    let pos = lines.pos(file, d.span.offset());
                    self.errors.push(error(&self.files, pos, message));
                }
                return Vec::new();
            }
        };
        let nodes: Vec<RawNode> = doc
            .nodes()
            .iter()
            .map(|n| self.node(&lines, file, n))
            .collect();

        self.stack.push(path.to_path_buf());
        let mut out = Vec::new();
        for node in nodes {
            if node.name != "include" {
                merge_into(&mut out, node);
                continue;
            }
            let target = match (
                &node.args[..],
                node.props.is_empty(),
                node.children.is_empty(),
            ) {
                (
                    [RawValue {
                        value: Value::String(rel),
                        ..
                    }],
                    true,
                    true,
                ) => path.parent().unwrap_or(FsPath::new("")).join(rel),
                _ => {
                    self.errors.push(error(
                        &self.files,
                        node.pos,
                        "`include` takes one path, relative to this file",
                    ));
                    continue;
                }
            };
            for included in self.load(&target, Some(node.pos)) {
                if included.name != "cdl" {
                    merge_into(&mut out, included);
                }
            }
        }
        self.stack.pop();
        out
    }

    fn node(&mut self, lines: &Lines, file: u32, n: &KdlNode) -> RawNode {
        let pos = lines.pos(file, n.span().offset());
        if n.ty().is_some() {
            self.errors.push(error(
                &self.files,
                pos,
                "type annotations are not used in CDL 0.1",
            ));
        }
        let mut args = Vec::new();
        let mut props: Vec<(String, RawValue)> = Vec::new();
        for entry in n.entries() {
            let pos = lines.pos(file, entry.span().offset());
            if entry.ty().is_some() {
                self.errors.push(error(
                    &self.files,
                    pos,
                    "type annotations are not used in CDL 0.1",
                ));
            }
            let value = match entry.value() {
                KdlValue::String(s) => Value::String(s.clone()),
                KdlValue::Integer(i) => match i64::try_from(*i) {
                    Ok(i) => Value::Int(i),
                    Err(_) => {
                        self.errors
                            .push(error(&self.files, pos, format!("{i} is out of range")));
                        Value::Null
                    }
                },
                KdlValue::Float(f) => Value::Float(*f),
                KdlValue::Bool(b) => Value::Bool(*b),
                KdlValue::Null => Value::Null,
            };
            let value = RawValue { value, pos };
            match entry.name() {
                None => args.push(value),
                // KDL: the rightmost property with a name wins.
                Some(key) => match props.iter_mut().find(|(k, _)| k == key.value()) {
                    Some(slot) => slot.1 = value,
                    None => props.push((key.value().to_string(), value)),
                },
            }
        }
        let children = n
            .children()
            .map(|doc| {
                doc.nodes()
                    .iter()
                    .map(|c| self.node(lines, file, c))
                    .collect()
            })
            .unwrap_or_default();
        RawNode {
            name: n.name().value().to_string(),
            pos,
            args,
            props,
            children,
        }
    }
}

/// Adds `node` to `out`. A node from another file with the same kind and
/// name merges: the later one's arguments and properties win, and a child
/// with the same kind and name replaces the earlier child. Duplicates within
/// one file stay separate, so that conversion reports them.
fn merge_into(out: &mut Vec<RawNode>, node: RawNode) {
    let existing = out
        .iter_mut()
        .find(|e| e.pos.file != node.pos.file && e.key() == node.key());
    let Some(existing) = existing else {
        out.push(node);
        return;
    };
    existing.pos = node.pos;
    existing.args = node.args;
    for (key, value) in node.props {
        match existing.props.iter_mut().find(|(k, _)| *k == key) {
            Some(slot) => slot.1 = value,
            None => existing.props.push((key, value)),
        }
    }
    if STEP_LISTS.contains(&node.name.as_str()) && !node.children.is_empty() {
        existing.children = node.children;
        return;
    }
    for child in node.children {
        match existing
            .children
            .iter_mut()
            .find(|c| c.key() == child.key())
        {
            Some(slot) => *slot = child,
            None => existing.children.push(child),
        }
    }
}

/// Byte offset to line and column.
struct Lines<'t> {
    text: &'t str,
    starts: Vec<usize>,
}

impl<'t> Lines<'t> {
    fn new(text: &'t str) -> Self {
        let starts = std::iter::once(0)
            .chain(text.match_indices('\n').map(|(i, _)| i + 1))
            .collect();
        Self { text, starts }
    }

    fn pos(&self, file: u32, offset: usize) -> Pos {
        let line = self.starts.partition_point(|&s| s <= offset).max(1);
        let start = self.starts[line - 1];
        let col = self
            .text
            .get(start..offset.min(self.text.len()))
            .map_or(offset - start, |s| s.chars().count());
        Pos {
            file,
            line: line as u32,
            col: col as u32 + 1,
        }
    }
}
