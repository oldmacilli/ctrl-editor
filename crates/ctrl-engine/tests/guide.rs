//! Guides (CDL §17) from the repository's descriptors: edit guides for a
//! device without a dump (UC-33) and guide steps inside a procedure.

use std::collections::HashMap;

use ctrl_cdl::Gate;
use ctrl_codec::Planned;
use ctrl_engine::guide::cost;
use ctrl_engine::guide::to_text;
use ctrl_engine::kinds;
use ctrl_engine::{
    changes_between, edit_guide, edit_guide_at, Change, Document, EngineError, Event, Here, Line,
    Operator, Port, Run, Wait,
};

fn uc33() -> ctrl_cdl::Descriptor {
    ctrl_descriptors::load("evolution/uc33.ctrl.kdl").unwrap()
}

fn texts(lines: &[Line]) -> Vec<&str> {
    lines.iter().map(|l| l.text.as_str()).collect()
}

#[test]
fn a_cc_change_on_a_fader_is_typed_in() {
    let d = uc33();
    let want = HashMap::from([("preset[2].control[0].assign", 12)]);
    let lines = edit_guide(
        &d,
        &[Change {
            path: "preset[2].control[0].assign".into(),
            want: 12,
            have: Some(7),
        }],
        &|p| want.get(p).copied(),
    )
    .unwrap();
    assert_eq!(
        texts(&lines),
        [
            "Press RECALL",
            "Type each digit within 3 seconds; the LCD symbol flashes while the UC-33 takes digits.",
            "Type 0 3",
            "Move F1",
            "Press ASSIGN",
            "Type 0 1 2",
            "Press STORE",
            "Type 0 3",
            "Press + and - together",
            "Press each changed button once, so the editor sees what it sends.",
        ]
    );
    assert_eq!(lines[2].shows.as_deref(), Some("P03"));
    assert_eq!(
        lines[2].hint.as_deref(),
        Some("Makes memory 3 the current setup. Skip it if the LCD already shows P03.")
    );
    assert_eq!(lines[3].shows.as_deref(), Some("01"));
    // the longer way is kept for a front end that offers it
    assert_eq!(
        lines[3].alternatives,
        ["Press CONTROL SELECT, then type 0 1"]
    );
}

#[test]
fn a_number_key_is_selected_by_number_and_unused_slots_are_skipped() {
    let d = uc33();
    // B41 (c = 40) as a CC on/off button: assign 146 uses MSB, LSB and
    // program; a plain CC on a knob uses none of them.
    let want = HashMap::from([
        ("preset[0].control[40].assign", 146),
        ("preset[0].control[40].data-msb", 127),
        ("preset[0].control[10].assign", 20),
    ]);
    let ch = |path: &str, want: i64| Change {
        path: path.into(),
        want,
        have: None,
    };
    let lines = edit_guide(
        &d,
        &[
            ch("preset[0].control[40].data-msb", 127),
            ch("preset[0].control[40].assign", 146),
            ch("preset[0].control[10].data-msb", 5),
        ],
        &|p| want.get(p).copied(),
    )
    .unwrap();
    let t = to_text(&lines);
    assert!(t.contains("Press CONTROL SELECT.\n"), "{t}");
    assert!(t.contains("Type 4 1"), "{t}");
    assert!(t.contains("The number keys type digits"), "{t}");
    // assign comes before the data slot, whatever the order of the changes
    let assign = t.find("Type 1 4 6").unwrap();
    let msb = t.find("Press DATA MSB twice").unwrap();
    assert!(assign < msb, "{t}");
    // the knob's data MSB does not apply to a plain CC: no C11 at all
    assert!(!t.contains("C11"), "{t}");
}

#[test]
fn controller_off_and_the_global_channel() {
    let d = uc33();
    let none = |_: &str| None;
    let off = edit_guide(
        &d,
        &[Change {
            path: "preset[0].control[9].assign".into(),
            want: 255,
            have: Some(7),
        }],
        &none,
    )
    .unwrap();
    assert!(texts(&off).contains(&"Type 1 4 4"));
    assert!(texts(&off).contains(&"Press +"));

    let global = edit_guide(
        &d,
        &[Change {
            path: "global.channel".into(),
            want: 5,
            have: Some(1),
        }],
        &none,
    )
    .unwrap();
    assert_eq!(texts(&global)[..1], ["Press ASSIGN and CHANNEL together"]);
    assert!(texts(&global).contains(&"Type 0 5"));
}

#[test]
fn a_field_without_a_hand_rule_is_refused() {
    let d = uc33();
    let e = edit_guide(
        &d,
        &[Change {
            path: "global.device-id".into(),
            want: 3,
            have: None,
        }],
        &|_| None,
    )
    .unwrap_err();
    assert!(matches!(e, EngineError::Unsupported(_)), "{e}");
}

struct NoPort;

impl Port for NoPort {
    fn send(&mut self, _: &Planned) -> Result<(), String> {
        Err("nothing is sent in a tour".into())
    }
    fn recv(&mut self, _: u32) -> Result<Option<Vec<u8>>, String> {
        Ok(None)
    }
}

#[derive(Default)]
struct Cards {
    cards: Vec<Vec<String>>,
    stop_after: usize,
}

impl Operator for Cards {
    fn gate(&mut self, _: Gate, _: &str) -> bool {
        true
    }
    fn note(&mut self, _: &str) {}
    fn restore_point(&mut self, _: &str, _: &[Vec<u8>]) -> Result<(), String> {
        Ok(())
    }
    fn guide(&mut self, lines: &[Line], wait: &Wait) -> bool {
        assert_eq!(wait, &Wait::Confirm);
        self.cards
            .push(lines.iter().map(|l| l.text.clone()).collect());
        self.cards.len() < self.stop_after
    }
}

#[test]
fn the_tour_shows_one_card_per_memory() {
    let d = uc33();
    let mut run = Run::new(&d);
    let mut op = Cards {
        stop_after: 2,
        ..Default::default()
    };
    let r = run.run("tour", &mut NoPort, &mut op);
    // the operator stopped after the second memory
    assert!(matches!(r, Err(EngineError::Declined(_))), "{r:?}");
    assert_eq!(op.cards.len(), 2);
    // the keypad is explained on every card: each card is a guide of its own
    assert_eq!(
        op.cards[1][..4],
        [
            "Press RECALL",
            "Type each digit within 3 seconds; the LCD symbol flashes while the UC-33 takes digits.",
            "Type 0 2",
            "Press + and - together"
        ]
    );
    assert_eq!(
        run.events
            .iter()
            .filter(|e| matches!(e, Event::Guided(_)))
            .count(),
        2
    );
}

#[test]
fn uc4_edit_mode_turns_encoders_to_labels() {
    let d = ctrl_descriptors::load("faderfox/uc4.ctrl.kdl").unwrap();
    let ch = |path: &str, want: i64| Change {
        path: path.into(),
        want,
        have: None,
    };
    let lines = edit_guide(
        &d,
        &[
            ch("controls[15].group[1].encoder[2].channel", 9),
            ch("controls[15].group[1].encoder[2].type", 2),
            ch("controls[15].group[1].fader[0].mode", 1),
        ],
        &|_| None,
    )
    .unwrap();
    let t = texts(&lines);
    assert_eq!(
        t[..4],
        [
            "Hold Shift and press Edit twice",
            "Turn encoder 1 to SE16",
            "Press Edit",
            "Hold Shift and press Edit",
        ]
    );
    // the encoder's fields in the order of the rule, then the fader
    assert_eq!(
        t[4..],
        [
            "Hold Shift and press encoder 2's push button",
            "Turn encoder 1 to Enc3",
            format!("Turn encoder 3 to “{}”", label(&d, "encoder-type", 2)).as_str(),
            "Turn encoder 5 to Ch10",
            "Hold Shift and press green button 2",
            "Turn encoder 1 to FAd1",
            format!("Turn encoder 4 to “{}”", label(&d, "fader-mode", 1)).as_str(),
            "Press Edit",
        ]
    );

    let names = edit_guide(&d, &[ch("names[0].group[3].char[1]", 10)], &|_| None).unwrap();
    assert!(
        texts(&names).contains(&"Hold Shift and turn encoder 2 to “A”"),
        "{}",
        to_text(&names)
    );
}

fn label(d: &ctrl_cdl::Descriptor, ty: &str, v: i64) -> String {
    match d.types.iter().find(|t| t.name() == ty) {
        Some(ctrl_cdl::TypeDef::Enum(e)) => e
            .values
            .iter()
            .find(|x| x.value == v)
            .unwrap()
            .label
            .clone(),
        _ => panic!("{ty}"),
    }
}

#[test]
fn the_guide_takes_the_shortest_way_from_what_the_device_holds() {
    let d = uc33();
    let none = |_: &str| None;
    // B35, a transport button: pressing it selects it
    let note = |have: Option<i64>| Change {
        path: "preset[4].control[34].program".into(),
        want: 62,
        have,
    };
    let fresh = edit_guide_at(&d, &[note(None)], &none, &Here::default()).unwrap();
    assert!(
        texts(&fresh.0).contains(&"Press B35"),
        "{}",
        to_text(&fresh.0)
    );
    assert_eq!(
        fresh.1.element,
        Some(4),
        "the memory stays current after STORE"
    );

    // memory 5 is current already: no RECALL
    let here = edit_guide_at(&d, &[note(None)], &none, &fresh.1).unwrap();
    assert!(!texts(&here.0).contains(&"Press RECALL"));
    assert!(cost(&here.0) < cost(&fresh.0));

    // the snapshot showed the value is there already: nothing to do
    let same = edit_guide_at(&d, &[note(Some(62))], &none, &Here::default()).unwrap();
    assert!(same.0.is_empty());
}

#[test]
fn kinds_offer_what_a_control_can_send() {
    let d = uc33();
    let names = |c: i64| -> Vec<String> {
        kinds::offered(&d, "preset.control", &[0, c])
            .unwrap()
            .iter()
            .map(|k| k.name.clone())
            .collect()
    };
    // a fader cannot send a note; a button can
    assert!(!names(0).contains(&"note".to_string()));
    assert!(names(40).contains(&"note".to_string()));

    let note = kinds::kind_of(&d, "preset.control", &[0, 40], 147)
        .unwrap()
        .unwrap();
    assert_eq!(note.label, "Note");
    let labels: Vec<String> = kinds::fields(&d, "preset.control", &[0, 40], note)
        .unwrap()
        .into_iter()
        .map(|f| f.label)
        .collect();
    assert_eq!(
        labels,
        [
            "Momentary or toggle",
            "Channel (00 = global)",
            "Note",
            "Velocity on",
            "Velocity off"
        ]
    );

    // switching a CC button to a note sets the assign and the defaults
    let cc = |name: &str| match name {
        "assign" => Some(7),
        _ => None,
    };
    let set = kinds::switch_to(&d, "preset.control", &[0, 40], note, &cc).unwrap();
    assert_eq!(set[0], ("assign".into(), 147));
    assert!(set.contains(&("program".into(), 60)));

    // then the guide types exactly what the kind needs
    let want: HashMap<String, i64> = set
        .iter()
        .map(|(f, v)| (format!("preset[0].control[40].{f}"), *v))
        .collect();
    let changes: Vec<Change> = want
        .iter()
        .map(|(p, v)| Change {
            path: p.clone(),
            want: *v,
            have: None,
        })
        .collect();
    let lines = edit_guide(&d, &changes, &|p| want.get(p).copied()).unwrap();
    let t = to_text(&lines);
    assert!(
        t.contains("Type 1 4 7") && t.contains("Press PROGRAM twice"),
        "{t}"
    );
}

#[test]
fn a_read_back_gives_the_changes_to_make() {
    let d = ctrl_descriptors::load("faderfox/uc4.ctrl.kdl").unwrap();
    let codec = ctrl_codec::Codec::new(&d);
    let mut device = Document::default();
    for a in 0x1C00u32..0x1C00 + 1280 {
        device.memory.set(a, &[0x20]); // channel 0, type 2 (CCAb)
    }
    let mut doc = device.clone();
    let path = ctrl_cdl::Path::parse("controls[0].group[0].encoder[3].channel").unwrap();
    doc.set(&codec, &d, &path, 4, &|_| None).unwrap();
    let changes = changes_between(&d, &codec, &doc, &device, "controls", 0).unwrap();
    assert_eq!(
        changes,
        [Change {
            path: "controls[0].group[0].encoder[3].channel".into(),
            want: 4,
            have: Some(0)
        }]
    );
    assert_eq!(
        d.edit("controls").unwrap().read.as_deref(),
        Some("read-setup")
    );
}
