//! Guides (CDL §17): what the user does on the device, as lines of text.
//!
//! [`Guide`] turns guide steps into [`Line`]s: keys by their printed
//! labels, numbers padded and spaced, recipes and `when`s expanded.
//! [`edit_guide`] composes the steps that put changed fields into a stored
//! element by hand, from the descriptor's `edit` rules.

use std::collections::HashMap;

use ctrl_cdl::{
    Act, Descriptor, Edit, EditField, EditItem, Expr, GuideStep, Index, Path, TypeDef, Value,
};

use crate::EngineError;

/// One instruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    /// What to do: "Press RECALL", "Type 0 3".
    pub text: String,
    /// What the device shows afterwards.
    pub shows: Option<String>,
    /// Why, or what to watch for.
    pub hint: Option<String>,
    /// Set when the user reads a value off the device and enters it: the
    /// path it goes to.
    pub ask: Option<String>,
}

impl Line {
    fn say(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            shows: None,
            hint: None,
            ask: None,
        }
    }
}

/// Lines as numbered plain text, for a terminal or a file.
pub fn to_text(lines: &[Line]) -> String {
    let mut s = String::new();
    for (k, l) in lines.iter().enumerate() {
        s.push_str(&format!("{:2}. {}", k + 1, l.text));
        if !l.text.ends_with(['.', '?', '!', ':']) {
            s.push('.');
        }
        if let Some(shows) = &l.shows {
            s.push_str(&format!(" The display shows {shows}."));
        }
        s.push('\n');
        if let Some(h) = &l.hint {
            s.push_str(&format!("    {h}\n"));
        }
    }
    s
}

/// Variables a guide's expressions read.
pub type Vars = HashMap<String, i64>;

/// Renders guide steps; remembers which entries it has explained.
pub struct Guide<'d> {
    d: &'d Descriptor,
    /// Entries whose text was shown already.
    pub(crate) explained: Vec<String>,
    pub lines: Vec<Line>,
}

/// Recipes may run recipes; deeper than this is a loop.
const MAX_DEPTH: usize = 16;

impl<'d> Guide<'d> {
    pub fn new(d: &'d Descriptor) -> Self {
        Self {
            d,
            explained: Vec::new(),
            lines: Vec::new(),
        }
    }

    pub fn steps(&mut self, steps: &[GuideStep], vars: &Vars) -> Result<(), EngineError> {
        self.steps_at(steps, vars, 0)
    }

    fn steps_at(
        &mut self,
        steps: &[GuideStep],
        vars: &Vars,
        depth: usize,
    ) -> Result<(), EngineError> {
        for g in steps {
            self.step(g, vars, depth)?;
        }
        Ok(())
    }

    fn step(&mut self, g: &GuideStep, vars: &Vars, depth: usize) -> Result<(), EngineError> {
        match &g.act {
            Act::When { expr, steps } => {
                if eval(expr, vars)? != 0 {
                    self.steps_at(steps, vars, depth)?;
                }
                return Ok(());
            }
            Act::Do { recipe, args } => {
                if depth >= MAX_DEPTH {
                    return Err(EngineError::Check(format!("recipe `{recipe}` runs itself")));
                }
                let r = self
                    .d
                    .recipe(recipe)
                    .ok_or_else(|| EngineError::Check(format!("unknown recipe `{recipe}`")))?;
                let mut inner = vars.clone();
                for a in args {
                    inner.insert(a.name.clone(), value(&a.value, vars)?);
                }
                let first = self.lines.len();
                self.steps_at(&r.steps, &inner, depth + 1)?;
                // `do … shows=… hint=…` apply to the recipe's last line.
                if self.lines.len() > first {
                    let last = self.lines.last_mut().expect("a line");
                    if let Some(s) = &g.shows {
                        last.shows = Some(text(s, vars));
                    }
                    if let Some(h) = &g.hint {
                        last.hint = Some(text(h, vars));
                    }
                }
                return Ok(());
            }
            Act::Type { entry: Some(e), .. } if !self.explained.contains(e) => {
                self.explained.push(e.clone());
                let note = self
                    .d
                    .hands
                    .as_ref()
                    .and_then(|h| h.entries.iter().find(|x| &x.name == e))
                    .and_then(|x| x.text.clone());
                if let Some(n) = note {
                    self.lines.push(Line::say(text(&n, vars)));
                }
            }
            _ => {}
        }
        let line = Line {
            text: self.sentence(g, vars)?,
            shows: g.shows.as_deref().map(|s| text(s, vars)),
            hint: g.hint.as_deref().map(|s| text(s, vars)),
            ask: match &g.act {
                Act::Ask { path, .. } => Some(resolve_path(path, vars)?),
                _ => None,
            },
        };
        self.lines.push(line);
        Ok(())
    }

    fn label(&self, key: &str, vars: &Vars) -> String {
        let id = text(key, vars);
        self.d
            .hands
            .as_ref()
            .and_then(|h| h.key(&id))
            .map_or(id, |k| k.label.clone())
    }

    /// The act as one sentence, without the trailing full stop.
    fn sentence(&self, g: &GuideStep, vars: &Vars) -> Result<String, EngineError> {
        let label = |k: &str| self.label(k, vars);
        let held = g.with.iter().map(|k| label(k)).collect::<Vec<_>>();
        let (verb, rest) = match &g.act {
            Act::Press { key, times } => (
                "press",
                format!(
                    "{}{}",
                    label(key),
                    match times {
                        1 => String::new(),
                        2 => " twice".into(),
                        n => format!(" {n} times"),
                    }
                ),
            ),
            Act::Hold { key, ms, until } => (
                "hold",
                match (until, ms) {
                    (Some(u), _) => {
                        format!("{} until the display shows {}", label(key), text(u, vars))
                    }
                    (None, Some(ms)) => format!("{} for {}", label(key), seconds(*ms)),
                    (None, None) => label(key),
                },
            ),
            Act::Chord { keys } => (
                "press",
                format!(
                    "{} together",
                    join_and(&keys.iter().map(|k| label(k)).collect::<Vec<_>>())
                ),
            ),
            Act::Type {
                value: v, digits, ..
            } => {
                let n = text(v, vars);
                let n = match (n.parse::<i64>(), digits) {
                    (Ok(i), Some(w)) => format!("{i:0w$}", w = *w as usize),
                    _ => n,
                };
                let spaced: Vec<String> = n.chars().map(String::from).collect();
                ("type", spaced.join(" "))
            }
            Act::Turn { key, to, as_type } => {
                let shown = match as_type {
                    Some(t) => {
                        let v = text(to, vars);
                        let v = v.parse::<i64>().map_err(|_| {
                            EngineError::Check(format!("`turn … to={to}`: not a number"))
                        })?;
                        format!("“{}”", self.value_label(t, v))
                    }
                    None => text(to, vars),
                };
                ("turn", format!("{} to {shown}", label(key)))
            }
            Act::Move { key } => ("move", label(key)),
            Act::Say { text: t } => return Ok(text(t, vars)),
            Act::Ask { path, text: t } => {
                return Ok(match t {
                    Some(t) => text(t, vars),
                    None => format!(
                        "Read {} on the device and enter it",
                        resolve_path(path, vars)?
                    ),
                })
            }
            Act::OneOf { options } => {
                let mut clauses = Vec::new();
                for o in options {
                    // each option in a guide of its own, without entry texts
                    let mut sub = Guide::new(self.d);
                    sub.explained = self
                        .d
                        .hands
                        .iter()
                        .flat_map(|h| h.entries.iter().map(|e| e.name.clone()))
                        .collect();
                    sub.steps(o, vars)?;
                    let parts: Vec<String> =
                        sub.lines.iter().map(|l| lower_first(&l.text)).collect();
                    clauses.push(parts.join(", then "));
                }
                return Ok(upper_first(&clauses.join(", or ")));
            }
            Act::When { .. } | Act::Do { .. } => unreachable!("expanded in `step`"),
        };
        Ok(if held.is_empty() {
            upper_first(&format!("{verb} {rest}"))
        } else {
            format!("Hold {} and {verb} {rest}", join_and(&held))
        })
    }

    /// An enum value's label, an int type's display, else the number.
    fn value_label(&self, ty: &str, v: i64) -> String {
        match self.d.types.iter().find(|t| t.name() == ty) {
            Some(TypeDef::Enum(e)) | Some(TypeDef::Errors(e)) => e
                .values
                .iter()
                .find(|x| x.value == v)
                .map_or_else(|| v.to_string(), |x| x.label.clone()),
            Some(TypeDef::Int(t)) => {
                if let Some(s) = t.special.iter().find(|s| s.value == v) {
                    return s.label.clone();
                }
                match &t.display {
                    Some(d) => {
                        let vars = Vars::from([("v".to_string(), v)]);
                        text(d, &vars)
                    }
                    None => v.to_string(),
                }
            }
            None => v.to_string(),
        }
    }
}

fn seconds(ms: u32) -> String {
    if ms % 1000 == 0 {
        format!("{} s", ms / 1000)
    } else {
        format!("{:.1} s", f64::from(ms) / 1000.0)
    }
}

fn join_and(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

fn upper_first(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

/// "Press X" into "press X"; a key label in capitals stays as it is.
fn lower_first(s: &str) -> String {
    let mut c = s.chars();
    match (c.next(), c.next()) {
        (Some(f), Some(n)) if n.is_lowercase() => f
            .to_lowercase()
            .chain(std::iter::once(n))
            .chain(c)
            .collect(),
        _ => s.to_string(),
    }
}

pub(crate) fn eval(expr: &str, vars: &Vars) -> Result<i64, EngineError> {
    Expr::parse(expr)
        .and_then(|e| e.eval(&|n| vars.get(n).copied()))
        .map_err(|e| EngineError::Check(format!("`{expr}`: {e}")))
}

fn text(t: &str, vars: &Vars) -> String {
    ctrl_cdl::text::interpolate(t, &|e| {
        Expr::parse(e).and_then(|x| x.eval(&|n| vars.get(n).copied()))
    })
}

fn value(v: &Value, vars: &Vars) -> Result<i64, EngineError> {
    match v {
        Value::Int(i) => Ok(*i),
        Value::String(s) => {
            let e = s
                .strip_prefix('{')
                .and_then(|s| s.strip_suffix('}'))
                .unwrap_or(s);
            eval(e, vars)
        }
        other => Err(EngineError::Check(format!(
            "argument {other:?} is not a number"
        ))),
    }
}

/// `preset[p].control[c].assign` with p = 2, c = 0: `preset[2].control[0].assign`.
fn resolve_path(path: &Path, vars: &Vars) -> Result<String, EngineError> {
    let mut parts = Vec::new();
    for s in &path.segments {
        parts.push(match &s.index {
            None => s.name.clone(),
            Some(Index::At(e)) => format!("{}[{}]", s.name, eval(e, vars)?),
            Some(Index::Range { .. }) => {
                return Err(EngineError::Unsupported(format!("`{path}`: a range")))
            }
        });
    }
    Ok(parts.join("."))
}

// ---- edit guides -------------------------------------------------------------

/// A field whose stored value should change: `preset[2].control[0].assign`
/// from `have` (None: not known) to `want`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub path: String,
    pub want: i64,
    pub have: Option<i64>,
}

/// A change located in the descriptor's `edit` rules.
struct Placed<'d> {
    change: &'d Change,
    /// The block element's index.
    element: i64,
    /// Which `item` (None: a field of the element itself), its indexes.
    item: Option<(usize, Vec<i64>)>,
    field: usize,
    /// The field's own index, for an array of values.
    at: Option<i64>,
}

fn index_value(path: &str, i: &Option<Index>) -> Result<i64, EngineError> {
    match i {
        Some(Index::At(e)) => eval(e, &Vars::new()),
        _ => Err(EngineError::Check(format!(
            "`{path}`: every array needs an index"
        ))),
    }
}

fn place<'d>(edit: &Edit, c: &'d Change) -> Result<Placed<'d>, EngineError> {
    let path = Path::parse(&c.path).map_err(EngineError::Check)?;
    let segs = &path.segments;
    let element = match &segs[0].index {
        None => 0,
        i => index_value(&c.path, i)?,
    };
    let (arrays, field) = segs[1..].split_at(segs.len().saturating_sub(2));
    let at = match &field[0].index {
        None => None,
        i => Some(index_value(&c.path, i)?),
    };
    let field = &field[0].name;
    let no_rule = || {
        EngineError::Unsupported(format!(
            "`{}` cannot be set by hand: `edit \"{}\"` has no rule for it",
            c.path, edit.block
        ))
    };
    let find = |fields: &[EditField]| fields.iter().position(|f| &f.name == field);
    if arrays.is_empty() {
        let f = find(&edit.fields).ok_or_else(no_rule)?;
        return Ok(Placed {
            change: c,
            element,
            item: None,
            field: f,
            at,
        });
    }
    let names: Vec<&str> = arrays.iter().map(|s| s.name.as_str()).collect();
    let k = edit
        .items
        .iter()
        .position(|it| {
            it.arrays
                .iter()
                .map(String::as_str)
                .eq(names.iter().copied())
        })
        .ok_or_else(no_rule)?;
    let indexes = arrays
        .iter()
        .map(|s| index_value(&c.path, &s.index))
        .collect::<Result<Vec<_>, _>>()?;
    let f = find(&edit.items[k].fields).ok_or_else(no_rule)?;
    Ok(Placed {
        change: c,
        element,
        item: Some((k, indexes)),
        field: f,
        at,
    })
}

/// The steps that put `changes` into one stored element of one block by
/// hand: open the element, select each item, set each field, close, check.
/// `want` gives the wanted value of any field by path, for `applies` and
/// `when` conditions that read the other fields of an item. Empty when
/// nothing changes or nothing applies.
pub fn edit_guide(
    d: &Descriptor,
    changes: &[Change],
    want: &dyn Fn(&str) -> Option<i64>,
) -> Result<Vec<Line>, EngineError> {
    let Some(first) = changes.first() else {
        return Ok(Vec::new());
    };
    let block = Path::parse(&first.path)
        .map_err(EngineError::Check)?
        .segments[0]
        .name
        .clone();
    let edit = d.edit(&block).ok_or_else(|| {
        EngineError::Unsupported(format!(
            "block `{block}` has no `edit`: it cannot be changed by hand"
        ))
    })?;
    let mut placed = changes
        .iter()
        .map(|c| place(edit, c))
        .collect::<Result<Vec<_>, _>>()?;
    let element = placed[0].element;
    if let Some(p) = placed
        .iter()
        .find(|p| p.element != element || !p.change.path.starts_with(&block))
    {
        return Err(EngineError::Check(format!(
            "`{}`: one guide covers one element of one block",
            p.change.path
        )));
    }
    placed.sort_by(|a, b| (&a.item, a.field, a.at).cmp(&(&b.item, b.field, b.at)));
    // Element-level fields first.
    placed.sort_by_key(|p| p.item.is_some());

    let mut vars = Vars::from([(edit.index.clone(), element)]);
    let mut body = Guide::new(d);
    body.steps(&edit.open, &vars)?;
    let opened = body.lines.len();
    let mut current: Option<(usize, Vec<i64>)> = None;
    for p in &placed {
        let (fields, prefix): (&[EditField], String) = match &p.item {
            None => (&edit.fields, format!("{block}[{element}]")),
            Some((k, idx)) => {
                let item: &EditItem = &edit.items[*k];
                for (name, v) in item.index.iter().zip(idx) {
                    vars.insert(name.clone(), *v);
                }
                let mut s = format!("{block}[{element}]");
                for (a, v) in item.arrays.iter().zip(idx) {
                    s.push_str(&format!(".{a}[{v}]"));
                }
                (&item.fields, s)
            }
        };
        let f = &fields[p.field];
        let mut fv = vars.clone();
        // The item's other fields, as wanted.
        for other in fields {
            if let Some(v) = want(&format!("{prefix}.{}", other.name)) {
                fv.insert(other.name.clone(), v);
            }
        }
        if let (Some(name), Some(at)) = (&f.index, p.at) {
            fv.insert(name.clone(), at);
        }
        fv.insert("v".into(), p.change.want);
        if let Some(h) = p.change.have {
            fv.insert("was".into(), h);
        }
        // A field whose `applies` reads a value nobody gave is kept: the
        // user can skip a step that does not apply, not invent a missing one.
        if let Some(a) = &f.applies {
            if eval(a, &fv).is_ok_and(|v| v == 0) {
                continue;
            }
        }
        if p.item != current {
            if let Some((k, _)) = &p.item {
                body.steps(&edit.items[*k].select, &fv)?;
            }
            current = p.item.clone();
        }
        body.steps(&f.steps, &fv)?;
    }
    if body.lines.len() == opened {
        return Ok(Vec::new());
    }
    body.steps(&edit.close, &vars)?;
    body.steps(&edit.check, &vars)?;
    Ok(body.lines)
}
