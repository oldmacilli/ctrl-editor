//! Platform-neutral document and settings storage.
//!
//! The core sees an opaque `DocRef`.  A platform adapter decides what it
//! means.  `MemoryStore` supports tests.  `FsStore` is opt-in.

use std::collections::HashMap;
use std::sync::Mutex;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DocRef(pub String);

impl DocRef {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocName {
    pub stem: String,
    pub extension: String,
}

impl DocName {
    pub fn from_path(path: &str) -> Self {
        let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
        match name.rsplit_once('.') {
            Some((stem, extension)) => Self {
                stem: stem.to_owned(),
                extension: extension.to_owned(),
            },
            None => Self {
                stem: name.to_owned(),
                extension: String::new(),
            },
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum StoreError {
    #[error("document not found")]
    NotFound,
    #[error("permission denied")]
    PermissionDenied,
    #[error("storage error: {0}")]
    Io(String),
    #[error("setting key must be a single file name")]
    InvalidKey,
}

pub trait FileStore {
    fn read(&self, doc: &DocRef) -> Result<Vec<u8>, StoreError>;
    /// Atomic: old content survives a failed write.
    fn write(&self, doc: &DocRef, bytes: &[u8]) -> Result<(), StoreError>;
    fn name(&self, doc: &DocRef) -> DocName;
    fn read_setting(&self, key: &str) -> Result<Option<Vec<u8>>, StoreError>;
    fn write_setting(&self, key: &str, bytes: &[u8]) -> Result<(), StoreError>;
}

/// In-memory store for tests and for the core's default.
#[derive(Debug, Default)]
pub struct MemoryStore {
    docs: Mutex<HashMap<String, Vec<u8>>>,
    settings: Mutex<HashMap<String, Vec<u8>>>,
}

impl MemoryStore {
    pub fn insert(&self, doc: impl Into<String>, bytes: impl Into<Vec<u8>>) {
        self.docs.lock().unwrap().insert(doc.into(), bytes.into());
    }
}

impl FileStore for MemoryStore {
    fn read(&self, doc: &DocRef) -> Result<Vec<u8>, StoreError> {
        self.docs
            .lock()
            .unwrap()
            .get(&doc.0)
            .cloned()
            .ok_or(StoreError::NotFound)
    }

    fn write(&self, doc: &DocRef, bytes: &[u8]) -> Result<(), StoreError> {
        self.docs
            .lock()
            .unwrap()
            .insert(doc.0.clone(), bytes.to_vec());
        Ok(())
    }

    fn name(&self, doc: &DocRef) -> DocName {
        DocName::from_path(&doc.0)
    }

    fn read_setting(&self, key: &str) -> Result<Option<Vec<u8>>, StoreError> {
        Ok(self.settings.lock().unwrap().get(key).cloned())
    }

    fn write_setting(&self, key: &str, bytes: &[u8]) -> Result<(), StoreError> {
        self.settings
            .lock()
            .unwrap()
            .insert(key.to_owned(), bytes.to_vec());
        Ok(())
    }
}

#[cfg(feature = "fs")]
mod fs_store {
    use super::*;
    use std::io::Write;
    use std::path::{Path, PathBuf};

    /// Filesystem store.  `settings_dir` is supplied by the platform shell.
    #[derive(Debug, Clone)]
    pub struct FsStore {
        settings_dir: PathBuf,
    }

    impl FsStore {
        pub fn new(settings_dir: impl Into<PathBuf>) -> Self {
            Self {
                settings_dir: settings_dir.into(),
            }
        }

        fn setting_path(&self, key: &str) -> Result<PathBuf, StoreError> {
            if key.is_empty() || key.contains('/') || key.contains('\\') || key.contains("..") {
                return Err(StoreError::InvalidKey);
            }
            Ok(self.settings_dir.join(key))
        }
    }

    fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), StoreError> {
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        let file_name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let temp = parent.join(format!(".{file_name}.tmp"));
        let result = (|| {
            let mut file =
                std::fs::File::create(&temp).map_err(|error| StoreError::Io(error.to_string()))?;
            file.write_all(bytes)
                .map_err(|error| StoreError::Io(error.to_string()))?;
            file.sync_all()
                .map_err(|error| StoreError::Io(error.to_string()))?;
            std::fs::rename(&temp, path).map_err(|error| StoreError::Io(error.to_string()))
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temp);
        }
        result
    }

    impl FileStore for FsStore {
        fn read(&self, doc: &DocRef) -> Result<Vec<u8>, StoreError> {
            std::fs::read(&doc.0).map_err(|error| match error.kind() {
                std::io::ErrorKind::NotFound => StoreError::NotFound,
                std::io::ErrorKind::PermissionDenied => StoreError::PermissionDenied,
                _ => StoreError::Io(error.to_string()),
            })
        }

        fn write(&self, doc: &DocRef, bytes: &[u8]) -> Result<(), StoreError> {
            let path = PathBuf::from(&doc.0);
            atomic_write(&path, bytes)
        }

        fn name(&self, doc: &DocRef) -> DocName {
            DocName::from_path(&doc.0)
        }

        fn read_setting(&self, key: &str) -> Result<Option<Vec<u8>>, StoreError> {
            let path = self.setting_path(key)?;
            match std::fs::read(path) {
                Ok(bytes) => Ok(Some(bytes)),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(error) => Err(StoreError::Io(error.to_string())),
            }
        }

        fn write_setting(&self, key: &str, bytes: &[u8]) -> Result<(), StoreError> {
            let path = self.setting_path(key)?;
            std::fs::create_dir_all(&self.settings_dir)
                .map_err(|error| StoreError::Io(error.to_string()))?;
            atomic_write(&path, bytes)
        }
    }
}

#[cfg(feature = "fs")]
pub use fs_store::FsStore;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_store_round_trips() {
        let store = MemoryStore::default();
        let doc = DocRef::new("scene.nanokey2_data");
        store.write(&doc, &[1, 2, 3]).unwrap();
        assert_eq!(store.read(&doc).unwrap(), vec![1, 2, 3]);
        store.write_setting("preferences.json", b"{}").unwrap();
        assert_eq!(
            store.read_setting("preferences.json").unwrap(),
            Some(b"{}".to_vec())
        );
    }

    #[test]
    fn doc_name_splits_extension() {
        let name = DocName::from_path("/a/b/nanoKEY2 Scene Data.nanokey2_data");
        assert_eq!(name.stem, "nanoKEY2 Scene Data");
        assert_eq!(name.extension, "nanokey2_data");
    }
}
