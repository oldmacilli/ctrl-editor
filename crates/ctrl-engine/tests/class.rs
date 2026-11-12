//! The engine refuses what the device class forbids (CDL §5.2), even when
//! a front end asks for it.

use ctrl_cdl::{parse_str, Gate};
use ctrl_codec::Planned;
use ctrl_engine::{EngineError, Operator, Port, Run};

struct Wire(usize);

impl Port for Wire {
    fn send(&mut self, _: &Planned) -> Result<(), String> {
        self.0 += 1;
        Ok(())
    }
    fn recv(&mut self, _: u32) -> Result<Option<Vec<u8>>, String> {
        Ok(Some(vec![0xF0, 0x02, 0xF7]))
    }
}

struct Yes;

impl Operator for Yes {
    fn gate(&mut self, _: Gate, _: &str) -> bool {
        true
    }
    fn note(&mut self, _: &str) {}
    fn restore_point(&mut self, _: &str, _: &[Vec<u8>]) -> Result<(), String> {
        Ok(())
    }
}

const BODY: &str = r#"
transport "aux" kind="midi" role="config"
message "get" transport="aux" status="measured" { send "F0 01 F7"; reply "F0 02 F7" }
procedure "read-all" kind="read" transport="aux" status="measured" { exchange "get" }
"#;

fn run(header: &str) -> (Result<(), EngineError>, usize) {
    let text = format!("cdl \"0.1\"\ndevice \"t.x\"\n{header}\n{BODY}");
    let d = parse_str("t.ctrl.kdl", &text).unwrap();
    let mut r = Run::new(&d);
    r.apply = true;
    let mut wire = Wire(0);
    let result = r.run("read-all", &mut wire, &mut Yes);
    (result, wire.0)
}

#[test]
fn a_declared_class_runs() {
    let (result, sent) = run("interaction \"active\" status=\"measured\"");
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(sent, 1);
}

#[test]
fn nothing_runs_without_a_class() {
    let (result, sent) = run("");
    assert!(
        matches!(result, Err(EngineError::Unsupported(_))),
        "{result:?}"
    );
    assert_eq!(sent, 0);
}

#[test]
fn nothing_runs_for_a_wrong_or_reserved_class() {
    for class in ["passive", "push-only", "bus-mixed"] {
        let (result, sent) = run(&format!("interaction \"{class}\""));
        assert!(
            matches!(result, Err(EngineError::Unsupported(_))),
            "{class}: {result:?}"
        );
        assert_eq!(sent, 0, "{class}");
    }
}
