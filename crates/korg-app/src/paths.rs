//! Per-user directories of the editor.
//!
//! macOS keeps both settings and data under
//! `~/Library/Application Support/KORG Kontrol Editor/`. Other systems
//! follow the XDG base directories: `$XDG_CONFIG_HOME/korg-kontrol-editor`
//! for settings and `$XDG_DATA_HOME/korg-kontrol-editor` for data. Inside a
//! Flatpak sandbox those variables point into `~/.var/app/<app id>/`.

use std::path::PathBuf;

/// Folder name of the editor on XDG systems.
pub const XDG_NAME: &str = "korg-kontrol-editor";

/// Folder name of the editor on macOS.
pub const MACOS_NAME: &str = "KORG Kontrol Editor";

/// Directory for settings such as `preferences.json`.
pub fn config_dir() -> Option<PathBuf> {
    base("XDG_CONFIG_HOME", ".config")
}

/// Directory for user-installed data such as the device pictures.
pub fn data_dir() -> Option<PathBuf> {
    base("XDG_DATA_HOME", ".local/share")
}

/// Where `tools/install_device_assets.py` puts the device pictures by
/// default, and where the GUI looks for them after `KORG_DEVICE_ASSETS`.
pub fn device_assets_dir() -> Option<PathBuf> {
    data_dir().map(|dir| dir.join("device-assets"))
}

fn base(variable: &str, fallback: &str) -> Option<PathBuf> {
    resolve(
        cfg!(target_os = "macos"),
        std::env::var_os("HOME").map(PathBuf::from),
        std::env::var_os(variable).map(PathBuf::from),
        fallback,
    )
}

fn resolve(
    macos: bool,
    home: Option<PathBuf>,
    xdg: Option<PathBuf>,
    fallback: &str,
) -> Option<PathBuf> {
    if macos {
        return home.map(|home| home.join("Library/Application Support").join(MACOS_NAME));
    }
    // The XDG specification ignores relative values.
    match xdg.filter(|path| path.is_absolute()) {
        Some(path) => Some(path.join(XDG_NAME)),
        None => home.map(|home| home.join(fallback).join(XDG_NAME)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macos_uses_application_support() {
        let dir = resolve(true, Some("/Users/a".into()), None, ".config").unwrap();
        assert_eq!(
            dir,
            PathBuf::from("/Users/a/Library/Application Support/KORG Kontrol Editor")
        );
    }

    #[test]
    fn xdg_variable_wins_when_absolute() {
        let dir = resolve(
            false,
            Some("/home/a".into()),
            Some("/home/a/.var/app/x/config".into()),
            ".config",
        )
        .unwrap();
        assert_eq!(
            dir,
            PathBuf::from("/home/a/.var/app/x/config/korg-kontrol-editor")
        );
    }

    #[test]
    fn relative_xdg_value_falls_back_to_home() {
        let dir = resolve(false, Some("/home/a".into()), Some("rel".into()), ".config").unwrap();
        assert_eq!(dir, PathBuf::from("/home/a/.config/korg-kontrol-editor"));
    }
}
