//! The validator: the repository's descriptors pass, and each rule fires on
//! a small descriptor that breaks it.

use std::path::Path as FsPath;

use ctrl_cdl::*;

/// Validation errors (not warnings or notes) for an in-memory descriptor,
/// as `line:col: message`.
fn errors(body: &str) -> Vec<String> {
    findings(body, Severity::Error)
}

fn findings(body: &str, severity: Severity) -> Vec<String> {
    let text = format!("cdl \"0.1\"\ndevice \"t.x\"\n{body}");
    let d = parse_str("t.ctrl.kdl", &text).unwrap_or_else(|e| panic!("{e}"));
    validate(&d)
        .findings
        .into_iter()
        .filter(|f| f.severity == severity)
        .map(|f| format!("{}:{}: {}", f.error.line, f.error.col, f.error.message))
        .collect()
}

fn assert_one(found: Vec<String>, needle: &str) {
    assert_eq!(found.len(), 1, "{found:#?}");
    assert!(found[0].contains(needle), "{found:#?}");
}

const DB1_LIKE: &str = r#"
transport "aux" kind="midi" role="config" status="measured"
sysex-frame "f" status="measured" { prefix "F0 09"; suffix "F7" }
block "bar" count=9 status="measured" {
    field "cc" type="u7"
}
message "get" frame="f" transport="aux" status="measured" { send "00 {i}"; reply "01 {cc}" }
message "set" frame="f" transport="aux" kind="write" status="measured" { send "02 {i} {cc}"; reply "03" }
procedure "read" kind="read" {
    repeat "i" "0..=8" { exchange "get" into="bar[i]" }
}
"#;

#[test]
fn repository_descriptors_pass() {
    let dir = FsPath::new(env!("CARGO_MANIFEST_DIR")).join("../../descriptors");
    for vendor in std::fs::read_dir(dir).unwrap() {
        for file in std::fs::read_dir(vendor.unwrap().path()).unwrap() {
            let path = file.unwrap().path();
            if !path.to_string_lossy().ends_with(".ctrl.kdl") {
                continue;
            }
            let d = load(&path).unwrap();
            let report = validate(&d);
            let errors: Vec<_> = report.errors().map(ToString::to_string).collect();
            assert!(
                errors.is_empty(),
                "{}:\n{}",
                path.display(),
                errors.join("\n")
            );
            let warnings: Vec<_> = report
                .findings
                .iter()
                .filter(|f| f.severity == Severity::Warning)
                .collect();
            assert!(warnings.is_empty(), "{}: {warnings:#?}", path.display());
        }
    }
}

#[test]
fn a_correct_write_passes() {
    let body = format!(
        "{DB1_LIKE}procedure \"write\" kind=\"write\" {{\n    gate \"confirm\"\n    restore-point \"bar\"\n    repeat \"i\" \"0..=8\" {{ exchange \"set\" from=\"bar[i]\" }}\n    verify \"read\" compare=\"bar\"\n}}\n"
    );
    assert_eq!(errors(&body), Vec::<String>::new());
}

#[test]
fn references_must_resolve() {
    assert_one(
        errors(r#"message "m" transport="usb" status="measured""#),
        "unknown transport `usb`",
    );
    assert_one(
        errors(r#"message "m" frame="korg" status="measured""#),
        "unknown sysex-frame `korg`",
    );
    assert_one(
        errors(&format!(
            "{DB1_LIKE}procedure \"p\" {{ exchange \"gett\" }}"
        )),
        "unknown message `gett`",
    );
    assert_one(
        errors(&format!("{DB1_LIKE}procedure \"p\" {{ call \"nope\" }}")),
        "unknown procedure `nope`",
    );
    assert_one(
        errors(&format!(
            "{DB1_LIKE}procedure \"p\" {{ exchange \"get\" into=\"bars[0]\" }}"
        )),
        "unknown block `bars`",
    );
    assert_one(
        errors(&format!(
            "{DB1_LIKE}procedure \"p\" {{ exchange \"get\" into=\"bar[0].chan\" }}"
        )),
        "no item `chan`",
    );
    assert_one(
        errors(&format!(
            "{DB1_LIKE}procedure \"p\" {{ exchange \"get\" into=\"bar[0].cc[1]\" }}"
        )),
        "`cc` is not an array",
    );
    assert_one(
        errors("block \"g\" { field \"x\" type=\"u7\" }\npanel { control \"c\" kind=\"knob\" bind=\"g[0]\" }"),
        "has no `count`",
    );
    assert_one(
        errors("block \"g\" { field \"x\" type=\"channel\" }"),
        "unknown type `channel`",
    );
    assert_one(
        errors("block \"g\" { array \"c\" count=4 driver=\"curve\" }"),
        "driver `curve` has no `driver` node",
    );
    assert_one(
        errors("driver \"d\" provides=\"procedure:gone\""),
        "provides unknown procedure `gone`",
    );
    assert_one(
        errors("test-vector \"v\" { bytes \"F0 F7\" }"),
        "exactly one `message=` or `block=`",
    );
}

#[test]
fn sizes_and_ranges() {
    assert_one(
        errors("block \"s\" size=16 { field \"name\" offset=4 type=\"ascii(16)\" }"),
        "ends at byte 20, past the block size 16",
    );
    assert_one(
        errors(
            "block \"s\" size=8 { group \"g\" offset=6 { field \"v\" offset=1 type=\"u14le\" } }",
        ),
        "ends at byte 9",
    );
    assert_one(
        errors("types { int \"ch\" storage=\"u7\" range=\"0..=200\" }"),
        "does not fit u7",
    );
    assert_one(
        errors("types { enum \"e\" { value 1 \"a\"; value 1 \"b\" } }"),
        "has value 1 twice",
    );
    assert_one(
        errors("block \"b\" { field \"f\" type=\"u7\" range=\"0..=300\" }"),
        "outside its type",
    );
    assert_one(
        errors("block \"b\" { field \"f\" type=\"u7\" fixed=200 }"),
        "fixed 200 is outside its range",
    );
    assert_one(
        errors("block \"b\" { field \"f\" type=\"u7\"; field \"f\" type=\"u8\" }"),
        "two items named `f`",
    );
}

#[test]
fn port_matches_must_not_overlap() {
    // "nanoKEY Studio" also matches the port "nanoKEY Studio Bluetooth".
    assert_one(
        errors(
            r#"transport "usb" kind="midi" port-match="nanoKEY Studio" role="config"
transport "ble" kind="midi" port-match="nanoKEY Studio Bluetooth" role="config""#,
        ),
        "input and output match \"nanoKEY Studio Bluetooth\" overlaps \"nanoKEY Studio\"",
    );
    assert_eq!(
        errors(
            r#"transport "usb" kind="midi" port-match-in="nanoKEY Studio KEYBOARD/CTRL" port-match-out="nanoKEY Studio CTRL" role="config"
transport "ble" kind="midi" port-match="nanoKEY Studio Bluetooth" role="config""#,
        ),
        Vec::<String>::new()
    );
    assert_one(
        errors(
            r#"transport "a" kind="midi" role="config" preferred=#true
transport "b" kind="midi" role="config" preferred=#true"#,
        ),
        "only one transport may be preferred",
    );
    assert_one(errors(r#"transport "h" kind="hid""#), "needs `frame=`");
}

#[test]
fn status_rules() {
    let unknown = DB1_LIKE.replace(
        r#"message "get" frame="f" transport="aux" status="measured""#,
        r#"message "get" frame="f" transport="aux" status="unknown""#,
    );
    // A runnable procedure that sends an unknown message is an error ...
    assert_one(errors(&unknown), "sends message `get`, which is unknown");
    // ... and marking the procedure unknown disables it instead.
    let disabled = unknown.replace(
        r#"procedure "read" kind="read" {"#,
        r#"procedure "read" kind="read" status="unknown" {"#,
    );
    assert_eq!(errors(&disabled), Vec::<String>::new());
    let notes = findings(&disabled, Severity::Note);
    assert!(
        notes.iter().any(|n| n.contains("message `get` is unknown")),
        "{notes:#?}"
    );
    assert!(
        notes
            .iter()
            .any(|n| n.contains("procedure `read` is unknown")),
        "{notes:#?}"
    );

    // Writes need measured bytes.
    let decoded = format!(
        "{}procedure \"w\" kind=\"write\" {{ gate \"confirm\"; restore-point \"bar\"; exchange \"set\" i=\"0\" from=\"bar[0]\" }}\n",
        DB1_LIKE.replace(
            r#"kind="write" status="measured""#,
            r#"kind="write" status="decoded""#
        )
    );
    assert_one(errors(&decoded), "decoded only; writes need measured bytes");

    // Sent messages carry a status.
    assert_one(
        errors(&DB1_LIKE.replace(
            r#"message "get" frame="f" transport="aux" status="measured""#,
            r#"message "get" frame="f" transport="aux""#,
        )),
        "message `get` is sent by procedure `read` but has no `status`",
    );

    // Inferred data on a send path needs opt-in: a note, not an error.
    let inferred = format!(
        "{}procedure \"w\" kind=\"write\" {{ gate \"confirm\"; restore-point \"bar\"; exchange \"set\" from=\"bar[0]\" }}\n",
        DB1_LIKE.replace(r#"block "bar" count=9 status="measured""#, r#"block "bar" count=9 status="inferred""#)
    );
    assert_eq!(errors(&inferred), Vec::<String>::new());
    assert!(findings(&inferred, Severity::Note)
        .iter()
        .any(|n| n.contains("`bar[0]`, which is inferred")));

    // Every `owns` entry carries a status.
    assert_one(
        errors(r#"driver "d" { owns "x" bytes="01 02" }"#),
        "`owns \"x\"` needs a `status`",
    );
}

#[test]
fn write_policy() {
    assert_eq!(
        errors(&format!(
            "{DB1_LIKE}procedure \"w\" kind=\"write\" {{ exchange \"set\" from=\"bar[0]\" }}"
        )),
        [
            "14:1: procedure `w` writes without a `gate` before it",
            "14:1: procedure `w` writes without a `restore-point` before it"
        ]
    );
    // A gate after the first send does not count.
    assert_eq!(
        errors(&format!(
            "{DB1_LIKE}procedure \"w\" kind=\"write\" {{ exchange \"set\"; gate \"confirm\"; restore-point \"bar\" }}"
        ))
        .len(),
        2
    );
    assert_one(
        errors(&format!(
            "{DB1_LIKE}message \"c\" transport=\"aux\" kind=\"commit\" status=\"measured\" {{ send \"F0 50 F7\" }}\nprocedure \"p\" kind=\"commit\" {{ gate \"confirm\"; exchange \"c\" }}"
        )),
        "commits without `gate \"arm\"`",
    );
    assert_one(
        errors(&format!(
            "{DB1_LIKE}procedure \"p\" kind=\"read\" {{ exchange \"set\" }}"
        )),
        "procedure `p` is a read but sends `set`, a write",
    );
    assert_one(
        errors(&format!(
            "{DB1_LIKE}procedure \"w\" kind=\"write\" {{ gate \"confirm\"; restore-point \"bar\"; exchange \"set\"; verify \"w\" }}"
        )),
        "verify runs `w`, which is not a read",
    );
}

#[test]
fn wire_bytes() {
    assert_one(
        errors(&format!(
            "{DB1_LIKE}message \"bad\" frame=\"f\" transport=\"aux\" status=\"measured\" {{ send \"02 80\" }}"
        )),
        "byte 80 inside SysEx",
    );
    // A message without a frame may carry its own F0 ... F7.
    assert_eq!(
        errors(
            r#"transport "m" kind="midi" role="config"
message "id" transport="m" status="measured" { send "F0 7E 7F 06 01 F7" }"#
        ),
        Vec::<String>::new()
    );
    let hid = "transport \"hid\" kind=\"hid\" frame=8\n";
    assert_one(
        errors(&format!(
            "{hid}message \"w\" transport=\"hid\" status=\"measured\" {{ send \"{{0x40 + s}} {{start}} {{curve[7]}}\" }}"
        )),
        "send: 9 bytes, the HID frame is 8",
    );
    assert_one(
        errors(&format!(
            "{hid}message \"r\" transport=\"hid\" status=\"measured\" {{ send \"10\"; reply \"{{cc}} 00\" }}"
        )),
        "reply: 2 bytes, the HID frame is 8",
    );
    assert_eq!(
        errors(&format!(
            "{hid}message \"w\" transport=\"hid\" status=\"measured\" {{ send \"{{0x40 + s}} {{start}} {{curve[6] pad=7F}}\" }}"
        )),
        Vec::<String>::new()
    );
}

#[test]
fn scan_rules() {
    assert_eq!(
        errors(&DB1_LIKE.replace(
            r#"transport "aux" kind="midi" role="config""#,
            r#"transport "aux" kind="midi" role="config" scan="get""#
        )),
        Vec::<String>::new()
    );
    assert_one(
        errors(&DB1_LIKE.replace(
            r#"transport "aux" kind="midi" role="config""#,
            r#"transport "aux" kind="midi" role="config" scan="set""#,
        )),
        "the scan message `set` must be a read",
    );
    assert_one(
        errors(&DB1_LIKE.replace(
            r#"transport "aux" kind="midi" role="config""#,
            r#"transport "aux" kind="midi" role="config" scan="ping""#,
        )),
        "scan with unknown message `ping`",
    );
}

#[test]
fn firmware_rules() {
    assert_eq!(
        errors(r#"firmware status="measured""#),
        [
            "3:1: firmware needs `recovery` text (§12)",
            "3:1: firmware needs an `image` (§12)"
        ]
    );
    assert_eq!(errors(r#"firmware status="unknown""#), Vec::<String>::new());
    assert_one(
        errors(
            r#"transport "aux" kind="midi" role="config"
message "erase" transport="aux" kind="commit" status="unknown"
driver "img" status="decoded"
firmware status="decoded" {
    image format="x" driver="img"
    stage "erase" { exchange "erase" }
    recovery "Run it again."
}"#,
        ),
        "firmware stage `erase` sends message `erase`, which is unknown",
    );
}
