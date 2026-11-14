//! Ports bind themselves to a device by the descriptors' `port-match`; an
//! unbound port names only SysEx that a template pins to a vendor.
#![cfg(feature = "descriptors")]

use ctrl_monitor::*;

fn described(port_name: &str, dir: Dir, bytes: &[u8]) -> Vec<String> {
    let mut store = Store::default();
    let port = store.port(PortRef::new(PortKind::CoreMidi, port_name, port_name));
    let mut collector = Collector::new(ParserConfig::default());
    collector.describe_with(Box::new(CdlDescriber::embedded()));
    let (mut tap, reader) = tap(TapConfig::default());
    collector.add(reader);
    assert!(tap.push(Stamp::default(), port, dir, bytes));
    collector.drain(&mut store, 0);
    store
        .events()
        .flat_map(|e| store.notes(e.seq).to_vec())
        .filter_map(|n| match n {
            Annotation::Described { name, .. } => Some(name),
            _ => None,
        })
        .collect()
}

#[test]
fn a_port_named_like_a_device_is_bound_to_it() {
    // A CC is named only on a bound port.
    let names = described(
        "UC-33 USB MIDI Controller Anschluss 1",
        Dir::In,
        &[0xB8, 0x07, 0x7F],
    );
    assert_eq!(names.len(), 1, "{names:?}");
    assert!(
        names[0].contains("UC-33") && names[0].ends_with(": cc"),
        "{names:?}"
    );
    assert!(described("Some Synth", Dir::In, &[0xB8, 0x07, 0x7F]).is_empty());
}

#[test]
fn universal_sysex_on_an_unbound_port_is_left_to_the_generic_names() {
    let names = described("Some Synth", Dir::In, &[0xF0, 0x7E, 0x7F, 0x06, 0x01, 0xF7]);
    assert!(names.is_empty(), "{names:?}");
}

#[test]
fn a_placeholder_template_does_not_claim_foreign_sysex() {
    // UC-33's memory-dump is `F0 {data*} F7`: it must not name a Roland message.
    let names = described("Some Synth", Dir::In, &[0xF0, 0x41, 0x10, 0x42, 0x12, 0xF7]);
    assert!(
        names.iter().all(|n| !n.contains("memory-dump")),
        "{names:?}"
    );
}
