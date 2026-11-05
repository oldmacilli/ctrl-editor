//! Export the current panel baseline as CDL includes for the shared editor.
//! Usage: cargo run -p korg-app --features panel-baseline --example export_panels -- /path/to/descriptors/korg

use korg_app::panel::{baseline_panel, Binding, ControlKind, PANEL_CLASSES};
use std::{env, error::Error, fs, path::Path};

fn quote(s: &str) -> String {
    serde_json::to_string(s).expect("strings serialize")
}

fn main() -> Result<(), Box<dyn Error>> {
    let destination = env::args_os()
        .nth(1)
        .ok_or("expected descriptor directory")?;
    let root = Path::new(&destination);
    for class in PANEL_CLASSES {
        let mut candidates = Vec::new();
        for entry in fs::read_dir(root)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.ends_with(".gen.inc.kdl") {
                continue;
            }
            let data = fs::read_to_string(entry.path())?;
            if data
                .lines()
                .any(|line| line.trim() == format!("class {class:?}"))
            {
                candidates.push(name.trim_end_matches(".gen.inc.kdl").to_string());
            }
        }
        if candidates.len() != 1 {
            return Err(format!(
                "{class}: expected exactly one generated descriptor, found {candidates:?}"
            )
            .into());
        }
        let p = baseline_panel(class).ok_or("missing panel")?;
        let mut text = format!(
            "// Generated from the KORG panel baseline; do not edit by hand.\ncdl \"0.1\"\npanel picture=\"{}x{}\" binding-inferred=#{} source=\"KORG editor panel geometry; positions estimated by eye\" status=\"inferred\" evidence=\"Panel geometry estimated by eye\" {{\n",
            p.width, p.height, p.binding_inferred,
        );
        for (i, c) in p.controls.iter().enumerate() {
            let (kind, black) = match c.kind {
                ControlKind::Knob => ("knob", false),
                ControlKind::Fader => ("fader", false),
                ControlKind::Button => ("button", false),
                ControlKind::Pad => ("pad", false),
                ControlKind::Key { black } => ("key", black),
                ControlKind::TouchPad => ("touchpad", false),
                ControlKind::Jog => ("jog", false),
            };
            let binding = match c.binding {
                Binding::None => String::new(),
                Binding::Element(index) => format!(" element-index={index}"),
                Binding::Jog => " jog-binding=#true".to_owned(),
            };
            let key = if black { " key-black=#true" } else { "" };
            text.push_str(&format!(
                "    control \"control-{i}\" kind=\"{kind}\" label={} group={} box=\"{},{},{},{}\"{key}{binding} status=\"inferred\" evidence=\"Control geometry estimated by eye\"\n",
                quote(&c.label), quote(&c.group), c.x, c.y, c.w, c.h,
            ));
        }
        text.push_str("}\n");
        fs::write(root.join(format!("{}.panel.inc.kdl", candidates[0])), text)?;
    }
    Ok(())
}
