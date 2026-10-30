//! `ctrl-cli`: check, explain and trace CDL descriptors (CDL §16).
//!
//!     ctrl-cli check <file>...
//!     ctrl-cli explain <file> <message> [name=value]...
//!     ctrl-cli trace <file> <capture>...
//!     ctrl-cli run <file> <procedure> [options]
//!     ctrl-cli guide <file> <path>=<value>...

mod run;

use std::collections::BTreeSet;
use std::process::ExitCode;

use ctrl_cdl::{Descriptor, Expr, Piece, Status, Template};
use ctrl_codec::{hex, Bindings, Codec, Direction, Match, Outcome, Val};

const USAGE: &str = "\
usage: ctrl-cli check <file>...
       ctrl-cli explain <file> <message> [name=value]...
       ctrl-cli trace <file> [--matched] <capture>...
       ctrl-cli schema > docs/cdl-schema.md
       ctrl-cli run <file> <procedure> [var=value] [options]
       ctrl-cli guide <file> <path>=<value>...

check    load and validate each descriptor, check its images, run its test
         vectors and list send paths that are not measured or decoded
explain  print the bytes a message would send, piece by piece (dry run).
         Values: 12, 0x0F, hex:AA55, text:Name, @file (bytes)
trace    decode captures with the descriptor. A capture is text, one frame
         per line, optionally prefixed `>` (to the device) or `<` (from
         it); JSON lines with a \"hex\" field; HID trace logs
         (`OUT … data=…`, `IN … data=…`); or a binary .syx/.bin MIDI
         stream. A reply is matched to the message sent just before it
         when that message accepts it. --matched hides frames that match
         nothing
schema   print docs/cdl-schema.md: every CDL node, its arguments, properties
         and children
guide    print the key presses that set these fields of one stored element
         on the device by hand (CDL §17.4), e.g.
         preset[2].control[0].assign=12. The values given are also what
         `applies` and `when` read for the other fields";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("check") if args.len() > 1 => check(&args[1..]),
        Some("explain") if args.len() > 2 => explain(&args[1], &args[2], &args[3..]),
        Some("trace") if args.len() > 2 => {
            let matched_only = args[2..].iter().any(|a| a == "--matched");
            let rest: Vec<String> = args[2..]
                .iter()
                .filter(|a| *a != "--matched")
                .cloned()
                .collect();
            trace(&args[1], &rest, matched_only)
        }
        Some("run") if args.len() > 2 => {
            load(&args[1]).and_then(|d| run::run(&d, &args[2], &args[3..]))
        }
        Some("guide") if args.len() > 2 => load(&args[1]).and_then(|d| guide(&d, &args[2..])),
        Some("schema") if args.len() == 1 => {
            print!("{}", ctrl_cdl::schema::markdown());
            Ok(true)
        }
        Some("-h" | "--help" | "help") => {
            println!("{USAGE}\n{}", run::USAGE);
            Ok(true)
        }
        _ => Err(USAGE.to_string()),
    };
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(2)
        }
    }
}

/// `ctrl-cli guide`: an edit guide for `path=value` changes.
fn guide(d: &Descriptor, sets: &[String]) -> Result<bool, String> {
    let mut changes = Vec::new();
    for s in sets {
        let (path, value) = s
            .split_once('=')
            .ok_or_else(|| format!("`{s}`: expected <path>=<value>"))?;
        let want = Expr::parse(value)
            .and_then(|e| e.eval(&|_| None))
            .map_err(|e| format!("`{s}`: {e}"))?;
        changes.push(ctrl_engine::Change {
            path: path.to_string(),
            want,
            have: None,
        });
    }
    let want = |p: &str| changes.iter().find(|c| c.path == p).map(|c| c.want);
    let lines = ctrl_engine::edit_guide(d, &changes, &want).map_err(|e| e.to_string())?;
    if lines.is_empty() {
        println!("Nothing to do by hand.");
    } else {
        print!("{}", ctrl_engine::guide::to_text(&lines));
    }
    Ok(true)
}

fn load(path: &str) -> Result<Descriptor, String> {
    ctrl_cdl::load(path).map_err(|errors| format!("FAIL {path}\n{errors}"))
}

fn status(s: Option<Status>) -> &'static str {
    match s {
        Some(Status::Measured) => "measured",
        Some(Status::Decoded) => "decoded",
        Some(Status::Inferred) => "inferred",
        Some(Status::Unknown) => "unknown",
        None => "no status",
    }
}

fn certain(s: Option<Status>) -> bool {
    matches!(s, Some(Status::Measured | Status::Decoded))
}

fn at(d: &Descriptor, pos: ctrl_cdl::Pos) -> String {
    let e = d.error_at(pos, "");
    format!("{}:{}:{}", e.file, e.line, e.col)
}

// ---- check ------------------------------------------------------------------

fn check(paths: &[String]) -> Result<bool, String> {
    let mut ok = true;
    for path in paths {
        let d = match load(path) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("{e}");
                ok = false;
                continue;
            }
        };
        let report = ctrl_cdl::validate(&d);
        ok &= report.is_ok();
        println!(
            "{} {path}: {} ({} transports, {} blocks, {} messages, {} procedures, {} drivers)",
            if report.is_ok() { "ok  " } else { "FAIL" },
            d.device.id,
            d.transports.len(),
            d.blocks.len(),
            d.messages.len(),
            d.procedures.len(),
            d.drivers.len(),
        );
        for finding in &report.findings {
            println!("  {finding}");
        }
        ok &= check_images(&d);
        for v in Codec::new(&d).run_vectors() {
            match v.outcome {
                Outcome::Pass => println!("  vector {}: pass", v.name),
                Outcome::Skipped(why) => println!("  vector {}: skipped ({why})", v.name),
                Outcome::Fail(why) => {
                    ok = false;
                    println!("  vector {}: FAIL {why}", v.name);
                }
            }
        }
        send_paths(&d);
        if !d.lbmt.is_empty() {
            println!(
                "  lbmt: {} snippet(s) not compiled; there is no LBMT compiler yet",
                d.lbmt.len()
            );
        }
    }
    Ok(ok)
}

/// Images resolve against the file that names them.
fn check_images(d: &Descriptor) -> bool {
    let images = d
        .device
        .images
        .iter()
        .map(|i| (i.meta.pos, i.path.as_str()))
        .chain(
            d.panel
                .iter()
                .filter_map(|p| Some((p.meta.pos, p.image.as_deref()?))),
        );
    let mut ok = true;
    for (pos, image) in images {
        let declared = std::path::Path::new(&d.files[pos.file as usize]);
        let file = declared
            .parent()
            .unwrap_or(std::path::Path::new(""))
            .join(image);
        if !file.is_file() {
            ok = false;
            println!(
                "  {}: error: image {} not found",
                at(d, pos),
                file.display()
            );
        }
    }
    ok
}

/// Names a template reads, including those inside expressions.
fn template_names(t: &Template) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for p in &t.pieces {
        match p {
            Piece::Field { name, .. } | Piece::Slice { name, .. } => {
                out.insert(name.clone());
            }
            Piece::Expr { expr } => {
                if let Ok(e) = Expr::parse(expr) {
                    out.extend(e.names().into_iter().map(String::from));
                }
            }
            _ => {}
        }
    }
    out
}

/// Each message that sends bytes, with every part of those bytes (the
/// message, its frame, the constants it uses) that is not measured or
/// decoded. The validator rejects writes built on such parts; this lists
/// them all, reads included, so the evidence gaps are in one place.
fn send_paths(d: &Descriptor) {
    let mut lines = Vec::new();
    let mut total = 0;
    for m in d.messages.iter().filter(|m| m.send.is_some()) {
        total += 1;
        let mut gaps = Vec::new();
        if !certain(m.meta.status) {
            gaps.push(format!("message {}", status(m.meta.status)));
        }
        let frame = m
            .frame
            .as_ref()
            .and_then(|f| d.frames.iter().find(|x| &x.name == f));
        let mut names = template_names(m.send.as_ref().unwrap());
        if let Some(f) = frame {
            if !certain(f.meta.status) {
                gaps.push(format!("frame `{}` {}", f.name, status(f.meta.status)));
            }
            for t in f.prefix.iter().chain(&f.suffix) {
                names.extend(template_names(t));
            }
        }
        for c in names.iter().filter_map(|n| d.constant(n)) {
            if !certain(c.meta.status) {
                gaps.push(format!("const `{}` {}", c.name, status(c.meta.status)));
            }
        }
        if !gaps.is_empty() {
            lines.push(format!(
                "    {} ({}): {}",
                m.name,
                at(d, m.meta.pos),
                gaps.join(", ")
            ));
        }
    }
    if total > 0 {
        println!(
            "  send paths: {} of {total} not fully measured or decoded",
            lines.len()
        );
        for l in lines {
            println!("{l}");
        }
    }
}

// ---- explain ----------------------------------------------------------------

fn parse_value(name: &str, v: &str) -> Result<Val, String> {
    let bad = |why: &str| format!("`{name}={v}`: {why}");
    if let Some(h) = v.strip_prefix("hex:") {
        return parse_hex(h).map(Val::Bytes).map_err(|e| bad(&e));
    }
    if let Some(t) = v.strip_prefix("text:") {
        return Ok(Val::Bytes(t.as_bytes().to_vec()));
    }
    if let Some(f) = v.strip_prefix('@') {
        return std::fs::read(f)
            .map(Val::Bytes)
            .map_err(|e| bad(&e.to_string()));
    }
    let (neg, digits) = match v.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, v),
    };
    let n = match digits
        .strip_prefix("0x")
        .or_else(|| digits.strip_prefix("0X"))
    {
        Some(h) => i64::from_str_radix(h, 16),
        None => digits.parse(),
    }
    .map_err(|_| bad("not a number; use hex:…, text:… or @file for bytes"))?;
    Ok(Val::Int(if neg { -n } else { n }))
}

fn explain(path: &str, message: &str, assignments: &[String]) -> Result<bool, String> {
    let d = load(path)?;
    let m = d.message(message).ok_or_else(|| {
        let names: Vec<_> = d.messages.iter().map(|m| m.name.as_str()).collect();
        format!("no message `{message}`; messages: {}", names.join(", "))
    })?;
    let mut args = Bindings::new();
    for a in assignments {
        let (k, v) = a
            .split_once('=')
            .ok_or_else(|| format!("`{a}`: expected name=value"))?;
        args.insert(k.to_string(), parse_value(k, v)?);
    }
    let codec = Codec::new(&d);
    let mut where_ = Vec::new();
    if let Some(t) = &m.transport {
        where_.push(format!("transport {t}"));
    }
    if let Some(f) = &m.frame {
        where_.push(format!("frame {f}"));
    }
    where_.push(format!("{:?}", codec.medium(m)).to_uppercase());
    println!(
        "message {} ({}; {}) {}",
        m.name,
        status(m.meta.status),
        where_.join(", "),
        at(&d, m.meta.pos)
    );
    if let Some(e) = &m.meta.evidence {
        println!("  evidence: {e}");
    }
    let (bytes, spans) = match codec.explain(message, &args) {
        Ok(r) => r,
        Err(e) => {
            let mut needs = m.send.as_ref().map(template_names).unwrap_or_default();
            if let Some(f) = m
                .frame
                .as_ref()
                .and_then(|f| d.frames.iter().find(|x| &x.name == f))
            {
                for t in f.prefix.iter().chain(&f.suffix) {
                    needs.extend(template_names(t));
                }
            }
            needs.retain(|n| d.constant(n).is_none());
            return Err(format!(
                "cannot encode `{message}`: {e}\n  it takes: {}",
                needs.into_iter().collect::<Vec<_>>().join(", ")
            ));
        }
    };
    println!("\n{}\n", hex(&bytes));
    // Adjacent literal bytes in one region print as one line.
    let mut i = 0;
    while i < spans.len() {
        let s = &spans[i];
        let mut end = s.start + s.len;
        if s.label == "literal" {
            while let Some(n) = spans.get(i + 1) {
                if n.label != "literal" || n.region != s.region {
                    break;
                }
                end = n.start + n.len;
                i += 1;
            }
        }
        if end > s.start {
            println!(
                "  {:>4}  {:<24} {:<8} {}",
                s.start,
                hex(&bytes[s.start..end]),
                s.region,
                s.label
            );
        }
        i += 1;
    }
    match m.replies.as_slice() {
        [] => println!("\nsend-only: no reply expected"),
        replies => {
            println!();
            for r in replies {
                let name = r
                    .variant
                    .as_deref()
                    .map(|v| format!(" {v}"))
                    .unwrap_or_default();
                let named = r
                    .pattern_name
                    .as_deref()
                    .map(|p| format!(" (pattern {p})"))
                    .unwrap_or_default();
                let timeout = r
                    .timeout
                    .map(|t| format!(", timeout {t} ms"))
                    .unwrap_or_default();
                println!("reply{name}: {}{named}{timeout}", r.pattern);
                if let Some(ok) = r.ok_when.as_ref().or(m.ok_when.as_ref()) {
                    println!("  ok-when {ok}");
                }
            }
        }
    }
    Ok(true)
}

// ---- trace ------------------------------------------------------------------

struct Frame {
    line: usize,
    direction: Option<Direction>,
    bytes: Vec<u8>,
}

fn parse_hex(s: &str) -> Result<Vec<u8>, String> {
    let mut digits = String::new();
    for token in s.split(|c: char| c.is_whitespace() || c == ',') {
        let t = token
            .strip_prefix("0x")
            .or_else(|| token.strip_prefix("0X"))
            .unwrap_or(token);
        if t.len() % 2 == 1 {
            return Err(format!("`{token}` is not whole bytes"));
        }
        digits.push_str(t);
    }
    (0..digits.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&digits[i..i + 2], 16)
                .map_err(|_| format!("`{}` is not hex", &digits[i..i + 2]))
        })
        .collect()
}

/// Splits a MIDI byte stream into messages: SysEx from F0 to F7, others
/// from one status byte to the next.
fn split_midi(bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut out: Vec<Vec<u8>> = Vec::new();
    let mut in_sysex = false;
    for &b in bytes {
        match b {
            0xF0 => {
                out.push(vec![b]);
                in_sysex = true;
            }
            0xF7 if in_sysex => {
                out.last_mut().unwrap().push(b);
                in_sysex = false;
            }
            0x80..=0xFF if !in_sysex => out.push(vec![b]),
            _ => match out.last_mut() {
                Some(last) => last.push(b),
                None => out.push(vec![b]),
            },
        }
    }
    out
}

fn direction_word(w: &str) -> Option<Direction> {
    match w.to_ascii_lowercase().as_str() {
        ">" | "out" | "tx" | "send" | "to-device" => Some(Direction::Out),
        "<" | "in" | "rx" | "recv" | "receive" | "from-device" => Some(Direction::In),
        _ => None,
    }
}

fn read_capture(path: &str) -> Result<Vec<Frame>, String> {
    let data = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    let binary = path.ends_with(".syx") || path.ends_with(".bin");
    let text = match (binary, String::from_utf8(data.clone())) {
        (false, Ok(t)) => t,
        _ => {
            return Ok(split_midi(&data)
                .into_iter()
                .map(|bytes| Frame {
                    line: 0,
                    direction: None,
                    bytes,
                })
                .collect())
        }
    };
    let mut frames = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with("//") {
            continue;
        }
        let err = |e: String| format!("{path}:{}: {e}", i + 1);
        let (direction, bytes) = if line.starts_with('{') {
            let v: serde_json::Value =
                serde_json::from_str(line).map_err(|e| err(e.to_string()))?;
            let h = ["hex", "bytes", "data"]
                .iter()
                .find_map(|k| v.get(*k).and_then(|x| x.as_str()))
                .ok_or_else(|| err("no \"hex\" field".into()))?;
            let dir = ["dir", "direction"]
                .iter()
                .find_map(|k| v.get(*k).and_then(|x| x.as_str()))
                .and_then(direction_word);
            (dir, parse_hex(h).map_err(err)?)
        } else if let Some((head, data)) = line.split_once("data=") {
            // A HID trace log: `<time> OUT id=0 len=8 … data=10 00 …`.
            let dir = head.split_whitespace().find_map(direction_word);
            (dir, parse_hex(data).map_err(err)?)
        } else {
            let (first, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
            let (dir, hexpart) = match direction_word(first) {
                Some(d) => (Some(d), rest),
                None => match (line.strip_prefix('>'), line.strip_prefix('<')) {
                    (Some(r), _) => (Some(Direction::Out), r),
                    (_, Some(r)) => (Some(Direction::In), r),
                    _ => (None, line),
                },
            };
            (dir, parse_hex(hexpart).map_err(err)?)
        };
        // Several SysEx messages on one line are separate frames; anything
        // else (a HID report) stays whole.
        let parts = if bytes.first() == Some(&0xF0) {
            split_midi(&bytes)
        } else {
            vec![bytes]
        };
        frames.extend(parts.into_iter().map(|bytes| Frame {
            line: i + 1,
            direction,
            bytes,
        }));
    }
    Ok(frames)
}

fn show_values(values: &Bindings) -> String {
    values
        .iter()
        .map(|(k, v)| match v {
            Val::Int(i) if (0..10).contains(i) => format!("{k}={i}"),
            Val::Int(i) => format!("{k}={i}/{i:#04X}"),
            Val::Bytes(b) if b.len() <= 16 => format!("{k}=[{}]", hex(b)),
            Val::Bytes(b) => format!("{k}=[{} bytes]", b.len()),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn show_match(m: &Match) -> String {
    let arrow = match m.direction {
        Direction::Out => ">",
        Direction::In => "<",
    };
    let variant = m
        .variant
        .as_deref()
        .map(|v| format!("[{v}]"))
        .unwrap_or_default();
    let ok = match m.ok {
        Some(true) => "  ok",
        Some(false) => "  NOT OK (ok-when fails)",
        None => "",
    };
    let values = show_values(&m.values);
    let mut line = format!("{arrow} {}{variant}", m.message);
    if !values.is_empty() {
        line += &format!("  {values}");
    }
    line + ok
}

fn trace(path: &str, captures: &[String], matched_only: bool) -> Result<bool, String> {
    let d = load(path)?;
    let codec = Codec::new(&d);
    // Transport captures (a device id) carry over from the frame that
    // first shows them, so later frames must agree with them.
    let carried: BTreeSet<&str> = d
        .transports
        .iter()
        .flat_map(|t| t.captures.iter().map(|c| c.name.as_str()))
        .collect();
    for capture in captures {
        let frames = read_capture(capture)?;
        let mut known = Bindings::new();
        // The last frame that matched exactly one send, and whether a reply
        // already paired with it: replies pair with it. A device may send a
        // report twice (one poll answered by two input reports), so a
        // second reply pairs too, as a repeat.
        let mut last_sent: Option<(usize, String, bool)> = None;
        let (mut matched, mut ambiguous) = (0, 0);
        println!("{capture}: {} frame(s)", frames.len());
        for (n, f) in frames.iter().enumerate() {
            let arrow = match f.direction {
                Some(Direction::Out) => "> ",
                Some(Direction::In) => "< ",
                None => "",
            };
            let line = if f.line > 0 {
                format!(" (line {})", f.line)
            } else {
                String::new()
            };
            let head = format!("#{n}{line} {arrow}{}", hex(&f.bytes));
            let mut found = codec.identify(&f.bytes, f.direction, &known);
            let mut note = "";
            if found.is_empty() && !known.is_empty() {
                found = codec.identify(&f.bytes, f.direction, &Bindings::new());
                note = "  (differs from the carried ";
            }
            if found.is_empty() {
                if !matched_only {
                    println!("{head}\n    no match");
                }
                continue;
            }
            println!("{head}");
            let mut paired = None;
            if let Some((at, sent, answered)) = &mut last_sent {
                if found.len() > 1
                    && found
                        .iter()
                        .any(|m| m.direction == Direction::In && &m.message == sent)
                {
                    found.retain(|m| m.direction == Direction::In && &m.message == sent);
                    paired = Some((*at, *answered));
                    *answered = true;
                }
            }
            // Keep the most specific matches: the most literal bytes, then
            // no negative values (`{0x60 + s}` solves 50 as s = -16), then
            // the fewest captured values. `{0x40 + s}` takes any byte, a
            // literal `50` only itself.
            let rank = |m: &Match| {
                let negative = m
                    .values
                    .values()
                    .filter(|v| matches!(v, Val::Int(i) if *i < 0))
                    .count();
                (
                    m.literals,
                    std::cmp::Reverse(negative),
                    std::cmp::Reverse(m.values.len()),
                )
            };
            let best = found.iter().map(rank).max().unwrap();
            let loose = found.len();
            found.retain(|m| rank(m) == best);
            let loose = loose - found.len();
            matched += 1;
            if found.len() > 1 {
                ambiguous += 1;
            }
            for m in &found {
                let carried_note = if note.is_empty() {
                    String::new()
                } else {
                    format!("{note}{})", show_values(&known))
                };
                let reply_to = match paired {
                    Some((at, false)) => format!("  (reply to #{at})"),
                    Some((at, true)) => format!("  (repeated reply to #{at})"),
                    None => String::new(),
                };
                println!("    {}{reply_to}{carried_note}", show_match(m));
            }
            if loose > 0 {
                println!("    ({loose} less specific match(es) not shown)");
            }
            let sends: Vec<_> = found
                .iter()
                .filter(|m| m.direction == Direction::Out)
                .collect();
            if let [only] = sends.as_slice() {
                last_sent = Some((n, only.message.clone(), false));
            }
            if found.len() == 1 {
                for (k, v) in &found[0].values {
                    if carried.contains(k.as_str()) && !known.contains_key(k) {
                        known.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        println!(
            "{capture}: {matched} of {} matched, {ambiguous} ambiguous",
            frames.len()
        );
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_forms() {
        assert_eq!(
            parse_hex("F0 42,0x40 0001").unwrap(),
            [0xF0, 0x42, 0x40, 0, 1]
        );
        assert!(parse_hex("F").is_err());
        assert!(parse_hex("GG").is_err());
    }

    #[test]
    fn midi_stream_splits() {
        let s = [0xB8, 0x11, 0x00, 0xF0, 0x7E, 0x7F, 0xF7, 0x90, 0x3C, 0x40];
        assert_eq!(
            split_midi(&s),
            vec![
                vec![0xB8, 0x11, 0x00],
                vec![0xF0, 0x7E, 0x7F, 0xF7],
                vec![0x90, 0x3C, 0x40]
            ]
        );
    }

    #[test]
    fn values() {
        assert_eq!(parse_value("a", "12").unwrap(), Val::Int(12));
        assert_eq!(parse_value("a", "0x0F").unwrap(), Val::Int(15));
        assert_eq!(parse_value("a", "-1").unwrap(), Val::Int(-1));
        assert_eq!(
            parse_value("a", "hex:AA55").unwrap(),
            Val::Bytes(vec![0xAA, 0x55])
        );
        assert_eq!(
            parse_value("a", "text:Hi").unwrap(),
            Val::Bytes(b"Hi".to_vec())
        );
        assert!(parse_value("a", "x").is_err());
    }
}
