//! Open, load and save documents.
//!
//! Taken from the Slint shell (`korg-slint/src/main.rs`) so that every front
//! end loads files and devices the same way.  These functions do no file
//! dialogs; the front end picks the path.

use korg_core::{Clock, ControllerAction, ControllerInput, DeviceTarget, EditorController};
use korg_devices::Catalog;
use korg_format::{Chunk, ChunkKey, Payload};
use korg_midi::{Direction, MidiTransport, PortInfo};
use korg_model::{ChunkKind, ClassId, DeviceInfo, FileModelKey, GlobalData, Scene, SceneSet};
use std::path::{Path, PathBuf};

/// MIDI channel for SysEx.  The device's own global channel is in its global
/// data, which the editor may not have loaded.  Placeholder, not measured.
pub const SYSEX_CHANNEL: u8 = 0;
/// Reply timeout per transfer step.  The original app's value is not
/// recovered; this is a placeholder.
pub const TRANSFER_TIMEOUT_MS: u64 = 3_000;
/// A scene set the editor can address: the device selects a scene with one
/// 7-bit SysEx argument, so no later conversion to `u8` may truncate.
pub const MAX_SCENES: usize = 255;
/// Largest file the editor opens.  The biggest vendor file under
/// `/Applications/KORG KONTROL EDITOR/Products` is about 10 KiB; this leaves
/// room for larger user sets without reading an arbitrary file into memory.
pub const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;

/// File extensions the editor opens.
pub const EXTENSIONS: &[&str] = &[
    "microkey_data",
    "microkey25_data",
    "microkey2_data",
    "microkey25air_data",
    "nanokey2_data",
    "nanokey_st_data",
    "nanokey_st_set",
    "nanokey_st_glob",
    "nanopad2_data",
    "nanopad2_set",
    "nanopad2_glob",
    "nktrl2_data",
    "nktrl_st_data",
    "nktrl_st_set",
    "nktrl_st_glob",
    "taktile_data",
    "taktile_set",
    "taktile_glob",
    "tr_taktile_data",
    "tr_taktile_set",
    "tr_taktile_glob",
    "keystage_data",
    "keystage_set",
    "keystage_glob",
    "nanokey_fold_data",
    "nanokey_fold_set",
    "nanokey_fold_glob",
];

/// What the transfer engine needs to know about a catalog device.
pub fn device_target(device: &DeviceInfo) -> DeviceTarget {
    DeviceTarget {
        class: ClassId::new(device.class.clone()),
        descriptor: korg_devices::sysex_descriptor(&device.class),
        channel: SYSEX_CHANNEL,
        scene_payload_size: device.scene_payload_size.map(usize::from),
        global_payload_size: device.global_payload_size.map(usize::from),
        timeout_ms: TRANSFER_TIMEOUT_MS,
    }
}

/// The factory data directory: `KORG_PRODUCTS_DIR`, else the installed app.
pub fn products_dir() -> PathBuf {
    std::env::var_os("KORG_PRODUCTS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/Applications/KORG KONTROL EDITOR/Products"))
}

pub use korg_model::scene_name;

/// Turn raw scene payloads into named scenes.
pub fn named_scenes(model: &FileModelKey, payloads: Vec<Vec<u8>>) -> Vec<Scene> {
    payloads
        .into_iter()
        .map(|data| Scene {
            name: scene_name(model, &data),
            data,
        })
        .collect()
}

/// The factory scene file for `device`: one of its own extensions and a
/// scene set or scene data file.  A model key can cover two devices, so the
/// extension selects the class.
fn factory_file(device: &DeviceInfo) -> Option<&str> {
    device.files.iter().find_map(|file| {
        let extension = file.rsplit('.').next()?;
        (device.extensions.iter().any(|owned| owned == extension)
            && (extension.ends_with("_set") || extension.ends_with("_data")))
        .then_some(file.as_str())
    })
}

/// The device's factory scene set from `products`, if it ships one.
pub fn factory_scenes(device: &DeviceInfo, products: &Path) -> Option<Vec<Scene>> {
    let file = factory_file(device)?;
    let bytes = std::fs::read(products.join(file)).ok()?;
    let chunk = korg_format::decode(&bytes).ok()?;
    let model = device.file_key()?;
    match chunk.payload {
        Payload::SceneSet(scenes) => Some(named_scenes(&model, scenes)),
        Payload::Scene(data) => Some(named_scenes(&model, vec![data])),
        Payload::Global(_) => None,
    }
}

/// Open a catalog device offline: factory scenes, or zeroed scenes.
pub fn open_device<T: MidiTransport, C: Clock>(
    controller: &mut EditorController<T, C>,
    device: &DeviceInfo,
    products: &Path,
) -> Result<String, String> {
    let model = device
        .file_key()
        .ok_or_else(|| format!("{} has no file model key", device.device))?;
    let size = device
        .scene_payload_size
        .ok_or_else(|| format!("{} has no scene payload size", device.device))?;
    let count = device.scenes.unwrap_or(1) as usize;
    let factory = factory_scenes(device, products);
    let unreadable = factory.is_none();
    let scenes = factory.unwrap_or_else(|| {
        (0..count)
            .map(|index| Scene {
                name: Some(format!("Scene {}", index + 1)),
                data: vec![0u8; size as usize],
            })
            .collect()
    });
    controller.set_schema(korg_devices::parameter_schema(&device.class, "scene"));
    controller.set_device(Some(device_target(device)));
    controller.load(ControllerInput::SceneSet(SceneSet {
        model: model.clone(),
        element_size: size,
        scenes,
    }));
    // Work offline before the first receive: the Global dialog edits this
    // default block, and a later Receive Global replaces it.
    if let Some(global_size) = device.global_payload_size {
        let mut data = vec![0u8; usize::from(global_size)];
        if data.len() >= 59 {
            let bpm = 1200u16;
            data[57] = (bpm & 0xFF) as u8;
            data[58] = (bpm >> 8) as u8;
        }
        controller.load(ControllerInput::Global(GlobalData { model, data }));
    }
    Ok(if unreadable {
        format!(
            "Opened {} (factory scenes unreadable; starting from empty scenes)",
            device.device
        )
    } else {
        format!("Opened {}", device.device)
    })
}

/// Decode a file and load it.  Returns the catalog class, when one matches,
/// and a status line.
pub fn load_bytes<T: MidiTransport, C: Clock>(
    controller: &mut EditorController<T, C>,
    catalog: &Catalog,
    bytes: &[u8],
    extension: Option<&str>,
    label: &str,
) -> Result<(Option<String>, String), String> {
    let chunk = korg_format::decode(bytes).map_err(|error| format!("{label}: {error}"))?;
    if let Payload::SceneSet(scenes) = &chunk.payload {
        if scenes.len() > MAX_SCENES {
            return Err(format!(
                "{label}: scene set has {} scenes; the editor addresses at most {MAX_SCENES}",
                scenes.len()
            ));
        }
    }
    let model = chunk.key.model.clone();
    let class = catalog
        .find_by_file_key(&model)
        .into_iter()
        .find(|device| {
            extension.is_none_or(|extension| {
                device
                    .extensions
                    .iter()
                    .any(|candidate| candidate == extension)
            })
        })
        .map(|device| device.class.clone());
    let payload_name = if matches!(&chunk.payload, Payload::Global(_)) {
        "global"
    } else {
        "scene"
    };
    let schema = class
        .as_deref()
        .and_then(|class| korg_devices::parameter_schema(class, payload_name));
    let target = class
        .as_deref()
        .and_then(|class| catalog.find_by_class(class))
        .map(device_target);
    controller.set_schema(schema);
    controller.set_device(target);
    let status = format!("Loaded {label} ({})", chunk.key);
    match chunk.payload {
        Payload::SceneSet(scenes) => {
            let element_size = scenes.first().map(|scene| scene.len()).unwrap_or(0) as u16;
            let scenes = named_scenes(&model, scenes);
            controller.load(ControllerInput::SceneSet(SceneSet {
                model,
                element_size,
                scenes,
            }));
        }
        Payload::Scene(data) => {
            let element_size = data.len() as u16;
            let scenes = named_scenes(&model, vec![data]);
            controller.load(ControllerInput::SceneSet(SceneSet {
                model,
                element_size,
                scenes,
            }));
        }
        Payload::Global(data) => {
            controller.load(ControllerInput::Global(GlobalData { model, data }));
        }
    }
    Ok((class, status))
}

/// Read and load a file from disk.
pub fn load_path<T: MidiTransport, C: Clock>(
    controller: &mut EditorController<T, C>,
    catalog: &Catalog,
    path: &Path,
) -> Result<(Option<String>, String), String> {
    let metadata =
        std::fs::metadata(path).map_err(|error| format!("{}: {error}", path.display()))?;
    if !metadata.is_file() {
        return Err(format!("{}: not a regular file", path.display()));
    }
    if metadata.len() > MAX_FILE_BYTES {
        return Err(format!(
            "{}: {} bytes; the editor opens at most {} bytes",
            path.display(),
            metadata.len(),
            MAX_FILE_BYTES
        ));
    }
    let bytes = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let extension = path.extension().and_then(|extension| extension.to_str());
    load_bytes(
        controller,
        catalog,
        &bytes,
        extension,
        &path.display().to_string(),
    )
}

/// The loaded document as a chunk, with the file extension to save it under.
/// The scene set wins when both a scene set and global data are loaded.
pub fn export_chunk<T: MidiTransport, C: Clock>(
    controller: &EditorController<T, C>,
    catalog: &Catalog,
    class: Option<&str>,
) -> Result<(Chunk, String), String> {
    let document = controller.session().document();
    let extension_for = |pick: fn(&str) -> bool, fallback: &str| {
        class
            .and_then(|class| catalog.find_by_class(class))
            .and_then(|device| device.extensions.iter().find(|e| pick(e)).cloned())
            .unwrap_or_else(|| fallback.to_owned())
    };
    if let Some(scene_set) = &document.scene_set {
        let set_extension = class
            .and_then(|class| catalog.find_by_class(class))
            .and_then(|device| {
                device
                    .extensions
                    .iter()
                    .find(|e| e.ends_with("_set"))
                    .cloned()
            });
        Ok((
            Chunk {
                key: ChunkKey::new(scene_set.model.clone(), ChunkKind::SceneSet),
                payload: Payload::SceneSet(
                    scene_set
                        .scenes
                        .iter()
                        .map(|scene| scene.data.clone())
                        .collect(),
                ),
            },
            // A scene set is a `_set` file (the vendor ships
            // `nanoKEY Studio Scene Set.nanokey_st_set`); models without one
            // keep their set in `_data`.  The catalog lists `_data` first.
            set_extension.unwrap_or_else(|| extension_for(|e| e.ends_with("_data"), "korg_data")),
        ))
    } else if let Some(global) = &document.global {
        Ok((
            Chunk {
                key: ChunkKey::new(global.model.clone(), ChunkKind::GlobalData),
                payload: Payload::Global(global.data.clone()),
            },
            extension_for(|e| e.ends_with("_glob"), "korg_glob"),
        ))
    } else {
        Err("No document is loaded.".to_owned())
    }
}

/// Export one scene of the loaded set as a `_data` chunk.
pub fn export_scene_chunk<T: MidiTransport, C: Clock>(
    controller: &EditorController<T, C>,
    catalog: &Catalog,
    class: Option<&str>,
    index: usize,
) -> Result<(Chunk, String), String> {
    let document = controller.session().document();
    let set = document
        .scene_set
        .as_ref()
        .ok_or_else(|| "No scene set is loaded.".to_owned())?;
    let scene = set
        .scenes
        .get(index)
        .ok_or_else(|| format!("Scene {} is missing.", index + 1))?;
    let extension = class
        .and_then(|class| catalog.find_by_class(class))
        .and_then(|device| {
            device
                .extensions
                .iter()
                .find(|extension| extension.ends_with("_data"))
                .cloned()
        })
        .unwrap_or_else(|| "korg_data".to_owned());
    Ok((
        Chunk {
            key: ChunkKey::new(set.model.clone(), ChunkKind::SceneData),
            payload: Payload::Scene(scene.data.clone()),
        },
        extension,
    ))
}

/// Write one scene without marking the whole document saved.
pub fn save_scene_path<T: MidiTransport, C: Clock>(
    controller: &mut EditorController<T, C>,
    catalog: &Catalog,
    class: Option<&str>,
    index: usize,
    path: &Path,
) -> Result<String, String> {
    let (chunk, _) = export_scene_chunk(controller, catalog, class, index)?;
    let bytes = korg_format::encode(&chunk).map_err(|error| error.to_string())?;
    std::fs::write(path, bytes).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(format!("Saved scene {} to {}", index + 1, path.display()))
}

/// The current document as one or more SysEx write frames, for a `.syx` dump.
/// The device id is the editor's zero-based SysEx channel; measured layouts
/// supply the per-model data headers.
pub fn sysex_dump<T: MidiTransport, C: Clock>(
    controller: &EditorController<T, C>,
    _class: Option<&str>,
) -> Result<Vec<u8>, String> {
    let document = controller.session().document();
    if let Some(set) = &document.scene_set {
        let layout = korg_devices::measured_layout_for_key(set.model.as_str())
            .ok_or_else(|| format!("no measured layout for key {}", set.model))?;
        let header = layout
            .write_header
            .ok_or_else(|| format!("no measured scene write header for key {}", layout.key))?;
        let mut out = Vec::new();
        for (index, scene) in set.scenes.iter().enumerate() {
            out.extend_from_slice(
                &korg_sysex::build_write_frame(0, layout.model, header, &scene.data)
                    .map_err(|error| error.to_string())?,
            );
            out.extend_from_slice(
                &korg_sysex::build_commit(0, layout.model, index as u8)
                    .map_err(|error| error.to_string())?,
            );
        }
        return Ok(out);
    }
    if let Some(global) = &document.global {
        let layout = korg_devices::measured_layout_for_key(global.model.as_str())
            .ok_or_else(|| format!("no measured layout for key {}", global.model))?;
        let header = layout
            .global_write_header
            .ok_or_else(|| format!("no measured global write header for key {}", layout.key))?;
        return korg_sysex::build_write_frame(0, layout.model, header, &global.data)
            .map_err(|error| error.to_string());
    }
    Err("No document is loaded.".to_owned())
}

/// Write the SysEx dump to `path`.
pub fn save_sysex_path<T: MidiTransport, C: Clock>(
    controller: &EditorController<T, C>,
    class: Option<&str>,
    path: &Path,
) -> Result<String, String> {
    let bytes = sysex_dump(controller, class)?;
    std::fs::write(path, &bytes).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(format!(
        "Saved {} SysEx bytes to {}",
        bytes.len(),
        path.display()
    ))
}

/// Encode the document and write it to `path`; marks the document saved.
pub fn save_path<T: MidiTransport, C: Clock>(
    controller: &mut EditorController<T, C>,
    catalog: &Catalog,
    class: Option<&str>,
    path: &Path,
) -> Result<String, String> {
    let (chunk, _) = export_chunk(controller, catalog, class)?;
    let bytes = korg_format::encode(&chunk).map_err(|error| error.to_string())?;
    std::fs::write(path, bytes).map_err(|error| format!("{}: {error}", path.display()))?;
    controller.dispatch(ControllerAction::MarkSaved);
    Ok(format!("Saved {}", path.display()))
}

/// Catalog classes with a MIDI port whose name matches the device name.
pub fn connected_classes(ports: &[PortInfo], catalog: &Catalog) -> Vec<String> {
    catalog
        .devices
        .iter()
        .filter(|device| port_matches(ports, device).is_some())
        .map(|device| device.class.clone())
        .collect()
}

/// Port-name prefixes that belong to this device. Never a bare first word:
/// `nanoKEY Fold` must not match `nanoKEY Studio`.
pub fn device_port_prefixes(device: &DeviceInfo) -> Vec<String> {
    let mut names = Vec::new();
    for name in [
        Some(device.device.as_str()),
        device.localized_name.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        names.push(name.to_lowercase());
        if let Some((head, rest)) = name.split_once('/') {
            let prefix = head
                .rsplit_once(' ')
                .map(|(before, _)| before)
                .unwrap_or(head);
            for variant in rest.split('/') {
                names.push(format!("{prefix} {variant}").to_lowercase());
            }
        }
    }
    names
}

/// True when `port_name` starts with one of the device's own name prefixes,
/// or when the device answered on that port (`detect::remember`).
pub fn port_name_matches_device(device: &DeviceInfo, port_name: &str) -> bool {
    let port = port_name.to_lowercase();
    device_port_prefixes(device)
        .iter()
        .any(|prefix| port.starts_with(prefix))
        || crate::detect::port_class(port_name).as_deref() == Some(device.class.as_str())
}

/// The first port name that matches the device, by the same rule as
/// `connected_classes`.
pub fn port_matches<'a>(ports: &'a [PortInfo], device: &DeviceInfo) -> Option<&'a str> {
    port_matches_direction(ports, device, None)
}

/// The first port name that matches the device and, when given, the direction.
pub fn port_matches_direction<'a>(
    ports: &'a [PortInfo],
    device: &DeviceInfo,
    direction: Option<Direction>,
) -> Option<&'a str> {
    let matches: Vec<&PortInfo> = ports
        .iter()
        .filter(|port| direction.is_none_or(|wanted| port.direction == wanted))
        .filter(|port| port_name_matches_device(device, &port.name))
        .collect();
    if direction == Some(Direction::Input) {
        if let Some(port) = matches
            .iter()
            .find(|port| port.name.to_lowercase().contains("keyboard"))
        {
            return Some(port.name.as_str());
        }
    }
    matches.first().map(|port| port.name.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use korg_core::ManualClock;
    use korg_midi::{Direction, MockTransport, PortId};

    fn new_controller() -> EditorController<MockTransport, ManualClock> {
        EditorController::new(MockTransport::new(Vec::new()), ManualClock::new())
    }

    fn catalog() -> Catalog {
        Catalog::load().expect("catalog")
    }

    #[test]
    fn opens_nano_kontrol_studio_offline_with_five_scenes() {
        let catalog = catalog();
        let device = catalog.find_by_class("X14802").expect("X14802");
        let mut controller = new_controller();
        let missing = Path::new("/nonexistent-products");
        open_device(&mut controller, device, missing).unwrap();
        let view = controller.view();
        assert_eq!(view.scene_count, 5);
        assert!(view.jog.is_some());
    }

    #[test]
    fn sysex_dump_starts_with_the_measured_frame() {
        let catalog = catalog();
        let device = catalog.find_by_class("X14802").unwrap();
        let mut controller = new_controller();
        open_device(&mut controller, device, Path::new("/nonexistent")).unwrap();
        let dump = sysex_dump(&controller, Some("X14802")).unwrap();
        assert_eq!(&dump[..6], &[0xF0, 0x42, 0x40, 0x00, 0x01, 0x37]);
        assert!(dump.contains(&0xF7));
    }

    #[test]
    fn save_and_load_round_trip() {
        let catalog = catalog();
        let device = catalog.find_by_class("X14802").unwrap();
        let mut controller = new_controller();
        open_device(&mut controller, device, Path::new("/nonexistent")).unwrap();
        let dir = std::env::temp_dir().join(format!("korg-app-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("set.nktrl_st_set");
        save_path(&mut controller, &catalog, Some("X14802"), &path).unwrap();
        assert!(!controller.view().dirty);

        let mut other = new_controller();
        let (class, _) = load_path(&mut other, &catalog, &path).unwrap();
        assert_eq!(class.as_deref(), Some("X14802"));
        let data = |c: &EditorController<MockTransport, ManualClock>| {
            c.session().document().scene_set.as_ref().map(|set| {
                set.scenes
                    .iter()
                    .map(|scene| scene.data.clone())
                    .collect::<Vec<_>>()
            })
        };
        assert_eq!(data(&other), data(&controller));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_scene_set_saves_as_a_set_file() {
        let catalog = catalog();
        let mut controller = new_controller();
        let studio = catalog.find_by_class("X14802").unwrap();
        open_device(&mut controller, studio, Path::new("/nonexistent")).unwrap();
        let (_, extension) = export_chunk(&controller, &catalog, Some("X14802")).unwrap();
        assert_eq!(extension, "nktrl_st_set");
        let nano_key2 = catalog.find_by_class("X10820").unwrap();
        open_device(&mut controller, nano_key2, Path::new("/nonexistent")).unwrap();
        let (_, extension) = export_chunk(&controller, &catalog, Some("X10820")).unwrap();
        assert_eq!(extension, "nanokey2_data");
    }

    #[test]
    fn matches_ports_by_device_name() {
        let catalog = catalog();
        let ports = vec![PortInfo {
            id: PortId::new("1"),
            name: "nanoKONTROL Studio".into(),
            direction: Direction::Input,
        }];
        let connected = connected_classes(&ports, &catalog);
        assert!(connected.iter().any(|class| class == "X14802"));
    }

    #[test]
    fn port_matching_respects_direction() {
        let catalog = catalog();
        let device = catalog.find_by_class("X14800").unwrap();
        let ports = vec![
            PortInfo {
                id: PortId::new("out"),
                name: "nanoKEY Studio CTRL".into(),
                direction: Direction::Output,
            },
            PortInfo {
                id: PortId::new("ctrl-in"),
                name: "nanoKEY Studio CTRL".into(),
                direction: Direction::Input,
            },
            PortInfo {
                id: PortId::new("in"),
                name: "nanoKEY Studio KEYBOARD/CTRL".into(),
                direction: Direction::Input,
            },
        ];
        assert_eq!(
            port_matches_direction(&ports, device, Some(Direction::Input)),
            Some("nanoKEY Studio KEYBOARD/CTRL")
        );
        assert_eq!(
            port_matches_direction(&ports, device, Some(Direction::Output)),
            Some("nanoKEY Studio CTRL")
        );
    }

    #[test]
    fn connected_classes_uses_the_full_device_name() {
        let catalog = catalog();
        let ports = vec![
            PortInfo {
                id: PortId::new("in"),
                name: "nanoKEY Studio KEYBOARD/CTRL".into(),
                direction: Direction::Input,
            },
            PortInfo {
                id: PortId::new("out"),
                name: "nanoKEY Studio CTRL".into(),
                direction: Direction::Output,
            },
        ];
        assert_eq!(connected_classes(&ports, &catalog), ["X14800"]);
    }

    fn temp_dir(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("korg-app-{}-{tag}", std::process::id()))
    }

    #[test]
    fn a_scene_set_over_the_limit_is_rejected() {
        let catalog = catalog();
        let mut controller = new_controller();
        let key = ChunkKey::new(FileModelKey::new("1482").unwrap(), ChunkKind::SceneSet);
        let chunk = Chunk {
            key,
            payload: Payload::SceneSet(vec![vec![0u8; 1]; MAX_SCENES + 1]),
        };
        let bytes = korg_format::encode(&chunk).unwrap();
        let error = load_bytes(
            &mut controller,
            &catalog,
            &bytes,
            Some("nktrl_st_set"),
            "big-set",
        )
        .unwrap_err();
        assert!(error.contains("256"), "{error}");
        assert!(error.contains("255"), "{error}");
        assert!(error.contains("big-set"), "{error}");
    }

    #[test]
    fn load_path_rejects_a_directory() {
        let dir = temp_dir("dir");
        std::fs::create_dir_all(&dir).unwrap();
        let mut controller = new_controller();
        let error = load_path(&mut controller, &catalog(), &dir).unwrap_err();
        assert!(error.contains("not a regular file"), "{error}");
        assert!(error.contains(&dir.display().to_string()), "{error}");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn load_path_rejects_an_oversize_file() {
        let dir = temp_dir("big");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("big.nktrl_st_set");
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(MAX_FILE_BYTES + 1).unwrap();
        drop(file);
        let mut controller = new_controller();
        let error = load_path(&mut controller, &catalog(), &path).unwrap_err();
        assert!(error.contains("at most"), "{error}");
        assert!(error.contains(&path.display().to_string()), "{error}");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn factory_file_uses_the_class_extensions() {
        let catalog = catalog();
        let mut device = catalog.find_by_class("X12880").unwrap().clone();
        device.files = vec![
            "taktile/taktile-25 Scene Set.taktile_set".into(),
            "TRITON taktile/TRITON taktile-25 Scene SET.tr_taktile_set".into(),
        ];
        assert_eq!(
            factory_file(&device),
            Some("TRITON taktile/TRITON taktile-25 Scene SET.tr_taktile_set")
        );
        let mut device = catalog.find_by_class("X11850").unwrap().clone();
        device.files = vec![
            "TRITON taktile/TRITON taktile-25 Scene SET.tr_taktile_set".into(),
            "taktile/taktile-25 Scene Set.taktile_set".into(),
        ];
        assert_eq!(
            factory_file(&device),
            Some("taktile/taktile-25 Scene Set.taktile_set")
        );
    }

    #[test]
    fn opening_without_factory_data_reports_it() {
        let catalog = catalog();
        let device = catalog.find_by_class("X14802").unwrap();
        let mut controller = new_controller();
        let status =
            open_device(&mut controller, device, Path::new("/nonexistent-products")).unwrap();
        assert!(status.starts_with("Opened nanoKONTROL Studio"), "{status}");
        assert!(status.contains("factory scenes unreadable"), "{status}");
        assert_eq!(controller.view().scene_count, 5);
    }
}
