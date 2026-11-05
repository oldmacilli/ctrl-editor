//! Device jobs over the measured exchange.
//!
//! One job opens the port, runs the identity check, and does one receive or
//! write with `korg_core::MeasuredSession`, the path verified on hardware
//! (`docs/rewrite/60-implementation-todo.md`).  It blocks; a front end runs
//! it on a worker thread and shows the progress it reports.

use korg_core::{Clock, MeasuredSession, ReceiveLayout, ReceivedPart};
use korg_devices::{measured_layout, measured_layout_for_key, MeasuredLayout};
use korg_midi::{Direction, MidiTransport, PortId, PortInfo};
use korg_model::{DeviceInfo, FileModelKey};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceJob {
    ReceiveSceneSet,
    /// Read one scene only.
    ReceiveScene {
        index: u8,
    },
    ReceiveGlobal,
    /// Write one scene, then read it back and compare.
    WriteScene {
        key: String,
        index: u8,
        payload: Vec<u8>,
    },
    /// Write every scene in order, then read each back and compare.
    WriteSceneSet {
        key: String,
        scenes: Vec<Vec<u8>>,
    },
    /// Write the global block, then read it back and compare.
    WriteGlobal {
        key: String,
        data: Vec<u8>,
    },
    /// One nanoKEY Studio touch-pad calibration step.
    Calibrate {
        command: u8,
    },
}

impl DeviceJob {
    pub fn writes(&self) -> bool {
        matches!(
            self,
            DeviceJob::WriteScene { .. }
                | DeviceJob::WriteSceneSet { .. }
                | DeviceJob::WriteGlobal { .. }
                | DeviceJob::Calibrate { .. }
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobProgress {
    Opened { input: String, output: String },
    Identified { model: u32, key: &'static str },
    Receiving,
    WritingGlobal,
    VerifyingGlobal,
    Writing { scene: u8, of: usize },
    Verifying { scene: u8, of: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobOutput {
    SceneSet {
        key: FileModelKey,
        scenes: Vec<Vec<u8>>,
    },
    Global {
        key: FileModelKey,
        data: Vec<u8>,
    },
    /// One scene, with its index in the device's set.
    Scene {
        index: u8,
        data: Vec<u8>,
    },
    /// Every written scene read back identical.
    Written {
        scenes: usize,
    },
    /// Global data read back identical.
    GlobalWritten,
    /// A calibration step was acknowledged.
    CalibrationDone,
}

/// The port of `direction` whose name contains `needle`, ignoring case.
pub fn find_port(ports: &[PortInfo], needle: &str, direction: Direction) -> Result<PortId, String> {
    let lower = needle.to_ascii_lowercase();
    let found = ports.iter().find(|port| {
        port.direction == direction && port.name.to_ascii_lowercase().contains(&lower)
    });
    // The nanoKEY Studio exposes one config port for input and output under
    // different names: "KEYBOARD/CTRL" in, "CTRL" out.
    let alias = match direction {
        Direction::Input => needle.replace(" CTRL", " KEYBOARD/CTRL"),
        Direction::Output => needle.replace(" KEYBOARD/CTRL", " CTRL"),
    };
    found
        .or_else(|| {
            let alias = alias.to_ascii_lowercase();
            ports.iter().find(|port| {
                port.direction == direction && port.name.to_ascii_lowercase().contains(&alias)
            })
        })
        .map(|port| port.id.clone())
        .ok_or_else(|| format!("no {direction:?} port matches {needle:?}"))
}

/// Check a write against the measured layout before any byte is sent.
fn write_layout(
    key: &str,
    scenes: &[&[u8]],
    whole_set: bool,
) -> Result<(MeasuredLayout, [u8; 4]), String> {
    let layout =
        measured_layout_for_key(key).ok_or_else(|| format!("no measured layout for key {key}"))?;
    let header = layout
        .write_header
        .ok_or_else(|| format!("no measured write header for key {key}"))?;
    if whole_set && scenes.len() != usize::from(layout.scene_count) {
        return Err(format!(
            "the set has {} scenes, the device has {}",
            scenes.len(),
            layout.scene_count
        ));
    }
    if let Some(scene) = scenes.iter().find(|scene| scene.len() != layout.scene_len) {
        return Err(format!(
            "a scene is {} bytes, the device expects {}",
            scene.len(),
            layout.scene_len
        ));
    }
    Ok((layout, header))
}

/// Run one job.  `input_port` and `output_port` are matched against the port
/// names and directions.
pub fn run_job<T: MidiTransport, C: Clock>(
    transport: T,
    clock: C,
    input_port: &str,
    output_port: &str,
    timeout_ms: u64,
    job: DeviceJob,
    progress: &mut dyn FnMut(JobProgress),
) -> Result<JobOutput, String> {
    run_job_restoring(
        transport,
        clock,
        input_port,
        output_port,
        timeout_ms,
        job,
        None,
        progress,
    )
}

/// The document part a verified write left on the device, for
/// `ControllerAction::MarkReceived`: only while the document still holds
/// exactly what the job wrote, so an edit made during the write stays
/// revertible.
pub fn written_part(job: &DeviceJob, document: &korg_core::Document) -> Option<ReceivedPart> {
    match job {
        DeviceJob::WriteScene { index, payload, .. } => document
            .scene_set
            .as_ref()?
            .scenes
            .get(usize::from(*index))
            .filter(|scene| scene.data == *payload)
            .map(|_| ReceivedPart::Scene(usize::from(*index))),
        DeviceJob::WriteSceneSet { scenes, .. } => {
            let set = document.scene_set.as_ref()?;
            (set.scenes.len() == scenes.len()
                && set.scenes.iter().zip(scenes).all(|(a, b)| a.data == *b))
            .then_some(ReceivedPart::SceneSet)
        }
        DeviceJob::WriteGlobal { data, .. } => document
            .global
            .as_ref()
            .filter(|global| global.data == *data)
            .map(|_| ReceivedPart::Global),
        _ => None,
    }
}

/// The scene `run_job_restoring` leaves the device on, or `None` when the
/// job does not change it.  The device does not announce a scene the host
/// selects (measured 2026-10-21), so a monitor must take it from here.
pub fn restore_scene(job: &DeviceJob, active_scene: Option<u8>) -> Option<u8> {
    match job {
        DeviceJob::ReceiveSceneSet | DeviceJob::WriteSceneSet { .. } => {
            Some(active_scene.unwrap_or(0))
        }
        DeviceJob::ReceiveScene { .. } | DeviceJob::WriteScene { .. } => active_scene,
        _ => None,
    }
}

/// `run_job`, then leave the device on `active_scene`.  Receiving or writing
/// a scene selects it on the device (`0x14`).  The official editor selects
/// scene 0 after a whole set and nothing after one scene. With the active
/// scene known from `SceneMonitor`, both kinds restore it instead.
#[allow(clippy::too_many_arguments)]
pub fn run_job_restoring<T: MidiTransport, C: Clock>(
    mut transport: T,
    clock: C,
    input_port: &str,
    output_port: &str,
    timeout_ms: u64,
    job: DeviceJob,
    active_scene: Option<u8>,
    progress: &mut dyn FnMut(JobProgress),
) -> Result<JobOutput, String> {
    let restore = restore_scene(&job, active_scene);
    // Validate a write before touching the device.
    let checked = match &job {
        DeviceJob::WriteScene {
            key,
            payload,
            index,
        } => {
            let checked = write_layout(key, &[payload], false)?;
            if *index >= checked.0.scene_count {
                return Err(format!(
                    "scene {} is outside the device's {} scenes",
                    index + 1,
                    checked.0.scene_count
                ));
            }
            Some(checked)
        }
        DeviceJob::WriteSceneSet { key, scenes } => {
            let refs: Vec<&[u8]> = scenes.iter().map(Vec::as_slice).collect();
            Some(write_layout(key, &refs, true)?)
        }
        DeviceJob::WriteGlobal { key, data } => {
            let layout = measured_layout_for_key(key)
                .ok_or_else(|| format!("no measured layout for key {key}"))?;
            let header = layout
                .global_write_header
                .ok_or_else(|| format!("no measured global write header for key {key}"))?;
            if data.len() != layout.global_len {
                return Err(format!(
                    "global is {} bytes, the device expects {}",
                    data.len(),
                    layout.global_len
                ));
            }
            Some((layout, header))
        }
        _ => None,
    };

    let ports = transport.list_ports().map_err(|error| error.to_string())?;
    let input = find_port(&ports, input_port, Direction::Input)?;
    let output = find_port(&ports, output_port, Direction::Output)?;
    let name = |id: &PortId| {
        ports
            .iter()
            .find(|info| &info.id == id)
            .map(|info| info.name.clone())
            .unwrap_or_default()
    };
    progress(JobProgress::Opened {
        input: name(&input),
        output: name(&output),
    });

    let mut session = MeasuredSession::new(transport, input, output, clock, timeout_ms);
    session.open().map_err(|error| error.to_string())?;
    let identity = session.mode_check().map_err(|error| error.to_string())?;
    let layout = measured_layout(identity.model_id)
        .ok_or_else(|| format!("no measured layout for model {:#06x}", identity.model_id))?;
    progress(JobProgress::Identified {
        model: identity.model_id,
        key: layout.key,
    });
    if let Some((expected, _)) = &checked {
        if expected.model != layout.model {
            return Err(format!(
                "the device is model {:#06x} ({}), the document is {}",
                layout.model, layout.key, expected.key
            ));
        }
    }
    let key = FileModelKey::new(layout.key).map_err(|error| error.to_string())?;
    let fail = |error: korg_core::MeasuredError| error.to_string();
    // A receive sends nothing that changes the device, and the identity was
    // checked above; repeating it per request doubles the Bluetooth time.
    if !job.writes() {
        session.check_identity_once();
    }

    let output = (|| match job {
        DeviceJob::ReceiveSceneSet => {
            progress(JobProgress::Receiving);
            let scenes = session
                .receive_scene_set(ReceiveLayout {
                    scene_count: layout.scene_count,
                    scene_len: layout.scene_len,
                    global_len: layout.global_len,
                })
                .map_err(fail)?;
            Ok(JobOutput::SceneSet { key, scenes })
        }
        DeviceJob::ReceiveScene { index } => {
            if index >= layout.scene_count {
                return Err(format!(
                    "scene {} is outside the device's {} scenes",
                    index + 1,
                    layout.scene_count
                ));
            }
            progress(JobProgress::Receiving);
            let data = session
                .receive_scene(index, layout.scene_len)
                .map_err(fail)?;
            Ok(JobOutput::Scene { index, data })
        }
        DeviceJob::ReceiveGlobal => {
            progress(JobProgress::Receiving);
            let data = session.receive_global(layout.global_len).map_err(fail)?;
            Ok(JobOutput::Global { key, data })
        }
        DeviceJob::WriteScene { index, payload, .. } => {
            let header = checked.map(|(_, header)| header).unwrap_or_default();
            progress(JobProgress::Writing {
                scene: index,
                of: 1,
            });
            session.write_scene(index, &payload, header).map_err(fail)?;
            progress(JobProgress::Verifying {
                scene: index,
                of: 1,
            });
            verify(&mut session, index, &payload, layout.scene_len)?;
            Ok(JobOutput::Written { scenes: 1 })
        }
        DeviceJob::Calibrate { command } => {
            progress(JobProgress::WritingGlobal);
            session.calibrate(command).map_err(fail)?;
            Ok(JobOutput::CalibrationDone)
        }
        DeviceJob::WriteGlobal { data, .. } => {
            let header = checked.map(|(_, header)| header).unwrap_or_default();
            progress(JobProgress::WritingGlobal);
            session.write_global(&data, header).map_err(fail)?;
            progress(JobProgress::VerifyingGlobal);
            let actual = session.receive_global(layout.global_len).map_err(fail)?;
            if actual != data {
                let at = actual
                    .iter()
                    .zip(&data)
                    .position(|(a, b)| a != b)
                    .unwrap_or(actual.len().min(data.len()));
                return Err(format!(
                    "global read back different from what was written (first difference at byte {at})"
                ));
            }
            Ok(JobOutput::GlobalWritten)
        }
        DeviceJob::WriteSceneSet { scenes, .. } => {
            let header = checked.map(|(_, header)| header).unwrap_or_default();
            // `write_scene_set` is this loop; looping here reports progress.
            for (index, scene) in scenes.iter().enumerate() {
                progress(JobProgress::Writing {
                    scene: index as u8,
                    of: scenes.len(),
                });
                session
                    .write_scene(index as u8, scene, header)
                    .map_err(fail)?;
            }
            for (index, scene) in scenes.iter().enumerate() {
                progress(JobProgress::Verifying {
                    scene: index as u8,
                    of: scenes.len(),
                });
                verify(&mut session, index as u8, scene, layout.scene_len)?;
            }
            Ok(JobOutput::Written {
                scenes: scenes.len(),
            })
        }
    })()?;
    if let Some(scene) = restore {
        session
            .select_scene(scene)
            .map_err(|error| format!("could not reselect scene {}: {error}", scene + 1))?;
    }
    Ok(output)
}

fn verify<T: MidiTransport, C: Clock>(
    session: &mut MeasuredSession<T, C>,
    index: u8,
    expected: &[u8],
    scene_len: usize,
) -> Result<(), String> {
    let actual = session
        .receive_scene(index, scene_len)
        .map_err(|error| format!("read-back of scene {}: {error}", index + 1))?;
    if actual != expected {
        let at = actual
            .iter()
            .zip(expected)
            .position(|(a, b)| a != b)
            .unwrap_or(actual.len().min(expected.len()));
        return Err(format!(
            "scene {} read back different from what was written (first difference at byte {at})",
            index + 1
        ));
    }
    Ok(())
}

/// Whether a catalog device passes the device-screen filter.
///
/// `connected_only` keeps the devices that have a MIDI port.  `search`
/// matches the device name, the class, or a port that belongs to the device,
/// ignoring case.  Both front ends use this, so they list the same devices.
pub fn device_passes_filter(
    device: &DeviceInfo,
    connected: &[String],
    connected_only: bool,
    ports: &[PortInfo],
    search: &str,
) -> bool {
    if connected_only && !connected.contains(&device.class) {
        return false;
    }
    let needle = search.trim().to_lowercase();
    if needle.is_empty() {
        return true;
    }
    let name = device.device.to_lowercase();
    if name.contains(&needle) || device.class.to_lowercase().contains(&needle) {
        return true;
    }
    ports.iter().any(|port| {
        let port = port.name.to_lowercase();
        port.contains(&name) && port.contains(&needle)
    })
}

/// The files of `device` under `root`, as `(label, path)`.
///
/// The label is the last part of the catalog path.  Both front ends list the
/// same files.
pub fn device_files(device: &DeviceInfo, root: &Path) -> Vec<(String, PathBuf)> {
    device
        .files
        .iter()
        .map(|file| {
            let label = file.rsplit('/').next().unwrap_or(file).to_owned();
            (label, root.join(file))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use korg_core::Millis;
    use korg_midi::MockTransport;
    use korg_sysex::{pack_7in8, SYSEX_END, SYSEX_START};
    use std::cell::Cell;

    struct StepClock(Cell<u64>);

    impl Clock for StepClock {
        fn now(&self) -> Millis {
            let value = self.0.get();
            self.0.set(value + 10);
            Millis(value)
        }
    }

    fn ports() -> Vec<PortInfo> {
        ["in", "out"]
            .into_iter()
            .zip([Direction::Input, Direction::Output])
            .map(|(id, direction)| PortInfo {
                id: PortId::new(id),
                name: format!("nanoKONTROL Studio {id}"),
                direction,
            })
            .collect()
    }

    fn run(
        transport: MockTransport,
        job: DeviceJob,
    ) -> (Result<JobOutput, String>, Vec<JobProgress>) {
        let mut seen = Vec::new();
        let result = run_job(
            transport,
            StepClock(Cell::new(0)),
            "nanokontrol",
            "nanokontrol",
            100,
            job,
            &mut |event| seen.push(event),
        );
        (result, seen)
    }

    const DEVICE: u8 = 0x0F;

    /// Identity reply for model 0x137 (nanoKONTROL Studio, key 1482).
    fn identity() -> Vec<u8> {
        vec![
            SYSEX_START,
            0x7E,
            DEVICE,
            0x06,
            0x02,
            0x42,
            0x37,
            0x01,
            0x00,
            0x00,
            0x08,
            0x00,
            0x01,
            0x00,
            SYSEX_END,
        ]
    }

    fn frame(body: &[u8], payload: &[u8]) -> Vec<u8> {
        let mut out = vec![SYSEX_START, 0x42, 0x40 | DEVICE, 0x00, 0x01, 0x37];
        out.extend_from_slice(body);
        out.extend_from_slice(&pack_7in8(payload));
        out.push(SYSEX_END);
        out
    }

    fn script(transport: &mut MockTransport, replies: &[Vec<u8>]) {
        for reply in replies {
            transport.push_packet(identity());
            transport.push_packet(reply.clone());
        }
    }

    fn info() -> Vec<u8> {
        frame(&[0x02, 0x00, 0x00, 0x42, 0x00], &[])
    }

    fn selected() -> Vec<u8> {
        frame(&[0x01, 0x00, 0x00, 0x23], &[])
    }

    fn scene(data: &[u8]) -> Vec<u8> {
        frame(&[0x01, 0x05, 0x00, 0x40], data)
    }

    #[test]
    fn receives_a_scene_set_with_the_device_layout() {
        let scenes: Vec<Vec<u8>> = (0..5u8).map(|index| vec![index; 560]).collect();
        let mut transport = MockTransport::new(ports());
        transport.push_packet(identity());
        let mut replies = vec![info()];
        for data in &scenes {
            replies.push(selected());
            replies.push(scene(data));
        }
        // The device ends on scene 1 again, like the official editor.
        replies.push(selected());
        // A receive checks the identity once, not before every request.
        for reply in replies {
            transport.push_packet(reply);
        }
        let (result, seen) = run(transport, DeviceJob::ReceiveSceneSet);
        assert_eq!(
            result.unwrap(),
            JobOutput::SceneSet {
                key: FileModelKey::new("1482").unwrap(),
                scenes
            }
        );
        assert!(seen.contains(&JobProgress::Identified {
            model: 0x137,
            key: "1482"
        }));
    }

    #[test]
    fn a_write_is_read_back_and_compared() {
        let payload = vec![3u8; 560];
        let written = |read_back: &[u8]| {
            let mut transport = MockTransport::new(ports());
            transport.push_packet(identity());
            script(
                &mut transport,
                &[
                    info(),
                    frame(&[0x01, 0x00, 0x00, 0x23], &[]),
                    frame(&[0x01, 0x00, 0x00, 0x21], &[]),
                    selected(),
                    scene(read_back),
                ],
            );
            run(
                transport,
                DeviceJob::WriteScene {
                    key: "1482".into(),
                    index: 2,
                    payload: payload.clone(),
                },
            )
            .0
        };
        assert_eq!(written(&payload), Ok(JobOutput::Written { scenes: 1 }));
        let mut changed = payload.clone();
        changed[17] = 9;
        assert!(written(&changed).unwrap_err().contains("byte 17"));
    }

    #[test]
    fn calibration_rejects_a_different_studio_model() {
        let mut transport = MockTransport::new(ports());
        transport.push_packet(identity());
        let (result, _) = run(transport, DeviceJob::Calibrate { command: 0x03 });
        assert!(result
            .unwrap_err()
            .contains("calibration is not available for model"));
    }

    #[test]
    fn calibration_rechecks_model_before_sending() {
        let mut transport = MockTransport::new(ports());
        let mut key_identity = identity();
        key_identity[6] = 0x36;
        let mut key_info = info();
        key_info[5] = 0x36;
        transport.push_packet(key_identity.clone());
        transport.push_packet(key_identity);
        transport.push_packet(key_info);
        transport.push_packet(identity());
        let (result, _) = run(transport, DeviceJob::Calibrate { command: 0x03 });
        assert!(result
            .unwrap_err()
            .contains("calibration is not available for model"));
    }

    #[test]
    fn a_calibration_step_waits_for_the_data_ack() {
        let mut transport = MockTransport::new(ports());
        let mut key_identity = identity();
        key_identity[6] = 0x36;
        transport.push_packet(key_identity.clone());
        for mut reply in [info(), frame(&[0x01, 0x00, 0x00, 0x23], &[])] {
            reply[5] = 0x36;
            transport.push_packet(key_identity.clone());
            transport.push_packet(reply);
        }
        let (result, _) = run(transport, DeviceJob::Calibrate { command: 0x03 });
        assert_eq!(result, Ok(JobOutput::CalibrationDone));
    }

    #[test]
    fn a_global_write_is_read_back_and_compared() {
        let data = vec![7u8; 128];
        let global = |read_back: &[u8]| {
            let mut transport = MockTransport::new(ports());
            transport.push_packet(identity());
            script(
                &mut transport,
                &[
                    info(),
                    frame(&[0x01, 0x00, 0x00, 0x23], &[]),
                    info(),
                    frame(&[0x14, 0x01, 0x00, 0x51], read_back),
                ],
            );
            run(
                transport,
                DeviceJob::WriteGlobal {
                    key: "1482".into(),
                    data: data.clone(),
                },
            )
            .0
        };
        assert_eq!(global(&data), Ok(JobOutput::GlobalWritten));
        let mut changed = data.clone();
        changed[64] = 1;
        assert!(global(&changed).unwrap_err().contains("byte 64"));
    }

    #[test]
    fn a_wrong_size_global_write_sends_nothing() {
        let (result, seen) = run(
            MockTransport::new(ports()),
            DeviceJob::WriteGlobal {
                key: "1482".into(),
                data: vec![0; 10],
            },
        );
        assert!(result.unwrap_err().contains("128"));
        assert!(seen.is_empty());
    }

    #[test]
    fn a_write_to_another_model_is_refused_after_identity() {
        let mut transport = MockTransport::new(ports());
        transport.push_packet(identity());
        let (result, _) = run(
            transport,
            DeviceJob::WriteScene {
                key: "1480".into(),
                index: 0,
                payload: vec![0; 240],
            },
        );
        assert!(result.unwrap_err().contains("document is 1480"));
    }

    #[test]
    fn a_wrong_size_write_sends_nothing() {
        let (result, seen) = run(
            MockTransport::new(ports()),
            DeviceJob::WriteScene {
                key: "1482".into(),
                index: 0,
                payload: vec![0; 10],
            },
        );
        assert!(result.unwrap_err().contains("560"));
        assert!(seen.is_empty());
    }

    #[test]
    fn an_unmeasured_key_is_refused() {
        let (result, _) = run(
            MockTransport::new(ports()),
            DeviceJob::WriteSceneSet {
                key: "1850".into(),
                scenes: vec![],
            },
        );
        assert!(result.unwrap_err().contains("no measured layout"));
    }

    #[test]
    fn the_nanokey_config_ports_resolve_by_direction() {
        let ports = vec![
            PortInfo {
                id: PortId::new("key-in"),
                name: "nanoKEY Studio KEYBOARD/CTRL".into(),
                direction: Direction::Input,
            },
            PortInfo {
                id: PortId::new("key-out"),
                name: "nanoKEY Studio CTRL".into(),
                direction: Direction::Output,
            },
        ];
        assert_eq!(
            find_port(&ports, "nanoKEY Studio CTRL", Direction::Input).unwrap(),
            PortId::new("key-in")
        );
        assert_eq!(
            find_port(&ports, "nanoKEY Studio CTRL", Direction::Output).unwrap(),
            PortId::new("key-out")
        );
    }

    #[test]
    fn a_missing_port_is_an_error() {
        let (result, _) = run(MockTransport::new(Vec::new()), DeviceJob::ReceiveGlobal);
        assert!(result.unwrap_err().contains("no Input port"));
    }

    #[test]
    fn a_silent_device_times_out_after_opening() {
        let (result, seen) = run(MockTransport::new(ports()), DeviceJob::ReceiveSceneSet);
        assert!(result.unwrap_err().contains("no reply"));
        assert!(matches!(seen.as_slice(), [JobProgress::Opened { .. }]));
    }
}

#[cfg(test)]
mod filter_tests {
    use super::*;
    use std::collections::BTreeMap;

    fn device(name: &str, class: &str, files: &[&str]) -> DeviceInfo {
        DeviceInfo {
            device: name.to_owned(),
            class: class.to_owned(),
            localized_name: None,
            file_model_key: None,
            extensions: Vec::new(),
            ostypes: BTreeMap::new(),
            scenes: None,
            scene_payload_size: None,
            global_payload_size: None,
            files: files.iter().map(|file| (*file).to_owned()).collect(),
        }
    }

    fn port(name: &str) -> PortInfo {
        PortInfo {
            id: PortId::new(name),
            name: name.to_owned(),
            direction: Direction::Input,
        }
    }

    #[test]
    fn the_filter_keeps_only_connected_devices() {
        let studio = device("nanoKONTROL Studio", "X14802", &[]);
        let keystage = device("Keystage", "X20870", &[]);
        let connected = vec!["X14802".to_owned()];
        assert!(device_passes_filter(&studio, &connected, true, &[], ""));
        assert!(!device_passes_filter(&keystage, &connected, true, &[], ""));
        assert!(device_passes_filter(&keystage, &connected, false, &[], ""));
    }

    #[test]
    fn the_search_matches_a_name_a_class_or_a_port() {
        let studio = device("nanoKONTROL Studio", "X14802", &[]);
        let keystage = device("Keystage", "X20870", &[]);
        let ports = vec![port("Keystage KBD/CTRL")];
        assert!(device_passes_filter(&studio, &[], false, &ports, "KONTROL"));
        assert!(device_passes_filter(&studio, &[], false, &ports, "x148"));
        // A port matches only the device it belongs to.
        assert!(device_passes_filter(&keystage, &[], false, &ports, "KBD"));
        assert!(!device_passes_filter(&studio, &[], false, &ports, "KBD"));
        assert!(!device_passes_filter(
            &keystage,
            &[],
            false,
            &ports,
            "microkorg"
        ));
        assert!(device_passes_filter(&keystage, &[], false, &ports, "  "));
    }

    #[test]
    fn the_file_list_uses_the_catalog_paths() {
        let device = device("Keystage", "X20870", &["Scenes/Set.nktrl_st_set"]);
        let files = device_files(&device, Path::new("/products"));
        assert_eq!(
            files,
            vec![(
                "Set.nktrl_st_set".to_owned(),
                PathBuf::from("/products/Scenes/Set.nktrl_st_set")
            )]
        );
    }
}
