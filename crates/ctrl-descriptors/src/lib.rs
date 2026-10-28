//! The repository's CDL descriptors, embedded at build time.
//!
//! The editors (KORG, DB-1, TEC, UC-33) keep their own protocol code for
//! now (TODO P2). Each proves it speaks the same bytes as the shared
//! descriptor with a conformance test that loads the descriptor from here:
//!
//! ```toml
//! [dev-dependencies]
//! ctrl-descriptors = { git = "https://github.com/oldmacilli/ctrl-editor.git", rev = "…" }
//! ctrl-codec = { git = "https://github.com/oldmacilli/ctrl-editor.git", rev = "…" }
//! ```
//!
//! ```ignore
//! let d = ctrl_descriptors::load("korg/nanokontrol-studio.ctrl.kdl").unwrap();
//! let bytes = ctrl_codec::Codec::new(&d).encode("select-scene", &args).unwrap();
//! assert_eq!(bytes, korg_sysex::build_request(…));
//! ```

use std::io;
use std::path::{Component, Path, PathBuf};

pub use ctrl_cdl::{Descriptor, Errors};

include!(concat!(env!("OUT_DIR"), "/files.rs"));

/// The text of one embedded file, by its path below `descriptors/`.
pub fn file(rel: &str) -> Option<&'static str> {
    FILES.iter().find(|(p, _)| *p == rel).map(|(_, t)| *t)
}

/// Every device descriptor (`*.ctrl.kdl`), by path.
pub fn devices() -> impl Iterator<Item = &'static str> {
    FILES
        .iter()
        .map(|(p, _)| *p)
        .filter(|p| p.ends_with(".ctrl.kdl"))
}

fn normalise(p: &Path) -> String {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            c => out.push(c),
        }
    }
    out.to_string_lossy().replace('\\', "/")
}

/// Loads a device descriptor and its includes from the embedded files.
pub fn load(rel: &str) -> Result<Descriptor, Errors> {
    ctrl_cdl::load_with(Path::new(rel), &mut |p| {
        file(&normalise(p)).map(str::to_string).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("{} is not embedded", p.display()),
            )
        })
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_device_loads_and_validates() {
        let mut n = 0;
        for rel in super::devices() {
            let d = super::load(rel).unwrap_or_else(|e| panic!("{rel}:\n{e}"));
            let report = ctrl_cdl::validate(&d);
            let errors: Vec<_> = report.errors().map(|f| f.to_string()).collect();
            assert!(errors.is_empty(), "{rel}: {errors:#?}");
            n += 1;
        }
        assert!(n >= 20, "{n} devices");
    }
}
