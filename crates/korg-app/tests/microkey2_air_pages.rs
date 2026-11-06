use std::collections::BTreeSet;

use korg_app::{pages, panel};

#[test]
#[ignore = "pages are not enabled until text padding preserves the existing encoding"]
fn microkey_air_pages_keep_every_scene_field_reachable() {
    let panel = panel::panel("X14810").unwrap();
    let scene = vec![0; 256];
    let expected: BTreeSet<_> = [0, 8, 9, 10, 16, 17, 19, 20, 41, 45, 208, 234, 240, 241]
        .into_iter()
        .collect();
    let mut seen = BTreeSet::new();
    for control in std::iter::once(None).chain(panel.controls.iter().map(Some)) {
        let rows = pages::fields("X14810", &panel, control, &scene)
            .expect("microKEY Air descriptor must provide pages");
        assert!(!rows.is_empty(), "control has no fields: {control:?}");
        seen.extend(rows.into_iter().filter_map(|row| match row {
            pages::PageRow::Field(field) => Some(field.offset),
            pages::PageRow::Heading(_) => None,
        }));
    }
    assert_eq!(seen, expected);
}

#[test]
#[ignore = "page text editing must preserve the existing NUL padding"]
fn microkey_air_overview_edits_only_the_selected_field() {
    let panel = panel::panel("X14810").unwrap();
    let mut scene = vec![0; 256];
    let rows = pages::fields("X14810", &panel, None, &scene).unwrap();
    let field = |offset| {
        rows.iter()
            .find_map(|row| match row {
                pages::PageRow::Field(field) if field.offset == offset => Some(field),
                _ => None,
            })
            .unwrap()
    };
    let transpose = field(8);
    assert_eq!(transpose.label, "Transpose");
    assert!(pages::store("X14810", &panel, &mut scene, transpose, -12));
    assert_eq!(scene[8], 52);
    assert!(scene
        .iter()
        .enumerate()
        .all(|(i, byte)| i == 8 || *byte == 0));

    let switch = field(45);
    assert_eq!(switch.label, "Pedal Polarity");
    assert_eq!(switch.kind.labels(), ["Auto", "Minus", "Plus"]);
    assert!(pages::store("X14810", &panel, &mut scene, switch, 2));
    assert_eq!(scene[45], 2);
    assert_eq!(scene[8], 52);

    let name = field(208);
    assert_eq!(name.label, "Device Name");
    assert!(pages::store_text(&mut scene, name, "microKEY"));
    assert_eq!(&scene[208..216], b"microKEY");
    assert!(scene[216..234].iter().all(|byte| *byte == 0));
    assert_eq!((scene[8], scene[45], scene[234]), (52, 2, 0));
}
