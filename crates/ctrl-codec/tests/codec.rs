//! The codec against the repository's descriptors and small synthetic ones.

use std::path::Path;

use ctrl_cdl::{load, parse_str, Descriptor};
use ctrl_codec::*;

fn descriptor(rel: &str) -> Descriptor {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../descriptors")
        .join(rel);
    load(&path).unwrap_or_else(|e| panic!("{rel}:\n{e}"))
}

fn synthetic(body: &str) -> Descriptor {
    parse_str(
        "t.ctrl.kdl",
        &format!("cdl \"0.1\"\ndevice \"t.x\"\n{body}"),
    )
    .unwrap_or_else(|e| panic!("{e}"))
}

fn args(pairs: &[(&str, Val)]) -> Bindings {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect()
}

fn int(v: i64) -> Val {
    Val::Int(v)
}

#[test]
fn every_repository_vector_passes() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../descriptors");
    let mut passed = 0;
    for vendor in std::fs::read_dir(dir).unwrap() {
        for file in std::fs::read_dir(vendor.unwrap().path()).unwrap() {
            let path = file.unwrap().path();
            if !path.to_string_lossy().ends_with(".ctrl.kdl") {
                continue;
            }
            let d = load(&path).unwrap();
            for v in Codec::new(&d).run_vectors() {
                match v.outcome {
                    Outcome::Pass => passed += 1,
                    Outcome::Skipped(_) => {}
                    Outcome::Fail(why) => panic!("{}: {}: {why}", path.display(), v.name),
                }
            }
        }
    }
    assert!(passed >= 12, "{passed} vectors passed");
}

#[test]
fn korg_frame_checks_and_captures_the_device_id() {
    let d = descriptor("korg/nanokontrol-studio.ctrl.kdl");
    let codec = Codec::new(&d);
    let ack = [
        0xF0, 0x42, 0x40, 0x00, 0x01, 0x37, 0x01, 0x00, 0x00, 0x21, 0xF7,
    ];
    // Unknown device id: captured from 0x40 | device-id.
    let got = codec
        .decode_reply("commit-scene", &ack, &Bindings::new())
        .unwrap();
    assert_eq!(got["device-id"], int(0));
    assert_eq!(got["status"], int(0x21));
    assert!(codec.ok("commit-scene", &got).unwrap());
    // Known device id: checked.
    let err = codec
        .decode_reply("commit-scene", &ack, &args(&[("device-id", int(0x0F))]))
        .unwrap_err();
    assert!(
        matches!(err, CodecError::Mismatch { index: 2, .. }),
        "{err}"
    );
    // Another model (0x136) does not match this device's constant.
    let other = [
        0xF0, 0x42, 0x40, 0x00, 0x01, 0x36, 0x01, 0x00, 0x00, 0x21, 0xF7,
    ];
    let err = codec
        .decode_reply("commit-scene", &other, &Bindings::new())
        .unwrap_err();
    assert!(err.to_string().contains("`model` = 0x137"), "{err}");
    // A data ack (23) is not a commit ack.
    let mut data_ack = ack;
    data_ack[9] = 0x23;
    let got = codec
        .decode_reply("commit-scene", &data_ack, &Bindings::new())
        .unwrap();
    assert!(!codec.ok("commit-scene", &got).unwrap());
}

#[test]
fn korg_scene_write_round_trips_a_whole_scene() {
    let d = descriptor("korg/nanokontrol-studio.ctrl.kdl");
    let codec = Codec::new(&d);
    let scene: Vec<u8> = (0..560).map(|i| (i * 7 + 3) as u8).collect();
    let bytes = codec
        .encode(
            "scene-write",
            &args(&[("device-id", int(0)), ("data", Val::Bytes(scene.clone()))]),
        )
        .unwrap();
    assert_eq!(
        &bytes[..10],
        &[0xF0, 0x42, 0x40, 0x00, 0x01, 0x37, 0x01, 0x05, 0x00, 0x40]
    );
    assert_eq!(bytes.len(), 10 + 560 + 80 + 1);
    let back = codec
        .decode_send("scene-write", &bytes, &Bindings::new())
        .unwrap();
    assert_eq!(back["data"], Val::Bytes(scene));
}

#[test]
fn db1_reply_and_status() {
    let d = descriptor("oceanbeach/db1.ctrl.kdl");
    let codec = Codec::new(&d);
    let reply = [
        0xF0, 0x09, 0, 0, 0, 0x01, 0x03, 0x00, 0x04, 0x00, 0x10, 0x02, 0xF7,
    ];
    let got = codec
        .decode_reply("get-drawbar", &reply, &Bindings::new())
        .unwrap();
    assert_eq!(got["index"], int(4));
    assert_eq!(got["cc"], int(0x10));
    assert_eq!(got["channel"], int(2));
    assert!(codec.ok("get-drawbar", &got).unwrap());
    let mut failed = reply;
    failed[7] = 0x05; // Invalid CC
    let got = codec
        .decode_reply("get-drawbar", &failed, &Bindings::new())
        .unwrap();
    assert!(!codec.ok("get-drawbar", &got).unwrap());
    // Asking for drawbar 4 and getting drawbar 5 is a mismatch.
    let err = codec
        .decode_reply("get-drawbar", &reply, &args(&[("index", int(5))]))
        .unwrap_err();
    assert!(
        matches!(err, CodecError::Mismatch { index: 8, .. }),
        "{err}"
    );
    assert!(matches!(
        codec.decode_reply("get-drawbar", &reply[..12], &Bindings::new()),
        Err(CodecError::Truncated(_))
    ));
    // Drawbars are 0..=8, as in db1-protocol, both ways.
    assert!(codec
        .encode("get-drawbar", &args(&[("index", int(9))]))
        .is_err());
    let mut nine = reply;
    nine[8] = 9;
    assert!(codec
        .decode_reply("get-drawbar", &nine, &Bindings::new())
        .is_err());
    // A non-volatile write error (7) decodes and is not ok.
    let nv = [0xF0, 0x09, 0, 0, 0, 0x03, 0x03, 0x07, 0xF7];
    let got = codec
        .decode_reply("set-drawbar", &nv, &Bindings::new())
        .unwrap();
    assert_eq!(got["status"], int(7));
    assert!(!codec.ok("set-drawbar", &got).unwrap());
}

#[test]
fn tec_hid_frames_and_curve_pages() {
    let d = descriptor("tecontrol/breath.ctrl.kdl");
    let codec = Codec::new(&d);
    // The last curve page: start 30, two values, then 7F padding.
    let last = codec
        .encode(
            "write-curve-page",
            &args(&[
                ("s", int(0)),
                ("start", int(30)),
                ("curve", Val::Bytes(vec![0x10, 0x20])),
            ]),
        )
        .unwrap();
    assert_eq!(last, [0x40, 0x1E, 0x10, 0x20, 0x7F, 0x7F, 0x7F, 0x7F]);
    // USB bytes are 8-bit, and short sends are padded to the 8-byte frame.
    let meta = codec
        .encode(
            "write-meta",
            &args(&[
                ("s", int(1)),
                ("cc", int(2)),
                ("channel", int(0x10)),
                ("message", int(0)),
                ("param", int(9)),
                ("attack", int(200)),
                ("decay", int(255)),
            ]),
        )
        .unwrap();
    assert_eq!(meta, [0x21, 0x02, 0x10, 0x00, 0x09, 0xC8, 0xFF, 0x00]);
    assert_eq!(
        codec.encode("poll", &Bindings::new()).unwrap(),
        [0, 0, 0, 0, 0, 0, 0, 0]
    );
    let reply = [0x02, 0x10, 0x00, 0x09, 0x05, 0x06, 0x00, 0x00];
    let got = codec
        .decode_reply("read-meta", &reply, &Bindings::new())
        .unwrap();
    assert!(codec.ok("read-meta", &got).unwrap());
    let mut odd = reply;
    odd[3] = 0x08;
    let got = codec
        .decode_reply("read-meta", &odd, &Bindings::new())
        .unwrap();
    assert!(!codec.ok("read-meta", &got).unwrap());
    // The sensor number is recovered from 0x10 + s.
    let sent = codec
        .decode_send("read-meta", &[0x13, 0, 0, 0, 0, 0, 0, 0], &Bindings::new())
        .unwrap();
    assert_eq!(sent["s"], int(3));
}

/// A Roland-style DT1 and a Yamaha-style bulk dump, written only to test
/// the generic parts; not device evidence.
const GENERIC: &str = r#"
transport "m" kind="midi" role="config"
const "manufacturer" 0x41
const "model" 0x6A
message "dt1" transport="m" status="inferred" {
    send "F0 {manufacturer:midi-id} {device-id} {model} 12 {address:u14be} {value} {sum:roland(5,-2)} F7"
}
message "bulk" transport="m" status="inferred" {
    send "F0 43 {device-id} 7A {len:u14be} {data:raw} {sum:sum7(4,-2)} F7"
}
message "named" transport="m" status="inferred" {
    send "F0 7D {name:ascii(6)} F7"
}
"#;

#[test]
fn checksums_and_lengths() {
    let d = synthetic(GENERIC);
    let codec = Codec::new(&d);
    let dt1 = codec
        .encode(
            "dt1",
            &args(&[
                ("device-id", int(0x10)),
                ("address", int(0x0102)),
                ("value", int(0x40)),
            ]),
        )
        .unwrap();
    // body: 12 02 02 40, sum over 02 02 40 = 0x44, roland = 0x3C
    assert_eq!(
        dt1,
        [0xF0, 0x41, 0x10, 0x6A, 0x12, 0x02, 0x02, 0x40, 0x3C, 0xF7]
    );
    let back = codec.decode_send("dt1", &dt1, &Bindings::new()).unwrap();
    assert_eq!(back["address"], int(0x0102));
    let mut bad = dt1.clone();
    bad[8] = 0x3D;
    assert!(matches!(
        codec.decode_send("dt1", &bad, &Bindings::new()),
        Err(CodecError::Checksum {
            index: 8,
            expected: 0x3C,
            found: 0x3D
        })
    ));

    let bulk = codec
        .encode(
            "bulk",
            &args(&[("device-id", int(0)), ("data", Val::Bytes(vec![1, 2, 3]))]),
        )
        .unwrap();
    // The length counts the data, up to the checksum.
    assert_eq!(&bulk[..8], &[0xF0, 0x43, 0x00, 0x7A, 0x00, 0x03, 1, 2]);
    let back = codec.decode_send("bulk", &bulk, &Bindings::new()).unwrap();
    assert_eq!(back["data"], Val::Bytes(vec![1, 2, 3]));
    let mut long = bulk.clone();
    long[5] = 4;
    assert!(matches!(
        codec.decode_send("bulk", &long, &Bindings::new()),
        Err(CodecError::Length {
            expected: 3,
            found: 4
        })
    ));

    let named = codec
        .encode("named", &args(&[("name", Val::Bytes(b"Lead".to_vec()))]))
        .unwrap();
    assert_eq!(&named[2..8], b"Lead  ");
}

#[test]
fn reply_variants_first_match_wins() {
    let d = synthetic(
        r#"transport "m" kind="midi" role="config"
const "data-received" 0x23
pattern "ack" "F0 42 01 00 00 {status} F7"
message "info" transport="m" status="inferred" {
    send "F0 42 01 00 00 12 F7"
    reply pattern="ack" variant="ack" ok-when="status == data-received"
    reply "F0 42 02 00 00 42 {count} F7" variant="info"
}
test-vector "ack" message="info" reply=#true variant="ack" {
    args status=0x23
    bytes "F0 42 01 00 00 23 F7"
}
test-vector "info" message="info" reply=#true variant="info" {
    args count=5
    bytes "F0 42 02 00 00 42 05 F7"
}
test-vector "wrong" message="info" reply=#true variant="ack" {
    bytes "F0 42 02 00 00 42 05 F7"
}"#,
    );
    let codec = Codec::new(&d);
    let ack = codec
        .decode_variant("info", &[0xF0, 0x42, 1, 0, 0, 0x23, 0xF7], &Bindings::new())
        .unwrap();
    assert_eq!(ack.variant.as_deref(), Some("ack"));
    assert!(codec.ok_variant("info", Some("ack"), &ack.values).unwrap());
    let info = codec
        .decode_variant(
            "info",
            &[0xF0, 0x42, 2, 0, 0, 0x42, 5, 0xF7],
            &Bindings::new(),
        )
        .unwrap();
    assert_eq!(info.variant.as_deref(), Some("info"));
    // The info variant has no ok-when of its own and the message none.
    assert!(codec
        .ok_variant("info", Some("info"), &info.values)
        .unwrap());
    let err = codec
        .decode_variant("info", &[0xF0, 0x42, 3, 0xF7], &Bindings::new())
        .unwrap_err();
    assert!(
        err.to_string().contains("ack: ") && err.to_string().contains("info: "),
        "{err}"
    );
    let results: Vec<_> = codec.run_vectors().into_iter().map(|v| v.outcome).collect();
    assert_eq!(results[0], Outcome::Pass);
    assert_eq!(results[1], Outcome::Pass);
    assert!(matches!(&results[2], Outcome::Fail(why) if why.contains("expected Some(\"ack\")")));
}

#[test]
fn fill_takes_exactly_the_block_size() {
    let values: Bindings = [
        ("status".to_string(), int(0x23)),
        ("data".to_string(), Val::Bytes(vec![1, 2, 3])),
    ]
    .into_iter()
    .collect();
    assert_eq!(fill(&values, 3).unwrap(), [1, 2, 3]);
    assert!(matches!(
        fill(&values, 4),
        Err(CodecError::Length {
            expected: 4,
            found: 3
        })
    ));
    assert!(fill(&args(&[("status", int(1))]), 3).is_err());
}

#[test]
fn values_out_of_range_are_refused() {
    let d = synthetic(&format!(
        "{GENERIC}message \"wide\" transport=\"m\" status=\"inferred\" {{ send \"F0 {{id:midi-id}} {{v:u16le}} {{w}} F7\" }}"
    ));
    let codec = Codec::new(&d);
    let try_wide = |id: i64, v: i64, w: i64| {
        codec.encode(
            "wide",
            &args(&[("id", int(id)), ("v", int(v)), ("w", int(w))]),
        )
    };
    assert_eq!(
        try_wide(0x2029, 0x0102, 5).unwrap(),
        [0xF0, 0x00, 0x20, 0x29, 0x02, 0x01, 0x05, 0xF7]
    );
    assert!(try_wide(0x80, 0, 0).is_err(), "0x80: low byte not 7-bit");
    assert!(try_wide(0x8000, 0, 0).is_err());
    // u16le 0x0180 puts 0x80 on the wire, which MIDI cannot carry.
    let err = try_wide(1, 0x0180, 0).unwrap_err();
    assert!(err.to_string().contains("MIDI data bytes"), "{err}");
    // A plain placeholder on MIDI is u7.
    assert!(try_wide(1, 0, 0x80).is_err());
    assert!(matches!(
        codec.encode("wide", &args(&[("id", int(1)), ("v", int(0))])),
        Err(CodecError::UnknownName(n)) if n == "w"
    ));
    assert!(codec
        .encode(
            "named",
            &args(&[("name", Val::Bytes(b"Too long".to_vec()))])
        )
        .is_err());
}

#[test]
fn slices_need_a_pad_to_run_short() {
    let d = synthetic(
        "transport \"h\" kind=\"hid\" frame=8\nmessage \"p\" transport=\"h\" status=\"inferred\" { send \"40 {v[3]}\" }",
    );
    let codec = Codec::new(&d);
    assert_eq!(
        codec
            .encode("p", &args(&[("v", Val::Bytes(vec![1, 2, 3]))]))
            .unwrap(),
        [0x40, 1, 2, 3, 0, 0, 0, 0]
    );
    assert!(codec
        .encode("p", &args(&[("v", Val::Bytes(vec![1, 2]))]))
        .unwrap_err()
        .to_string()
        .contains("no `pad`"));
    assert!(codec
        .encode("p", &args(&[("v", Val::Bytes(vec![1, 2, 3, 4]))]))
        .is_err());
    // Non-zero bytes after the template are not padding.
    assert!(matches!(
        codec.decode_send("p", &[0x40, 1, 2, 3, 0, 0, 9, 0], &Bindings::new()),
        Err(CodecError::Trailing(4))
    ));
}

#[test]
fn explain_labels_each_run_of_bytes() {
    let d = descriptor("korg/nanokontrol-studio.ctrl.kdl");
    let codec = Codec::new(&d);
    let (bytes, spans) = codec
        .explain(
            "select-scene",
            &args(&[("scene", int(3)), ("device-id", int(0))]),
        )
        .unwrap();
    assert_eq!(hex(&bytes), "F0 42 40 00 01 37 02 00 00 14 03 F7");
    // The spans cover every byte once, in order.
    let mut next = 0;
    for s in &spans {
        assert_eq!(s.start, next);
        next += s.len;
    }
    assert_eq!(next, bytes.len());
    let model = spans.iter().find(|s| s.start == 3).unwrap();
    assert_eq!((model.len, model.region), (3, "prefix"));
    assert_eq!(model.label, "{model:midi-id} = 311 (0x137)");
    let scene = spans.iter().find(|s| s.start == 10).unwrap();
    assert_eq!(
        (scene.region, scene.label.as_str()),
        ("body", "{scene} = 3 (0x3)")
    );

    // A HID frame says where its padding starts.
    let d = synthetic(
        r#"transport "hid" kind="hid" frame=8
message "m" transport="hid" { send "50" }"#,
    );
    let (bytes, spans) = Codec::new(&d).explain("m", &Bindings::new()).unwrap();
    assert_eq!(bytes.len(), 8);
    let pad = spans.last().unwrap();
    assert_eq!((pad.start, pad.len, pad.region), (1, 7, "padding"));
}

#[test]
fn identify_finds_sends_and_replies() {
    let d = descriptor("korg/nanokontrol-studio.ctrl.kdl");
    let codec = Codec::new(&d);
    let sent = [
        0xF0, 0x42, 0x40, 0x00, 0x01, 0x37, 0x02, 0x00, 0x00, 0x14, 0x03, 0xF7,
    ];
    let found = codec.identify(&sent, Some(Direction::Out), &Bindings::new());
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].message, "select-scene");
    assert_eq!(found[0].values["scene"], int(3));
    // The literal bytes: F0, F7 and 02 00 00 14.
    assert_eq!(found[0].literals, 6);

    // An ack fits every message that uses the shared pattern; its ok-when
    // tells them apart.
    let ack = [
        0xF0, 0x42, 0x40, 0x00, 0x01, 0x37, 0x01, 0x00, 0x00, 0x21, 0xF7,
    ];
    let found = codec.identify(&ack, Some(Direction::In), &Bindings::new());
    let commit = found.iter().find(|m| m.message == "commit-scene").unwrap();
    assert_eq!(commit.ok, Some(true));
    let select = found.iter().find(|m| m.message == "select-scene").unwrap();
    assert_eq!(select.ok, Some(false));

    // A known device id must agree.
    let other = codec.identify(&sent, None, &args(&[("device-id", int(1))]));
    assert!(other.is_empty());
}

#[test]
fn args_out_of_range_neither_encode_nor_match() {
    let d = descriptor("tecontrol/breath.ctrl.kdl");
    let codec = Codec::new(&d);
    assert_eq!(
        codec.encode("read-meta", &args(&[("s", int(3))])).unwrap()[0],
        0x13
    );
    let e = codec
        .encode("read-meta", &args(&[("s", int(4))]))
        .unwrap_err();
    assert!(e.to_string().contains("outside 0..=3"), "{e}");
    // `10 00 …` is read-meta for sensor 0, not write-meta with s = -16.
    let frame = [0x10, 0, 0, 0, 0, 0, 0, 0];
    assert!(codec
        .decode_send("write-meta", &frame, &Bindings::new())
        .is_err());
    let found = codec.identify(&frame, Some(Direction::Out), &Bindings::new());
    let names: Vec<_> = found.iter().map(|m| m.message.as_str()).collect();
    assert_eq!(names, ["read-meta"]);

    // The KORG scene index follows each device's block count.
    let small = descriptor("korg/nanokontrol-studio.ctrl.kdl");
    let big = descriptor("korg/nanokey-studio.ctrl.kdl");
    let seven = args(&[("scene", int(7)), ("device-id", int(0))]);
    assert!(Codec::new(&small).encode("select-scene", &seven).is_err());
    assert!(Codec::new(&big).encode("select-scene", &seven).is_ok());
}

#[test]
fn korg_lengths_u21le_and_varlen7() {
    // Studio generation: a 3-byte 7-bit length, low first, then the command
    // (01 05 00 40 is length 641: the marker and 640 packed bytes).
    let d = synthetic(
        r#"const "manufacturer" 0x42
const "model" 0x137
transport "m" kind="midi" role="config" { capture "device-id" from="x"; }
sysex-frame "korg-native" { prefix "F0 {manufacturer:midi-id} {0x40 | device-id} {model:midi-id}"; suffix "F7"; }
message "scene-write" frame="korg-native" transport="m" { send "{len:u21le} 40 {data:packed7}"; }
message "request" frame="korg-native" transport="m" { send "{len:u21le} 12"; }
message "gen1-data" frame="korg-native" transport="m" { send "7F {len:varlen7} 51 {data:raw}"; }"#,
    );
    let codec = Codec::new(&d);
    let scene = codec
        .encode(
            "scene-write",
            &args(&[("device-id", int(0)), ("data", Val::Bytes(vec![0; 560]))]),
        )
        .unwrap();
    assert_eq!(
        &scene[..10],
        &[0xF0, 0x42, 0x40, 0x00, 0x01, 0x37, 0x01, 0x05, 0x00, 0x40]
    );
    let req = codec
        .encode("request", &args(&[("device-id", int(0))]))
        .unwrap();
    assert_eq!(
        req,
        [0xF0, 0x42, 0x40, 0x00, 0x01, 0x37, 0x01, 0x00, 0x00, 0x12, 0xF7]
    );

    // first generation: one length byte below 7F, else 7F 02 hi lo
    let short = codec
        .encode(
            "gen1-data",
            &args(&[("device-id", int(0)), ("data", Val::Bytes(vec![1; 10]))]),
        )
        .unwrap();
    assert_eq!(&short[6..9], &[0x7F, 11, 0x51]);
    let long = codec
        .encode(
            "gen1-data",
            &args(&[("device-id", int(0)), ("data", Val::Bytes(vec![1; 200]))]),
        )
        .unwrap();
    assert_eq!(&long[6..11], &[0x7F, 0x7F, 0x02, 0x01, 0x49]); // 201 = 1 * 128 + 73
    let back = codec
        .decode_send("gen1-data", &long, &Bindings::new())
        .unwrap();
    assert_eq!(back["data"], Val::Bytes(vec![1; 200]));
}

#[test]
fn reply_arg_ranges_separate_status_bytes() {
    let d = synthetic(
        r#"transport "t" kind="midi" role="performance"
message "cc" transport="t" { arg "ch" range="0..=15"; reply "{0xB0 + ch} {cc} {value}"; }
message "note-on" transport="t" { arg "ch" range="0..=15"; reply "{0x90 + ch} {note} {velocity}"; }
message "sensing" transport="t" { reply "FE"; }"#,
    );
    let codec = Codec::new(&d);
    let names = |b: &[u8]| -> Vec<String> {
        codec
            .identify(b, Some(Direction::In), &Bindings::new())
            .into_iter()
            .map(|m| m.message)
            .collect()
    };
    assert_eq!(names(&[0xB8, 0x07, 0x7F]), ["cc"]);
    assert_eq!(names(&[0x95, 0x40, 0x64]), ["note-on"]);
    assert_eq!(names(&[0xFE]), ["sensing"]);
}

#[test]
fn long_sysex_is_split_for_the_host_and_by_the_protocol() {
    let d = synthetic(
        r#"transport "linux" kind="midi" role="config" sysex-limit=4096 segment-delay=5
message "dump" transport="linux" kind="write" {
    send "F0 7D 01 {data:raw} F7"
    segment message="dump-part" over="data" size=2000
}
message "dump-part" transport="linux" kind="write" { send "F0 7D 02 {part} {parts} {data:raw} F7"; }
message "blob" transport="linux" kind="write" { send "F0 7D 03 {data:raw} F7"; }"#,
    );
    let report = ctrl_cdl::validate(&d);
    assert_eq!(report.errors().count(), 0, "{:?}", report.findings);
    let codec = Codec::new(&d);
    let data = |n: usize| Val::Bytes((0..n).map(|i| (i % 128) as u8).collect());

    // fits: one message, one write
    let small = codec.plan("dump", &args(&[("data", data(100))])).unwrap();
    assert_eq!(small.len(), 1);
    assert_eq!(small[0].message, "dump");
    assert_eq!(small[0].writes.len(), 1);

    // too long for 4096: the device's own parts, each under the limit
    let big = codec.plan("dump", &args(&[("data", data(5000))])).unwrap();
    assert_eq!(big.len(), 3);
    assert!(big
        .iter()
        .all(|p| p.message == "dump-part" && p.frame.len() <= 4096));
    assert_eq!(&big[2].frame[..5], &[0xF0, 0x7D, 0x02, 2, 3]);

    // no protocol form: the host writes are split, the wire is one message
    let blob = codec.plan("blob", &args(&[("data", data(9000))])).unwrap();
    assert_eq!(blob.len(), 1);
    assert_eq!(blob[0].writes.len(), 3);
    assert!(blob[0]
        .writes
        .iter()
        .all(|w| w.bytes.len() <= 4096 && w.delay_ms == 5));
    let joined: Vec<u8> = blob[0]
        .writes
        .iter()
        .flat_map(|w| w.bytes.clone())
        .collect();
    assert_eq!(joined, blob[0].frame);
}

#[test]
fn korg_scene_write_plans_like_korg_midi_midir() {
    // korg-midi-midir: SysEx over 256 bytes in 256-byte chunks, 5 ms apart
    let d = descriptor("korg/nanokontrol-studio.ctrl.kdl");
    let codec = Codec::new(&d);
    let plan = codec
        .plan(
            "scene-write",
            &args(&[("device-id", int(0)), ("data", Val::Bytes(vec![0; 560]))]),
        )
        .unwrap();
    assert_eq!(plan.len(), 1);
    assert_eq!(plan[0].frame.len(), 651);
    let sizes: Vec<_> = plan[0]
        .writes
        .iter()
        .map(|w| (w.bytes.len(), w.delay_ms))
        .collect();
    assert_eq!(sizes, [(256, 5), (256, 5), (139, 5)]);
    // a short request stays one write
    let req = codec
        .plan("global-data-request", &args(&[("device-id", int(0))]))
        .unwrap();
    assert_eq!(req[0].writes.len(), 1);
}

#[test]
fn typed_args_refuse_values_outside_their_type() {
    // found by the DB-1 conformance test: polarity 2 or channel 16 encoded
    // although db1-protocol refuses them
    let d = descriptor("oceanbeach/db1.ctrl.kdl");
    let codec = Codec::new(&d);
    let set = |polarity: i64, channel: i64| {
        codec.encode(
            "set-drawbar",
            &args(&[
                ("index", int(0)),
                ("polarity", int(polarity)),
                ("cc", int(1)),
                ("channel", int(channel)),
            ]),
        )
    };
    assert!(set(1, 15).is_ok());
    assert!(matches!(set(2, 0), Err(CodecError::Range(_))));
    assert!(matches!(set(0, 16), Err(CodecError::Range(_))));
    // an unknown status code in a reply does not decode, as in db1-protocol
    let bad = [
        0xF0, 0x09, 0, 0, 0, 0x01, 0x03, 0x0A, 0x03, 0x00, 0x14, 0x01, 0xF7,
    ];
    assert!(codec
        .decode_variant("get-drawbar", &bad, &Bindings::new())
        .is_err());
    let ok = [
        0xF0, 0x09, 0, 0, 0, 0x01, 0x03, 0x00, 0x03, 0x00, 0x14, 0x01, 0xF7,
    ];
    assert!(codec
        .decode_variant("get-drawbar", &ok, &Bindings::new())
        .is_ok());
}

#[test]
fn nib3_records_round_trip_and_check_their_markers() {
    // Faderfox: every byte goes out as `4t 2h 1l`; synthetic values.
    let d = synthetic(
        r#"transport "m" kind="midi" role="config"
message "chunk" transport="m" {
    send "F0 00 00 00 {dev:nib3(1)} {data:nib3-bytes(13)} {end:nib3(15)} F7"
}"#,
    );
    let codec = Codec::new(&d);
    let bytes = codec
        .encode(
            "chunk",
            &args(&[
                ("dev", int(6)),
                ("data", Val::Bytes(vec![0x00, 0xA5, 0xFF])),
                ("end", int(6)),
            ]),
        )
        .unwrap();
    assert_eq!(
        bytes,
        [
            0xF0, 0x00, 0x00, 0x00, 0x41, 0x20, 0x16, 0x4D, 0x20, 0x10, 0x4D, 0x2A, 0x15, 0x4D,
            0x2F, 0x1F, 0x4F, 0x20, 0x16, 0xF7
        ]
    );
    let back = codec
        .decode_send("chunk", &bytes, &Bindings::new())
        .unwrap();
    assert_eq!(back["dev"], int(6));
    assert_eq!(back["data"], Val::Bytes(vec![0x00, 0xA5, 0xFF]));

    // a wrong tag, a missing marker and a broken record are refused
    let mut bad = bytes.clone();
    bad[7] = 0x4C;
    assert!(codec.decode_send("chunk", &bad, &Bindings::new()).is_err());
    let mut bad = bytes.clone();
    bad[8] = 0x0A;
    assert!(codec.decode_send("chunk", &bad, &Bindings::new()).is_err());
    let mut short = bytes.clone();
    short.remove(9);
    assert!(codec
        .decode_send("chunk", &short, &Bindings::new())
        .is_err());
    // one record holds one byte
    assert!(codec
        .encode(
            "chunk",
            &args(&[
                ("dev", int(256)),
                ("data", Val::Bytes(vec![])),
                ("end", int(6))
            ]),
        )
        .is_err());
}

#[test]
fn records_split_address_and_check_their_sums() {
    // a run of addressed records in one SysEx (CDL §8.3), synthetic bytes
    let d = synthetic(
        r#"transport "m" kind="midi" role="config"
pattern "rec" "{hi:nib3(9)} {lo:nib3(10)} {data:nib3-bytes(13)} {s-hi:nib3(11)} {s-lo:nib3(12)} 00 00 00"
block "mem" address=0x1000 size=8 chunk=4 count=2 { field "x" offset=0 type="u8" }
message "out" transport="m" kind="write" {
    send "F0 00 {recs:raw} F7"
    records "recs" pattern="rec" address="(hi << 8) | lo" data="data" {
        derive "hi" "address >> 8"
        derive "lo" "address & 0xFF"
        derive "s-hi" "(data.sum >> 8) & 0xFF"
        derive "s-lo" "data.sum & 0xFF"
    }
}"#,
    );
    let codec = Codec::new(&d);
    // block element 1 lives at 0x1008, in chunks of 4
    let pieces = codec
        .block_pieces("mem", 1, &[0xFF, 0xFF, 1, 2, 3, 4, 5, 6])
        .unwrap();
    assert_eq!(
        pieces,
        vec![(0x1008, vec![0xFF, 0xFF, 1, 2]), (0x100C, vec![3, 4, 5, 6])]
    );
    let recs = codec.records_for("out", "recs", &pieces).unwrap();
    let bytes = codec.encode_records("out", "recs", &recs).unwrap();
    // first record: 49 21 10 | 4A 20 18 | data | sum 0x0201 | 00 00 00
    assert_eq!(&bytes[..6], &[0x49, 0x21, 0x10, 0x4A, 0x20, 0x18]);
    assert_eq!(
        &bytes[18..27],
        &[0x4B, 0x20, 0x12, 0x4C, 0x20, 0x11, 0, 0, 0]
    );
    let back = codec.decode_records("out", "recs", &bytes).unwrap();
    assert_eq!(back.len(), 2);
    assert_eq!(back[1].address, Some(0x100C));
    let mem = Memory::from_records(&back);
    assert_eq!(
        codec.block_bytes(&mem, "mem", 1),
        Some(vec![0xFF, 0xFF, 1, 2, 3, 4, 5, 6])
    );
    assert_eq!(codec.block_bytes(&mem, "mem", 0), None);

    // a wrong sum is refused, and so is a given value that disagrees
    let mut bad = bytes.clone();
    bad[23] = 0x12;
    assert!(codec.decode_records("out", "recs", &bad).is_err());
    let mut wrong = recs[0].clone();
    wrong.insert("s-lo".into(), int(0));
    assert!(codec.encode_records("out", "recs", &[wrong]).is_err());
}

#[test]
fn paced_records_go_out_one_write_each() {
    let d = synthetic(
        r#"transport "m" kind="midi" role="config"
pattern "rec" "{a:nib3(9)} {data:nib3-bytes(13)}"
message "out" transport="m" kind="write" {
    send "F0 01 {recs:raw} F7"
    records "recs" pattern="rec" address="a" data="data" pace=20 { derive "a" "address" }
}"#,
    );
    let codec = Codec::new(&d);
    let recs = codec
        .records_for("out", "recs", &[(1, vec![7]), (2, vec![8, 9])])
        .unwrap();
    let array = codec.encode_records("out", "recs", &recs).unwrap();
    let plan = codec
        .plan("out", &args(&[("recs", Val::Bytes(array))]))
        .unwrap();
    let writes = &plan[0].writes;
    assert_eq!(writes.len(), 2);
    assert_eq!(
        writes[0].bytes,
        [0xF0, 0x01, 0x49, 0x20, 0x11, 0x4D, 0x20, 0x17]
    );
    assert_eq!(writes[0].delay_ms, 20);
    assert_eq!(writes[1].bytes.last(), Some(&0xF7));
    let joined: Vec<u8> = writes.iter().flat_map(|w| w.bytes.clone()).collect();
    assert_eq!(joined, plan[0].frame);
}
