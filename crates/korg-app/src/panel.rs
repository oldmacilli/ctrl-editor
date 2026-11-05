//! Device panels: the controls of each device and where they sit.
//!
//! Front ends draw these as a schematic and use them for hit-testing, like
//! the device picture in the original editor. Coordinates are pixels of the
//! editor's own panel picture for that device (embedded PNGs, see
//! `docs/rewrite/71-panels.md`), measured by eye; y grows downward. The
//! editor's exact click areas live in code and are not decoded yet, so a
//! front end should hit-test with a tolerance (`Panel::hit`).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlKind {
    Knob,
    Fader,
    Button,
    Pad,
    /// A keyboard key; `black` keys sit in the upper row.
    Key {
        black: bool,
    },
    TouchPad,
    Jog,
}

/// What a control edits in the document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Binding {
    /// Not decoded yet: the control has no parameter rows.
    None,
    /// The parameter rows of this schema element.
    Element(usize),
    /// The jog-wheel block (`ViewModel::jog`).
    Jog,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Control {
    /// Group name, as the editor's control list names it (for example
    /// "Knob", "Track Mute", "Jog Wheel").
    pub group: String,
    pub label: String,
    pub kind: ControlKind,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub binding: Binding,
}

impl Control {
    pub fn center(&self) -> (f64, f64) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Panel {
    pub class: String,
    pub width: f64,
    pub height: f64,
    pub controls: Vec<Control>,
    /// The binding order is an inference, not decoded (`docs/rewrite/71-panels.md`).
    pub binding_inferred: bool,
}

impl Panel {
    /// The control at `(x, y)`. A control counts as hit when the point lies
    /// within its box grown by `tolerance` on each side; of several, the one
    /// whose center is closest wins.
    pub fn hit(&self, x: f64, y: f64, tolerance: (f64, f64)) -> Option<usize> {
        let (tx, ty) = tolerance;
        self.controls
            .iter()
            .enumerate()
            .filter(|(_, c)| {
                x >= c.x - tx && x <= c.x + c.w + tx && y >= c.y - ty && y <= c.y + c.h + ty
            })
            .map(|(index, c)| {
                let (cx, cy) = c.center();
                (index, (cx - x).powi(2) + (cy - y).powi(2))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(index, _)| index)
    }

    /// The first control bound to `binding`.
    pub fn find(&self, binding: Binding) -> Option<usize> {
        self.controls.iter().position(|c| c.binding == binding)
    }

    /// The nearest control from `from` in direction `(dx, dy)` (each -1, 0
    /// or 1). Distance across the direction counts three times.
    pub fn neighbor(&self, from: usize, dx: f64, dy: f64) -> Option<usize> {
        let (fx, fy) = self.controls.get(from)?.center();
        self.controls
            .iter()
            .enumerate()
            .filter(|&(index, _)| index != from)
            .filter_map(|(index, c)| {
                let (cx, cy) = c.center();
                let (vx, vy) = (cx - fx, cy - fy);
                let along = vx * dx + vy * dy;
                let across = (vx * dy - vy * dx).abs();
                (along > 0.5).then_some((index, along + 3.0 * across))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(index, _)| index)
    }

    /// Controls grouped by `group`, in the order each group first appears.
    pub fn groups(&self) -> Vec<(&str, Vec<usize>)> {
        let mut groups: Vec<(&str, Vec<usize>)> = Vec::new();
        for (index, control) in self.controls.iter().enumerate() {
            match groups.iter_mut().find(|(name, _)| *name == control.group) {
                Some((_, members)) => members.push(index),
                None => groups.push((control.group.as_str(), vec![index])),
            }
        }
        groups
    }

    /// The control list as an accordion: one header per group of two or
    /// more controls, with the members of `open` below it.  A group of one
    /// is a plain row.  Both front ends draw these rows.
    pub fn tree(&self, open: Option<&str>, selected: Option<usize>) -> Vec<TreeRow> {
        let mut rows = Vec::new();
        for (name, members) in self.groups() {
            if let [index] = members[..] {
                rows.push(TreeRow::Control {
                    index,
                    label: self.controls[index].label.clone(),
                    nested: false,
                    selected: selected == Some(index),
                });
                continue;
            }
            let is_open = open == Some(name);
            rows.push(TreeRow::Group {
                name: name.to_owned(),
                count: members.len(),
                open: is_open,
                selected: selected.is_some_and(|index| members.contains(&index)),
            });
            if is_open {
                rows.extend(members.into_iter().map(|index| TreeRow::Control {
                    index,
                    label: member_label(name, &self.controls[index].label),
                    nested: true,
                    selected: selected == Some(index),
                }));
            }
        }
        rows
    }

    /// `tree` for drawing: the open group's members become chips, short
    /// labels side by side, packed into lines of at most `line_chars`
    /// characters (a chip takes its label plus one space).
    pub fn tree_lines(
        &self,
        open: Option<&str>,
        selected: Option<usize>,
        line_chars: usize,
    ) -> Vec<TreeLine> {
        let mut lines = Vec::new();
        let mut chips: Vec<Chip> = Vec::new();
        let mut width = 0;
        let flush = |chips: &mut Vec<Chip>, lines: &mut Vec<TreeLine>| {
            if !chips.is_empty() {
                lines.push(TreeLine::Chips(std::mem::take(chips)));
            }
        };
        for row in self.tree(open, selected) {
            match row {
                TreeRow::Control {
                    index,
                    nested: true,
                    selected,
                    ..
                } => {
                    let label = crate::assign::short_label(&self.controls[index].label);
                    let chip_width = label.chars().count() + 1;
                    if width + chip_width > line_chars.max(chip_width) {
                        flush(&mut chips, &mut lines);
                        width = 0;
                    }
                    width += chip_width;
                    chips.push(Chip {
                        index,
                        label,
                        selected,
                    });
                }
                row => {
                    flush(&mut chips, &mut lines);
                    width = 0;
                    lines.push(TreeLine::Row(row));
                }
            }
        }
        flush(&mut chips, &mut lines);
        lines
    }
}

/// One line of `Panel::tree_lines`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeLine {
    /// A group header or a single control.
    Row(TreeRow),
    /// Members of the open group, side by side.
    Chips(Vec<Chip>),
}

/// An open group's member, named by its number where it has one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chip {
    pub index: usize,
    pub label: String,
    pub selected: bool,
}

/// One row of `Panel::tree`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeRow {
    Group {
        name: String,
        count: usize,
        open: bool,
        /// The selected control is in this group.
        selected: bool,
    },
    Control {
        index: usize,
        label: String,
        /// Shown under an open group header.
        nested: bool,
        selected: bool,
    },
}

/// A member's label under its group header, without the repeated group
/// name: "Knob 3" under "Knob" is "3", "Marker ◀" under "Transport" stays.
pub fn member_label(group: &str, label: &str) -> String {
    label
        .strip_prefix(group)
        .map(str::trim_start)
        .filter(|rest| !rest.is_empty())
        .unwrap_or(label)
        .to_owned()
}

/// The group to open after a header click: the same header closes it.
pub fn toggle_group(open: Option<&str>, clicked: &str) -> Option<String> {
    (open != Some(clicked)).then(|| clicked.to_owned())
}

/// Load and validate all installed panel descriptors once per application run.
pub fn validate_external_panels() -> Result<(), String> {
    external_panels().map(|_| ())
}

fn validate_panel_classes<'a>(
    classes: impl IntoIterator<Item = &'a str>,
    panels: &std::collections::BTreeMap<String, Panel>,
) -> Result<(), String> {
    for class in classes {
        if !panels.contains_key(class) {
            return Err(format!("missing panel descriptor for {class}"));
        }
    }
    Ok(())
}

fn external_panels() -> Result<&'static std::collections::BTreeMap<String, Panel>, String> {
    static PANELS: std::sync::OnceLock<Result<std::collections::BTreeMap<String, Panel>, String>> =
        std::sync::OnceLock::new();
    PANELS
        .get_or_init(|| {
            let root = korg_devices::external::descriptor_root().map_err(|e| e.to_string())?;
            let panels = crate::panel_external::load_all_from(&root).map_err(|e| e.to_string())?;
            let catalog = korg_devices::Catalog::load().map_err(|e| e.to_string())?;
            validate_panel_classes(catalog.devices.iter().map(|d| d.class.as_str()), &panels)?;
            Ok(panels)
        })
        .as_ref()
        .map_err(Clone::clone)
}

/// The panel of `class`, from externally installed CDL descriptors.
pub fn panel(class: &str) -> Option<Panel> {
    external_panels().ok()?.get(class).cloned()
}

#[cfg(feature = "panel-baseline")]
#[path = "panel_baseline.rs"]
mod baseline;
#[cfg(feature = "panel-baseline")]
pub use baseline::baseline_panel;

/// Classes that have a panel: every device of `analysis/products/catalog.json`.
#[cfg(any(test, feature = "panel-baseline"))]
pub const PANEL_CLASSES: [&str; 16] = [
    "X10820", "X10830", "X10840", "X11810", "X14800", "X14810", "X14802", "X14823", "X11850",
    "X12880", "X20870", "X20880", "X09800", "X19210", "X21212", "X21210",
];

/// Size in pixels of each device's panel picture, read from the extracted
/// PNGs (`_ANALYZE_/OUT/korg-device-assets`, see `docs/rewrite/71-panels.md`).
#[cfg(test)]
pub const PICTURE_SIZES: [(&str, f64, f64); 16] = [
    ("X10820", 705.0, 185.0),
    ("X10830", 705.0, 190.0),
    ("X10840", 780.0, 210.0),
    ("X11810", 598.0, 211.0),
    ("X14800", 500.0, 294.0),
    ("X14810", 705.0, 183.0),
    ("X14802", 550.0, 329.0),
    ("X14823", 598.0, 211.0),
    ("X11850", 892.0, 197.0),
    ("X12880", 892.0, 197.0),
    ("X20870", 800.0, 200.0),
    ("X20880", 700.0, 183.0),
    ("X09800", 695.0, 185.0),
    ("X19210", 3498.0, 1225.0),
    ("X21212", 130.0, 82.0),
    ("X21210", 96.0, 90.0),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_inventory_comes_from_the_external_catalog() {
        let classes = ["X10830", "X99999"];
        let mut panels = std::collections::BTreeMap::new();
        panels.insert("X10830".to_owned(), panel("X10830").unwrap());
        let error = validate_panel_classes(classes, &panels).unwrap_err();
        assert!(error.contains("X99999"), "{error}");
        panels.insert("X99999".to_owned(), panel("X10830").unwrap());
        assert!(validate_panel_classes(classes, &panels).is_ok());
    }

    fn headers(rows: &[TreeRow]) -> Vec<String> {
        rows.iter()
            .map(|row| match row {
                TreeRow::Group {
                    name, count, open, ..
                } => format!("{}{name} ({count})", if *open { "▾ " } else { "▸ " }),
                TreeRow::Control { label, nested, .. } => {
                    format!("{}{label}", if *nested { "  " } else { "" })
                }
            })
            .collect()
    }

    #[test]
    fn the_studio_list_folds_to_nine_rows() {
        let panel = panel("X14802").unwrap();
        let rows = panel.tree(None, None);
        assert_eq!(
            headers(&rows),
            [
                "Scene",
                "▸ Transport (12)",
                "Jog Wheel",
                "▸ Track Mute (8)",
                "▸ Track Solo (8)",
                "▸ Track Rec (8)",
                "▸ Track Select (8)",
                "▸ Knob (8)",
                "▸ Slider (8)",
            ]
        );
    }

    #[test]
    fn an_open_group_lists_its_members_without_the_group_name() {
        let panel = panel("X14802").unwrap();
        let knob = panel
            .controls
            .iter()
            .position(|c| c.label == "Knob 3")
            .unwrap();
        let rows = panel.tree(Some("Knob"), Some(knob));
        let text = headers(&rows);
        let at = text.iter().position(|row| row == "▾ Knob (8)").unwrap();
        assert_eq!(text[at + 1..at + 4], ["  1", "  2", "  3"]);
        assert!(rows.iter().any(|row| matches!(
            row,
            TreeRow::Group {
                name,
                selected: true,
                ..
            } if name == "Knob"
        )));
        assert!(rows.contains(&TreeRow::Control {
            index: knob,
            label: "3".into(),
            nested: true,
            selected: true,
        }));
        assert_eq!(rows.len(), 9 + 8);
    }

    #[test]
    fn an_open_group_packs_its_members_into_chip_lines() {
        let panel = panel("X14802").unwrap();
        let lines = panel.tree_lines(Some("Track Mute"), None, 24);
        let chips: Vec<Vec<String>> = lines
            .iter()
            .filter_map(|line| match line {
                TreeLine::Chips(chips) => {
                    Some(chips.iter().map(|chip| chip.label.clone()).collect())
                }
                TreeLine::Row(_) => None,
            })
            .collect();
        assert_eq!(chips, [["1", "2", "3", "4", "5", "6", "7", "8"]]);
        assert_eq!(lines.len(), 10);
        let transport = panel.tree_lines(Some("Transport"), None, 24);
        let widest = transport
            .iter()
            .filter_map(|line| match line {
                TreeLine::Chips(chips) => Some(
                    chips
                        .iter()
                        .map(|c| c.label.chars().count() + 1)
                        .sum::<usize>(),
                ),
                TreeLine::Row(_) => None,
            })
            .max()
            .unwrap();
        assert!(widest <= 24, "{widest}");
    }

    #[test]
    fn member_labels_and_toggling() {
        assert_eq!(member_label("Knob", "Knob 3"), "3");
        assert_eq!(member_label("Transport", "Marker ◀"), "Marker ◀");
        assert_eq!(member_label("Knob", "Knob"), "Knob");
        assert_eq!(toggle_group(None, "Knob"), Some("Knob".into()));
        assert_eq!(toggle_group(Some("Knob"), "Knob"), None);
        assert_eq!(toggle_group(Some("Knob"), "Slider"), Some("Slider".into()));
    }

    #[test]
    fn every_panel_fits_its_picture() {
        for class in PANEL_CLASSES {
            let panel = panel(class).unwrap();
            assert_eq!(panel.class, class);
            for c in &panel.controls {
                assert!(
                    c.x >= 0.0
                        && c.y >= 0.0
                        && c.x + c.w <= panel.width
                        && c.y + c.h <= panel.height,
                    "{class} {} lies outside the panel",
                    c.label
                );
            }
        }
    }

    #[test]
    fn every_catalog_class_has_a_panel_with_its_picture() {
        assert_eq!(PANEL_CLASSES.len(), PICTURE_SIZES.len());
        for (class, width, height) in PICTURE_SIZES {
            let panel = panel(class).unwrap_or_else(|| panic!("{class} has no panel"));
            assert_eq!(panel.class, class);
            assert_eq!(
                (panel.width, panel.height),
                (width, height),
                "{class} does not match its picture size"
            );
            assert!(!panel.controls.is_empty(), "{class} has no controls");
        }
        for class in PANEL_CLASSES {
            assert!(
                PICTURE_SIZES.iter().any(|(name, _, _)| *name == class),
                "{class} has no picture size"
            );
        }
    }

    #[test]
    fn nanokontrol_studio_has_eight_strips_and_a_jog() {
        let panel = panel("X14802").unwrap();
        let count = |group| panel.controls.iter().filter(|c| c.group == group).count();
        assert_eq!(count("Knob"), 8);
        assert_eq!(count("Slider"), 8);
        assert_eq!(count("Track Select"), 8);
        let jog = panel.find(Binding::Jog).unwrap();
        assert_eq!(panel.hit(82.0, 245.0, (0.0, 0.0)), Some(jog));
    }

    #[test]
    fn hit_prefers_the_closest_center_and_respects_tolerance() {
        let panel = panel("X14802").unwrap();
        let mute1 = panel
            .controls
            .iter()
            .position(|c| c.label == "Mute 1")
            .unwrap();
        assert_eq!(panel.hit(180.0, 59.0, (0.0, 0.0)), Some(mute1));
        // Between two buttons: no hit without tolerance, the nearer with it.
        assert_eq!(panel.hit(180.0, 68.0, (0.0, 0.0)), None);
        assert_eq!(panel.hit(180.0, 66.0, (4.0, 4.0)), Some(mute1));
    }

    #[test]
    fn neighbor_walks_the_strip() {
        let panel = panel("X14802").unwrap();
        let find = |label: &str| {
            panel
                .controls
                .iter()
                .position(|c| c.label == label)
                .unwrap()
        };
        assert_eq!(
            panel.neighbor(find("Mute 1"), 1.0, 0.0),
            Some(find("Mute 2"))
        );
        assert_eq!(
            panel.neighbor(find("Mute 1"), 0.0, 1.0),
            Some(find("Solo 1"))
        );
        assert_eq!(
            panel.neighbor(find("Knob 1"), 0.0, 1.0),
            Some(find("Slider 1"))
        );
    }

    #[test]
    fn nanopad2_pads_bind_elements_in_reading_order() {
        let panel = panel("X10830").unwrap();
        assert!(!panel.binding_inferred);
        for index in 0..16 {
            let control = &panel.controls[panel.find(Binding::Element(index)).unwrap()];
            assert_eq!(control.label, format!("Pad {}", index + 1));
        }
    }

    #[test]
    fn nanokey2_binds_every_control_to_its_only_element() {
        let panel = panel("X10820").unwrap();
        assert_eq!(panel.controls.len(), 7);
        let mut labels: Vec<&str> = panel
            .controls
            .iter()
            .map(|control| control.label.as_str())
            .collect();
        labels.sort();
        assert_eq!(
            labels,
            [
                "Keyboard",
                "Mod Button",
                "Octave Down",
                "Octave Up",
                "Pitch Button",
                "Pitch Up",
                "Sustain Button"
            ]
        );
        assert!(panel
            .controls
            .iter()
            .all(|c| c.binding == Binding::Element(0)));
    }

    #[test]
    fn nanokey_studio_controls_stay_inside_the_picture() {
        let panel = panel("X14800").unwrap();
        for control in &panel.controls {
            assert!(control.x >= 0.0 && control.y >= 0.0, "{}", control.label);
            assert!(
                control.x + control.w <= panel.width + 0.001
                    && control.y + control.h <= panel.height + 0.001,
                "{}: {:?}",
                control.label,
                (control.x, control.y, control.w, control.h)
            );
        }
        let sustain = panel
            .controls
            .iter()
            .find(|control| control.label == "Sustain")
            .unwrap();
        assert_eq!(
            (sustain.x, sustain.y, sustain.w, sustain.h),
            (68.0, 129.0, 21.0, 11.0)
        );
    }
}
