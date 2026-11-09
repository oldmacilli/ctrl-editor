use korg_app::assign::FieldKind;
use korg_app::pages::{self, PageField};
use korg_app::panel::Panel;

#[test]
fn disabled_text_field_does_not_modify_scene() {
    let field = PageField {
        offset: 1,
        label: "Name",
        kind: FieldKind::Text { width: 4 },
        enabled: false,
        note: false,
        min_from_offset: None,
        max_from_offset: None,
    };
    let mut scene = *b"ABCDEF";
    let before = scene;
    assert!(!pages::store_text(&mut scene, &field, "test"));
    assert_eq!(scene, before);

    let enabled = PageField {
        enabled: true,
        ..field
    };
    assert!(pages::store_text(&mut scene, &enabled, "test"));
    assert_eq!(&scene, b"AtestF");
}

#[test]
fn configured_zero_padding_stops_before_reserved_scene_bytes() {
    let field = PageField {
        offset: 208,
        label: "Device Name",
        kind: FieldKind::TextPadded {
            width: 26,
            write_width: 16,
            padding: 0,
        },
        enabled: true,
        note: false,
        min_from_offset: None,
        max_from_offset: None,
    };
    let mut scene = [0xA5; 256];

    assert!(pages::store_text(&mut scene, &field, "Korg AIR2 BT"));
    assert_eq!(&scene[208..224], b"Korg AIR2 BT\0\0\0\0");
    assert_eq!(&scene[224..234], &[0xA5; 10]);
    assert_eq!(&scene[234..], &[0xA5; 22]);
}

#[test]
fn configured_zero_padding_caps_text_at_the_terminated_write_width() {
    let field = PageField {
        offset: 208,
        label: "Device Name",
        kind: FieldKind::TextPadded {
            width: 26,
            write_width: 16,
            padding: 0,
        },
        enabled: true,
        note: false,
        min_from_offset: None,
        max_from_offset: None,
    };
    let mut scene = [0xA5; 256];

    assert!(pages::store_text(
        &mut scene,
        &field,
        "12345678901234567890"
    ));
    assert_eq!(&scene[208..224], b"123456789012345\0");
    assert_eq!(&scene[224..], &[0xA5; 32]);
}

#[test]
#[ignore = "requires KORG_DESCRIPTOR_DIR with the external KORG descriptors"]
fn external_microkey_air_page_uses_the_declared_text_write_range() {
    let panel = Panel {
        class: "X14810".to_owned(),
        width: 0.0,
        height: 0.0,
        controls: vec![],
        binding_inferred: false,
    };
    let mut scene = [0xA5; 256];
    let rows = pages::fields("X14810", &panel, None, &scene).unwrap();
    let field = rows
        .iter()
        .find_map(|row| match row {
            pages::PageRow::Field(field) if field.label == "Device Name" => Some(field),
            _ => None,
        })
        .expect("external descriptor's Device Name field");
    assert!(matches!(
        field.kind,
        FieldKind::TextPadded {
            width: 26,
            write_width: 16,
            padding: 0
        }
    ));
    assert!(pages::store_text(&mut scene, field, "Korg AIR2 BT"));
    assert_eq!(&scene[208..224], b"Korg AIR2 BT\0\0\0\0");
    assert_eq!(&scene[224..234], &[0xA5; 10]);
}

#[test]
fn dynamic_bounds_do_not_expand_the_field_range_or_allow_missing_bytes() {
    let panel = Panel {
        class: String::new(),
        width: 0.0,
        height: 0.0,
        controls: vec![],
        binding_inferred: false,
    };
    let mut field = PageField {
        offset: 0,
        label: "Min",
        kind: FieldKind::Number { min: 0, max: 127 },
        enabled: true,
        note: false,
        min_from_offset: None,
        max_from_offset: Some(1),
    };
    let mut scene = [0, 255];
    assert!(pages::store("", &panel, &mut scene, &field, 200));
    assert_eq!(scene[0], 127);
    let mut short_scene = [42];
    assert!(!pages::store("", &panel, &mut short_scene, &field, 100));
    assert_eq!(short_scene, [42]);
    field.min_from_offset = Some(1);
    field.max_from_offset = None;
    let mut scene = [42, 200];
    assert!(!pages::store("", &panel, &mut scene, &field, 100));
    assert_eq!(scene, [42, 200]);
}
