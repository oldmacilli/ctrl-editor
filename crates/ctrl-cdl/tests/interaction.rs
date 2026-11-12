//! The device classes (CDL §5.2): each class keeps to its rules, and a
//! declaration the descriptor does not support is refused.

use ctrl_cdl::interaction::derive;
use ctrl_cdl::*;

fn load(body: &str) -> Descriptor {
    let text = format!("cdl \"0.1\"\ndevice \"t.x\"\n{body}");
    parse_str("t.ctrl.kdl", &text).unwrap_or_else(|e| panic!("{e}"))
}

fn errors(body: &str) -> Vec<String> {
    validate(&load(body))
        .findings
        .into_iter()
        .filter(|f| f.severity == Severity::Error)
        .map(|f| f.error.message)
        .collect()
}

fn has(body: &str, needle: &str) {
    let e = errors(body);
    assert!(
        e.iter().any(|m| m.contains(needle)),
        "no `{needle}` in {e:#?}"
    );
}

const ACTIVE: &str = r#"
transport "aux" kind="midi" role="config"
message "get" transport="aux" status="measured" { send "F0 01 F7"; reply "F0 02 F7" }
procedure "read-all" kind="read" transport="aux" status="measured" { exchange "get" }
"#;

const PASSIVE: &str = r#"
transport "usb" kind="midi" role="config"
hands { key "push8" label="PUSH 8" }
message "dump" transport="usb" status="measured" { reply "F0 03 F7" }
procedure "read-all" kind="read" transport="usb" status="measured" { press "push8"; await "dump" timeout=1000 }
"#;

const PUSH_ONLY: &str = r#"
transport "controls" kind="midi" role="performance"
hands { key "recall" label="RECALL" }
procedure "snapshot" kind="read" transport="controls" status="measured" { press "recall" }
"#;

const USB_MIXED: &str = r#"
transport "hid" kind="hid" usage-page=0x0C frame=8 role="config"
transport "midi" kind="midi" role="performance"
message "poll" transport="hid" status="measured" { send "00 00 00 00 00 00 00 00"; reply "00 00 00 00 00 00 00 00" }
procedure "read-all" kind="read" transport="hid" status="measured" { exchange "poll" }
"#;

#[test]
fn each_class_is_derived_from_the_contents() {
    for (body, class) in [
        (ACTIVE, Interaction::Active),
        (PASSIVE, Interaction::Passive),
        (PUSH_ONLY, Interaction::PushOnly),
        (USB_MIXED, Interaction::UsbMixed),
    ] {
        assert_eq!(derive(&load(body)).class, Some(class), "{body}");
        let declared = format!("interaction \"{}\"\n{body}", class.keyword());
        assert_eq!(errors(&declared), Vec::<String>::new(), "{declared}");
    }
}

#[test]
fn a_missing_class_is_an_error_that_names_the_derived_one() {
    has(ACTIVE, "needs an `interaction` class");
    has(PASSIVE, "supports `interaction \"passive\"`");
}

#[test]
fn a_wrong_class_is_refused() {
    has(
        &format!("interaction \"passive\"\n{ACTIVE}"),
        "the descriptor is `active`",
    );
    has(
        &format!("interaction \"active\"\n{USB_MIXED}"),
        "the descriptor is `usb-mixed`",
    );
    has(
        &format!("interaction \"usb-mixed\"\n{PUSH_ONLY}"),
        "the descriptor is `push-only`",
    );
}

#[test]
fn bus_mixed_is_reserved() {
    has(&format!("interaction \"bus-mixed\"\n{ACTIVE}"), "reserved");
    assert!(!Interaction::BusMixed.capabilities().supported);
}

#[test]
fn passive_procedures_guide_first_and_never_exchange() {
    let unguided = PASSIVE.replace("press \"push8\"; ", "");
    has(
        &format!("interaction \"passive\"\n{unguided}"),
        "must guide the user",
    );
    let exchange = format!(
        "interaction \"passive\"\n{PASSIVE}\nmessage \"ask\" transport=\"usb\" {{ send \"F0 F7\"; reply \"F0 F7\" }}\nprocedure \"probe\" kind=\"read\" transport=\"usb\" {{ press \"push8\"; exchange \"ask\" }}"
    );
    has(&exchange, "uses `exchange`");
}

#[test]
fn push_only_sends_nothing_and_needs_hands() {
    let config = PUSH_ONLY.replace("role=\"performance\"", "role=\"config\"");
    has(
        &format!("interaction \"push-only\"\n{config}"),
        "cannot be `role=\"config\"`",
    );
    let no_hands = PUSH_ONLY
        .replace("hands { key \"recall\" label=\"RECALL\" }", "")
        .replace("press \"recall\"", "");
    has(
        &format!("interaction \"push-only\"\n{no_hands}"),
        "needs `hands`",
    );
    let sends = format!(
        "interaction \"push-only\"\n{PUSH_ONLY}\nmessage \"x\" transport=\"controls\" {{ send \"B0 01 00\" }}\nprocedure \"poke\" kind=\"read\" transport=\"controls\" {{ send \"x\" }}"
    );
    has(&sends, "sends to the device");
}

#[test]
fn usb_mixed_never_configures_over_midi() {
    let midi_write = format!(
        "interaction \"usb-mixed\"\n{USB_MIXED}\nmessage \"set\" transport=\"midi\" kind=\"write\" {{ send \"F0 01 F7\" }}"
    );
    has(&midi_write, "writes over a MIDI transport");
    let no_midi = USB_MIXED.replace("transport \"midi\" kind=\"midi\" role=\"performance\"", "");
    has(
        &format!("interaction \"usb-mixed\"\n{no_midi}"),
        "needs a MIDI transport",
    );
}

#[test]
fn active_reads_are_started_by_software() {
    let awaited = format!(
        "interaction \"active\"\n{ACTIVE}\nmessage \"dump\" transport=\"aux\" {{ reply \"F0 03 F7\" }}\nprocedure \"wait\" kind=\"read\" transport=\"aux\" {{ await \"dump\" timeout=1000 }}"
    );
    has(&awaited, "waits for a dump");
}

#[test]
fn every_shipped_descriptor_declares_its_class() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../descriptors");
    let mut n = 0;
    for vendor in std::fs::read_dir(&root).unwrap() {
        for f in std::fs::read_dir(vendor.unwrap().path()).unwrap() {
            let p = f.unwrap().path();
            if !p.to_string_lossy().ends_with(".ctrl.kdl") {
                continue;
            }
            let d = load_path(&p);
            assert!(
                d.interaction.is_some(),
                "{} declares no interaction",
                p.display()
            );
            n += 1;
        }
    }
    assert!(n > 0);
}

fn load_path(p: &std::path::Path) -> Descriptor {
    ctrl_cdl::load(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}
