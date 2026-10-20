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
        .decode_reply("commit-ack", &ack, &Bindings::new())
        .unwrap();
    assert_eq!(got["device-id"], int(0));
    assert_eq!(got["status"], int(0x21));
    assert!(codec.ok("commit-ack", &got).unwrap());
    // Known device id: checked.
    let err = codec
        .decode_reply("commit-ack", &ack, &args(&[("device-id", int(0x0F))]))
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
        .decode_reply("commit-ack", &other, &Bindings::new())
        .unwrap_err();
    assert!(err.to_string().contains("`model` = 0x137"), "{err}");
    // A data ack (23) is not a commit ack.
    let mut data_ack = ack;
    data_ack[9] = 0x23;
    let got = codec
        .decode_reply("commit-ack", &data_ack, &Bindings::new())
        .unwrap();
    assert!(!codec.ok("commit-ack", &got).unwrap());
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
