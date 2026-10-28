//! Every descriptor's test vectors come back described with their own
//! message when they pass through the monitor (docs/MONITOR.md §10, M2).
#![cfg(feature = "descriptors")]

use ctrl_codec::{Codec, Medium};
use ctrl_monitor::*;

#[test]
fn every_test_vector_is_described_by_its_message() {
    let mut checked = 0;
    let mut failures = Vec::new();
    for path in ctrl_descriptors::devices() {
        let descriptor = ctrl_descriptors::load(path).unwrap();
        let codec = Codec::new(&descriptor);
        for v in &descriptor.test_vectors {
            let (Some(message), Some(bytes)) = (&v.message, &v.bytes) else {
                continue;
            };
            let medium = descriptor.message(message).map(|m| codec.medium(m));
            let kind = if medium == Some(Medium::Usb) {
                PortKind::Hid
            } else {
                PortKind::Alsa
            };

            let mut store = Store::default();
            let port = store.port(PortRef::new(kind, "device", "device"));
            let mut describer = CdlDescriber::new();
            let device = describer.add(descriptor.clone());
            describer.bind(port, device);
            let mut collector = Collector::new(ParserConfig::default());
            collector.describe_with(Box::new(describer));
            let (mut tap, reader) = tap(TapConfig::default());
            collector.add(reader);
            // A reply follows its request when the descriptor has a vector
            // for it, as in a real exchange.
            let request = v.reply.then(|| {
                descriptor
                    .test_vectors
                    .iter()
                    .find(|r| !r.reply && r.message.as_ref() == Some(message))
                    .and_then(|r| r.bytes.as_ref())
            });
            if let Some(Some(request)) = request {
                assert!(tap.push(Stamp::default(), port, Dir::Out, request));
                collector.drain(&mut store, 0);
            }
            // Only the vector's own events count.
            let first = store.next_seq();
            let dir = if v.reply { Dir::In } else { Dir::Out };
            assert!(tap.push(Stamp::default(), port, dir, bytes));
            collector.drain(&mut store, 0);

            checked += 1;
            let events: Vec<Event> = store.since(first).copied().collect();
            // The message, or among the others a shared reply matched
            // (without the request before it, the vector cannot tell).
            let names: Vec<String> = events
                .iter()
                .flat_map(|e| store.notes(e.seq))
                .filter_map(|n| match n {
                    Annotation::Described { name, fields } => {
                        let also = fields
                            .iter()
                            .find(|(k, _)| k == "also")
                            .map(|(_, v)| v.as_str());
                        Some(format!("{name} also {}", also.unwrap_or("")))
                    }
                    _ => None,
                })
                .collect();
            let want = format!(": {message}");
            let ok = events.len() == 1
                && names.len() == 1
                && (names[0].contains(&format!("{want} "))
                    || names[0]
                        .split(" also ")
                        .nth(1)
                        .is_some_and(|a| a.split(", ").any(|m| m == message)));
            if !ok {
                failures.push(format!(
                    "{path} {}: {} events, described as {names:?}",
                    v.name,
                    events.len()
                ));
            }
        }
    }
    assert!(checked >= 41, "only {checked} vectors");
    assert!(
        failures.is_empty(),
        "{} of {checked}:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
