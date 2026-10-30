//! Message kinds (CDL §17.6): what the editor offers when the user picks
//! what a control should send, and which fields that choice needs.

use ctrl_cdl::{Descriptor, KindDef, Kinds};

use crate::guide::{eval, Vars};
use crate::EngineError;

/// A field of the chosen kind, under the kind's name for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KindField {
    pub field: String,
    pub label: String,
    pub default: Option<i64>,
}

/// The kinds of `array` (`preset.control`) offered at these indexes
/// (block first, as the descriptor's `index` names them).
pub fn offered<'d>(
    d: &'d Descriptor,
    array: &str,
    at: &[i64],
) -> Result<Vec<&'d KindDef>, EngineError> {
    let Some(k) = d.kinds_of(array) else {
        return Ok(Vec::new());
    };
    let vars = vars(k, at);
    let mut out = Vec::new();
    for kind in &k.kinds {
        if holds(&kind.when, &vars)? {
            out.push(kind);
        }
    }
    Ok(out)
}

/// The kind a stored `by` value belongs to, among those offered.
pub fn kind_of<'d>(
    d: &'d Descriptor,
    array: &str,
    at: &[i64],
    value: i64,
) -> Result<Option<&'d KindDef>, EngineError> {
    Ok(offered(d, array, at)?
        .into_iter()
        .find(|k| (k.values.lo..=k.values.hi).contains(&value)))
}

/// The fields `kind` uses at these indexes, the `by` field first when the
/// kind spans several values.
pub fn fields(
    d: &Descriptor,
    array: &str,
    at: &[i64],
    kind: &KindDef,
) -> Result<Vec<KindField>, EngineError> {
    let Some(k) = d.kinds_of(array) else {
        return Ok(Vec::new());
    };
    let vars = vars(k, at);
    let mut out = Vec::new();
    for p in &kind.params {
        if holds(&p.when, &vars)? {
            out.push(KindField {
                field: p.field.clone(),
                label: p.label.clone(),
                default: p.default,
            });
        }
    }
    Ok(out)
}

/// The values to set when the user switches an element to `kind`: the
/// `by` field (kept when already of this kind, else the kind's first
/// value) and each field's default where the element had another kind.
/// `current` reads the element's fields by name.
pub fn switch_to(
    d: &Descriptor,
    array: &str,
    at: &[i64],
    kind: &KindDef,
    current: &dyn Fn(&str) -> Option<i64>,
) -> Result<Vec<(String, i64)>, EngineError> {
    let Some(k) = d.kinds_of(array) else {
        return Ok(Vec::new());
    };
    let range = kind.values.lo..=kind.values.hi;
    let was = current(&k.by);
    let same = was.is_some_and(|v| range.contains(&v));
    let mut out = vec![(
        k.by.clone(),
        if same {
            was.unwrap_or(kind.values.lo)
        } else {
            kind.values.lo
        },
    )];
    if !same {
        for f in fields(d, array, at, kind)? {
            if let Some(v) = f.default {
                if f.field != k.by {
                    out.push((f.field, v));
                }
            }
        }
    }
    Ok(out)
}

fn vars(k: &Kinds, at: &[i64]) -> Vars {
    k.index.iter().cloned().zip(at.iter().copied()).collect()
}

fn holds(when: &Option<String>, vars: &Vars) -> Result<bool, EngineError> {
    match when {
        None => Ok(true),
        Some(w) => Ok(eval(w, vars)? != 0),
    }
}
