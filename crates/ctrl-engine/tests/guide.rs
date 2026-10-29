//! Guides (CDL §17) from the repository's descriptors: edit guides for a
//! device without a dump (UC-33) and guide steps inside a procedure.

use std::collections::HashMap;

use ctrl_cdl::Gate;
use ctrl_codec::Planned;
use ctrl_engine::guide::to_text;
use ctrl_engine::{edit_guide, Change, EngineError, Event, Line, Operator, Port, Run, Wait};

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
            "Move F1, or press CONTROL SELECT, then type 0 1",
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
