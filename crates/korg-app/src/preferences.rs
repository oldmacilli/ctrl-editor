//! Editor preferences, shared by the GUI and the TUI.
//!
//! Stored as `preferences.json` in `paths::config_dir()`.
//! Unknown fields are ignored so an older file keeps working.

use crate::links::LinkKind;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub warn_before_write: bool,
    pub auto_receive: bool,
    pub input_port: String,
    pub output_port: String,
    /// USB or Bluetooth when the device is on both; `None` picks USB.
    pub link: Option<LinkKind>,
    /// The MIDI backend id (`alsa`, `jack`, `coremidi`); empty picks the
    /// platform's own.  `--midi-backend` and `CTRL_MIDI_BACKEND` override it.
    pub midi_backend: String,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            warn_before_write: true,
            auto_receive: true,
            input_port: String::new(),
            output_port: String::new(),
            link: None,
            midi_backend: String::new(),
        }
    }
}

pub fn path() -> Option<PathBuf> {
    crate::paths::config_dir().map(|dir| dir.join("preferences.json"))
}

pub fn load() -> Preferences {
    path()
        .and_then(|path| std::fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

pub fn save(preferences: &Preferences) {
    let Some(path) = path() else {
        return;
    };
    // Errors are ignored, as before: the next run falls back to defaults.
    let _ = write(&path, preferences);
}

/// Replace `path` in one step.  The new JSON goes to a temporary file in the
/// same directory and is renamed over the old one, so a crash leaves either
/// the previous file or the new one, never a truncated one.
fn write(path: &Path, preferences: &Preferences) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(preferences).map_err(std::io::Error::other)?;
    let mut temp = path.as_os_str().to_owned();
    temp.push(".tmp");
    let temp = PathBuf::from(temp);
    std::fs::write(&temp, bytes)?;
    if let Err(error) = std::fs::rename(&temp, path) {
        let _ = std::fs::remove_file(&temp);
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_replaces_the_file_without_leaving_a_temp_file() {
        let dir = std::env::temp_dir().join(format!("korg-prefs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("preferences.json");
        write(&path, &Preferences::default()).unwrap();

        let changed = Preferences {
            warn_before_write: false,
            auto_receive: false,
            input_port: "nanoKONTROL Studio CTRL".into(),
            output_port: String::new(),
            link: Some(LinkKind::Bluetooth),
            midi_backend: "jack".into(),
        };
        write(&path, &changed).unwrap();

        let read: Preferences = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(read, changed);
        let names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["preferences.json"]);
        std::fs::remove_dir_all(dir).ok();
    }
}
