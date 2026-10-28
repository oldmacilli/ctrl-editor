//! The UC4 write procedure against a scripted device, synthetic data only.

use std::collections::VecDeque;

use ctrl_cdl::Gate;
use ctrl_codec::{Bindings, Codec, Memory, Planned, Val};
use ctrl_engine::{EngineError, Event, Operator, Port, Run};

fn uc4() -> ctrl_cdl::Descriptor {
    ctrl_descriptors::load("faderfox/uc4.ctrl.kdl").unwrap()
}

/// Synthetic memory: every byte of 0x1480..0x8000 is `(address * 7) & 0x7F`.
fn memory() -> Memory {
    let mut m = Memory::default();
    for a in 0x1480u32..0x8000 {
        m.set(a, &[((a * 7) & 0x7F) as u8]);
    }
    m
}

/// A dump as the device sends it, built by hand (not through the codec):
/// header, one `4t 2h 1l` record per value, 30 zero bytes per chunk, end.
fn dump(kind: u8, pieces: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let rec = |t: u8, v: u8| [0x40 | t, 0x20 | v >> 4, 0x10 | (v & 0x0F)];
    let mut out = vec![0xF0, 0, 0, 0];
    out.extend(rec(1, 6));
    out.extend(rec(2, kind));
    out.extend(rec(3, 2));
    out.extend(rec(4, 3));
    for (a, data) in pieces {
        out.extend(rec(9, (a >> 8) as u8));
        out.extend(rec(10, (a & 0xFF) as u8));
        for b in data {
            out.extend(rec(13, *b));
        }
        let sum: u32 = data.iter().map(|b| u32::from(*b)).sum();
        out.extend(rec(11, (sum >> 8) as u8));
        out.extend(rec(12, (sum & 0xFF) as u8));
        out.extend([0u8; 30]);
    }
    out.extend([0x4F, 0x20, 0x16, 0xF7]);
    out
}

fn all_pieces(m: &Memory) -> Vec<(u32, Vec<u8>)> {
    (0x1480u32..0x8000)
        .step_by(64)
        .map(|a| (a, m.get(a, 64).unwrap()))
        .collect()
}

/// Setup `s` as the device's Sndc lays it out: names (32), fader 9 (64),
/// controls (20 × 64).
fn setup_pieces(m: &Memory, s: u32) -> Vec<(u32, Vec<u8>)> {
    let mut p = vec![(0x1480 + 32 * s, m.get(0x1480 + 32 * s, 32).unwrap())];
    p.push((0x1700 + 64 * s, m.get(0x1700 + 64 * s, 64).unwrap()));
    for k in 0..20 {
        let a = 0x1C00 + 0x500 * s + 64 * k;
        p.push((a, m.get(a, 64).unwrap()));
    }
    p
}

struct Device {
    inbox: VecDeque<Vec<u8>>,
    sent: Vec<Planned>,
}

impl Port for Device {
    fn send(&mut self, p: &Planned) -> Result<(), String> {
        self.sent.push(p.clone());
        Ok(())
    }
    fn recv(&mut self, _: u32) -> Result<Option<Vec<u8>>, String> {
        Ok(self.inbox.pop_front())
    }
}

struct Person {
    /// Answers to the gates, in order.
    answers: VecDeque<bool>,
    keep: bool,
    kept: Vec<Vec<u8>>,
    gates: Vec<String>,
}

impl Operator for Person {
    fn gate(&mut self, _: Gate, text: &str) -> bool {
        self.gates.push(text.into());
        self.answers.pop_front().unwrap_or(false)
    }
    fn note(&mut self, _: &str) {}
    fn restore_point(&mut self, _: &str, messages: &[Vec<u8>]) -> Result<(), String> {
        if !self.keep {
            return Err("no space".into());
        }
        self.kept.extend(messages.iter().cloned());
        Ok(())
    }
}

fn person(yes: usize) -> Person {
    Person {
        answers: std::iter::repeat_n(true, yes).collect(),
        keep: true,
        kept: Vec::new(),
        gates: Vec::new(),
    }
}

/// Setup 16 (index 15) with group 1 renamed to "dAtA".
fn edited(m: &Memory) -> Memory {
    let mut e = m.clone();
    e.set(0x1480 + 32 * 15, &[0x0D, 0x0A, 0x1C, 0x0A]);
    e
}

fn run<'d>(d: &'d ctrl_cdl::Descriptor, apply: bool) -> Run<'d> {
    let mut r = Run::new(d);
    r.apply = apply;
    r.vars.insert("setup".into(), Val::Int(15));
    r.load("dump", &dump(3, &all_pieces(&memory()))).unwrap();
    for (i, v) in [0x0D, 0x0A, 0x1C, 0x0A].iter().enumerate() {
        r.set(&format!("names[setup].group[0].char[{i}]"), *v)
            .unwrap();
    }
    r
}

#[test]
fn write_setup_sends_the_device_s_own_layout_byte_for_byte() {
    let d = uc4();
    let m = memory();
    let after = edited(&m);
    let mut device = Device {
        inbox: [
            dump(3, &all_pieces(&m)),           // backup
            dump(3, &all_pieces(&after)),       // verify: stored copy
            dump(2, &setup_pieces(&after, 15)), // verify: working copy
        ]
        .into(),
        sent: Vec::new(),
    };
    let mut op = person(5);
    let mut r = run(&d, true);
    r.run("write-setup", &mut device, &mut op).unwrap();
    // one message: kind 2, setup 16 as Sndc lays it out, edited
    assert_eq!(device.sent.len(), 1);
    assert_eq!(device.sent[0].frame, dump(2, &setup_pieces(&after, 15)));
    // the host writes join to the frame
    let joined: Vec<u8> = device.sent[0]
        .writes
        .iter()
        .flat_map(|w| w.bytes.clone())
        .collect();
    assert_eq!(joined, device.sent[0].frame);
    // the backup was kept before the send, and the gates named setup 16
    assert_eq!(op.kept, vec![dump(3, &all_pieces(&m))]);
    assert!(op.gates[1].contains("SE16") && op.gates[1].contains("rc00"));
    let order: Vec<&Event> = r
        .events
        .iter()
        .filter(|e| matches!(e, Event::RestorePoint(_) | Event::Sent { .. }))
        .collect();
    assert!(matches!(order[0], Event::RestorePoint(_)));
}

#[test]
fn no_at_the_arm_gate_sends_nothing() {
    let d = uc4();
    let m = memory();
    let mut device = Device {
        inbox: [dump(3, &all_pieces(&m))].into(),
        sent: Vec::new(),
    };
    let mut op = person(1); // yes to the backup, no to "arm receive"
    let err = run(&d, true)
        .run("write-setup", &mut device, &mut op)
        .unwrap_err();
    assert!(matches!(err, EngineError::Declined(_)));
    assert!(device.sent.is_empty());
}

#[test]
fn no_backup_no_write() {
    let d = uc4();
    // the device never sends the backup dump
    let mut device = Device {
        inbox: VecDeque::new(),
        sent: Vec::new(),
    };
    let err = run(&d, true)
        .run("write-setup", &mut device, &mut person(5))
        .unwrap_err();
    assert!(matches!(err, EngineError::Timeout(_)));
    assert!(device.sent.is_empty());
    // the backup arrives but cannot be kept
    let mut device = Device {
        inbox: [dump(3, &all_pieces(&memory()))].into(),
        sent: Vec::new(),
    };
    let mut op = person(5);
    op.keep = false;
    let err = run(&d, true)
        .run("write-setup", &mut device, &mut op)
        .unwrap_err();
    assert!(matches!(err, EngineError::Check(_)));
    assert!(device.sent.is_empty());
}

#[test]
fn a_read_back_that_differs_fails_the_write() {
    let d = uc4();
    let m = memory();
    let mut device = Device {
        // the device kept its old setup 16
        inbox: [dump(3, &all_pieces(&m)), dump(3, &all_pieces(&m))].into(),
        sent: Vec::new(),
    };
    let err = run(&d, true)
        .run("write-setup", &mut device, &mut person(5))
        .unwrap_err();
    assert!(
        matches!(err, EngineError::Check(ref e) if e.contains("verify")),
        "{err}"
    );
}

#[test]
fn a_dry_run_sends_nothing_but_shows_the_message() {
    let d = uc4();
    let m = memory();
    let after = edited(&m);
    let mut device = Device {
        inbox: [
            dump(3, &all_pieces(&m)),
            dump(3, &all_pieces(&after)),
            dump(2, &setup_pieces(&after, 15)),
        ]
        .into(),
        sent: Vec::new(),
    };
    let mut r = run(&d, false);
    r.run("write-setup", &mut device, &mut person(5)).unwrap();
    assert!(device.sent.is_empty());
    assert!(r.events.iter().any(
        |e| matches!(e, Event::Sent { applied: false, bytes, .. } if *bytes == dump(2, &setup_pieces(&after, 15)))
    ));
}

#[test]
fn restore_all_sends_the_backup_as_it_came_paced() {
    let d = uc4();
    let m = memory();
    let backup = dump(3, &all_pieces(&m));
    // the device holds something else; the document is the backup
    let mut now = m.clone();
    now.set(0x1480, &[0x7F]);
    let mut device = Device {
        inbox: [dump(3, &all_pieces(&now)), backup.clone()].into(),
        sent: Vec::new(),
    };
    let mut r = Run::new(&d);
    r.apply = true;
    r.load("dump", &backup).unwrap();
    let mut op = person(4);
    r.run("restore-all", &mut device, &mut op).unwrap();
    assert_eq!(device.sent.len(), 1);
    assert_eq!(device.sent[0].frame, backup);
    // one write per chunk (the last carries the end), 20 ms after each
    let writes = &device.sent[0].writes;
    assert_eq!(writes.len(), 430);
    assert!(writes.iter().all(|w| w.delay_ms == 20));
    assert_eq!(writes[429].bytes.last(), Some(&0xF7));
    // the restore point is what the device held before
    assert_eq!(op.kept, vec![dump(3, &all_pieces(&now))]);
    assert!(op.gates[1].contains("ALL 18 setups"));
}

#[test]
fn the_one_setup_message_refuses_kind_3() {
    let d = uc4();
    let codec = Codec::new(&d);
    let mut args = Bindings::new();
    args.insert("kind".into(), Val::Int(3));
    args.insert("fw-major".into(), Val::Int(2));
    args.insert("fw-minor".into(), Val::Int(3));
    args.insert("chunks".into(), Val::Bytes(Vec::new()));
    assert!(codec.encode("dump-out", &args).is_err());
}

/// With the owner's files (UC4_EVIDENCE_DIR = _ANALYZE_/OUT/faderfox-uc4),
/// the procedure rebuilds the measured patch file from the 13:28 backup.
#[test]
fn the_measured_patch_file_when_present() {
    let Ok(dir) = std::env::var("UC4_EVIDENCE_DIR") else {
        return;
    };
    let read = |p: &str| std::fs::read(format!("{dir}/{p}")).unwrap();
    let d = uc4();
    let backup = read("backup-20261028/uc4-all-setups-fw0203.syx");
    let patch = read("writetest-20261028/3-kind2-patch-SE16-name1-dAtA.syx");
    let mut device = Device {
        inbox: [
            backup.clone(),
            read("capture-20261028-c/sysex-2.syx"),
            patch.clone(),
        ]
        .into(),
        sent: Vec::new(),
    };
    let mut r = Run::new(&d);
    r.apply = true;
    r.vars.insert("setup".into(), Val::Int(15));
    r.load("dump", &backup).unwrap();
    for (i, v) in [0x0D, 0x0A, 0x1C, 0x0A].iter().enumerate() {
        r.set(&format!("names[15].group[0].char[{i}]"), *v).unwrap();
    }
    r.run("write-setup", &mut device, &mut person(5)).unwrap();
    assert_eq!(device.sent[0].frame, patch);
}

/// With the owner's files: restore-all of the 13:28 backup sends that file.
#[test]
fn restore_all_rebuilds_the_backup_file_when_present() {
    let Ok(dir) = std::env::var("UC4_EVIDENCE_DIR") else {
        return;
    };
    let backup = std::fs::read(format!("{dir}/backup-20261028/uc4-all-setups-fw0203.syx")).unwrap();
    let d = uc4();
    let mut device = Device {
        inbox: [backup.clone(), backup.clone()].into(),
        sent: Vec::new(),
    };
    let mut r = Run::new(&d);
    r.apply = true;
    r.load("dump", &backup).unwrap();
    r.run("restore-all", &mut device, &mut person(4)).unwrap();
    assert_eq!(device.sent[0].frame, backup);
}
