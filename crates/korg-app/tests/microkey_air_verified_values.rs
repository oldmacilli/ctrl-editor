use korg_app::{pages, panel};

#[test]
fn microkey_air_velocity_and_connection_use_verified_values() {
    let panel = panel::panel("X14810").unwrap();
    let data = vec![0; 256];
    let rows = pages::fields("X14810", &panel, None, &data).unwrap();
    let field = |offset| {
        rows.iter()
            .find_map(|row| match row {
                pages::PageRow::Field(field) if field.offset == offset => Some(field),
                _ => None,
            })
            .unwrap()
    };
    assert_eq!(field(9).kind.labels().len(), 9);
    assert_eq!(field(9).kind.labels()[8], "Const");
    assert_eq!(field(10).range(), (1, 127));
    assert_eq!(field(234).label, "Connection Type");
    assert_eq!(field(234).kind.labels(), ["2", "1"]);
    assert!(!field(10).enabled);
    let mut changed = data.clone();
    for value in 0..=8 {
        assert!(pages::store(
            "X14810",
            &panel,
            &mut changed,
            field(9),
            value
        ));
        assert_eq!(changed[9], value as u8);
    }
    let constant = pages::fields("X14810", &panel, None, &changed).unwrap();
    assert!(constant
        .iter()
        .any(|row| matches!(row, pages::PageRow::Field(f) if f.offset == 10 && f.enabled)));
    for (value, expected) in [(0, "2"), (1, "1")] {
        assert!(pages::store(
            "X14810",
            &panel,
            &mut changed,
            field(234),
            value
        ));
        assert_eq!(field(234).display(&changed), expected);
    }
}
