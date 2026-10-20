//! `ctrl-cli` against the repository's descriptors and synthetic captures.

use std::path::{Path, PathBuf};
use std::process::Command;

fn descriptor(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../descriptors")
        .join(rel)
}

fn run(args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_ctrl-cli"))
        .args(args)
        .output()
        .unwrap();
    let text =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    (out.status.success(), text)
}

fn capture(name: &str, text: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ctrl-cli-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    path
}

#[test]
fn check_passes_the_repository_descriptors() {
    let file = descriptor("korg/nanokontrol-studio.ctrl.kdl");
    let (ok, out) = run(&["check", file.to_str().unwrap()]);
    assert!(ok, "{out}");
    assert!(out.contains("send paths: 0 of 8"), "{out}");
}

#[test]
fn explain_prints_bytes_and_names_missing_values() {
    let file = descriptor("korg/nanokontrol-studio.ctrl.kdl");
    let f = file.to_str().unwrap();
    let (ok, out) = run(&["explain", f, "select-scene", "scene=3", "device-id=0"]);
    assert!(ok, "{out}");
    assert!(out.contains("F0 42 40 00 01 37 02 00 00 14 03 F7"), "{out}");
    assert!(out.contains("reply ack:"), "{out}");
    let (ok, out) = run(&["explain", f, "select-scene", "scene=3"]);
    assert!(!ok);
    assert!(out.contains("it takes: device-id, scene"), "{out}");
}

#[test]
fn trace_pairs_replies_and_prefers_literal_matches() {
    let file = descriptor("tecontrol/breath.ctrl.kdl");
    let cap = capture(
        "tec.txt",
        "# synthetic\n> 50 00 00 00 00 00 00 00\n< 00 00 00 00 00 00 00 00\n\
         > 00\n< 40 3F 00 00 01 00 00 00\n< 40 3F 00 00 00 00 00 00\n",
    );
    let (ok, out) = run(&["trace", file.to_str().unwrap(), cap.to_str().unwrap()]);
    assert!(ok, "{out}");
    // `50` is commit, not `{0x40 + s}` with s = 16.
    assert!(out.contains("    > commit\n"), "{out}");
    assert!(out.contains("< commit[ack]  ok  (reply to #0)"), "{out}");
    // The poll reply pairs with the poll, although read-meta's pattern has
    // more literal bytes.
    assert!(out.contains("< poll[values]  bite=63/0x3F"), "{out}");
    assert!(out.contains("(reply to #2)"), "{out}");
    // A second input report for one request is a repeat, not a guess.
    assert!(out.contains("(repeated reply to #2)"), "{out}");
    assert!(out.contains("5 of 5 matched, 0 ambiguous"), "{out}");
}

#[test]
fn trace_reads_json_lines_and_carries_the_device_id() {
    let file = descriptor("korg/nanokontrol-studio.ctrl.kdl");
    let cap = capture(
        "korg.jsonl",
        concat!(
            r#"{"dir": "in", "hex": "F0 7E 00 06 02 42 37 01 00 00 05 00 01 00 F7"}"#,
            "\n",
            r#"{"dir": "out", "hex": "F0 42 41 00 01 37 02 00 00 14 03 F7"}"#,
            "\n",
            r#"{"hex": "B8 11 00"}"#,
            "\n",
        ),
    );
    let (ok, out) = run(&["trace", file.to_str().unwrap(), cap.to_str().unwrap()]);
    assert!(ok, "{out}");
    assert!(out.contains("< identity  device-id=0"), "{out}");
    assert!(
        out.contains("(differs from the carried device-id=0)"),
        "{out}"
    );
    assert!(out.contains("no match"), "{out}");
    let (_, quiet) = run(&[
        "trace",
        file.to_str().unwrap(),
        "--matched",
        cap.to_str().unwrap(),
    ]);
    assert!(!quiet.contains("no match"), "{quiet}");
}
