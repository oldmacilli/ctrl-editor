//! Owned panel data converted from the shared CDL IR.
//!
//! This adapter is opt-in until the UI can consume owned group and class
//! names and the remaining descriptor data has passed the parity gates.

use crate::panel::{Binding, Control, ControlKind, Panel};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq)]
pub struct OwnedControl {
    pub group: String,
    pub label: String,
    pub kind: ControlKind,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub binding: Binding,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OwnedPanel {
    pub class: String,
    pub width: f64,
    pub height: f64,
    pub binding_inferred: bool,
    pub controls: Vec<OwnedControl>,
}

impl From<OwnedPanel> for Panel {
    fn from(panel: OwnedPanel) -> Self {
        Self {
            class: panel.class,
            width: panel.width,
            height: panel.height,
            binding_inferred: panel.binding_inferred,
            controls: panel
                .controls
                .into_iter()
                .map(|control| Control {
                    group: control.group,
                    label: control.label,
                    kind: control.kind,
                    x: control.x,
                    y: control.y,
                    w: control.w,
                    h: control.h,
                    binding: control.binding,
                })
                .collect(),
        }
    }
}

#[derive(Debug)]
pub struct PanelError(String);

impl std::fmt::Display for PanelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PanelError {}

/// Load one KORG panel from an explicit external descriptor root.
pub fn load_from(root: &std::path::Path, class: &str) -> Result<Panel, PanelError> {
    let mut descriptors = korg_devices::external::load_korg_descriptors(root)
        .map_err(|e| PanelError(e.to_string()))?;
    let descriptor = descriptors
        .remove(class)
        .ok_or_else(|| PanelError(format!("{class}: no external descriptor")))?;
    from_cdl(&descriptor).map(Panel::from)
}

/// Load all panels from one descriptor root, failing atomically on invalid data.
pub fn load_all_from(
    root: &std::path::Path,
) -> Result<std::collections::BTreeMap<String, Panel>, PanelError> {
    let descriptors = korg_devices::external::load_korg_descriptors(root)
        .map_err(|e| PanelError(e.to_string()))?;
    descriptors
        .into_iter()
        .map(|(class, descriptor)| {
            let panel = from_cdl(&descriptor).map(Panel::from)?;
            Ok((class, panel))
        })
        .collect()
}

#[derive(Deserialize)]
struct Input {
    device: Device,
    panel: Option<SourcePanel>,
}

#[derive(Deserialize)]
struct Device {
    class: Option<String>,
}

#[derive(Deserialize)]
struct SourcePanel {
    picture: Option<Size>,
    binding_inferred: bool,
    items: Vec<Item>,
}

#[derive(Deserialize)]
struct Size {
    width: u32,
    height: u32,
}

#[derive(Deserialize)]
#[serde(tag = "item", rename_all = "kebab-case")]
enum Item {
    Control {
        kind: String,
        group: Option<String>,
        label: Option<String>,
        bbox: Option<[Coord; 4]>,
        key_black: bool,
        element_index: Option<u32>,
        jog_binding: bool,
    },
    Repeat,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Coord {
    Num(f64),
    Expr(String),
}

/// Convert a serialized `ctrl_cdl::Descriptor` into an owned panel. This
/// rejects unsupported or incomplete input instead of silently dropping it.
pub fn from_cdl(descriptor: &impl Serialize) -> Result<OwnedPanel, PanelError> {
    let value = serde_json::to_value(descriptor).map_err(|e| PanelError(e.to_string()))?;
    let input: Input = serde_json::from_value(value).map_err(|e| PanelError(e.to_string()))?;
    let class = input
        .device
        .class
        .filter(|c| !c.is_empty())
        .ok_or_else(|| PanelError("missing class".into()))?;
    let panel = input
        .panel
        .ok_or_else(|| PanelError(format!("{class}: missing panel")))?;
    let size = panel
        .picture
        .ok_or_else(|| PanelError(format!("{class}: missing panel size")))?;
    if size.width == 0 || size.height == 0 {
        return Err(PanelError(format!("{class}: invalid panel size")));
    }
    let mut controls = Vec::with_capacity(panel.items.len());
    for (index, item) in panel.items.into_iter().enumerate() {
        let Item::Control {
            kind,
            group,
            label,
            bbox,
            key_black,
            element_index,
            jog_binding,
        } = item
        else {
            return Err(PanelError(format!(
                "{class} control {index}: repeat is not expanded"
            )));
        };
        let invalid = |message: &str| PanelError(format!("{class} control {index}: {message}"));
        let kind = match kind.as_str() {
            "knob" => ControlKind::Knob,
            "fader" => ControlKind::Fader,
            "button" => ControlKind::Button,
            "pad" => ControlKind::Pad,
            "key" => ControlKind::Key { black: key_black },
            "touchpad" => ControlKind::TouchPad,
            "jog" => ControlKind::Jog,
            _ => return Err(invalid("unsupported control kind")),
        };
        if key_black && !matches!(kind, ControlKind::Key { .. }) {
            return Err(invalid("key colour on a non-key control"));
        }
        let binding = match (element_index, jog_binding) {
            (None, false) => Binding::None,
            (Some(n), false) => Binding::Element(n as usize),
            (None, true) => Binding::Jog,
            (Some(_), true) => return Err(invalid("conflicting element and jog binding")),
        };
        let coords = bbox.ok_or_else(|| invalid("missing control box"))?;
        let mut box_values = [0.0; 4];
        for (dst, coord) in box_values.iter_mut().zip(coords) {
            *dst = match coord {
                Coord::Num(n) if n.is_finite() => n,
                Coord::Expr(expr) => return Err(invalid(&format!("unexpanded coordinate {expr}"))),
                _ => return Err(invalid("non-finite coordinate")),
            };
        }
        if box_values[2] <= 0.0 || box_values[3] <= 0.0 {
            return Err(invalid("nonpositive control size"));
        }
        controls.push(OwnedControl {
            group: group.ok_or_else(|| invalid("missing group"))?,
            label: label.ok_or_else(|| invalid("missing label"))?,
            kind,
            x: box_values[0],
            y: box_values[1],
            w: box_values[2],
            h: box_values[3],
            binding,
        });
    }
    Ok(OwnedPanel {
        class,
        width: f64::from(size.width),
        height: f64::from(size.height),
        binding_inferred: panel.binding_inferred,
        controls,
    })
}
