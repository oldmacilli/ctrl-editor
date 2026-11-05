//! Regression baseline for replacing hard-coded KORG panels with CDL files.
//!
//! Each fingerprint covers the complete ordered panel: class, dimensions,
//! inference flag, and every control's label, group, kind (including key
//! colour), exact geometry and binding. These values were recorded from the
//! pre-migration implementation. They must not be regenerated to make a
//! migration pass without independently reviewing the change in behavior.

#[cfg(feature = "panel-baseline")]
use korg_app::panel::baseline_panel;
use korg_app::panel::{panel, Binding, ControlKind, Panel};

const PANEL_CLASSES: [&str; 16] = [
    "X10820", "X10830", "X10840", "X11810", "X14800", "X14810", "X14802", "X14823", "X11850",
    "X12880", "X20870", "X20880", "X09800", "X19210", "X21212", "X21210",
];

#[test]
fn external_panel_adapter_rejects_conflicting_bindings() {
    let descriptor = serde_json::json!({"device": {"class": "X10830"}, "panel": {
        "picture": {"width": 100, "height": 100}, "binding_inferred": false,
        "items": [{"item": "control", "kind": "pad", "label": "Pad", "group": "Pad",
            "bbox": [{"num": 1.0}, {"num": 2.0}, {"num": 3.0}, {"num": 4.0}],
            "key_black": false, "element_index": 1, "jog_binding": true}]
    }});
    let error = korg_app::panel_external::from_cdl(&descriptor).unwrap_err();
    assert!(error.to_string().contains("binding"), "{error}");
}

fn put(hash: &mut u64, bytes: &[u8]) {
    // Length prefixes keep adjacent strings unambiguous.
    for byte in (bytes.len() as u64).to_le_bytes().iter().chain(bytes) {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(0x100000001b3);
    }
}

fn fingerprint(panel: &Panel) -> u64 {
    let mut hash = 0xcbf29ce484222325;
    put(&mut hash, panel.class.as_bytes());
    for n in [panel.width, panel.height] {
        put(&mut hash, &n.to_bits().to_le_bytes());
    }
    put(&mut hash, &[u8::from(panel.binding_inferred)]);
    for control in &panel.controls {
        put(&mut hash, control.group.as_bytes());
        put(&mut hash, control.label.as_bytes());
        let kind = match control.kind {
            ControlKind::Knob => 0,
            ControlKind::Fader => 1,
            ControlKind::Button => 2,
            ControlKind::Pad => 3,
            ControlKind::Key { black: false } => 4,
            ControlKind::Key { black: true } => 5,
            ControlKind::TouchPad => 6,
            ControlKind::Jog => 7,
        };
        put(&mut hash, &[kind]);
        for n in [control.x, control.y, control.w, control.h] {
            put(&mut hash, &n.to_bits().to_le_bytes());
        }
        match control.binding {
            Binding::None => put(&mut hash, &[0]),
            Binding::Element(index) => {
                put(&mut hash, &[1]);
                put(&mut hash, &(index as u64).to_le_bytes());
            }
            Binding::Jog => put(&mut hash, &[2]),
        }
    }
    hash
}

#[test]
fn every_device_panel_retains_its_full_geometry_and_bindings() {
    let expected: &[(&str, u64)] = &[
        ("X10820", 16257727328733325522),
        ("X10830", 11593203587723773802),
        ("X10840", 4241081878140305737),
        ("X11810", 2848761455966942660),
        ("X14800", 12686141376422136532),
        ("X14810", 10817252886599873675),
        ("X14802", 17651612223469176278),
        ("X14823", 10741568548861762611),
        ("X11850", 6872697068677680502),
        ("X12880", 12779040549743838904),
        ("X20870", 2415097788652439033),
        ("X20880", 1552302439956882158),
        ("X09800", 14562818055084190085),
        ("X19210", 9100260128099927463),
        ("X21212", 17516997982675172798),
        ("X21210", 14658618343912762479),
    ];
    let actual: Vec<_> = PANEL_CLASSES
        .iter()
        .map(|class| {
            (
                *class,
                fingerprint(&panel(class).expect("missing external panel")),
            )
        })
        .collect();
    assert_eq!(
        actual, expected,
        "panel behavior differs from the recorded baseline"
    );
}

/// Compare the external files directly while the Rust panels still serve as
/// the behavioral baseline. Run against the matching shared parser revision:
/// `KORG_DESCRIPTOR_DIR=/path/to/ctrl-editor/descriptors cargo test -p korg-app --test panel_parity -- --ignored`.
#[test]
#[cfg(feature = "panel-baseline")]
#[ignore = "requires external KORG descriptors and a matching ctrl-cdl revision"]
fn external_cdl_panels_match_the_rust_baseline() {
    let root = std::env::var_os("KORG_DESCRIPTOR_DIR")
        .expect("set KORG_DESCRIPTOR_DIR to the directory containing korg/");
    let descriptors = korg_devices::external::load_korg_descriptors(std::path::Path::new(&root))
        .expect("load external KORG descriptors");
    let external_panels = korg_app::panel_external::load_all_from(std::path::Path::new(&root))
        .expect("load external panels");
    for class in PANEL_CLASSES {
        let baseline = baseline_panel(class).expect("missing Rust baseline");
        assert_eq!(
            external_panels.get(class),
            Some(&baseline),
            "{class}: production panel"
        );
        let descriptor = descriptors.get(class).expect("missing external class");
        let converted = korg_app::panel_external::from_cdl(descriptor)
            .unwrap_or_else(|e| panic!("{class}: adapter error: {e}"));
        assert_eq!(converted.class, baseline.class, "{class}: converted class");
        assert_eq!(
            (converted.width, converted.height),
            (baseline.width, baseline.height),
            "{class}: converted size"
        );
        assert_eq!(
            converted.binding_inferred, baseline.binding_inferred,
            "{class}: converted inference"
        );
        assert_eq!(
            converted.controls.len(),
            baseline.controls.len(),
            "{class}: converted control count"
        );
        for (index, (actual, expected)) in converted
            .controls
            .iter()
            .zip(&baseline.controls)
            .enumerate()
        {
            assert_eq!(
                actual.group, expected.group,
                "{class} control {index}: converted group"
            );
            assert_eq!(
                actual.label, expected.label,
                "{class} control {index}: converted label"
            );
            assert_eq!(
                actual.kind, expected.kind,
                "{class} control {index}: converted kind"
            );
            assert_eq!(
                (actual.x, actual.y, actual.w, actual.h),
                (expected.x, expected.y, expected.w, expected.h),
                "{class} control {index}: converted box"
            );
            assert_eq!(
                actual.binding, expected.binding,
                "{class} control {index}: converted binding"
            );
        }
        let actual = serde_json::to_value(descriptor.panel.as_ref().expect("missing CDL panel"))
            .expect("serialize panel");
        assert_eq!(
            actual["picture"]["width"].as_f64(),
            Some(baseline.width),
            "{class}: width"
        );
        assert_eq!(
            actual["picture"]["height"].as_f64(),
            Some(baseline.height),
            "{class}: height"
        );
        assert_eq!(
            actual["binding_inferred"].as_bool(),
            Some(baseline.binding_inferred),
            "{class}: binding inference"
        );
        let items = actual["items"].as_array().expect("panel items");
        assert_eq!(
            items.len(),
            baseline.controls.len(),
            "{class}: control count"
        );
        for (index, (item, control)) in items.iter().zip(&baseline.controls).enumerate() {
            let location = format!("{class} control {index} ({})", control.label);
            assert_eq!(
                item["item"].as_str(),
                Some("control"),
                "{location}: expanded control"
            );
            assert_eq!(
                item["group"].as_str(),
                Some(control.group.as_str()),
                "{location}: group"
            );
            assert_eq!(
                item["label"].as_str(),
                Some(control.label.as_str()),
                "{location}: label"
            );
            let (kind, key_black) = match control.kind {
                ControlKind::Knob => ("knob", false),
                ControlKind::Fader => ("fader", false),
                ControlKind::Button => ("button", false),
                ControlKind::Pad => ("pad", false),
                ControlKind::Key { black } => ("key", black),
                ControlKind::TouchPad => ("touchpad", false),
                ControlKind::Jog => ("jog", false),
            };
            assert_eq!(item["kind"].as_str(), Some(kind), "{location}: kind");
            assert_eq!(
                item["key_black"].as_bool(),
                Some(key_black),
                "{location}: key colour"
            );
            let bbox = item["bbox"].as_array().expect("control box");
            assert_eq!(bbox.len(), 4, "{location}: box length");
            for (value, expected) in bbox
                .iter()
                .zip([control.x, control.y, control.w, control.h])
            {
                assert_eq!(
                    value["num"].as_f64(),
                    Some(expected),
                    "{location}: box coordinate"
                );
            }
            let element = match control.binding {
                Binding::Element(index) => Some(index as u64),
                _ => None,
            };
            assert_eq!(
                item["element_index"].as_u64(),
                element,
                "{location}: element binding"
            );
            assert_eq!(
                item["jog_binding"].as_bool(),
                Some(control.binding == Binding::Jog),
                "{location}: jog binding"
            );
        }
    }
}
