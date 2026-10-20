//! CDL, the Controller Description Language (docs/CDL.md): `*.ctrl.kdl`
//! descriptors into a typed IR.
//!
//! [`load`] reads a descriptor, resolves its `include`s (CDL §2) and
//! converts the merged KDL into a [`Descriptor`]. Every error carries file,
//! line and column, and one load reports all errors it finds.
//!
//! This crate checks shape: node and property names, argument counts, value
//! types, templates (§6), paths (§4.4) and ranges. Cross-references (does
//! the message exist, does the field fit the block) and the `status` rules
//! (§4.1) belong to the validator.

mod convert;
mod error;
mod ir;
mod path;
mod raw;
mod template;

use std::io;
use std::path::Path as FsPath;

pub use convert::SUPPORTED_VERSIONS;
pub use error::{Error, Errors};
pub use ir::*;
pub use path::{Index, Path, Segment};
pub use template::{Checksum, ChecksumAlgorithm, Encoding, Piece, Storage, Template};

/// Loads a descriptor from the file system.
pub fn load(path: impl AsRef<FsPath>) -> Result<Descriptor, Errors> {
    load_with(path.as_ref(), &mut |p| std::fs::read_to_string(p))
}

/// Loads a descriptor, reading it and its includes through `read`.
pub fn load_with(
    path: &FsPath,
    read: &mut dyn FnMut(&FsPath) -> io::Result<String>,
) -> Result<Descriptor, Errors> {
    let mut loader = raw::Loader::new(read);
    let nodes = loader.load(path, None);
    convert::convert(loader.files, nodes, loader.errors)
}

/// Parses one descriptor held in memory. `name` labels errors; an
/// `include` fails because there is nothing to read it from.
pub fn parse_str(name: &str, text: &str) -> Result<Descriptor, Errors> {
    let path = FsPath::new(name);
    load_with(path, &mut |p| {
        if p == path {
            Ok(text.to_string())
        } else {
            Err(io::Error::new(
                io::ErrorKind::NotFound,
                "parse_str reads no other files",
            ))
        }
    })
}
