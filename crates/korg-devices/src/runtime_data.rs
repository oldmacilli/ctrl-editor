//! Runtime device data projected from external per-device CDL descriptors.
use crate::{Catalog, SysexCatalog};
use korg_model::{ParameterDef, ParameterSchema};
use serde::Deserialize;
use std::sync::OnceLock;

#[derive(Deserialize)]
struct CatalogInfo {
    meta: CatalogMeta,
    order: u32,
    name: String,
    localized_name: Option<String>,
    scenes: Option<u8>,
    scene_size: Option<u16>,
    global_size: Option<u16>,
    extensions: Vec<CatalogExtension>,
    factory_files: Vec<String>,
}

#[derive(Deserialize)]
struct CatalogMeta {
    status: Option<ctrl_cdl::Status>,
}

#[derive(Deserialize)]
struct CatalogExtension {
    extension: String,
    os_type: Option<String>,
}

#[derive(Deserialize)]
struct SysexInfo {
    meta: CatalogMeta,
    model_id: Option<u32>,
    length_style: Option<String>,
    commands: Vec<SysexCommandInfo>,
}

#[derive(Deserialize)]
struct SysexCommandInfo {
    meta: CatalogMeta,
    name: String,
    byte: Option<u8>,
}

#[derive(Deserialize)]
struct ParameterLayoutsInfo {
    meta: CatalogMeta,
    layouts: Vec<ParameterLayoutInfo>,
}

#[derive(Deserialize)]
struct ParameterLayoutInfo {
    meta: CatalogMeta,
    payload: String,
    order: u32,
    size: u16,
    elements: u16,
    stride: u16,
    version: u8,
    parameters: Vec<serde_json::Value>,
}

fn trusted_inherited(status: Option<ctrl_cdl::Status>) -> bool {
    matches!(
        status,
        None | Some(ctrl_cdl::Status::Measured | ctrl_cdl::Status::Decoded)
    )
}

type Descriptors = std::collections::BTreeMap<String, ctrl_cdl::Descriptor>;

/// The vendor `parameter-names` list maps a parameter id to its name.
fn parameter_names(descriptor: &ctrl_cdl::Descriptor) -> std::collections::BTreeMap<i64, String> {
    descriptor
        .labels
        .iter()
        .flat_map(|labels| labels.lists.iter())
        .find(|list| list.name == "parameter-names")
        .map(|list| {
            list.items
                .iter()
                .map(|item| (item.value, item.label.clone()))
                .collect()
        })
        .unwrap_or_default()
}

fn parameters_from_loaded(
    descriptors: &Descriptors,
) -> Result<Vec<ParameterSchema>, serde_json::Error> {
    let mut ordered = std::collections::BTreeMap::new();
    for (class, descriptor) in descriptors {
        let value = serde_json::to_value(descriptor)?;
        let raw = value
            .get("parameter_layouts")
            .filter(|v| !v.is_null())
            .cloned()
            .ok_or_else(|| failure(format!("{class}: CDL parameter layouts missing")))?;
        let info: ParameterLayoutsInfo = serde_json::from_value(raw)
            .map_err(|e| failure(format!("{class}: invalid parameter layouts: {e}")))?;
        match info.meta.status {
            Some(ctrl_cdl::Status::Unknown) if info.layouts.is_empty() => continue,
            Some(ctrl_cdl::Status::Measured | ctrl_cdl::Status::Decoded) => {}
            _ => {
                return Err(failure(format!(
                    "{class}: parameter layouts are not decoded, measured or explicitly unknown"
                )))
            }
        }
        for layout in info.layouts {
            if !trusted_inherited(layout.meta.status) {
                return Err(failure(format!(
                    "{class}: unverified parameter layout {}",
                    layout.payload
                )));
            }
            let names = parameter_names(descriptor);
            let mut parameters = Vec::new();
            for value in layout.parameters {
                let meta: CatalogMeta = serde_json::from_value(
                    value
                        .get("meta")
                        .cloned()
                        .ok_or_else(|| failure(format!("{class}: parameter evidence missing")))?,
                )?;
                if !trusted_inherited(meta.status) {
                    return Err(failure(format!("{class}: unverified parameter")));
                }
                let mut parameter: ParameterDef = serde_json::from_value(value)?;
                if parameter.display_label.is_none() {
                    parameter.display_label = names.get(&i64::from(parameter.id)).cloned();
                }
                parameters.push(parameter);
            }
            let schema = ParameterSchema {
                schema_version: layout.version,
                model: class.clone(),
                table: String::new(),
                payload: layout.payload,
                payload_size: layout.size,
                elements: layout.elements,
                stride: layout.stride,
                source: Default::default(),
                parameters,
            };
            schema.validate().map_err(failure)?;
            if ordered.insert(layout.order, schema).is_some() {
                return Err(failure(format!(
                    "duplicate parameter layout order {}",
                    layout.order
                )));
            }
        }
    }
    if ordered.keys().copied().ne(0..ordered.len() as u32) {
        return Err(failure(
            "parameter layout order must be contiguous starting at zero",
        ));
    }
    Ok(ordered.into_values().collect())
}

fn sysex_from_loaded(descriptors: &Descriptors) -> Result<SysexCatalog, serde_json::Error> {
    let mut devices = std::collections::BTreeMap::new();
    for (class, descriptor) in descriptors {
        let value = serde_json::to_value(descriptor)?;
        let raw = value
            .get("sysex_commands")
            .filter(|v| !v.is_null())
            .cloned()
            .ok_or_else(|| failure(format!("{class}: CDL SysEx metadata missing")))?;
        let info: SysexInfo = serde_json::from_value(raw)
            .map_err(|e| failure(format!("{class}: invalid CDL SysEx metadata: {e}")))?;
        match info.meta.status {
            Some(ctrl_cdl::Status::Unknown)
                if info.model_id.is_none()
                    && info.length_style.is_none()
                    && info.commands.is_empty() =>
            {
                continue
            }
            Some(ctrl_cdl::Status::Measured | ctrl_cdl::Status::Decoded) => {}
            _ => {
                return Err(failure(format!(
                    "{class}: SysEx metadata is not decoded, measured or explicitly unknown"
                )))
            }
        }
        let mut commands = std::collections::BTreeMap::new();
        for command in info.commands {
            match (command.byte, command.meta.status) {
                (None, Some(ctrl_cdl::Status::Unknown))
                | (Some(0..=0x7f), Some(ctrl_cdl::Status::Measured | ctrl_cdl::Status::Decoded)) => {
                }
                _ => {
                    return Err(failure(format!(
                        "{class}: invalid or unverified SysEx command {}",
                        command.name
                    )))
                }
            }
            if commands
                .insert(command.name.clone(), command.byte)
                .is_some()
            {
                return Err(failure(format!(
                    "{class}: duplicate SysEx command {}",
                    command.name
                )));
            }
        }
        devices.insert(
            class.clone(),
            korg_model::SysexDescriptor {
                model_id: info.model_id.map(korg_model::SysexModelId),
                length_style: info.length_style,
                commands,
                source: None,
            },
        );
    }
    Ok(SysexCatalog { devices })
}

fn catalog_from_loaded(descriptors: &Descriptors) -> Result<Catalog, serde_json::Error> {
    let mut ordered = std::collections::BTreeMap::new();
    for (class, descriptor) in descriptors {
        // Deserialize the shared IR by name while the pinned ctrl-cdl revision
        // is being updated; older versions produce a clear error, never defaults.
        let value = serde_json::to_value(descriptor)?;
        let info: CatalogInfo = serde_json::from_value(
            value
                .get("catalog")
                .cloned()
                .ok_or_else(|| failure(format!("{class}: CDL catalog metadata missing")))?,
        )
        .map_err(|e| failure(format!("{class}: invalid CDL catalog metadata: {e}")))?;
        if !matches!(
            info.meta.status,
            Some(ctrl_cdl::Status::Measured | ctrl_cdl::Status::Decoded)
        ) {
            return Err(failure(format!(
                "{class}: catalog metadata is not decoded or measured"
            )));
        }
        let mut extensions = Vec::new();
        let mut ostypes = std::collections::BTreeMap::new();
        for entry in info.extensions {
            if extensions.contains(&entry.extension) {
                return Err(failure(format!(
                    "{class}: duplicate extension {}",
                    entry.extension
                )));
            }
            if let Some(os_type) = entry.os_type {
                ostypes.insert(entry.extension.clone(), os_type);
            }
            extensions.push(entry.extension);
        }
        let device = korg_model::DeviceInfo {
            device: info.name,
            class: class.clone(),
            localized_name: info.localized_name,
            file_model_key: descriptor.device.file_key.clone(),
            extensions,
            ostypes,
            scenes: info.scenes,
            scene_payload_size: info.scene_size,
            global_payload_size: info.global_size,
            files: info.factory_files,
        };
        if ordered.insert(info.order, device).is_some() {
            return Err(failure(format!("duplicate catalog order {}", info.order)));
        }
    }
    if ordered.keys().copied().ne(0..ordered.len() as u32) {
        return Err(failure("catalog order must be contiguous starting at zero"));
    }
    Ok(Catalog {
        devices: ordered.into_values().collect(),
        chunks: Vec::new(),
    })
}

pub(crate) struct RuntimeData {
    pub catalog: Catalog,
    pub schemas: Vec<ParameterSchema>,
    pub sysex: SysexCatalog,
}

fn failure(message: impl ToString) -> serde_json::Error {
    serde_json::Error::io(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        message.to_string(),
    ))
}

pub(crate) fn load_from(root: &std::path::Path) -> Result<RuntimeData, serde_json::Error> {
    let descriptors = crate::external::load_korg_descriptors(root).map_err(failure)?;
    Ok(RuntimeData {
        catalog: catalog_from_loaded(&descriptors)?,
        schemas: parameters_from_loaded(&descriptors)?,
        sysex: sysex_from_loaded(&descriptors)?,
    })
}

pub(crate) fn load() -> Result<&'static RuntimeData, serde_json::Error> {
    static DATA: OnceLock<Result<RuntimeData, String>> = OnceLock::new();
    DATA.get_or_init(|| {
        let root = crate::external::descriptor_root().map_err(|e| e.to_string())?;
        load_from(&root).map_err(|e| e.to_string())
    })
    .as_ref()
    .map_err(failure)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn private_root() -> PathBuf {
        std::env::var_os("KORG_ANALYSIS_ROOT")
            .map(PathBuf::from)
            .expect("set KORG_ANALYSIS_ROOT to the private KORG analysis checkout")
    }

    /// Compare labels only where the private analysis records one.  Vendor
    /// parameter names come from the extracted Strings XML, which the private
    /// schema JSON does not contain.
    fn align_optional_labels(actual: &mut [ParameterSchema], expected: &[ParameterSchema]) {
        for (actual, expected) in actual.iter_mut().zip(expected) {
            for (actual, expected) in actual.parameters.iter_mut().zip(&expected.parameters) {
                if expected.display_label.is_none() {
                    actual.display_label = None;
                }
            }
        }
    }

    #[test]
    #[ignore = "requires KORG_ANALYSIS_ROOT containing the private extracted analysis"]
    fn external_cdl_catalog_matches_the_existing_device_catalog() {
        let root = crate::external::descriptor_root().unwrap();
        let actual =
            catalog_from_loaded(&crate::external::load_korg_descriptors(&root).unwrap()).unwrap();
        let expected: Catalog = serde_json::from_slice(
            &std::fs::read(private_root().join("analysis/products/catalog.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(actual.devices, expected.devices);
    }

    #[test]
    #[ignore = "requires KORG_ANALYSIS_ROOT containing the private extracted analysis"]
    fn external_cdl_sysex_matches_the_existing_descriptors() {
        let root = crate::external::descriptor_root().unwrap();
        let actual =
            sysex_from_loaded(&crate::external::load_korg_descriptors(&root).unwrap()).unwrap();
        let mut expected: SysexCatalog = serde_json::from_slice(
            &std::fs::read(private_root().join("analysis/sysex/descriptors.json")).unwrap(),
        )
        .unwrap();
        for descriptor in expected.devices.values_mut() {
            descriptor.source = None;
        }
        assert_eq!(actual.devices, expected.devices);
    }

    #[test]
    #[ignore = "requires KORG_ANALYSIS_ROOT containing the private extracted analysis"]
    fn external_cdl_parameters_match_all_existing_schemas() {
        let root = crate::external::descriptor_root().unwrap();
        let mut actual =
            parameters_from_loaded(&crate::external::load_korg_descriptors(&root).unwrap())
                .unwrap();
        let mut paths = std::fs::read_dir(private_root().join("analysis/parameters"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|e| e == "json"))
            .collect::<Vec<_>>();
        paths.sort();
        let expected: Vec<ParameterSchema> = paths
            .into_iter()
            .map(|path| {
                let mut schema: ParameterSchema =
                    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
                schema.source.clear();
                schema.table.clear();
                for parameter in &mut schema.parameters {
                    parameter.notes = None;
                    if schema.payload == "global" {
                        let ui: serde_json::Value = serde_json::from_slice(
                            &std::fs::read(
                                private_root().join("analysis/presentation/global-ui.json"),
                            )
                            .unwrap(),
                        )
                        .unwrap();
                        if let Some(field) = ui
                            .get(&schema.model)
                            .and_then(|model| model.get(parameter.offset.to_string()))
                        {
                            parameter.display_label = field
                                .get("label")
                                .and_then(|v| v.as_str())
                                .map(str::to_owned);
                            parameter.display_choices = field
                                .get("choices")
                                .and_then(|v| v.as_array())
                                .map(|v| v.iter().map(|v| v.as_str().unwrap().to_owned()).collect())
                                .unwrap_or_default();
                            parameter.display_choices_inferred = field
                                .get("choices_inferred")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false);
                        }
                    }
                }
                schema
            })
            .collect();
        align_optional_labels(&mut actual, &expected);
        assert_eq!(actual, expected);
    }

    #[test]
    #[ignore = "requires KORG_ANALYSIS_ROOT containing the private extracted analysis"]
    fn runtime_descriptors_preserve_all_behavioral_inputs() {
        let root = crate::external::descriptor_root().expect("external descriptor root");
        let mut actual = load_from(&root).expect("valid external CDL");
        let private = private_root();
        let catalog: Catalog = serde_json::from_slice(
            &std::fs::read(private.join("analysis/products/catalog.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(actual.catalog.devices, catalog.devices);

        let mut paths = std::fs::read_dir(private.join("analysis/parameters"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|e| e == "json"))
            .collect::<Vec<_>>();
        paths.sort();
        let expected: Vec<ParameterSchema> = paths
            .into_iter()
            .map(|path| {
                let mut schema: ParameterSchema =
                    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
                schema.source.clear();
                schema.table.clear();
                for parameter in &mut schema.parameters {
                    parameter.notes = None;
                    if schema.payload == "global" {
                        let ui: serde_json::Value = serde_json::from_slice(
                            &std::fs::read(
                                private_root().join("analysis/presentation/global-ui.json"),
                            )
                            .unwrap(),
                        )
                        .unwrap();
                        if let Some(field) = ui
                            .get(&schema.model)
                            .and_then(|model| model.get(parameter.offset.to_string()))
                        {
                            parameter.display_label = field
                                .get("label")
                                .and_then(|v| v.as_str())
                                .map(str::to_owned);
                            parameter.display_choices = field
                                .get("choices")
                                .and_then(|v| v.as_array())
                                .map(|v| v.iter().map(|v| v.as_str().unwrap().to_owned()).collect())
                                .unwrap_or_default();
                            parameter.display_choices_inferred = field
                                .get("choices_inferred")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false);
                        }
                    }
                }
                schema
            })
            .collect();
        align_optional_labels(&mut actual.schemas, &expected);
        assert_eq!(actual.schemas, expected);

        let mut expected: SysexCatalog = serde_json::from_slice(
            &std::fs::read(private.join("analysis/sysex/descriptors.json")).unwrap(),
        )
        .unwrap();
        for descriptor in expected.devices.values_mut() {
            descriptor.source = None;
        }
        assert_eq!(actual.sysex.devices, expected.devices);
    }

    #[test]
    fn missing_descriptor_directory_is_an_error() {
        assert!(load_from(&std::env::temp_dir().join("korg-nonexistent-descriptor-root")).is_err());
    }

    #[test]
    fn changed_external_catalog_parameters_and_commands_are_read_on_reload() {
        let source = crate::external::descriptor_root().unwrap().join("korg");
        let temp =
            std::env::temp_dir().join(format!("korg-external-reload-{}", std::process::id()));
        let target = temp.join("korg");
        std::fs::create_dir_all(&target).unwrap();
        for entry in std::fs::read_dir(source).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|ext| ext == "kdl") {
                std::fs::copy(&path, target.join(path.file_name().unwrap())).unwrap();
            }
        }
        let original = load_from(&temp).unwrap();
        let replace = |file: &str, before: &str, after: &str| {
            let path = target.join(file);
            let text = std::fs::read_to_string(&path).unwrap();
            assert!(text.contains(before), "fixture is missing {before}");
            std::fs::write(path, text.replacen(before, after, 1)).unwrap();
        };
        replace(
            "nanopad2.catalog.inc.kdl",
            "name=\"nanoPAD2\"",
            "name=\"External edit\"",
        );
        replace(
            "nanopad2.parameters.inc.kdl",
            "kind=\"number\" default=0 default-source=\"factory\"",
            "kind=\"number\" default=1 default-source=\"factory\"",
        );
        replace(
            "nanopad2.sysex.inc.kdl",
            "command \"scene_data\" byte=0 status=\"decoded\"",
            "command \"scene_data\" status=\"unknown\"",
        );
        let changed = load_from(&temp).unwrap();
        assert_eq!(
            original.catalog.find_by_class("X10830").unwrap().device,
            "nanoPAD2"
        );
        assert_eq!(
            changed.catalog.find_by_class("X10830").unwrap().device,
            "External edit"
        );
        let global = |data: &RuntimeData| {
            data.schemas
                .iter()
                .find(|s| s.model == "X10830" && s.payload == "global")
                .unwrap()
                .parameters[0]
                .default
        };
        assert_eq!(global(&original), Some(0));
        assert_eq!(global(&changed), Some(1));
        assert_eq!(
            original.sysex.devices["X10830"].commands["scene_data"],
            Some(0)
        );
        assert_eq!(changed.sysex.devices["X10830"].commands["scene_data"], None);
        std::fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn parameter_names_project_into_scene_display_labels() {
        let descriptor = ctrl_cdl::parse_str(
            "test",
            "cdl \"0.1\"\n\
             device \"test\"\n\
             parameter-layouts status=\"decoded\" {\n\
                 layout \"scene\" order=0 size=16 elements=1 stride=1 version=1 {\n\
                     parameter \"Value\" id=5 offset=0 width=1 shift=0 mask=255 min=0 max=127 bias=0 kind=\"number\" default=0 default-source=\"factory\"\n\
                 }\n\
             }\n\
             labels \"en\" {\n\
                 list \"parameter-names\" {\n\
                     item 5 \"CC Number\"\n\
                 }\n\
             }",
        )
        .unwrap();
        let mut descriptors = std::collections::BTreeMap::new();
        descriptors.insert("X1".to_owned(), descriptor);
        let schemas = parameters_from_loaded(&descriptors).unwrap();
        assert_eq!(schemas.len(), 1);
        assert_eq!(
            schemas[0].parameters[0].display_label.as_deref(),
            Some("CC Number")
        );
    }
}
