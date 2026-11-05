//! Check the byte-level behavior shared by both front ends for nanoKEY2 pages.
use korg_app::{pages, panel};

#[test]
fn nano_key2_page_writes_preserve_the_original_offsets_ranges_and_choices() {
    let panel = panel::panel("X10820").expect("nanoKEY2 panel");
    // Control, row, scene offset, minimum, maximum and stored bias.
    let cases = [
        ("Octave Up", 0, 0, 0, 15, 0),
        ("Pitch Up", 0, 2, 0, 3, 0),
        ("Keyboard", 0, 8, -12, 12, 64),
        ("Keyboard", 1, 9, 0, 3, 0),
        ("Keyboard", 2, 10, 0, 127, 0),
        ("Sustain Button", 0, 24, 0, 1, 0),
        ("Sustain Button", 1, 26, 0, 1, 0),
        ("Sustain Button", 2, 25, 0, 127, 0),
        ("Sustain Button", 3, 28, 0, 127, 0),
        ("Sustain Button", 4, 27, 0, 127, 0),
        ("Sustain Button", 5, 29, 0, 2, 0),
        ("Mod Button", 0, 16, 0, 1, 0),
        ("Mod Button", 1, 18, 0, 1, 0),
        ("Mod Button", 2, 17, 0, 127, 0),
        ("Mod Button", 3, 20, 0, 127, 0),
        ("Mod Button", 4, 19, 0, 127, 0),
        ("Mod Button", 5, 21, 0, 2, 0),
    ];
    for (control, row, offset, min, max, bias) in cases {
        let control = panel.controls.iter().find(|c| c.label == control).unwrap();
        for value in min - 1..=max + 1 {
            let mut scene = vec![0x55; 560];
            scene[9] = 3; // Enable the constant velocity field.
            let rows = pages::fields("X10820", &panel, Some(control), &scene).unwrap();
            let pages::PageRow::Field(field) = &rows[row] else {
                panic!("field expected")
            };
            assert_eq!(field.offset, offset);
            assert_eq!(field.range(), (min, max));
            for (index, label) in field.kind.labels().iter().enumerate() {
                assert_eq!(
                    pages::choice_index(field, &label.to_lowercase()),
                    Some(index as i32)
                );
            }
            let before = scene.clone();
            assert!(pages::store("X10820", &panel, &mut scene, field, value));
            assert_eq!(scene[offset], (value.clamp(min, max) + bias) as u8);
            for (i, (old, new)) in before.iter().zip(&scene).enumerate() {
                if i != offset {
                    assert_eq!(old, new, "unexpected write at {i}");
                }
            }
        }
    }
}

#[test]
fn constant_velocity_cannot_be_written_when_the_curve_is_not_constant() {
    let panel = panel::panel("X10820").unwrap();
    let keyboard = panel
        .controls
        .iter()
        .find(|c| c.label == "Keyboard")
        .unwrap();
    for curve in [0, 1, 2, 4, 255] {
        let mut scene = vec![0; 560];
        scene[9] = curve;
        let rows = pages::fields("X10820", &panel, Some(keyboard), &scene).unwrap();
        let pages::PageRow::Field(field) = &rows[2] else {
            panic!("field expected")
        };
        assert!(!field.enabled);
        let before = scene.clone();
        assert!(!pages::store("X10820", &panel, &mut scene, field, 100));
        assert_eq!(scene, before);
    }
}
