//! The repository's descriptors load, and bad input is reported at the right
//! place.

use std::collections::HashMap;
use std::io;
use std::path::{Path as FsPath, PathBuf};

use ctrl_cdl::*;

fn descriptor(rel: &str) -> PathBuf {
    FsPath::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../descriptors")
        .join(rel)
}

fn load_ok(rel: &str) -> Descriptor {
    load(descriptor(rel)).unwrap_or_else(|e| panic!("{rel}:\n{e}"))
}

/// Errors from an in-memory descriptor, as `line:col: message`.
fn errors(text: &str) -> Vec<String> {
    match parse_str("t.ctrl.kdl", text) {
        Ok(_) => Vec::new(),
        Err(Errors(errors)) => errors
            .into_iter()
            .map(|e| {
                assert_eq!(e.file, "t.ctrl.kdl");
                format!("{}:{}: {}", e.line, e.col, e.message)
            })
            .collect(),
    }
}

const HEAD: &str = "cdl \"0.1\"\ndevice \"t.x\"\n";

#[test]
fn global_binding_choices_are_explicit_and_unique() {
    let input = format!("{HEAD}global-fields status=\"decoded\" {{ binding \"battery\" offset=2 status=\"decoded\" {{ choice \"Alkaline\" value=0 status=\"decoded\" evidence=\"UI\"; choice \"Ni-MH\" value=1 status=\"inferred\" evidence=\"UI\" }} }}");
    let descriptor = parse_str("test", &input).unwrap();
    let choices = &descriptor.global_fields.unwrap().bindings[0].choices;
    assert_eq!(
        choices
            .iter()
            .map(|c| (c.label.as_str(), c.value))
            .collect::<Vec<_>>(),
        [("Alkaline", 0), ("Ni-MH", 1)]
    );
    assert!(!errors(&format!("{HEAD}global-fields {{ binding \"battery\" offset=2 {{ choice \"A\" value=0; choice \"B\" value=0 }} }}")).is_empty());
    assert!(!errors(&format!(
        "{HEAD}global-fields {{ binding \"battery\" offset=2 {{ choice \"A\" value=256 }} }}"
    ))
    .is_empty());
}

#[test]
fn global_ui_bindings_are_external_and_reject_duplicates() {
    let doc = format!("{HEAD}global-fields status=\"decoded\" {{ binding \"channel\" offset=5; binding \"battery\" offset=7 }}");
    let fields = parse_str("test", &doc).unwrap().global_fields.unwrap();
    assert_eq!(fields.bindings[0].name, "channel");
    assert_eq!(fields.bindings[0].offset, 5);
    assert!(!errors(&format!(
        "{HEAD}global-fields {{ binding \"channel\" offset=1; binding \"channel\" offset=2 }}"
    ))
    .is_empty());
}

#[test]
fn live_routing_metadata_is_external_and_validated() {
    let input = format!("{HEAD}control-assignments global-channel-offset=4 status=\"decoded\" {{ assign group=\"G\" label=\"L\" kind=\"note\" {{ live-number offset=20 count=4 max=127; live-number offset=30 if-offset=3 if-not-value=1 }} }}");
    let descriptor = parse_str("test", &input).unwrap();
    let assignments = descriptor.control_assignments.unwrap();
    assert_eq!(assignments.global_channel_offset, Some(4));
    assert_eq!(assignments.assignments[0].live_numbers.len(), 2);
    assert_eq!(assignments.assignments[0].live_numbers[1].max, 127);
    assert!(!errors(&format!("{HEAD}control-assignments {{ assign group=\"G\" label=\"L\" kind=\"note\" {{ live-number offset=1 max=128 }} }}")).is_empty());
    assert!(!errors(&format!("{HEAD}control-assignments {{ assign group=\"G\" label=\"L\" kind=\"note\" {{ live-number offset=1 count=0 }} }}")).is_empty());
    assert!(!errors(&format!("{HEAD}control-assignments {{ assign group=\"G\" label=\"L\" kind=\"note\" {{ live-number offset=1 status=\"unknown\" }} }}")).is_empty());
}

#[test]
fn control_assignments_reject_ambiguous_and_invalid_mappings() {
    for body in [
        "assign group=\"g\" label=\"x\" kind=\"note\" kind-offset=1",
        "assign group=\"g\" label=\"x\" kind-offset=1 fallback=\"undecoded\" { map 0 kind=\"note\"; map 0 kind=\"no-assign\"; }",
        "assign group=\"g\" label=\"x\" kind=\"bogus\"",
        "assign group=\"g\" label=\"x\" kind=\"note\" number-required=#true",
        "assign group=\"g\" label=\"x\" kind=\"note\" number=300",
        "assign group=\"g\" label=\"x\" kind=\"note\" status=\"unknown\"",
        "assign group=\"g\" label=\"x\" kind-offset=1 fallback=\"undecoded\" { map 0 kind=\"note\" status=\"unknown\"; }",
        "assign group=\"g\" label=\"x\" codec=\"jog-config\" codec-offset=4 kind=\"note\"",
        "assign group=\"g\" label=\"x\" codec=\"bogus\" codec-offset=4",
        "assign group=\"g\" label=\"x\" kind=\"note\" description=\"x\" detail-offset=1 { detail 0 text=\"a\"; detail 0 text=\"b\"; }",
        "assign group=\"g\" label=\"x\" kind=\"note\" description=\"x\" { detail 0 text=\"a\"; }",
    ] {
        assert!(!errors(&format!("{HEAD}control-assignments {{ {body} }}\n")).is_empty(), "accepted {body}");
    }
}

#[test]
fn editor_page_field_retains_status_and_evidence() {
    let descriptor = parse_str("t.kdl", &format!("{HEAD}editor-pages default=\"a\" {{ page \"a\" {{ field \"f\" offset=0 kind=\"number\" min=0 max=127 status=\"unknown\" evidence=\"unconfirmed\" }} }}")).unwrap();
    let field = &descriptor.editor_pages.unwrap().pages[0].fields[0];
    assert_eq!(field.meta.status, Some(Status::Unknown));
    assert_eq!(field.meta.evidence.as_deref(), Some("unconfirmed"));
}

#[test]
fn editor_pages_reject_invalid_conditions_and_dangling_selectors() {
    for content in [
        "editor-pages default=\"missing\" { page \"a\" }",
        "editor-pages default=\"a\" { select \"missing\" group=\"g\"; page \"a\" }",
        "editor-pages default=\"a\" { page \"a\" { field \"f\" offset=0 kind=\"number\" min=0 max=127 enabled-if-offset=9 } }",
        "editor-pages default=\"a\" { page \"a\" { field \"f\" offset=0 kind=\"biased\" min=-12 max=12 bias=0 } }",
        "editor-pages default=\"a\" { page \"a\" { field \"f\" offset=0 kind=\"number\" min=0 max=127 enabled-if-offset=9 enabled-if-value=1 enabled-if-not-value=2 } }",
        "editor-pages default=\"a\" { page \"a\" { field \"f\" offset=0 kind=\"number\" min=0 max=127 enabled-if-value=1 enabled-if-not-value=2 } }",
        "editor-pages default=\"a\" { page \"a\" { field \"f\" offset=0 kind=\"optional-number\" min=0 max=127 note-if-offset=9 } }",
        "editor-pages default=\"a\" { page \"a\" { field \"f\" offset=0 kind=\"optional-number\" min=0 max=128 } }",
        "editor-pages default=\"a\" { select \"a\" label=\"x\"; select \"b\" label=\"x\"; page \"a\"; page \"b\" }",
        "editor-pages default=\"a\" { select \"a\" group=\"g\"; select \"b\" group=\"g\" label=\"x\"; page \"a\"; page \"b\" }",
        "editor-pages default=\"a\" { select \"a\" group=\"g\"; select \"b\" label=\"x\"; page \"a\"; page \"b\" }",
        "editor-pages default=\"a\" { page \"a\" { field \"f\" offset=0 kind=\"text\" width=0 } }",
        "editor-pages default=\"a\" { page \"a\" { field \"f\" offset=0 kind=\"number\" min=0 max=127 hidden-if-offset=2 } }",
        "editor-pages default=\"a\" { page \"a\" { field \"f\" offset=0 kind=\"number\" min=0 max=127 { hide-if-value 3; } } }",
    ] {
        assert!(!errors(&format!("{HEAD}{content}\n")).is_empty(), "accepted {content}");
    }
}

#[test]
fn device_catalog_preserves_order_extensions_and_factory_paths() {
    let descriptor = parse_str(
        "catalog.kdl",
        r#"cdl "0.1"
device "korg.example" { class "X12345"; file-key "1234"; }
catalog order=2 name="Example" localized-name="Example " scenes=2 scene-size=64 {
    extension "example_set" os-type="EXST"
    extension "example_data"
    factory-file "Example/Default.example_set"
}

"#,
    )
    .unwrap();
    let catalog = descriptor.catalog.unwrap();
    assert_eq!(catalog.order, 2);
    assert_eq!(catalog.name, "Example");
    assert_eq!(catalog.localized_name.as_deref(), Some("Example "));
    assert_eq!(catalog.scenes, Some(2));
    assert_eq!(catalog.scene_size, Some(64));
    assert_eq!(catalog.extensions[0].os_type.as_deref(), Some("EXST"));
    assert_eq!(catalog.extensions[1].os_type, None);
    assert_eq!(catalog.factory_files, ["Example/Default.example_set"]);
}

#[test]
fn device_catalog_rejects_missing_required_metadata_and_escaping_paths() {
    let missing = errors(&format!("{HEAD}catalog name=\"Example\"\n"));
    assert!(
        missing.iter().any(|e| e.contains("needs `order=`")),
        "{missing:?}"
    );
    let escape = errors(&format!(
        "{HEAD}catalog order=0 name=\"Example\" {{\n factory-file \"../secret\"\n}}\n"
    ));
    assert!(
        escape.iter().any(|e| e.contains("without `..`")),
        "{escape:?}"
    );
}

#[test]
fn sysex_command_set_preserves_unknown_commands() {
    let d = parse_str(
        "commands.kdl",
        r#"cdl "0.1"
device "korg.example"
sysex-commands model-id=274 length-style="short_or_long" status="decoded" {
    command "scene_data" byte=0 status="decoded"
    command "mode_check" status="unknown"
}

"#,
    )
    .unwrap();
    let set = d.sysex_commands.unwrap();
    assert_eq!(set.model_id, Some(274));
    assert_eq!(set.length_style.as_deref(), Some("short_or_long"));
    assert_eq!(set.commands[0].byte, Some(0));
    assert_eq!(set.commands[1].byte, None);
    assert_eq!(set.commands[1].meta.status, Some(Status::Unknown));
}

#[test]
fn sysex_command_set_rejects_guesses_and_duplicate_names() {
    let text = format!("{HEAD}sysex-commands {{\n command \"a\" status=\"unknown\" byte=1\n command \"a\" status=\"decoded\" byte=128\n}}\n");
    let found = errors(&text).join("\n");
    assert!(found.contains("unknown command must omit"), "{found}");
    assert!(found.contains("duplicate SysEx command"), "{found}");
    assert!(found.contains("7-bit"), "{found}");
}

#[test]
fn parameter_layout_preserves_packed_fields_and_order() {
    let descriptor = parse_str("parameters.kdl", r##"cdl "0.1"
device "korg.example" { class "X12345"; }
parameter-layouts status="decoded" {
    layout "scene" order=0 size=97 elements=16 stride=6 version=1 {
        parameter "AssignType" id=0 offset=0 width=1 shift=5 mask=7 min=0 max=3 bias=0 kind="enum" default=2 default-source="factory" {
            label "No Assign"
            label "Control Change"
        }
        parameter "AssignType" id=0 offset=1 width=1 shift=0 mask=255 min=0 max=127 bias=0 kind="number" default-source="unknown"
    }
}
"##).unwrap();
    let layout = &descriptor.parameter_layouts.unwrap().layouts[0];
    assert_eq!(
        (
            layout.payload.as_str(),
            layout.size,
            layout.elements,
            layout.stride
        ),
        ("scene", 97, 16, 6)
    );
    assert_eq!(layout.parameters.len(), 2);
    assert_eq!(layout.parameters[0].labels, ["No Assign", "Control Change"]);
    assert_eq!(layout.parameters[0].shift, 5);
    assert_eq!(layout.parameters[0].default, Some(2));
    assert_eq!(layout.parameters[1].default, None);
    assert_eq!(layout.parameters[1].key, "AssignType");
}

#[test]
fn parameter_layout_rejects_invalid_kind_range_and_duplicate_payload() {
    let text = format!("{HEAD}parameter-layouts {{\n layout \"scene\" order=0 size=8 elements=1 stride=1 version=1 {{\n parameter \"bad\" id=0 offset=0 width=1 shift=0 mask=255 min=5 max=2 bias=0 kind=\"bad\" default-source=\"unknown\"\n }}\n layout \"scene\" order=1 size=8 elements=1 stride=1 version=1\n}}\n");
    let found = errors(&text).join("\n");
    assert!(found.contains("invalid parameter kind"), "{found}");
    assert!(found.contains("invalid parameter range"), "{found}");
    assert!(found.contains("duplicate parameter layout"), "{found}");
}

#[test]
fn panel_preserves_korg_interaction_metadata() {
    let d = parse_str(
        "panel.kdl",
        r##"cdl "0.1"
device "t.x"
panel picture="100x50" binding-inferred=#true {
    control "a" kind="key" key-black=#true element-index=3 box="1,2,3,4"
    control "b" kind="jog" jog-binding=#true box="5,6,7,8"
}
"##,
    )
    .unwrap();
    let panel = d.panel.unwrap();
    assert!(panel.binding_inferred);
    let PanelItem::Control(key) = &panel.items[0] else {
        panic!()
    };
    assert!(key.key_black);
    assert_eq!(key.element_index, Some(3));
    assert!(!key.jog_binding);
    let PanelItem::Control(jog) = &panel.items[1] else {
        panic!()
    };
    assert!(jog.jog_binding);
    assert_eq!(jog.element_index, None);
}

#[test]
fn panel_repeat_preserves_expression_coordinates() {
    let d = parse_str(
        "panel.kdl",
        r#"cdl "0.1"
device "t.x"
panel picture="550x329" {
    repeat "n" "1..=8" {
        control "knob{n}" kind="knob" box="{135.5 + 44.5 * n},146,28,28"
    }
}
"#,
    )
    .unwrap();
    let panel = d.panel.unwrap();
    let PanelItem::Repeat { range, items, .. } = &panel.items[0] else {
        panic!()
    };
    assert_eq!(*range, IntRange { lo: 1, hi: 8 });
    let PanelItem::Control(knob) = &items[0] else {
        panic!()
    };
    assert_eq!(
        knob.bbox.as_ref().unwrap()[0],
        Coord::Expr("135.5 + 44.5 * n".into())
    );
    assert_eq!(knob.bbox.as_ref().unwrap()[1], Coord::Num(146.0));
}

#[test]
fn every_descriptor_loads() {
    let dir = FsPath::new(env!("CARGO_MANIFEST_DIR")).join("../../descriptors");
    let mut count = 0;
    for vendor in std::fs::read_dir(dir).unwrap() {
        for file in std::fs::read_dir(vendor.unwrap().path()).unwrap() {
            let path = file.unwrap().path();
            if path.to_string_lossy().ends_with(".ctrl.kdl") {
                load(&path).unwrap_or_else(|e| panic!("{}:\n{e}", path.display()));
                count += 1;
            }
        }
    }
    assert!(count >= 5, "found {count} descriptors");
}

#[test]
fn korg_panel_geometry_is_marked_inferred() {
    let dir = FsPath::new(env!("CARGO_MANIFEST_DIR")).join("../../descriptors/korg");
    let mut checked = 0;
    for file in std::fs::read_dir(dir).unwrap() {
        let path = file.unwrap().path();
        if !path.to_string_lossy().ends_with(".ctrl.kdl") {
            continue;
        }
        let d = load(&path).unwrap();
        let Some(panel) = d.panel else { continue };
        assert_eq!(
            panel.meta.status,
            Some(Status::Inferred),
            "{}",
            path.display()
        );
        for item in panel.items {
            let PanelItem::Control(control) = item else {
                panic!("unexpected repeated KORG panel")
            };
            assert_eq!(
                control.meta.status,
                Some(Status::Inferred),
                "{}: {}",
                path.display(),
                control.name
            );
        }
        checked += 1;
    }
    assert_eq!(checked, 16);
}

#[test]
fn db1_messages_and_procedures() {
    let d = load_ok("oceanbeach/db1.ctrl.kdl");
    assert_eq!(d.version, "0.1");
    assert_eq!(d.device.id, "oceanbeach.db1");
    let set = d.message("set-drawbar").unwrap();
    assert_eq!(set.frame.as_deref(), Some("db1"));
    assert_eq!(
        set.send.as_ref().unwrap().to_string(),
        "02 03 {index} {polarity} {cc} {channel}"
    );
    let reply = set.reply().unwrap();
    assert_eq!(reply.timeout, Some(500));
    assert_eq!(set.ok_when.as_deref(), Some("status == 0"));
    assert_eq!(set.meta.status, Some(Status::Measured));

    let read_all = d.procedure("read-all").unwrap();
    assert_eq!(read_all.kind, Some(Access::Read));
    let Step::Repeat {
        var, over, steps, ..
    } = &read_all.steps[1]
    else {
        panic!("{:?}", read_all.steps[1]);
    };
    assert_eq!(var, "i");
    assert_eq!(*over, RepeatOver::Range(IntRange { lo: 0, hi: 8 }));
    let Step::Exchange(get) = &steps[0] else {
        panic!("{steps:?}");
    };
    assert_eq!(get.message, "get-drawbar");
    assert_eq!(get.into.as_ref().unwrap().to_string(), "drawbar[i]");
    assert_eq!(
        get.args,
        vec![Arg {
            name: "index".into(),
            value: Value::String("{i}".into())
        }]
    );

    let write_all = d.procedure("write-all").unwrap();
    assert!(matches!(
        write_all.steps[0],
        Step::Gate {
            gate: Gate::Confirm,
            ..
        }
    ));
    let Step::RestorePoint { paths, .. } = &write_all.steps[1] else {
        panic!();
    };
    assert_eq!(paths.len(), 3);

    let tv = &d.test_vectors[0];
    assert_eq!(
        tv.bytes.as_deref(),
        Some(&[0xF0, 0x09, 0, 0, 0, 0x02, 0x03, 0, 0, 0x0C, 0, 0xF7][..])
    );
    assert_eq!(tv.args.len(), 4);
    assert!(d.lbmt[0].source.contains("@translator"));
    assert!(!d.lbmt[0].enabled);
}

#[test]
fn includes_merge_by_kind_and_name() {
    let d = load_ok("korg/nanokey-studio.ctrl.kdl");
    assert_eq!(d.files.len(), 11);
    assert!(d
        .files
        .iter()
        .any(|path| path.ends_with("nanokey-studio.pages.inc.kdl")));
    assert!(d.files[1].ends_with("common.inc.kdl"));
    assert!(d.files[2].ends_with("studio.inc.kdl"));
    assert!(d.files[3].ends_with("nanokey-studio.gen.inc.kdl"));
    assert!(d.files[4].ends_with("nanokey-studio.panel.inc.kdl"));
    assert!(d.files[5].ends_with("nanokey-studio.catalog.inc.kdl"));
    assert!(d.files[6].ends_with("nanokey-studio.sysex.inc.kdl"));
    assert!(d.files[7].ends_with("nanokey-studio.parameters.inc.kdl"));

    // studio.inc.kdl raises the common frame to measured; local facts win
    // over the generated base.
    assert_eq!(d.frames[0].meta.status, Some(Status::Measured));
    assert_eq!(d.frames[0].prefix.as_ref().unwrap().pieces.len(), 4);
    assert_eq!(
        d.constant("model").unwrap().meta.status,
        Some(Status::Measured)
    );
    assert_eq!(
        d.block("scene").unwrap().meta.status,
        Some(Status::Measured)
    );
    assert_eq!(d.device.images.len(), 2);
    let generic = load_ok("korg/nanokey2.ctrl.kdl");
    assert_eq!(generic.frames[0].meta.status, Some(Status::Decoded));
    assert_eq!(
        generic.transport("midi").unwrap().scan.as_deref(),
        Some("identity")
    );

    // The measured exchanges are CDL now; no protocol driver remains.
    assert!(d.driver("korg-measured").is_none());
    assert_eq!(
        d.message("read-block-request")
            .unwrap()
            .reply()
            .unwrap()
            .pattern_name
            .as_deref(),
        Some("scene-data")
    );
    // The Studio messages are shared (studio.inc.kdl); the length makes one
    // scene-write fit both scene sizes. The model comes from the device.
    let write = d.message("scene-write").unwrap();
    assert_eq!(
        write.send.as_ref().unwrap().to_string(),
        "{len:u21le} 40 {data:packed7}"
    );
    assert_eq!(write.meta.pos.file, 2);
    assert_eq!(d.message("commit-scene").unwrap().meta.pos.file, 2);
    assert_eq!(d.constant("model").unwrap().value, 0x136);
    assert_eq!(d.constant("model").unwrap().meta.pos.file, 0);
    assert_eq!(d.constant("manufacturer").unwrap().value, 0x42);

    // Shared procedures and the frame come from the include.
    assert!(d.procedure("write-global").is_some());
    assert_eq!(d.frames[0].name, "korg-native");

    let usb = d.transport("usb").unwrap();
    assert_eq!(
        usb.ports,
        Some(PortMatch::Split {
            input: "nanoKEY Studio KEYBOARD/CTRL".into(),
            output: "nanoKEY Studio CTRL".into()
        })
    );
    assert!(usb.preferred);
    assert_eq!(usb.captures[0].name, "device-id");
    assert_eq!(
        d.transport("ble").unwrap().ports,
        Some(PortMatch::Both("nanoKEY Studio Bluetooth".into()))
    );
}

#[test]
fn nanokontrol_types_block_and_panel() {
    let d = load_ok("korg/nanokontrol-studio.ctrl.kdl");
    // `types` merges: the shared "channel" and "ack" plus the local enums.
    let names: Vec<_> = d.types.iter().map(TypeDef::name).collect();
    // Generated value lists are labels, not enums: only hand-written types.
    assert_eq!(names, ["channel", "ack", "jog-type", "jog-accel"]);
    let TypeDef::Int(channel) = &d.types[0] else {
        panic!();
    };
    assert_eq!(channel.range, Some(IntRange { lo: 0, hi: 15 }));
    assert_eq!(channel.display.as_deref(), Some("{v + 1}"));

    let scene = d.block("scene").unwrap();
    assert_eq!((scene.size, scene.count), (Some(560), Some(5)));
    let Item::Group(jog) = &scene.items[1] else {
        panic!();
    };
    assert_eq!(jog.offset, Some(512));
    let Item::Field(sign) = &jog.items[3] else {
        panic!();
    };
    assert_eq!(sign.name, "sign-cc");
    assert_eq!(sign.ty, TypeRef::Storage(Storage::U7));
    assert_eq!(sign.visible_when.as_deref(), Some("type == 3 || type >= 5"));
    let Item::Field(channel) = &jog.items[0] else {
        panic!();
    };
    assert_eq!(channel.ty, TypeRef::Named("channel".into()));

    let panel = d.panel.as_ref().unwrap();
    assert_eq!(
        panel.picture,
        Some(Size {
            width: 550,
            height: 329
        })
    );
    let jog = panel
        .items
        .iter()
        .find_map(|item| match item {
            PanelItem::Control(control) if control.jog_binding => Some(control),
            _ => None,
        })
        .expect("jog binding");
    assert_eq!(jog.kind, ControlKind::Jog);
    assert_eq!(jog.label.as_deref(), Some("Jog Wheel"));
}

#[test]
fn tec_usb_identity_and_hid_messages() {
    let d = load_ok("tecontrol/breath.ctrl.kdl");
    assert!(d.identify.iter().any(|m| matches!(
        m,
        Matcher::Usb {
            vid: Some(0x2B06),
            pid: Some(0x0018),
            bcd: Some(0x0500),
            ..
        }
    )));
    let hid = d.transport("hid").unwrap();
    assert_eq!(hid.kind, TransportKind::Hid);
    assert_eq!(
        (hid.usage_page, hid.frame, hid.report_id),
        (Some(0x0C), Some(8), Some(0))
    );
    assert!(hid.seize);

    let meta = d.message("write-meta").unwrap();
    assert_eq!(meta.kind, Some(Access::Write));
    let send = meta.send.as_ref().unwrap();
    assert_eq!(send.pieces.len(), 8);
    assert_eq!(
        send.pieces[0],
        Piece::Expr {
            expr: "0x20 + s".into()
        }
    );
    for name in ["read-meta", "read-shaping", "poll"] {
        let pattern = &d.message(name).unwrap().reply().unwrap().pattern;
        assert_eq!(pattern.pieces.len(), 8, "{name} reply is one 8-byte frame");
    }

    let write = d.procedure("write-sensor").unwrap();
    let Step::Verify { compare, .. } = write.steps.last().unwrap() else {
        panic!();
    };
    assert_eq!(compare.len(), 2);
    assert_eq!(
        d.procedure("make-permanent").unwrap().steps[0],
        Step::Gate {
            meta: match &d.procedure("make-permanent").unwrap().steps[0] {
                Step::Gate { meta, .. } => meta.clone(),
                _ => unreachable!(),
            },
            gate: Gate::Arm,
            text: Some("Store the current RAM settings in the controller's flash?".into()),
        }
    );
    assert_eq!(d.midi_maps[0].endpoints[0].kind, MidiKind::FromField);
    let TypeDef::Int(channel) = d.types.iter().find(|t| t.name() == "tec-channel").unwrap() else {
        panic!();
    };
    assert_eq!(
        channel.special,
        vec![Special {
            value: 16,
            label: "Disable".into()
        }]
    );
}

#[test]
fn db1_is_found_by_scan() {
    let d = load_ok("oceanbeach/db1.ctrl.kdl");
    let aux = d.transport("aux").unwrap();
    assert_eq!(aux.ports, None);
    assert_eq!(aux.scan.as_deref(), Some("revision"));
    let d = parse_str(
        "t.ctrl.kdl",
        "cdl \"0.1\"\ndevice \"t.x\"\nidentify { listen \"FE\" status=\"inferred\" }\n",
    )
    .unwrap();
    assert!(
        matches!(&d.identify[0], Matcher::Listen { pattern, .. } if pattern.to_string() == "FE")
    );
}

#[test]
fn reply_variants_and_patterns() {
    let d = parse_str(
        "t.ctrl.kdl",
        r#"cdl "0.1"
device "t.x"
message "info" {
    send "01"
    reply pattern="ack" variant="ack" ok-when="status == 0x23"
    reply "02 00 00 42 {n}" variant="info" timeout=500
}
pattern "ack" "01 00 00 {status}"
"#,
    )
    .unwrap();
    let m = d.message("info").unwrap();
    assert_eq!(m.replies.len(), 2);
    assert_eq!(m.replies[0].pattern_name.as_deref(), Some("ack"));
    assert_eq!(m.replies[0].pattern.to_string(), "01 00 00 {status}");
    assert_eq!(m.replies[1].variant.as_deref(), Some("info"));
    assert_eq!(d.patterns.len(), 1);

    let e = errors(
        "cdl \"0.1\"\ndevice \"t.x\"\nmessage \"m\" {\n    reply \"01\"\n    reply \"02\" variant=\"b\"\n    reply pattern=\"nope\"\n    send \"01\" pattern=\"x\"\n}\n",
    );
    assert_eq!(e.len(), 3, "{e:?}");
    assert!(e[0].contains("unknown pattern `nope`"), "{e:?}");
    assert!(e[1].contains("not both"), "{e:?}");
    assert!(e[2].contains("names each with `variant=`"), "{e:?}");
}

#[test]
fn firmware_template() {
    let d = load_ok("examples/firmware-template.ctrl.kdl");
    let f = d.firmware.as_ref().unwrap();
    assert_eq!(f.meta.status, Some(Status::Unknown));
    assert_eq!(f.image.as_ref().unwrap().magic, Some(vec![0, 0, 0, 0]));
    assert_eq!(f.stages.len(), 5);
    let Step::Repeat { over, size, .. } = &f.stages[2].steps[0] else {
        panic!();
    };
    assert_eq!(*over, RepeatOver::Source("image-chunks".into()));
    assert_eq!(*size, Some(256));
    assert!(f.recovery.is_some());
}

#[test]
fn ir_round_trips_through_json() {
    for rel in [
        "oceanbeach/db1.ctrl.kdl",
        "tecontrol/breath.ctrl.kdl",
        "korg/nanokontrol-studio.ctrl.kdl",
        "examples/firmware-template.ctrl.kdl",
    ] {
        let d = load_ok(rel);
        let json = serde_json::to_string(&d).unwrap();
        let back: Descriptor = serde_json::from_str(&json).unwrap();
        assert_eq!(back, d, "{rel}");
    }
}

#[test]
fn kdl_syntax_errors_have_positions() {
    let e = errors("cdl \"0.1\"\ndevice \"t.x\" {\n    vendor \"a\n}\n");
    assert!(!e.is_empty());
    assert!(e.iter().all(|e| !e.starts_with("1:")), "{e:?}");
}

#[test]
fn unknown_nodes_and_properties() {
    let e = errors(&format!(
        "{HEAD}message \"m\" transprot=\"aux\" {{\n    sned \"F0 F7\"\n}}\nwidget 1\n"
    ));
    assert_eq!(e.len(), 3, "{e:?}");
    assert!(
        e[0].starts_with("3:13: unknown property `transprot` on `message`"),
        "{e:?}"
    );
    assert!(
        e[1].starts_with("4:5: unknown node `sned` in `message`"),
        "{e:?}"
    );
    assert!(e[2].starts_with("6:1: unknown node `widget`"), "{e:?}");
}

#[test]
fn column_counts_characters() {
    // "ü" is two bytes and one column.
    let e = errors(&format!("{HEAD}// ü\nblock \"b\" /* ü */ sise=1\n"));
    assert_eq!(e.len(), 1);
    assert!(e[0].starts_with("4:19: unknown property `sise`"), "{e:?}");
}

#[test]
fn templates_are_checked() {
    let e = errors(&format!(
        "{HEAD}message \"m\" {{\n    send \"F0 7G F7\"\n    reply \"F0 .. {{rest*}}\"\n}}\nmessage \"n\" {{ send \"F0 .. F7\" }}\n"
    ));
    assert_eq!(e.len(), 2, "{e:?}");
    assert!(
        e[0].starts_with("4:10: `send`: `7G` is not a byte"),
        "{e:?}"
    );
    assert!(e[1].contains("`..` matches replies only"), "{e:?}");
}

#[test]
fn values_need_the_right_type() {
    let e = errors(&format!(
        "{HEAD}block \"b\" size=\"560\" {{\n    field \"f\" offset=1\n}}\ntransport \"t\" kind=\"midi\" preferred=\"yes\" port-match=\"A\" port-match-in=\"B\"\n"
    ));
    assert_eq!(e.len(), 4, "{e:?}");
    assert!(e[0].contains("`size`: expected an integer"), "{e:?}");
    assert!(e[1].contains("`field` needs `type=`"), "{e:?}");
    assert!(e[2].contains("either `port-match` or both"), "{e:?}");
    assert!(
        e[3].contains("`preferred`: expected #true or #false"),
        "{e:?}"
    );
}

#[test]
fn keywords_list_the_choices() {
    let e = errors(&format!("{HEAD}procedure \"p\" kind=\"erase\"\n"));
    assert_eq!(e.len(), 1);
    assert!(
        e[0].contains("expected one of `read`, `write`, `commit`"),
        "{e:?}"
    );
}

#[test]
fn duplicates_and_structure() {
    let e = errors(&format!(
        "{HEAD}message \"m\"\nmessage \"m\"\ndevice \"t.y\"\n"
    ));
    assert_eq!(e.len(), 2, "{e:?}");
    assert!(e[0].starts_with("5:1: `device` appears twice"), "{e:?}");
    assert!(
        e[1].starts_with("4:1: duplicate message `m` (first at t.ctrl.kdl:3:1)"),
        "{e:?}"
    );

    assert_eq!(
        errors("device \"t.x\"\ncdl \"0.1\"\n"),
        ["1:1: a descriptor starts with `cdl \"0.1\"`"]
    );
    assert_eq!(
        errors("cdl \"0.2\"\ndevice \"t.x\"\n"),
        ["1:1: CDL 0.2 is not supported (supported: 0.1)"]
    );
    assert_eq!(errors("cdl \"0.1\"\n"), ["1:1: missing `device` node"]);
}

#[test]
fn paths_and_ranges_are_checked() {
    let e = errors(&format!(
        "{HEAD}procedure \"p\" {{\n    repeat \"i\" \"0..8\" {{ delay 1 }}\n    restore-point \"a[\"\n    verify \"r\" compare=\"a[0 ..]\"\n}}\n"
    ));
    assert_eq!(e.len(), 3, "{e:?}");
    assert!(e[0].contains("ranges are inclusive"), "{e:?}");
    assert!(e[1].contains("unclosed `[`"), "{e:?}");
    assert!(e[2].contains("both ends"), "{e:?}");
}

fn memory(files: &[(&str, &str)]) -> impl FnMut(&FsPath) -> io::Result<String> {
    let files: HashMap<PathBuf, String> = files
        .iter()
        .map(|(p, t)| (PathBuf::from(p), t.to_string()))
        .collect();
    move |p| {
        files
            .get(p)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no such file"))
    }
}

#[test]
fn include_errors_point_at_the_include() {
    let mut read = memory(&[
        (
            "d/a.ctrl.kdl",
            "cdl \"0.1\"\ninclude \"b.inc.kdl\"\ndevice \"t.x\"\ninclude \"gone.inc.kdl\"\n",
        ),
        (
            "d/b.inc.kdl",
            "cdl \"0.1\"\ninclude \"a.ctrl.kdl\"\nmessage \"m\" bogus=1\n",
        ),
    ]);
    let Errors(e) = load_with(FsPath::new("d/a.ctrl.kdl"), &mut read).unwrap_err();
    let e: Vec<String> = e.iter().map(ToString::to_string).collect();
    assert_eq!(e.len(), 3, "{e:?}");
    assert!(
        e[0].starts_with(
            "d/b.inc.kdl:2:1: include cycle: d/a.ctrl.kdl -> d/b.inc.kdl -> d/a.ctrl.kdl"
        ),
        "{e:?}"
    );
    assert!(
        e[1].starts_with("d/a.ctrl.kdl:4:1: cannot read d/gone.inc.kdl"),
        "{e:?}"
    );
    assert!(
        e[2].starts_with("d/b.inc.kdl:3:13: unknown property `bogus`"),
        "{e:?}"
    );
}

#[test]
fn local_children_replace_included_ones() {
    let mut read = memory(&[
        (
            "a.ctrl.kdl",
            "cdl \"0.1\"\ninclude \"b.inc.kdl\"\ndevice \"t.x\"\nmessage \"m\" transport=\"usb\" {\n    send \"F0 01 F7\"\n}\n",
        ),
        (
            "b.inc.kdl",
            "cdl \"0.1\"\nmessage \"m\" transport=\"ble\" frame=\"f\" {\n    send \"F0 00 F7\"\n    ok-when \"x\"\n}\n",
        ),
    ]);
    let d = load_with(FsPath::new("a.ctrl.kdl"), &mut read).unwrap();
    let m = d.message("m").unwrap();
    assert_eq!(m.transport.as_deref(), Some("usb"));
    assert_eq!(m.frame.as_deref(), Some("f"));
    assert_eq!(m.send.as_ref().unwrap().to_string(), "F0 01 F7");
    assert_eq!(m.ok_when.as_deref(), Some("x"));
    assert_eq!(d.messages.len(), 1);
}

#[test]
fn error_at_names_the_file() {
    let d = load_ok("korg/nanokey-studio.ctrl.kdl");
    let commit = d.message("commit-scene").unwrap();
    let e = d.error_at(commit.meta.pos, "x");
    assert!(e.file.ends_with("studio.inc.kdl"));
    assert!(e.line > 1);
}

#[test]
fn message_args_take_a_range_or_a_block_index() {
    let d = load_ok("tecontrol/breath.ctrl.kdl");
    let m = d.message("read-meta").unwrap();
    assert_eq!(d.arg_range(m, "s"), Some(IntRange { lo: 0, hi: 3 }));
    let page = d.message("write-curve-page").unwrap();
    assert_eq!(d.arg_range(page, "start"), Some(IntRange { lo: 0, hi: 30 }));
    // The shared KORG message takes each device's own scene count.
    for (file, hi) in [
        ("korg/nanokontrol-studio.ctrl.kdl", 4),
        ("korg/nanokey-studio.ctrl.kdl", 7),
    ] {
        let d = load_ok(file);
        let m = d.message("select-scene").unwrap();
        assert_eq!(
            d.arg_range(m, "scene"),
            Some(IntRange { lo: 0, hi }),
            "{file}"
        );
    }

    let found = errors(&format!(
        "{HEAD}message \"m\" {{ arg \"a\"; arg \"b\" range=\"0..=1\" index=\"x\"; arg \"b\" range=\"0..=1\"; send \"{{a}}\" }}"
    ));
    assert_eq!(found.len(), 3, "{found:#?}");
    assert!(
        found[0].contains("one of `range=`, `index=` and `type=`"),
        "{found:#?}"
    );
    assert!(
        found[1].contains("one of `range=`, `index=` and `type=`"),
        "{found:#?}"
    );
    assert!(found[2].contains("appears twice"), "{found:#?}");
}
