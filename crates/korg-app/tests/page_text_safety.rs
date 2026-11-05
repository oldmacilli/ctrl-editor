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
