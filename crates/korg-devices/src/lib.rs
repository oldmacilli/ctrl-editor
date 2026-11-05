//! Device catalog, generated from `analysis/products/catalog.json`.

use korg_model::{DeviceInfo, FileModelKey, JogConfig, JogError, ParameterSchema, SysexDescriptor};
use serde::Deserialize;
use std::collections::BTreeMap;

pub mod external;

mod runtime_data;

pub fn parameter_schemas() -> Result<Vec<ParameterSchema>, serde_json::Error> {
    Ok(runtime_data::load()?.schemas.clone())
}

#[derive(Debug, Clone, serde::Serialize, Deserialize)]
pub struct SysexCatalog {
    #[serde(default)]
    pub devices: BTreeMap<String, SysexDescriptor>,
}

pub fn sysex_catalog() -> Result<SysexCatalog, serde_json::Error> {
    Ok(runtime_data::load()?.sysex.clone())
}

pub fn sysex_descriptor(class: &str) -> Option<SysexDescriptor> {
    sysex_catalog().ok()?.devices.remove(class)
}

pub fn parameter_schema(model: &str, payload: &str) -> Option<ParameterSchema> {
    parameter_schemas()
        .ok()?
        .into_iter()
        .find(|schema| schema.model == model && schema.payload == payload)
}

#[derive(Debug, Clone, serde::Serialize, Deserialize)]
pub struct Catalog {
    #[serde(default)]
    pub devices: Vec<DeviceInfo>,
    #[serde(default)]
    pub chunks: Vec<serde_json::Value>,
}

impl Catalog {
    pub fn load() -> Result<Self, serde_json::Error> {
        Ok(runtime_data::load()?.catalog.clone())
    }

    pub fn find_by_class(&self, class: &str) -> Option<&DeviceInfo> {
        self.devices.iter().find(|device| device.class == class)
    }

    pub fn find_by_file_key(&self, model: &FileModelKey) -> Vec<&DeviceInfo> {
        self.devices
            .iter()
            .filter(|device| device.file_key().as_ref() == Some(model))
            .collect()
    }

    pub fn find_by_extension(&self, extension: &str) -> Option<&DeviceInfo> {
        self.devices.iter().find(|device| {
            device
                .extensions
                .iter()
                .any(|candidate| candidate == extension)
        })
    }
}

/// Measured receive and write sizes for one model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeasuredLayout {
    pub key: &'static str,
    pub model: u32,
    pub scene_count: u8,
    pub scene_len: usize,
    pub global_len: usize,
    /// Four-byte data header for a scene write.  `None` means not measured.
    pub write_header: Option<[u8; 4]>,
    /// Four-byte data header for a global write.  `None` means not measured.
    pub global_write_header: Option<[u8; 4]>,
}

struct DescriptorLayout {
    key: String,
    model: u32,
    scene_count: u8,
    scene_len: usize,
    global_len: usize,
    write_header: Option<[u8; 4]>,
    global_write_header: Option<[u8; 4]>,
}

fn layout_from_descriptor(d: &ctrl_cdl::Descriptor) -> Option<DescriptorLayout> {
    use ctrl_cdl::Status;
    let model_const = d.constant("model")?;
    let scene = d.block("scene")?;
    let global = d.block("global")?;
    if [
        model_const.meta.status,
        scene.meta.status,
        global.meta.status,
    ]
    .iter()
    .any(|status| *status != Some(Status::Measured))
    {
        return None;
    }
    let model = u32::try_from(model_const.value)
        .ok()
        .filter(|m| *m <= 0x3fff)?;
    let scene_count = u8::try_from(scene.count?).ok().filter(|n| *n > 0)?;
    let scene_len = usize::try_from(scene.size?)
        .ok()
        .filter(|n| (1..=u16::MAX as usize).contains(n))?;
    let global_len = usize::try_from(global.size?)
        .ok()
        .filter(|n| (1..=u16::MAX as usize).contains(n))?;
    let key = d
        .device
        .file_key
        .as_deref()?
        .parse::<FileModelKey>()
        .ok()?
        .to_string();
    let header = |message: &str, size: usize| -> Option<[u8; 4]> {
        let msg = d.message(message)?;
        if msg.meta.status != Some(Status::Measured) {
            return None;
        }
        let codec = ctrl_codec::Codec::new(d);
        let mut args = ctrl_codec::Bindings::new();
        args.insert("device-id".into(), ctrl_codec::Val::Int(0));
        args.insert("data".into(), ctrl_codec::Val::Bytes(vec![0; size]));
        let frame = codec.encode(message, &args).ok()?;
        if frame.get(..6)?
            != [
                0xf0,
                0x42,
                0x40,
                0,
                (model >> 8) as u8,
                (model & 0xff) as u8,
            ]
            || frame.last() != Some(&0xf7)
        {
            return None;
        }
        frame.get(6..10)?.try_into().ok()
    };
    Some(DescriptorLayout {
        key,
        model,
        scene_count,
        scene_len,
        global_len,
        write_header: header("scene-write", scene_len),
        global_write_header: header("global-write", global_len),
    })
}

fn descriptors() -> Option<&'static BTreeMap<String, ctrl_cdl::Descriptor>> {
    static DESCRIPTORS: std::sync::OnceLock<Option<BTreeMap<String, ctrl_cdl::Descriptor>>> =
        std::sync::OnceLock::new();
    DESCRIPTORS
        .get_or_init(|| {
            external::descriptor_root()
                .ok()
                .and_then(|root| external::load_korg_descriptors(&root).ok())
        })
        .as_ref()
}

/// Restrict the legacy calibration operation to its declared device and
/// measured identity. The procedure's unknown status is not promoted here.
pub fn legacy_calibration_matches_model(model: u32) -> bool {
    let Some(descriptor) = descriptors().and_then(|all| all.get("X14800")) else {
        return false;
    };
    descriptor.procedure("calibrate-touch-pad").is_some()
        && descriptor.constant("model").is_some_and(|id| {
            id.meta.status == Some(ctrl_cdl::Status::Measured)
                && u32::try_from(id.value).ok() == Some(model)
        })
}

/// Known global field positions for the file model, from external CDL.
pub fn global_fields(class: &str) -> Option<&'static ctrl_cdl::GlobalFields> {
    let fields = descriptors()?.get(class)?.global_fields.as_ref()?;
    matches!(
        fields.meta.status,
        Some(ctrl_cdl::Status::Decoded | ctrl_cdl::Status::Measured)
    )
    .then_some(fields)
}

/// UI roles are stable; their byte offsets come solely from the descriptor.
pub const GLOBAL_UI_ROLES: [&str; 21] = [
    "channel",
    "battery",
    "clock",
    "key-curve",
    "key-constant",
    "pad-curve",
    "pad-constant",
    "pad-sensitivity",
    "touch-velocity",
    "touch-curve",
    "usb-led-off",
    "usb-brightness",
    "usb-illumination",
    "usb-eco",
    "battery-power",
    "battery-led-off",
    "battery-brightness",
    "battery-illumination",
    "battery-eco",
    "controller-mode",
    "scrub",
];

/// Displayable choice labels and their explicitly described stored bytes.
pub fn global_ui_choices(
    fields: Option<&ctrl_cdl::GlobalFields>,
) -> Vec<Vec<&ctrl_cdl::GlobalChoice>> {
    let mut result = vec![Vec::new(); GLOBAL_UI_ROLES.len()];
    let Some(fields) = fields.filter(|f| {
        matches!(
            f.meta.status,
            Some(ctrl_cdl::Status::Decoded | ctrl_cdl::Status::Measured)
        )
    }) else {
        return result;
    };
    for (index, role) in GLOBAL_UI_ROLES.iter().enumerate() {
        if let Some(binding) = fields.bindings.iter().find(|b| {
            b.name == *role
                && matches!(
                    b.meta.status,
                    Some(ctrl_cdl::Status::Decoded | ctrl_cdl::Status::Measured)
                )
        }) {
            result[index] = binding
                .choices
                .iter()
                .filter(|c| {
                    matches!(
                        c.meta.status,
                        Some(
                            ctrl_cdl::Status::Decoded
                                | ctrl_cdl::Status::Measured
                                | ctrl_cdl::Status::Inferred
                        )
                    )
                })
                .collect();
        }
    }
    result
}

/// Translate a displayed choice back to its explicitly described byte.
pub fn global_ui_choice_selection(
    fields: Option<&ctrl_cdl::GlobalFields>,
    role: usize,
    index: usize,
) -> Option<(usize, u8)> {
    let offset = usize::try_from(*global_ui_offsets(fields).get(role)?).ok()?;
    let choices = global_ui_choices(fields);
    Some((offset, choices.get(role)?.get(index)?.value))
}

/// Missing or unverified roles use -1, so frontends cannot write a guessed byte.
pub fn global_ui_offsets(fields: Option<&ctrl_cdl::GlobalFields>) -> [i32; 22] {
    let mut offsets = [-1; 22];
    let Some(fields) = fields.filter(|f| {
        matches!(
            f.meta.status,
            Some(ctrl_cdl::Status::Decoded | ctrl_cdl::Status::Measured)
        )
    }) else {
        return offsets;
    };
    for (index, role) in GLOBAL_UI_ROLES.iter().enumerate() {
        if let Some(binding) = fields.bindings.iter().find(|b| {
            b.name == *role
                && matches!(
                    b.meta.status,
                    Some(ctrl_cdl::Status::Decoded | ctrl_cdl::Status::Measured)
                )
        }) {
            offsets[index] = i32::try_from(binding.offset).unwrap_or(-1);
        }
    }
    offsets[21] = fields
        .name_offset
        .and_then(|v| i32::try_from(v).ok())
        .unwrap_or(-1);
    offsets
}

pub fn global_fields_for_file_key(key: &FileModelKey) -> Option<&'static ctrl_cdl::GlobalFields> {
    let mut matches = descriptors()?.values().filter_map(|descriptor| {
        let file_key = descriptor
            .device
            .file_key
            .as_deref()?
            .parse::<FileModelKey>()
            .ok()?;
        let fields = descriptor.global_fields.as_ref()?;
        (file_key == *key
            && matches!(
                fields.meta.status,
                Some(ctrl_cdl::Status::Decoded | ctrl_cdl::Status::Measured)
            ))
        .then_some(fields)
    });
    let fields = matches.next()?;
    matches.next().is_none().then_some(fields)
}

/// Declarative editor pages from the externally loaded device descriptor.
pub use ctrl_cdl::Status as DescriptorStatus;

pub fn control_assignments(class: &str) -> Option<&'static ctrl_cdl::ControlAssignments> {
    let assignments = descriptors()?.get(class)?.control_assignments.as_ref()?;
    matches!(
        assignments.meta.status,
        Some(ctrl_cdl::Status::Decoded | ctrl_cdl::Status::Measured)
    )
    .then_some(assignments)
}

pub fn editor_pages(class: &str) -> Option<&'static ctrl_cdl::EditorPages> {
    let pages = descriptors()?.get(class)?.editor_pages.as_ref()?;
    matches!(
        pages.meta.status,
        Some(ctrl_cdl::Status::Decoded | ctrl_cdl::Status::Measured)
    )
    .then_some(pages)
}

/// Optional picture name within the externally installed device-asset root.
pub fn external_image(class: &str, role: &str) -> Option<&'static str> {
    image_from_descriptor(descriptors()?.get(class)?, role)
}

fn image_from_descriptor<'a>(descriptor: &'a ctrl_cdl::Descriptor, role: &str) -> Option<&'a str> {
    if !matches!(role, "external-thumbnail" | "external-panel") {
        return None;
    }
    descriptor
        .device
        .images
        .iter()
        .find(|image| image.role.as_deref() == Some(role))
        .map(|image| image.path.as_str())
}

fn measured_layouts() -> &'static [DescriptorLayout] {
    static LAYOUTS: std::sync::OnceLock<Vec<DescriptorLayout>> = std::sync::OnceLock::new();
    LAYOUTS.get_or_init(|| {
        descriptors()
            .map(|all| all.values().filter_map(layout_from_descriptor).collect())
            .unwrap_or_default()
    })
}

fn public_layout(layout: &'static DescriptorLayout) -> MeasuredLayout {
    MeasuredLayout {
        key: &layout.key,
        model: layout.model,
        scene_count: layout.scene_count,
        scene_len: layout.scene_len,
        global_len: layout.global_len,
        write_header: layout.write_header,
        global_write_header: layout.global_write_header,
    }
}

pub fn measured_layout(model: u32) -> Option<MeasuredLayout> {
    let mut matches = measured_layouts()
        .iter()
        .filter(|layout| layout.model == model);
    let layout = matches.next()?;
    matches.next().is_none().then(|| public_layout(layout))
}

pub fn measured_layout_for_key(key: &str) -> Option<MeasuredLayout> {
    let mut matches = measured_layouts().iter().filter(|layout| layout.key == key);
    let layout = matches.next()?;
    matches.next().is_none().then(|| public_layout(layout))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JogLayout {
    pub offset: usize,
}

fn jog_from_descriptor(descriptor: &ctrl_cdl::Descriptor) -> Option<JogLayout> {
    let scene = descriptor.block("scene")?;
    if scene.meta.status != Some(ctrl_cdl::Status::Measured) {
        return None;
    }
    let mut groups = scene.items.iter().filter_map(|item| match item {
        ctrl_cdl::Item::Group(group) if group.name == "jog" => Some(group),
        _ => None,
    });
    let group = groups.next()?;
    if groups.next().is_some()
        || matches!(
            group.meta.status,
            Some(ctrl_cdl::Status::Unknown | ctrl_cdl::Status::Inferred)
        )
    {
        return None;
    }
    let offset = usize::try_from(group.offset?).ok()?;
    let size = usize::try_from(scene.size?).ok()?;
    if offset.checked_add(JogConfig::LEN)? > size {
        return None;
    }
    Some(JogLayout { offset })
}

pub fn jog_class_for_file_key(key: &str) -> Option<&'static str> {
    let mut matches = descriptors()?.iter().filter(|(_, d)| {
        d.device.file_key.as_deref() == Some(key) && jog_from_descriptor(d).is_some()
    });
    let (class, _) = matches.next()?;
    matches.next().is_none().then_some(class.as_str())
}

pub fn jog_layout(class: &str) -> Option<JogLayout> {
    jog_from_descriptor(descriptors()?.get(class)?)
}

/// Decode the jog block from a scene payload of the given class.
pub fn scene_jog_config(class: &str, scene: &[u8]) -> Option<Result<JogConfig, JogError>> {
    let layout = jog_layout(class)?;
    let end = layout.offset + JogConfig::LEN;
    if scene.len() < end {
        return Some(Err(JogError::Short(
            scene.len().saturating_sub(layout.offset),
        )));
    }
    Some(JogConfig::decode(&scene[layout.offset..end]))
}

#[cfg(test)]
mod tests {
    #[test]
    fn global_ui_bindings_follow_external_offsets_and_reject_unknown() {
        let descriptor = ctrl_cdl::parse_str("test", "cdl \"0.1\"\ndevice \"test\"\nglobal-fields status=\"decoded\" { binding \"channel\" offset=9 status=\"decoded\"; binding \"battery\" offset=7 status=\"unknown\" }").unwrap();
        let offsets = super::global_ui_offsets(descriptor.global_fields.as_ref());
        assert_eq!(offsets[0], 9);
        assert_eq!(offsets[1], -1);
        assert_eq!(offsets[21], -1);
        assert!(super::global_ui_offsets(None)
            .iter()
            .all(|offset| *offset == -1));
    }

    #[test]
    fn global_ui_choices_keep_explicit_values_and_skip_unknown() {
        let descriptor = ctrl_cdl::parse_str("test", "cdl \"0.1\"\ndevice \"test\"\nglobal-fields status=\"decoded\" { binding \"battery\" offset=7 status=\"decoded\" { choice \"B\" value=4 status=\"inferred\"; choice \"A\" value=1 status=\"decoded\"; choice \"Unknown\" value=8 status=\"unknown\" } }").unwrap();
        let choices = super::global_ui_choices(descriptor.global_fields.as_ref());
        assert_eq!(
            choices[1]
                .iter()
                .map(|c| (c.label.as_str(), c.value))
                .collect::<Vec<_>>(),
            [("B", 4), ("A", 1)]
        );
        assert_eq!(
            super::global_ui_choice_selection(descriptor.global_fields.as_ref(), 1, 0),
            Some((7, 4))
        );
        assert_eq!(
            super::global_ui_choice_selection(descriptor.global_fields.as_ref(), 1, 1),
            Some((7, 1))
        );
        assert_eq!(
            super::global_ui_choice_selection(descriptor.global_fields.as_ref(), 1, 2),
            None
        );
        assert!(super::global_ui_choices(None).iter().all(Vec::is_empty));
    }

    #[test]
    fn studio_global_choices_preserve_existing_options_and_byte_order() {
        for (class, role, labels) in [
            ("X14800", "clock", &["Auto", "Internal", "External-USB"][..]),
            (
                "X14800",
                "key-curve",
                &[
                    "Light",
                    "Mid-Light",
                    "Normal",
                    "Mid-Heavy",
                    "Heavy",
                    "Const",
                ][..],
            ),
            (
                "X14800",
                "touch-curve",
                &["Immediate", "Fast", "Normal"][..],
            ),
            (
                "X14800",
                "battery-power",
                &["Disable", "30 min", "1 hour", "2 hours", "4 hours"][..],
            ),
            (
                "X14802",
                "controller-mode",
                &[
                    "Assignable",
                    "Cubase",
                    "Digital Performer",
                    "Live",
                    "Pro Tools",
                    "SONAR",
                    "Studio One",
                ][..],
            ),
            ("X14802", "scrub", &["Off", "On"][..]),
        ] {
            let index = super::GLOBAL_UI_ROLES
                .iter()
                .position(|candidate| *candidate == role)
                .unwrap();
            let choices = super::global_ui_choices(super::global_fields(class));
            assert_eq!(
                choices[index]
                    .iter()
                    .map(|choice| choice.label.as_str())
                    .collect::<Vec<_>>(),
                labels
            );
            assert!(choices[index]
                .iter()
                .enumerate()
                .all(|(i, choice)| usize::from(choice.value) == i));
        }
    }

    #[test]
    fn studio_global_ui_bindings_preserve_previous_byte_positions() {
        for (class, expected) in [
            (
                "X14800",
                [
                    0, 1, 56, 48, 49, 50, 51, 52, 53, 54, 73, 74, 75, 76, 80, 81, 82, 83, 84, -1,
                    -1, 8,
                ],
            ),
            (
                "X14802",
                [
                    0, 2, -1, -1, -1, -1, -1, -1, -1, -1, 65, 66, 67, -1, 72, 73, 74, 75, -1, 1, 3,
                    8,
                ],
            ),
        ] {
            assert_eq!(
                super::global_ui_offsets(super::global_fields(class)),
                expected,
                "{class}"
            );
        }
    }
    use super::*;

    #[test]
    fn external_images_are_selected_by_cdl_role() {
        assert_eq!(
            external_image("X10830", "external-thumbnail"),
            Some("X10830.png")
        );
        assert_eq!(
            external_image("X10830", "external-panel"),
            Some("X10830-panel.png")
        );
        assert_eq!(external_image("X10830", "thumbnail"), None);
        assert_eq!(external_image("X99999", "external-panel"), None);
    }

    #[test]
    fn external_image_name_comes_from_descriptor() {
        let dir = std::env::temp_dir().join(format!("korg-image-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("custom.kdl");
        std::fs::write(&file, "cdl \"0.1\"\ndevice \"test.custom\" {\n class \"X10830\"\n image \"alternate.png\" role=\"external-thumbnail\"\n}\n").unwrap();
        let descriptor = ctrl_cdl::load(&file).unwrap();
        assert_eq!(
            image_from_descriptor(&descriptor, "external-thumbnail"),
            Some("alternate.png")
        );
        assert_eq!(image_from_descriptor(&descriptor, "external-panel"), None);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn catalog_loads() {
        let catalog = Catalog::load().unwrap();
        assert_eq!(catalog.devices.len(), 16);
    }

    #[test]
    fn finds_nano_pad() {
        let catalog = Catalog::load().unwrap();
        let device = catalog.find_by_class("X10830").unwrap();
        assert_eq!(device.device, "nanoPAD2");
        assert_eq!(device.scenes, Some(4));
        assert_eq!(device.scene_payload_size, Some(97));
        assert_eq!(device.global_payload_size, Some(47));
    }

    #[test]
    fn catalog_sizes_match_schema_payload_sizes() {
        let catalog = Catalog::load().unwrap();
        for device in &catalog.devices {
            if let Some(size) = device.scene_payload_size {
                let schema = parameter_schema(&device.class, "scene")
                    .unwrap_or_else(|| panic!("{} has no scene schema", device.class));
                assert_eq!(size, schema.payload_size, "{} scene", device.class);
            }
            if let Some(size) = device.global_payload_size {
                let schema = parameter_schema(&device.class, "global")
                    .unwrap_or_else(|| panic!("{} has no global schema", device.class));
                assert_eq!(size, schema.payload_size, "{} global", device.class);
            }
        }
    }

    #[test]
    fn parameter_schemas_validate() {
        let schemas = parameter_schemas().unwrap();
        assert_eq!(schemas.len(), 20);
        let mut pairs = std::collections::BTreeSet::new();
        for schema in &schemas {
            schema.validate().unwrap();
            assert!(
                pairs.insert((schema.model.clone(), schema.payload.clone())),
                "duplicate schema for {} {}",
                schema.model,
                schema.payload
            );
        }
    }

    #[test]
    fn decodes_nano_key2_values() {
        let schema = parameter_schema("X10820", "scene").unwrap();
        let mut payload = vec![0u8; schema.payload_size as usize];
        payload[9] = 1;
        payload[18] = 1;
        payload[21] = 2;
        let curve = schema
            .parameters
            .iter()
            .find(|parameter| parameter.key == "VelocityCurve")
            .unwrap();
        assert_eq!(curve.decode(&payload), Some(1));
        let behavior = schema
            .parameters
            .iter()
            .find(|parameter| parameter.key == "ButtonBehavior")
            .unwrap();
        assert_eq!(behavior.decode(&payload), Some(1));
    }

    #[test]
    fn decodes_nano_pad_bitfields() {
        let schema = parameter_schema("X10830", "scene").unwrap();
        let mut payload = vec![0u8; schema.payload_size as usize];
        payload[..6].copy_from_slice(&[0x48, 0x25, 0x80, 0x80, 0x80, 0x10]);
        let assign = schema
            .parameters
            .iter()
            .find(|parameter| parameter.key == "AssignType")
            .unwrap();
        let midi = schema
            .parameters
            .iter()
            .find(|parameter| parameter.key == "MidiCh")
            .unwrap();
        assert_eq!(assign.decode(&payload), Some(2));
        assert_eq!(midi.decode(&payload), Some(16));
    }

    #[test]
    fn factory_payloads_match_schemas() {
        let Ok(root) = std::env::var("KORG_PRODUCTS_DIR") else {
            eprintln!("skip: set KORG_PRODUCTS_DIR to validate factory payloads");
            return;
        };
        let root = std::path::Path::new(&root);

        let key2 = std::fs::read(root.join("nanoKEY2/nanoKEY2 Scene Data.nanokey2_data")).unwrap();
        let key2 = korg_format::decode(&key2).unwrap();
        let korg_format::Payload::SceneSet(key2_scenes) = key2.payload else {
            panic!("expected a scene set");
        };
        parameter_schema("X10820", "scene")
            .unwrap()
            .validate_factory(&key2_scenes[0])
            .unwrap();

        let pad2 = std::fs::read(root.join("nanoPAD2/nanoPAD2 Scene Set.nanopad2_set")).unwrap();
        let pad2 = korg_format::decode(&pad2).unwrap();
        let korg_format::Payload::SceneSet(pad2_scenes) = pad2.payload else {
            panic!("expected a scene set");
        };
        parameter_schema("X10830", "scene")
            .unwrap()
            .validate_factory(&pad2_scenes[0])
            .unwrap();
    }

    #[test]
    fn sysex_descriptors_load() {
        let catalog = sysex_catalog().unwrap();
        assert_eq!(catalog.devices.len(), 9);
        let pad2 = catalog.devices.get("X10830").unwrap();
        assert_eq!(pad2.model_id.map(|id| id.0), Some(0x112));
        assert_eq!(pad2.commands.get("scene_data").copied().flatten(), Some(0));
        assert_eq!(pad2.commands.get("mode_check").copied().flatten(), None);
        let fold = catalog.devices.get("X20880").unwrap();
        assert_eq!(fold.model_id.map(|id| id.0), Some(0x16A));
        assert_eq!(fold.commands.get("global_data").copied().flatten(), Some(1));
        let keystage = catalog.devices.get("X20870").unwrap();
        assert_eq!(keystage.model_id.map(|id| id.0), Some(0x169));
        assert_eq!(keystage.length_style.as_deref(), Some("fixed3"));
        let key2 = catalog.devices.get("X10820").unwrap();
        assert_eq!(key2.model_id.map(|id| id.0), Some(0x111));
        assert!(!pad2.complete());
        assert!(sysex_descriptor("X99999").is_none());
    }

    #[test]
    fn finds_by_extension() {
        let catalog = Catalog::load().unwrap();
        let device = catalog.find_by_extension("tr_taktile_set").unwrap();
        assert_eq!(device.class, "X12880");
    }

    #[test]
    fn measured_layout_has_one_source() {
        let key = measured_layout(0x136).unwrap();
        assert_eq!(key.key, "1480");
        assert_eq!(key.scene_count, 8);
        assert_eq!(key.scene_len, 240);
        assert_eq!(key.write_header, Some([0x14, 0x02, 0x00, 0x40]));
        let control = measured_layout_for_key("1482").unwrap();
        assert_eq!(control.model, 0x137);
        assert_eq!(control.write_header, Some([0x01, 0x05, 0x00, 0x40]));
        assert!(measured_layout(0x112).is_none());
        assert!(measured_layout_for_key("0820").is_none());
    }

    #[test]
    fn measured_layout_comes_from_the_external_descriptor() {
        let root = external::descriptor_root().unwrap();
        let mut descriptors = external::load_korg_descriptors(&root).unwrap();
        let mut descriptor = descriptors.remove("X14802").unwrap();
        let original = layout_from_descriptor(&descriptor).unwrap();
        assert_eq!(original.scene_count, 5);
        assert_eq!(original.scene_len, 560);
        assert_eq!(original.write_header, Some([0x01, 0x05, 0x00, 0x40]));
        descriptor
            .messages
            .iter_mut()
            .find(|m| m.name == "scene-write")
            .unwrap()
            .meta
            .status = Some(ctrl_cdl::Status::Unknown);
        assert_eq!(
            layout_from_descriptor(&descriptor).unwrap().write_header,
            None
        );
        descriptor
            .messages
            .iter_mut()
            .find(|m| m.name == "scene-write")
            .unwrap()
            .meta
            .status = Some(ctrl_cdl::Status::Measured);
        let scene = descriptor
            .blocks
            .iter_mut()
            .find(|b| b.name == "scene")
            .unwrap();
        scene.count = Some(3);
        assert_eq!(layout_from_descriptor(&descriptor).unwrap().scene_count, 3);
        descriptor
            .blocks
            .iter_mut()
            .find(|b| b.name == "scene")
            .unwrap()
            .meta
            .status = Some(ctrl_cdl::Status::Unknown);
        assert!(layout_from_descriptor(&descriptor).is_none());
    }

    #[test]
    fn jog_layout_is_only_for_the_nano_control_studio() {
        assert_eq!(jog_layout("X14802"), Some(JogLayout { offset: 512 }));
        assert_eq!(jog_layout("X14800"), None);
        assert_eq!(jog_layout("X10830"), None);
    }

    #[test]
    fn jog_layout_comes_from_the_external_descriptor() {
        let root = external::descriptor_root().unwrap();
        let mut descriptors = external::load_korg_descriptors(&root).unwrap();
        let mut descriptor = descriptors.remove("X14802").unwrap();
        assert_eq!(jog_from_descriptor(&descriptor).unwrap().offset, 512);
        let scene = descriptor
            .blocks
            .iter_mut()
            .find(|b| b.name == "scene")
            .unwrap();
        let jog = scene
            .items
            .iter_mut()
            .find_map(|item| match item {
                ctrl_cdl::Item::Group(group) if group.name == "jog" => Some(group),
                _ => None,
            })
            .unwrap();
        jog.offset = Some(500);
        assert_eq!(jog_from_descriptor(&descriptor).unwrap().offset, 500);
        let scene = descriptor
            .blocks
            .iter_mut()
            .find(|b| b.name == "scene")
            .unwrap();
        let jog = scene
            .items
            .iter_mut()
            .find_map(|item| match item {
                ctrl_cdl::Item::Group(group) if group.name == "jog" => Some(group),
                _ => None,
            })
            .unwrap();
        jog.meta.status = Some(ctrl_cdl::Status::Inferred);
        assert!(jog_from_descriptor(&descriptor).is_none());
        descriptor
            .blocks
            .iter_mut()
            .find(|b| b.name == "scene")
            .unwrap()
            .meta
            .status = Some(ctrl_cdl::Status::Unknown);
        assert!(jog_from_descriptor(&descriptor).is_none());
    }

    #[test]
    fn decodes_the_jog_block_from_a_scene() {
        let mut scene = vec![0u8; 560];
        scene[512..521].copy_from_slice(&[0x06, 0x05, 0x02, 0x53, 0x54, 0x56, 0x57, 0x00, 0x7F]);
        let config = scene_jog_config("X14802", &scene).unwrap().unwrap();
        assert_eq!(config.channel, 6);
        assert_eq!(config.continuous_cc, 0x57);
        assert!(scene_jog_config("X14800", &scene).is_none());
        assert!(matches!(
            scene_jog_config("X14802", &scene[..100]).unwrap(),
            Err(JogError::Short(_))
        ));
    }
}
