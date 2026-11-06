//! Load external CDL files without embedding them in the binary.
//!
//! This is the runtime source for the catalog, the parameter schemas, the
//! SysEx descriptors and the panels.  The descriptor files are installed
//! beside the executable or named by `KORG_DESCRIPTOR_DIR`.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

pub use ctrl_cdl;

/// The catalog fields represented by current CDL. This is deliberately not
/// `DeviceInfo`: CDL does not yet encode localized names, OSType mappings or
/// factory file paths, and silently inventing those would lose behavior.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogProjection {
    pub class: String,
    pub model: Option<String>,
    pub file_model_key: Option<String>,
    pub scene_count: Option<u8>,
    pub scene_payload_size: Option<u16>,
    pub global_payload_size: Option<u16>,
    pub extensions: Vec<String>,
}

impl CatalogProjection {
    pub fn from_descriptor(descriptor: &ctrl_cdl::Descriptor) -> Result<Self, String> {
        let class = descriptor
            .device
            .class
            .clone()
            .filter(|c| !c.is_empty())
            .ok_or("device class is missing")?;
        let scene = descriptor.block("scene");
        let global = descriptor.block("global");
        let scene_count = scene
            .and_then(|b| b.count)
            .map(u8::try_from)
            .transpose()
            .map_err(|_| format!("{class}: scene count does not fit u8"))?;
        let size = |b: Option<&ctrl_cdl::Block>, label: &str| -> Result<Option<u16>, String> {
            b.and_then(|b| b.size)
                .map(u16::try_from)
                .transpose()
                .map_err(|_| format!("{class}: {label} size does not fit u16"))
        };
        let mut extensions = descriptor
            .drivers
            .iter()
            .filter(|driver| driver.name == "korg-file")
            .flat_map(|driver| &driver.provides)
            .filter_map(|provided| provided.strip_prefix("file:"))
            .map(str::to_owned)
            .collect::<Vec<_>>();
        extensions.sort();
        extensions.dedup();
        let scene_payload_size = size(scene, "scene")?;
        let global_payload_size = size(global, "global")?;
        Ok(Self {
            class,
            model: descriptor.device.model.clone(),
            file_model_key: descriptor.device.file_key.clone(),
            scene_count,
            scene_payload_size,
            global_payload_size,
            extensions,
        })
    }
}

/// A problem locating or loading a complete descriptor set.
#[derive(Debug)]
pub struct LoadError {
    path: PathBuf,
    message: String,
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path.display(), self.message)
    }
}

impl std::error::Error for LoadError {}

fn error(path: &Path, message: impl ToString) -> LoadError {
    LoadError {
        path: path.to_path_buf(),
        message: message.to_string(),
    }
}

/// Find the external descriptor root. An explicit override is authoritative:
/// a missing override is an error rather than a silent fallback.
pub fn descriptor_root() -> Result<PathBuf, LoadError> {
    let exe = std::env::current_exe().map_err(|e| error(Path::new("<executable>"), e))?;
    find_descriptor_root(std::env::var_os("KORG_DESCRIPTOR_DIR"), &exe)
}

fn find_descriptor_root(
    override_dir: Option<std::ffi::OsString>,
    exe: &Path,
) -> Result<PathBuf, LoadError> {
    if let Some(root) = override_dir {
        let root = PathBuf::from(root);
        return check_root(root);
    }
    let bin = exe
        .parent()
        .ok_or_else(|| error(exe, "executable has no parent"))?;
    for candidate in [
        bin.join("descriptors"),
        bin.join("../Resources/descriptors"),
        bin.join("../share/korg-kontrol-editor/descriptors"),
    ] {
        if candidate.join("korg").is_dir() {
            return Ok(candidate);
        }
    }
    Err(error(
        exe,
        "no installed KORG descriptors found; set KORG_DESCRIPTOR_DIR",
    ))
}

fn check_root(root: PathBuf) -> Result<PathBuf, LoadError> {
    if root.join("korg").is_dir() {
        Ok(root)
    } else {
        Err(error(
            &root,
            "descriptor root must contain a korg directory",
        ))
    }
}

/// Load every `korg/*.ctrl.kdl` from `root`, resolving its includes on disk.
/// Parsing failures, absent classes and duplicate classes reject the set.
pub fn load_korg_descriptors(
    root: &Path,
) -> Result<BTreeMap<String, ctrl_cdl::Descriptor>, LoadError> {
    let directory = root.join("korg");
    let entries = std::fs::read_dir(&directory).map_err(|e| error(&directory, e))?;
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| error(&directory, e))?;
        let path = entry.path();
        if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with(".ctrl.kdl"))
        {
            paths.push(path);
        }
    }
    paths.sort();
    if paths.is_empty() {
        return Err(error(&directory, "no *.ctrl.kdl descriptors found"));
    }
    let mut out = BTreeMap::new();
    for path in paths {
        let descriptor = ctrl_cdl::load(&path).map_err(|e| error(&path, e))?;
        let report = ctrl_cdl::validate(&descriptor);
        let findings = report.errors().map(ToString::to_string).collect::<Vec<_>>();
        if !findings.is_empty() {
            return Err(error(&path, findings.join("\n")));
        }
        let class = descriptor
            .device
            .class
            .clone()
            .filter(|c| !c.is_empty())
            .ok_or_else(|| error(&path, "device class is missing"))?;
        if out.insert(class.clone(), descriptor).is_some() {
            return Err(error(&path, format!("duplicate device class {class}")));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Fixture(std::path::PathBuf);

    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "korg-cdl-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(path.join("korg")).unwrap();
            Self(path)
        }
        fn write(&self, path: &str, text: &str) {
            std::fs::write(self.0.join("korg").join(path), text).unwrap();
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn loads_external_descriptors_and_resolves_includes() {
        let fixture = Fixture::new();
        fixture.write(
            "base.inc.kdl",
            "device \"korg.test\" {\n class \"X12345\"\n}\n",
        );
        fixture.write("test.ctrl.kdl", "cdl \"0.1\"\ninclude \"base.inc.kdl\"\n");
        let loaded = load_korg_descriptors(&fixture.0).unwrap();
        assert_eq!(loaded["X12345"].device.id, "korg.test");
        assert_eq!(loaded["X12345"].files.len(), 2);
    }

    #[test]
    fn reports_invalid_and_missing_inputs_instead_of_falling_back() {
        let fixture = Fixture::new();
        fixture.write("bad.ctrl.kdl", "cdl \"0.1\"\ninclude \"missing.inc.kdl\"\n");
        let error = load_korg_descriptors(&fixture.0).unwrap_err().to_string();
        assert!(error.contains("missing.inc.kdl"), "{error}");
        assert!(load_korg_descriptors(&fixture.0.join("missing")).is_err());
    }

    #[test]
    fn rejects_duplicate_classes() {
        let fixture = Fixture::new();
        for name in ["a.ctrl.kdl", "b.ctrl.kdl"] {
            fixture.write(
                name,
                "cdl \"0.1\"\ndevice \"korg.test\" {\n class \"X12345\"\n}\n",
            );
        }
        assert!(load_korg_descriptors(&fixture.0)
            .unwrap_err()
            .to_string()
            .contains("X12345"));
    }

    #[test]
    fn rejects_semantically_invalid_descriptors() {
        let fixture = Fixture::new();
        fixture.write(
            "bad.ctrl.kdl",
            r#"
cdl "0.1"
device "korg.test" { class "X12345"; }
block "scene" size=1 chunk=2 status="decoded" evidence="test"
"#,
        );
        assert!(load_korg_descriptors(&fixture.0).is_err());
    }

    #[test]
    fn explicit_root_is_authoritative() {
        let fixture = Fixture::new();
        let exe = fixture.0.join("bin/editor");
        assert_eq!(
            find_descriptor_root(Some(fixture.0.clone().into_os_string()), &exe).unwrap(),
            fixture.0
        );
        let invalid = fixture.0.join("absent");
        let err = find_descriptor_root(Some(invalid.into_os_string()), &exe).unwrap_err();
        assert!(err.to_string().contains("descriptor root"));
    }

    #[test]
    fn resolves_installed_unix_and_macos_layouts() {
        let fixture = Fixture::new();
        let unix_root = fixture
            .0
            .join("prefix/share/korg-kontrol-editor/descriptors");
        std::fs::create_dir_all(unix_root.join("korg")).unwrap();
        std::fs::create_dir_all(fixture.0.join("prefix/bin")).unwrap();
        let exe = fixture.0.join("prefix/bin/editor");
        assert_eq!(
            std::fs::canonicalize(find_descriptor_root(None, &exe).unwrap()).unwrap(),
            std::fs::canonicalize(unix_root).unwrap()
        );
        let mac_root = fixture.0.join("App.app/Contents/Resources/descriptors");
        std::fs::create_dir_all(mac_root.join("korg")).unwrap();
        std::fs::create_dir_all(fixture.0.join("App.app/Contents/MacOS")).unwrap();
        let exe = fixture.0.join("App.app/Contents/MacOS/editor");
        assert_eq!(
            std::fs::canonicalize(find_descriptor_root(None, &exe).unwrap()).unwrap(),
            std::fs::canonicalize(mac_root).unwrap()
        );
    }

    #[test]
    fn projects_only_catalog_fields_present_in_cdl() {
        let desc = ctrl_cdl::parse_str(
            "catalog.kdl",
            r#"
cdl "0.1"
device "korg.test" {
    model "Test"
    class "X12345"
    file-key "1234"
}
block "scene" size=97 count=4 status="decoded" evidence="test"
block "global" size=47 status="decoded" evidence="test"
driver "korg-file" provides="file:test_data file:test_set" status="decoded" evidence="test"
"#,
        )
        .unwrap();
        let p = CatalogProjection::from_descriptor(&desc).unwrap();
        assert_eq!(p.class, "X12345");
        assert_eq!(p.model.as_deref(), Some("Test"));
        assert_eq!(p.file_model_key.as_deref(), Some("1234"));
        assert_eq!(p.scene_count, Some(4));
        assert_eq!(p.scene_payload_size, Some(97));
        assert_eq!(p.global_payload_size, Some(47));
        assert_eq!(p.extensions, ["test_data", "test_set"]);
    }

    #[test]
    fn projection_rejects_values_the_existing_model_cannot_represent() {
        let desc = ctrl_cdl::parse_str(
            "oversize.kdl",
            r#"
cdl "0.1"
device "korg.test" { class "X12345"; }
block "scene" size=70000 count=4 status="decoded" evidence="test"
"#,
        )
        .unwrap();
        assert!(CatalogProjection::from_descriptor(&desc).is_err());
    }
}
