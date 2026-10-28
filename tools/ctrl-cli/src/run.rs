//! `ctrl-cli run`: a procedure through ctrl-engine (CDL §11).
//!
//! Nothing is written without `--apply`. Every gate asks on stdin and goes
//! on only on `yes`. A write keeps its restore point in `--backup-dir`
//! before it sends.

use std::io::{BufRead, Write as _};
use std::sync::mpsc;
use std::time::Duration;

use ctrl_cdl::{Access, Descriptor, Gate, Status};
use ctrl_codec::Planned;
use ctrl_engine::{Operator, Port, Run};

pub const USAGE: &str = "\
run      run a procedure: ctrl-cli run <file> <procedure> [var=value]...
           [--load <message>=<file.syx>]...  editor document from a dump
           [--set <path>=<value>]...         edit the document
           [--replay <file.syx>]...          dry run: what the device sends
           [--backup-dir <dir>]              where restore points go (writes)
           [--port <match>]                  MIDI port (default: the
                                             transport's port-match)
           [--save-sent <file>]              the messages sent (or, in a dry
                                             run, that would be), as .syx
           [--apply]                         really send (otherwise a dry run)";

struct Options {
    vars: Vec<(String, i64)>,
    loads: Vec<(String, String)>,
    sets: Vec<(String, i64)>,
    replay: Vec<String>,
    backup_dir: Option<String>,
    port: Option<String>,
    save_sent: Option<String>,
    apply: bool,
}

fn number(s: &str) -> Result<i64, String> {
    match s.strip_prefix("0x") {
        Some(h) => i64::from_str_radix(h, 16),
        None => s.parse(),
    }
    .map_err(|_| format!("`{s}` is not a number"))
}

fn options(args: &[String]) -> Result<Options, String> {
    let mut o = Options {
        vars: Vec::new(),
        loads: Vec::new(),
        sets: Vec::new(),
        replay: Vec::new(),
        backup_dir: None,
        port: None,
        save_sent: None,
        apply: false,
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut value = |what: &str| it.next().cloned().ok_or(format!("{what} needs a value"));
        match a.as_str() {
            "--apply" => o.apply = true,
            "--load" => {
                let v = value("--load")?;
                let (m, f) = v.split_once('=').ok_or("--load <message>=<file>")?;
                o.loads.push((m.into(), f.into()));
            }
            "--set" => {
                let v = value("--set")?;
                let (p, x) = v.split_once('=').ok_or("--set <path>=<value>")?;
                o.sets.push((p.into(), number(x)?));
            }
            "--replay" => o.replay.push(value("--replay")?),
            "--backup-dir" => o.backup_dir = Some(value("--backup-dir")?),
            "--port" => o.port = Some(value("--port")?),
            "--save-sent" => o.save_sent = Some(value("--save-sent")?),
            v => {
                let (k, x) = v
                    .split_once('=')
                    .ok_or(format!("`{v}`: expected var=value or an option"))?;
                o.vars.push((k.into(), number(x)?));
            }
        }
    }
    Ok(o)
}

/// Gates on the terminal.
struct Terminal {
    backup_dir: Option<String>,
}

impl Operator for Terminal {
    fn gate(&mut self, gate: Gate, text: &str) -> bool {
        let kind = match gate {
            Gate::Confirm => "confirm",
            Gate::Arm => "arm",
        };
        println!("\n[{kind}] {text}\nType yes to go on:");
        std::io::stdout().flush().ok();
        let mut line = String::new();
        std::io::stdin().lock().read_line(&mut line).ok();
        line.trim() == "yes"
    }

    fn note(&mut self, text: &str) {
        println!("  {text}");
    }

    fn restore_point(&mut self, procedure: &str, messages: &[Vec<u8>]) -> Result<(), String> {
        let dir = self
            .backup_dir
            .as_ref()
            .ok_or("a write needs --backup-dir for its restore point")?;
        std::fs::create_dir_all(dir).map_err(|e| format!("{dir}: {e}"))?;
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        for (i, m) in messages.iter().enumerate() {
            let path = format!("{dir}/restore-{procedure}-{stamp}-{i}.syx");
            std::fs::write(&path, m).map_err(|e| format!("{path}: {e}"))?;
            println!("  restore point kept: {path} ({} bytes)", m.len());
        }
        Ok(())
    }
}

/// Dry run: the device's messages come from files; nothing is sent.
struct Replay {
    messages: std::collections::VecDeque<Vec<u8>>,
}

impl Port for Replay {
    fn send(&mut self, _: &Planned) -> Result<(), String> {
        Err("a dry run sends nothing".into())
    }
    fn recv(&mut self, _: u32) -> Result<Option<Vec<u8>>, String> {
        Ok(self.messages.pop_front())
    }
}

/// The real device over ctrl-midi.
struct Midi {
    out: ctrl_midi::Output,
    rx: mpsc::Receiver<Vec<u8>>,
    _input: ctrl_midi::Input,
}

impl Port for Midi {
    fn send(&mut self, planned: &Planned) -> Result<(), String> {
        self.out
            .send_writes(&planned.writes)
            .map_err(|e| e.to_string())
    }
    fn recv(&mut self, timeout_ms: u32) -> Result<Option<Vec<u8>>, String> {
        match self
            .rx
            .recv_timeout(Duration::from_millis(u64::from(timeout_ms)))
        {
            Ok(m) => Ok(Some(m)),
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }
}

fn open_midi(input: &str, output: &str) -> Result<Midi, String> {
    use ctrl_midi::{Backend, Dir, SysexPolicy};
    let b = Backend::from_env().unwrap_or_else(Backend::native);
    let one = |dir| -> Result<ctrl_midi::Port, String> {
        let pattern = if dir == Dir::In { input } else { output };
        let found = ctrl_midi::find(b, dir, pattern).map_err(|e| e.to_string())?;
        match found.as_slice() {
            [p] => Ok(p.clone()),
            [] => Err(format!("no {dir:?} port matches {pattern:?}")),
            _ => Err(format!("{} {dir:?} ports match {pattern:?}", found.len())),
        }
    };
    let (tx, rx) = mpsc::channel();
    let input = ctrl_midi::open_input(
        b,
        &one(Dir::In)?,
        Box::new(move |_, m| {
            let _ = tx.send(m.to_vec());
        }),
    )
    .map_err(|e| e.to_string())?;
    let out = ctrl_midi::open_output(b, &one(Dir::Out)?, SysexPolicy::default())
        .map_err(|e| e.to_string())?;
    Ok(Midi {
        out,
        rx,
        _input: input,
    })
}

/// The procedure's transport's port names, input and output.
fn default_port(d: &Descriptor, procedure: &str) -> Option<(String, String)> {
    let p = d.procedure(procedure)?;
    let t = d.transport(p.transport.as_deref()?)?;
    match t.ports.clone()? {
        ctrl_cdl::PortMatch::Both(m) => Some((m.clone(), m)),
        ctrl_cdl::PortMatch::Split { input, output } => Some((input, output)),
    }
}

pub fn run(d: &Descriptor, procedure: &str, args: &[String]) -> Result<bool, String> {
    let o = options(args)?;
    let report = ctrl_cdl::validate(d);
    if !report.is_ok() {
        return Err(format!(
            "the descriptor does not validate; run `ctrl-cli check`:\n{}",
            report
                .errors()
                .map(|f| f.to_string())
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }
    let p = d
        .procedure(procedure)
        .ok_or(format!("unknown procedure `{procedure}`"))?;
    if p.meta.status == Some(Status::Unknown) {
        return Err(format!(
            "procedure `{procedure}` is unknown: it does not run"
        ));
    }
    let write = matches!(p.kind, Some(Access::Write | Access::Commit));
    if write && o.apply && o.backup_dir.is_none() {
        return Err("a write needs --backup-dir for its restore point".into());
    }
    let mut r = Run::new(d);
    r.apply = o.apply;
    for (k, v) in &o.vars {
        r.vars.insert(k.clone(), ctrl_codec::Val::Int(*v));
    }
    for (m, f) in &o.loads {
        let bytes = std::fs::read(f).map_err(|e| format!("{f}: {e}"))?;
        r.load(m, &bytes).map_err(|e| format!("{f}: {e}"))?;
    }
    for (path, v) in &o.sets {
        r.set(path, *v).map_err(|e| e.to_string())?;
    }
    let mut op = Terminal {
        backup_dir: o.backup_dir.clone(),
    };
    let result = if o.apply {
        let (input, output) = o
            .port
            .clone()
            .map(|p| (p.clone(), p))
            .or_else(|| default_port(d, procedure))
            .ok_or("no port: pass --port")?;
        let mut port = open_midi(&input, &output)?;
        println!("running `{procedure}` on {output:?}");
        r.run(procedure, &mut port, &mut op)
    } else {
        let mut messages = std::collections::VecDeque::new();
        for f in &o.replay {
            messages.push_back(std::fs::read(f).map_err(|e| format!("{f}: {e}"))?);
        }
        println!("dry run of `{procedure}`: nothing is sent");
        r.run(procedure, &mut Replay { messages }, &mut op)
    };
    if let Some(f) = &o.save_sent {
        let sent: Vec<u8> = r
            .events
            .iter()
            .filter_map(|e| match e {
                ctrl_engine::Event::Sent { bytes, .. } => Some(bytes.clone()),
                _ => None,
            })
            .flatten()
            .collect();
        std::fs::write(f, &sent).map_err(|e| format!("{f}: {e}"))?;
        println!("  sent messages saved: {f} ({} bytes)", sent.len());
    }
    match result {
        Ok(()) => {
            println!("done: `{procedure}`");
            Ok(true)
        }
        Err(e) => {
            println!("stopped: {e}");
            Ok(false)
        }
    }
}
