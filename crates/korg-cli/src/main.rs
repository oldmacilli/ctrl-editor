//! Small headless tool for the KORG data format.

use korg_core::{MeasuredSession, ReceiveLayout, SystemClock};
use korg_format::{decode, encode, Chunk, ChunkKey, Payload};
use korg_midi::{Direction, MidiTransport, PortId, PortInfo};
use korg_midi_midir::MidirTransport;
use korg_model::{ChunkKind, FileModelKey};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    let (flag, args) = match take_midi_backend(std::env::args().skip(1).collect()) {
        Ok(split) => split,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(2);
        }
    };
    // Flag, then CTRL_MIDI_BACKEND, then the GUI's and TUI's saved choice,
    // then the platform's own.
    let saved = korg_app::preferences::load().midi_backend;
    if let Err(error) = korg_midi_midir::init_backend(flag.as_deref(), &saved) {
        eprintln!("{error}");
        return ExitCode::from(2);
    }
    let mut args = args.into_iter();
    let Some(command) = args.next() else {
        eprintln!("usage: korg-cli [--midi-backend alsa|jack] inspect <file> | validate <products-dir> | ports | backends | detect [--timeout ms] | receive --port <name> --kind scene-set|scene|global --out <file> [--scene N]");
        return ExitCode::from(2);
    };
    match command.as_str() {
        "inspect" => {
            let Some(path) = args.next() else {
                eprintln!("usage: korg-cli inspect <file>");
                return ExitCode::from(2);
            };
            match inspect(Path::new(&path)) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("{error}");
                    ExitCode::FAILURE
                }
            }
        }
        "validate" => {
            let Some(path) = args.next() else {
                eprintln!("usage: korg-cli validate <products-dir>");
                return ExitCode::from(2);
            };
            match validate(Path::new(&path)) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("{error}");
                    ExitCode::FAILURE
                }
            }
        }
        "receive" => match receive(args.collect()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        },
        "write-scene" => match write_scene(args.collect()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        },
        "write-global" => match write_global(args.collect()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        },
        "backends" => {
            backends();
            ExitCode::SUCCESS
        }
        "ports" => match ports() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        },
        "detect" => match detect(args.collect()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        },
        "query" => match query(args.collect()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        },
        "write-set" => match write_set(args.collect()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        },
        _ => {
            eprintln!("unknown command: {command}");
            ExitCode::from(2)
        }
    }
}

fn inspect(path: &Path) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let chunk = decode(&bytes).map_err(|error| format!("{}: {error}", path.display()))?;
    println!("file:      {}", path.display());
    println!("size:      {} bytes", bytes.len());
    println!("key:       {}", chunk.key);
    println!("kind:      {:?}", chunk.key.kind);
    match &chunk.payload {
        Payload::SceneSet(scenes) => {
            println!("scenes:    {}", scenes.len());
            for (index, scene) in scenes.iter().enumerate() {
                let prefix: String = scene
                    .iter()
                    .take(16)
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<Vec<_>>()
                    .join(" ");
                println!("  [{index:2}] {:4} bytes  {prefix}", scene.len());
            }
        }
        Payload::Scene(scene) => println!("scene:     {} bytes", scene.len()),
        Payload::Global(global) => println!("global:    {} bytes", global.len()),
    }
    Ok(())
}

fn validate(root: &Path) -> Result<(), String> {
    let mut checked = 0usize;
    let mut directories: Vec<PathBuf> = std::fs::read_dir(root)
        .map_err(|error| format!("{}: {error}", root.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    directories.sort();
    for directory in directories {
        let mut files: Vec<PathBuf> = std::fs::read_dir(&directory)
            .map_err(|error| format!("{}: {error}", directory.display()))?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_file())
            .collect();
        files.sort();
        for file in files {
            let bytes =
                std::fs::read(&file).map_err(|error| format!("{}: {error}", file.display()))?;
            let chunk = decode(&bytes).map_err(|error| format!("{}: {error}", file.display()))?;
            let encoded = encode(&chunk).map_err(|error| format!("{}: {error}", file.display()))?;
            if encoded != bytes {
                return Err(format!("{}: re-encode differs", file.display()));
            }
            checked += 1;
        }
    }
    println!("validated {checked} files under {}", root.display());
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReceiveKind {
    SceneSet,
    Scene,
    Global,
}

fn find_port(ports: &[PortInfo], needle: &str, direction: Direction) -> Result<PortId, String> {
    let needle = needle.to_ascii_lowercase();
    let found = ports.iter().find(|port| {
        port.direction == direction && port.name.to_ascii_lowercase().contains(&needle)
    });
    // The nanoKEY Studio exposes one config port for input and output under
    // different names: "KEYBOARD/CTRL" in, "CTRL" out.
    let alias = match direction {
        Direction::Input => needle.replace(" ctrl", " keyboard/ctrl"),
        Direction::Output => needle.replace(" keyboard/ctrl", " ctrl"),
    };
    found
        .or_else(|| {
            ports.iter().find(|port| {
                port.direction == direction && port.name.to_ascii_lowercase().contains(&alias)
            })
        })
        .map(|port| port.id.clone())
        .ok_or_else(|| format!("no {:?} port matches {:?}", direction, needle))
}

fn receive(args: Vec<String>) -> Result<(), String> {
    let mut port = None;
    let mut kind = None;
    let mut out = None;
    let mut scene = 0u8;
    let mut timeout_ms = 3000u64;
    let mut index = 0;
    while index < args.len() {
        let value = |index: usize| {
            args.get(index + 1)
                .cloned()
                .ok_or_else(|| format!("{} needs a value", args[index]))
        };
        match args[index].as_str() {
            "--port" => {
                port = Some(value(index)?);
                index += 2;
            }
            "--kind" => {
                kind = Some(match value(index)?.as_str() {
                    "scene-set" => ReceiveKind::SceneSet,
                    "scene" => ReceiveKind::Scene,
                    "global" => ReceiveKind::Global,
                    other => return Err(format!("unknown kind: {other}")),
                });
                index += 2;
            }
            "--out" => {
                out = Some(PathBuf::from(value(index)?));
                index += 2;
            }
            "--scene" => {
                scene = value(index)?
                    .parse()
                    .map_err(|_| "scene must be a number".to_owned())?;
                index += 2;
            }
            "--timeout" => {
                timeout_ms = value(index)?
                    .parse()
                    .map_err(|_| "timeout must be milliseconds".to_owned())?;
                index += 2;
            }
            other => return Err(format!("unknown option: {other}")),
        }
    }
    let port = port.ok_or_else(|| "--port is required".to_owned())?;
    let kind = kind.ok_or_else(|| "--kind is required".to_owned())?;
    let out = out.ok_or_else(|| "--out is required".to_owned())?;

    let mut transport = MidirTransport::new();
    let ports = transport.list_ports().map_err(|error| error.to_string())?;
    let input = find_port(&ports, &port, Direction::Input)?;
    let output = find_port(&ports, &port, Direction::Output)?;
    println!("port:   {port} (input and output)");

    let mut session =
        MeasuredSession::new(transport, input, output, SystemClock::new(), timeout_ms);
    session.open().map_err(|error| error.to_string())?;
    let identity = session.mode_check().map_err(|error| error.to_string())?;
    session.check_identity_once();
    let layout = korg_devices::measured_layout(identity.model_id)
        .ok_or_else(|| format!("no layout for model {:#06x}", identity.model_id))?;
    println!(
        "model:  {:#06x}, key {}, scenes {}, scene {} bytes, global {} bytes",
        identity.model_id, layout.key, layout.scene_count, layout.scene_len, layout.global_len
    );

    let (key_kind, payload) = match kind {
        ReceiveKind::SceneSet => {
            let scenes = session
                .receive_scene_set(ReceiveLayout {
                    scene_count: layout.scene_count,
                    scene_len: layout.scene_len,
                    global_len: layout.global_len,
                })
                .map_err(|error| error.to_string())?;
            (ChunkKind::SceneSet, Payload::SceneSet(scenes))
        }
        ReceiveKind::Scene => {
            let data = session
                .receive_scene(scene, layout.scene_len)
                .map_err(|error| error.to_string())?;
            (ChunkKind::SceneData, Payload::Scene(data))
        }
        ReceiveKind::Global => {
            let data = session
                .receive_global(layout.global_len)
                .map_err(|error| error.to_string())?;
            (ChunkKind::GlobalData, Payload::Global(data))
        }
    };

    let model_key = FileModelKey::new(layout.key).map_err(|error| error.to_string())?;
    let chunk = Chunk {
        key: ChunkKey::new(model_key, key_kind),
        payload,
    };
    let bytes = encode(&chunk).map_err(|error| error.to_string())?;
    std::fs::write(&out, &bytes).map_err(|error| format!("{}: {error}", out.display()))?;
    println!(
        "wrote:  {} ({} bytes, {} elements)",
        out.display(),
        bytes.len(),
        chunk.scene_count()
    );
    Ok(())
}

/// Print every MIDI port with its direction and link kind.
/// Remove `--midi-backend <id>` (or `--midi-backend=<id>`) from anywhere
/// in the arguments; it applies to every command.
fn take_midi_backend(args: Vec<String>) -> Result<(Option<String>, Vec<String>), String> {
    let mut flag = None;
    let mut rest = Vec::with_capacity(args.len());
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if arg == "--midi-backend" {
            flag = Some(
                args.next()
                    .ok_or_else(|| "--midi-backend needs a value".to_owned())?,
            );
        } else if let Some(value) = arg.strip_prefix("--midi-backend=") {
            flag = Some(value.to_owned());
        } else {
            rest.push(arg);
        }
    }
    Ok((flag, rest))
}

/// The backends this build has, the active one marked, each probed.
fn backends() {
    let active = korg_midi_midir::backend();
    for backend in korg_midi_midir::backends() {
        let info = korg_midi_midir::backend_info(backend);
        let mark = if backend == active { '*' } else { ' ' };
        let state = info.problem.as_deref().unwrap_or("available");
        println!("{mark} {}\t{}\t{state}", info.id, info.label);
    }
}

fn ports() -> Result<(), String> {
    // On stderr, so the port lines stay one format for scripts.
    let info = korg_midi_midir::backend_info(korg_midi_midir::backend());
    eprintln!("{}", korg_app::midi_backend::status(&info));
    let mut transport = MidirTransport::new();
    for port in transport.list_ports().map_err(|error| error.to_string())? {
        println!("{:?}\t{}\t{:?}", port.direction, port.name, port.id);
    }
    Ok(())
}

/// Ask every port pair which KORG device answers (`korg_app::detect`).
fn detect(args: Vec<String>) -> Result<(), String> {
    let mut timeout = korg_app::detect::PROBE_TIMEOUT;
    match args.as_slice() {
        [] => {}
        [flag, ms] if flag == "--timeout" => {
            timeout = std::time::Duration::from_millis(
                ms.parse()
                    .map_err(|_| "timeout must be milliseconds".to_owned())?,
            )
        }
        _ => return Err("usage: korg-cli detect [--timeout ms]".to_owned()),
    }
    let catalog = korg_devices::Catalog::load().map_err(|error| error.to_string())?;
    let found = korg_app::detect::detect(MidirTransport::new, &catalog, timeout)?;
    if found.is_empty() {
        println!("no KORG device answered");
    }
    for device in &found {
        println!("{}", device.summary(&catalog));
    }
    Ok(())
}

/// Commands `query` may send: requests that only read device state
/// (`docs/midi-sysex.md`; 0x19 and 0x1A from the firmware, section 12).
const READ_COMMANDS: &[u8] = &[0x0E, 0x10, 0x12, 0x19, 0x1A];

/// The command byte of a KORG request frame
/// (`F0 42 4g 00 01 <model> <len> <len> 00 <command> ... F7`).
fn request_command(frame: &[u8]) -> Option<u8> {
    match frame {
        [0xF0, 0x42, id, 0x00, 0x01, _, _, _, 0x00, command, .., 0xF7] if id & 0xF0 == 0x40 => {
            Some(*command)
        }
        _ => None,
    }
}

fn parse_hex(text: &str) -> Result<Vec<u8>, String> {
    text.split_whitespace()
        .map(|byte| u8::from_str_radix(byte, 16).map_err(|_| format!("not a hex byte: {byte}")))
        .collect()
}

/// Send one read-only request and print every packet that arrives within
/// the timeout.  Anything that is not a known read is refused.
fn query(args: Vec<String>) -> Result<(), String> {
    let mut port = None;
    let mut frame = None;
    let mut timeout_ms = 1500u64;
    let mut index = 0;
    while index < args.len() {
        let value = |index: usize| {
            args.get(index + 1)
                .cloned()
                .ok_or_else(|| format!("{} needs a value", args[index]))
        };
        match args[index].as_str() {
            "--port" => port = Some(value(index)?),
            "--hex" => frame = Some(parse_hex(&value(index)?)?),
            "--timeout" => {
                timeout_ms = value(index)?
                    .parse()
                    .map_err(|_| "timeout must be milliseconds".to_owned())?
            }
            other => return Err(format!("unknown option: {other}")),
        }
        index += 2;
    }
    let port = port.ok_or_else(|| "--port is required".to_owned())?;
    let frame = frame.ok_or_else(|| "--hex is required".to_owned())?;
    match request_command(&frame) {
        Some(command) if READ_COMMANDS.contains(&command) => {}
        Some(command) => return Err(format!("command {command:#04x} is not a known read")),
        None => return Err("--hex is not a KORG request frame".to_owned()),
    }

    let mut transport = MidirTransport::new();
    let ports = transport.list_ports().map_err(|error| error.to_string())?;
    let input = find_port(&ports, &port, Direction::Input)?;
    let output = find_port(&ports, &port, Direction::Output)?;
    transport
        .open(&input, &output)
        .map_err(|error| error.to_string())?;
    transport.send(&frame).map_err(|error| error.to_string())?;
    let hex = |bytes: &[u8]| {
        bytes
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect::<Vec<_>>()
            .join(" ")
    };
    println!("sent:  {}", hex(&frame));
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
    let mut events = Vec::new();
    while std::time::Instant::now() < deadline {
        transport.wait(50);
        events.clear();
        transport.poll(&mut events);
        for event in &events {
            match event {
                korg_midi::TransportEvent::Packet(packet) => {
                    println!(
                        "recv:  {} ({} bytes)",
                        hex(&packet.bytes),
                        packet.bytes.len()
                    )
                }
                other => println!("event: {other:?}"),
            }
        }
    }
    transport.close();
    Ok(())
}

fn write_scene(args: Vec<String>) -> Result<(), String> {
    let mut port = None;
    let mut file = None;
    let mut scene = 0u8;
    let mut confirm = false;
    let mut timeout_ms = 3000u64;
    let mut index = 0;
    while index < args.len() {
        let value = |index: usize| {
            args.get(index + 1)
                .cloned()
                .ok_or_else(|| format!("{} needs a value", args[index]))
        };
        match args[index].as_str() {
            "--port" => {
                port = Some(value(index)?);
                index += 2;
            }
            "--file" => {
                file = Some(PathBuf::from(value(index)?));
                index += 2;
            }
            "--scene" => {
                scene = value(index)?
                    .parse()
                    .map_err(|_| "scene must be a number".to_owned())?;
                index += 2;
            }
            "--timeout" => {
                timeout_ms = value(index)?
                    .parse()
                    .map_err(|_| "timeout must be milliseconds".to_owned())?;
                index += 2;
            }
            "--confirm" => {
                confirm = true;
                index += 1;
            }
            other => return Err(format!("unknown option: {other}")),
        }
    }
    if !confirm {
        return Err("write-scene sends data to the device; pass --confirm".to_owned());
    }
    let port = port.ok_or_else(|| "--port is required".to_owned())?;
    let file = file.ok_or_else(|| "--file is required".to_owned())?;
    let bytes = std::fs::read(&file).map_err(|error| format!("{}: {error}", file.display()))?;
    let chunk = decode(&bytes).map_err(|error| format!("{}: {error}", file.display()))?;
    let payload = match chunk.payload {
        Payload::SceneSet(scenes) => scenes
            .get(usize::from(scene))
            .cloned()
            .ok_or_else(|| format!("scene {scene} is outside the file"))?,
        Payload::Scene(data) => data,
        Payload::Global(_) => return Err("write-scene needs a scene or scene-set file".to_owned()),
    };
    let model = match chunk.key.model.as_str() {
        "1480" => 0x136,
        "1482" => 0x137,
        other => return Err(format!("no measured layout for key {other}")),
    };
    let layout = korg_devices::measured_layout(model)
        .ok_or_else(|| format!("no layout for model {model:#06x}"))?;
    let header = layout
        .write_header
        .ok_or_else(|| format!("no measured write header for key {}", layout.key))?;
    if payload.len() != layout.scene_len {
        return Err(format!(
            "scene is {} bytes, layout expects {}",
            payload.len(),
            layout.scene_len
        ));
    }

    let mut transport = MidirTransport::new();
    let ports = transport.list_ports().map_err(|error| error.to_string())?;
    let input = find_port(&ports, &port, Direction::Input)?;
    let output = find_port(&ports, &port, Direction::Output)?;
    let mut session =
        MeasuredSession::new(transport, input, output, SystemClock::new(), timeout_ms);
    session.open().map_err(|error| error.to_string())?;
    let identity = session.mode_check().map_err(|error| error.to_string())?;
    if identity.model_id != layout.model {
        return Err(format!(
            "device model {:#06x} does not match file key {}",
            identity.model_id, layout.key
        ));
    }
    println!(
        "write: scene {scene}, {} bytes, model {:#06x}",
        payload.len(),
        identity.model_id
    );
    session
        .write_scene(scene, &payload, header)
        .map_err(|error| error.to_string())?;
    println!("wrote: scene {scene}");
    Ok(())
}

fn write_set(args: Vec<String>) -> Result<(), String> {
    let mut port = None;
    let mut file = None;
    let mut confirm = false;
    let mut timeout_ms = 3000u64;
    let mut index = 0;
    while index < args.len() {
        let value = |index: usize| {
            args.get(index + 1)
                .cloned()
                .ok_or_else(|| format!("{} needs a value", args[index]))
        };
        match args[index].as_str() {
            "--port" => {
                port = Some(value(index)?);
                index += 2;
            }
            "--file" => {
                file = Some(PathBuf::from(value(index)?));
                index += 2;
            }
            "--timeout" => {
                timeout_ms = value(index)?
                    .parse()
                    .map_err(|_| "timeout must be milliseconds".to_owned())?;
                index += 2;
            }
            "--confirm" => {
                confirm = true;
                index += 1;
            }
            other => return Err(format!("unknown option: {other}")),
        }
    }
    if !confirm {
        return Err("write-set sends data to the device; pass --confirm".to_owned());
    }
    let port = port.ok_or_else(|| "--port is required".to_owned())?;
    let file = file.ok_or_else(|| "--file is required".to_owned())?;
    let bytes = std::fs::read(&file).map_err(|error| format!("{}: {error}", file.display()))?;
    let chunk = decode(&bytes).map_err(|error| format!("{}: {error}", file.display()))?;
    let Payload::SceneSet(scenes) = chunk.payload else {
        return Err("write-set needs a scene-set file".to_owned());
    };
    let model = match chunk.key.model.as_str() {
        "1480" => 0x136,
        "1482" => 0x137,
        other => return Err(format!("no measured layout for key {other}")),
    };
    let layout = korg_devices::measured_layout(model)
        .ok_or_else(|| format!("no layout for model {model:#06x}"))?;
    let header = layout
        .write_header
        .ok_or_else(|| format!("no measured write header for key {}", layout.key))?;
    if scenes.len() != usize::from(layout.scene_count) {
        return Err(format!(
            "file has {} scenes, layout expects {}",
            scenes.len(),
            layout.scene_count
        ));
    }
    if scenes.iter().any(|scene| scene.len() != layout.scene_len) {
        return Err(format!("every scene must be {} bytes", layout.scene_len));
    }

    let mut transport = MidirTransport::new();
    let ports = transport.list_ports().map_err(|error| error.to_string())?;
    let input = find_port(&ports, &port, Direction::Input)?;
    let output = find_port(&ports, &port, Direction::Output)?;
    let mut session =
        MeasuredSession::new(transport, input, output, SystemClock::new(), timeout_ms);
    session.open().map_err(|error| error.to_string())?;
    let identity = session.mode_check().map_err(|error| error.to_string())?;
    if identity.model_id != layout.model {
        return Err(format!(
            "device model {:#06x} does not match file key {}",
            identity.model_id, layout.key
        ));
    }
    println!(
        "write: {} scenes of {} bytes, model {:#06x}",
        scenes.len(),
        layout.scene_len,
        identity.model_id
    );
    session
        .write_scene_set(&scenes, header)
        .map_err(|error| error.to_string())?;
    println!("wrote: {} scenes", scenes.len());
    Ok(())
}

fn write_global(args: Vec<String>) -> Result<(), String> {
    let mut port = None;
    let mut file = None;
    let mut confirm = false;
    let mut timeout_ms = 3000u64;
    let mut index = 0;
    while index < args.len() {
        let value = |index: usize| {
            args.get(index + 1)
                .cloned()
                .ok_or_else(|| format!("{} needs a value", args[index]))
        };
        match args[index].as_str() {
            "--port" => {
                port = Some(value(index)?);
                index += 2;
            }
            "--file" => {
                file = Some(PathBuf::from(value(index)?));
                index += 2;
            }
            "--timeout" => {
                timeout_ms = value(index)?
                    .parse()
                    .map_err(|_| "timeout must be milliseconds".to_owned())?;
                index += 2;
            }
            "--confirm" => {
                confirm = true;
                index += 1;
            }
            other => return Err(format!("unknown option: {other}")),
        }
    }
    if !confirm {
        return Err("write-global sends data to the device; pass --confirm".to_owned());
    }
    let port = port.ok_or_else(|| "--port is required".to_owned())?;
    let file = file.ok_or_else(|| "--file is required".to_owned())?;
    let bytes = std::fs::read(&file).map_err(|error| format!("{}: {error}", file.display()))?;
    let chunk = decode(&bytes).map_err(|error| format!("{}: {error}", file.display()))?;
    let Payload::Global(data) = chunk.payload else {
        return Err("write-global needs a global file".to_owned());
    };
    let model = match chunk.key.model.as_str() {
        "1480" => 0x136,
        "1482" => 0x137,
        other => return Err(format!("no measured layout for key {other}")),
    };
    let layout = korg_devices::measured_layout(model)
        .ok_or_else(|| format!("no layout for {model:#06x}"))?;
    let header = layout
        .global_write_header
        .ok_or_else(|| format!("no measured global write header for key {}", layout.key))?;
    if data.len() != layout.global_len {
        return Err(format!(
            "global is {} bytes, layout expects {}",
            data.len(),
            layout.global_len
        ));
    }
    let mut transport = MidirTransport::new();
    let ports = transport.list_ports().map_err(|error| error.to_string())?;
    let input = find_port(&ports, &port, Direction::Input)?;
    let output = find_port(&ports, &port, Direction::Output)?;
    let mut session =
        MeasuredSession::new(transport, input, output, SystemClock::new(), timeout_ms);
    session.open().map_err(|error| error.to_string())?;
    let identity = session.mode_check().map_err(|error| error.to_string())?;
    if identity.model_id != layout.model {
        return Err(format!(
            "device model {:#06x} does not match file key {}",
            identity.model_id, layout.key
        ));
    }
    println!(
        "write: global {} bytes, model {:#06x}",
        data.len(),
        identity.model_id
    );
    session
        .write_global(&data, header)
        .map_err(|error| error.to_string())?;
    println!("wrote: global");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_accepts_only_known_reads() {
        let read = parse_hex("F0 42 40 00 01 37 01 00 00 19 F7").unwrap();
        assert_eq!(request_command(&read), Some(0x19));
        let commit = parse_hex("F0 42 40 00 01 37 02 00 00 11 00 F7").unwrap();
        assert!(!READ_COMMANDS.contains(&request_command(&commit).unwrap()));
        assert_eq!(request_command(&[0xF0, 0xF7]), None);
        assert!(parse_hex("F0 zz").is_err());
    }

    fn strings(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn the_backend_flag_is_taken_from_anywhere() {
        let (flag, rest) =
            take_midi_backend(strings(&["ports", "--midi-backend", "jack"])).unwrap();
        assert_eq!(flag.as_deref(), Some("jack"));
        assert_eq!(rest, strings(&["ports"]));
        let (flag, rest) =
            take_midi_backend(strings(&["--midi-backend=alsa", "receive", "--port", "x"])).unwrap();
        assert_eq!(flag.as_deref(), Some("alsa"));
        assert_eq!(rest, strings(&["receive", "--port", "x"]));
        let (flag, rest) = take_midi_backend(strings(&["ports"])).unwrap();
        assert_eq!(flag, None);
        assert_eq!(rest, strings(&["ports"]));
        assert!(take_midi_backend(strings(&["ports", "--midi-backend"])).is_err());
    }
}
