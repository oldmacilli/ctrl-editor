//! Codec for KORG KONTROL EDITOR product data files.
//!
//! The format is documented in `docs/file-format.md`.  All rules here are
//! measured from the 24 shipped files and the decompiled accessors.

use korg_model::{ChunkKind, FileModelKey};
use std::fmt;
use thiserror::Error;

pub const HEADER_SIZE: usize = 32;
pub const MARKER: u16 = 0xFFFF;
pub const TAIL: u32 = 0xFFFF_FFFF;

/// Eight-byte chunk key: model id plus kind.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ChunkKey {
    pub model: FileModelKey,
    pub kind: ChunkKind,
}

impl ChunkKey {
    pub const fn new(model: FileModelKey, kind: ChunkKind) -> Self {
        Self { model, kind }
    }

    /// Use the five-digit `SnS`/`SnD` style for the microKEY2 family.
    pub fn compact(&self) -> bool {
        self.model.compact()
    }

    pub fn as_bytes(&self) -> [u8; 8] {
        let mut out = [0u8; 8];
        if self.compact() {
            let model = self.model.as_str();
            out[..5].copy_from_slice(model.as_bytes());
            out[5..].copy_from_slice(self.kind.compact_suffix().as_bytes());
        } else {
            let model = self.model.as_str();
            out[..4].copy_from_slice(model.as_bytes());
            out[4..].copy_from_slice(self.kind.suffix().as_bytes());
        }
        out
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, FormatError> {
        if bytes.len() != 8 {
            return Err(FormatError::InvalidKey);
        }
        let text = std::str::from_utf8(bytes).map_err(|_| FormatError::InvalidKey)?;
        for suffix in ["ScnS", "ScnD", "GlbD", "SnS", "SnD"] {
            if let Some(model_text) = text.strip_suffix(suffix) {
                let digits = model_text.len();
                if (digits == 4 || digits == 5) && model_text.bytes().all(|b| b.is_ascii_digit()) {
                    let model =
                        FileModelKey::new(model_text).map_err(|_| FormatError::InvalidKey)?;
                    let kind = ChunkKind::from_suffix(suffix).ok_or(FormatError::InvalidKey)?;
                    let key = ChunkKey { model, kind };
                    if key.as_bytes() != bytes {
                        return Err(FormatError::InvalidKey);
                    }
                    return Ok(key);
                }
            }
        }
        Err(FormatError::InvalidKey)
    }
}

impl fmt::Display for ChunkKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = String::from_utf8_lossy(&self.as_bytes()).to_string();
        write!(f, "{text}")
    }
}

/// Decoded payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Payload {
    SceneSet(Vec<Vec<u8>>),
    Scene(Vec<u8>),
    Global(Vec<u8>),
}

/// One decoded file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    pub key: ChunkKey,
    pub payload: Payload,
}

impl Chunk {
    pub fn scene_count(&self) -> usize {
        match &self.payload {
            Payload::SceneSet(scenes) => scenes.len(),
            Payload::Scene(_) => 1,
            Payload::Global(_) => 1,
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum FormatError {
    #[error("file is shorter than a chunk header")]
    Truncated,
    #[error("invalid chunk key")]
    InvalidKey,
    #[error("header size is {0}, expected 32")]
    HeaderSize(u16),
    #[error("reserved header field differs from the observed pattern")]
    Reserved,
    #[error("size fields disagree: {0} != {1}")]
    SizeMismatch(u32, u32),
    #[error("scene {index} has size {size}, expected element size {element}")]
    SceneSize {
        index: usize,
        size: usize,
        element: usize,
    },
    #[error("scene count {0} does not match the payload")]
    SceneCount(u32),
    #[error("scene {0} is truncated")]
    SceneTruncated(usize),
    #[error("payload is shorter than the declared size")]
    PayloadTruncated,
    #[error("global payload has {0} bytes, expected {1}")]
    GlobalPadding(usize, usize),
    #[error("scene set has no scenes")]
    EmptySceneSet,
    #[error("scenes have mixed sizes")]
    MixedSceneSizes,
    #[error("payload does not fit the 16-bit size field")]
    PayloadTooLarge,
    #[error("global payload padding is not 0xFF")]
    GlobalPaddingByte,
}

pub const fn align16(value: usize) -> usize {
    (value + 15) & !15
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn write_header(out: &mut Vec<u8>, key: &ChunkKey, declared: u16, count: u32, element: u32) {
    out.extend_from_slice(&key.as_bytes());
    out.extend_from_slice(&(HEADER_SIZE as u16).to_le_bytes());
    out.extend_from_slice(&declared.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&MARKER.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&element.to_le_bytes());
    out.extend_from_slice(&TAIL.to_le_bytes());
}

/// Decode one product data file.
pub fn decode(bytes: &[u8]) -> Result<Chunk, FormatError> {
    if bytes.len() < HEADER_SIZE {
        return Err(FormatError::Truncated);
    }
    let key = ChunkKey::parse(&bytes[..8])?;
    let header_size = read_u16(bytes, 8);
    let declared = read_u16(bytes, 10) as usize;
    let zero = read_u32(bytes, 12);
    let reserved = read_u16(bytes, 16);
    let marker = read_u16(bytes, 18);
    let count = read_u32(bytes, 20);
    let element = read_u32(bytes, 24) as usize;
    let tail = read_u32(bytes, 28);

    if header_size as usize != HEADER_SIZE {
        return Err(FormatError::HeaderSize(header_size));
    }
    if zero != 0 || reserved != 0 || marker != MARKER || tail != TAIL {
        return Err(FormatError::Reserved);
    }

    let body = &bytes[HEADER_SIZE..];
    let payload = match key.kind {
        ChunkKind::SceneSet => decode_scene_set(&key, declared, count, element, body)?,
        ChunkKind::GlobalData => {
            if count != 1 {
                return Err(FormatError::SceneCount(count));
            }
            if declared > body.len() || body.len() != align16(declared) {
                return Err(FormatError::GlobalPadding(body.len(), align16(declared)));
            }
            if body[declared..].iter().any(|byte| *byte != 0xFF) {
                return Err(FormatError::GlobalPaddingByte);
            }
            Payload::Global(body[..declared].to_vec())
        }
        ChunkKind::SceneData => {
            if count != 1 || element != declared {
                return Err(FormatError::SceneCount(count));
            }
            if body.len() != declared {
                return Err(FormatError::PayloadTruncated);
            }
            Payload::Scene(body.to_vec())
        }
    };
    Ok(Chunk { key, payload })
}

fn decode_scene_set(
    key: &ChunkKey,
    declared: usize,
    count: u32,
    element: usize,
    body: &[u8],
) -> Result<Payload, FormatError> {
    let mut scenes = Vec::new();
    let mut offset = 0usize;
    while offset + HEADER_SIZE <= body.len() {
        let scene_key = ChunkKey::parse(&body[offset..offset + 8])?;
        if scene_key.model != key.model || scene_key.kind != ChunkKind::SceneData {
            return Err(FormatError::InvalidKey);
        }
        let scene_header = read_u16(body, offset + 8);
        let scene_declared = read_u16(body, offset + 10) as usize;
        let scene_zero = read_u32(body, offset + 12);
        let scene_reserved = read_u16(body, offset + 16);
        let scene_marker = read_u16(body, offset + 18);
        let scene_count = read_u32(body, offset + 20);
        let scene_element = read_u32(body, offset + 24) as usize;
        let scene_tail = read_u32(body, offset + 28);
        if scene_header as usize != HEADER_SIZE
            || scene_zero != 0
            || scene_reserved != 0
            || scene_marker != MARKER
            || scene_tail != TAIL
            || scene_count != 1
            || scene_element != scene_declared
        {
            return Err(FormatError::InvalidKey);
        }
        if scene_declared != element {
            return Err(FormatError::SceneSize {
                index: scenes.len(),
                size: scene_declared,
                element,
            });
        }
        let start = offset + HEADER_SIZE;
        let end = start + scene_declared;
        if end > body.len() {
            return Err(FormatError::SceneTruncated(scenes.len()));
        }
        scenes.push(body[start..end].to_vec());
        offset = end;
    }
    if offset != body.len() {
        return Err(FormatError::PayloadTruncated);
    }
    if scenes.is_empty() {
        return Err(FormatError::EmptySceneSet);
    }
    if count as usize != scenes.len() {
        return Err(FormatError::SceneCount(count));
    }
    if declared != scenes.len() * align16(HEADER_SIZE + element) {
        return Err(FormatError::SizeMismatch(
            declared as u32,
            (scenes.len() * align16(HEADER_SIZE + element)) as u32,
        ));
    }
    Ok(Payload::SceneSet(scenes))
}

/// Encode one chunk.  The output follows the stored padding rules.
pub fn encode(chunk: &Chunk) -> Result<Vec<u8>, FormatError> {
    let mut out = Vec::new();
    match &chunk.payload {
        Payload::SceneSet(scenes) => {
            if scenes.is_empty() {
                return Err(FormatError::EmptySceneSet);
            }
            let element = scenes[0].len();
            if scenes.iter().any(|scene| scene.len() != element) {
                return Err(FormatError::MixedSceneSizes);
            }
            let scene_key = ChunkKey::new(chunk.key.model.clone(), ChunkKind::SceneData);
            let mut body = Vec::with_capacity(scenes.len() * (HEADER_SIZE + element));
            for scene in scenes {
                write_header(&mut body, &scene_key, element as u16, 1, element as u32);
                body.extend_from_slice(scene);
            }
            let declared = scenes.len() * align16(HEADER_SIZE + element);
            if declared > u16::MAX as usize {
                return Err(FormatError::PayloadTooLarge);
            }
            write_header(
                &mut out,
                &chunk.key,
                declared as u16,
                scenes.len() as u32,
                element as u32,
            );
            out.extend_from_slice(&body);
        }
        Payload::Global(data) => {
            if data.len() > u16::MAX as usize {
                return Err(FormatError::PayloadTooLarge);
            }
            write_header(
                &mut out,
                &chunk.key,
                data.len() as u16,
                1,
                data.len() as u32,
            );
            out.extend_from_slice(data);
            out.resize(HEADER_SIZE + align16(data.len()), 0xFF);
        }
        Payload::Scene(data) => {
            if data.len() > u16::MAX as usize {
                return Err(FormatError::PayloadTooLarge);
            }
            write_header(
                &mut out,
                &chunk.key,
                data.len() as u16,
                1,
                data.len() as u32,
            );
            out.extend_from_slice(data);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene_set() -> Chunk {
        Chunk {
            key: ChunkKey::new(FileModelKey::new("0820").unwrap(), ChunkKind::SceneSet),
            payload: Payload::SceneSet(vec![vec![1, 2, 3, 4], vec![5, 6, 7, 8]]),
        }
    }

    #[test]
    fn scene_set_round_trips() {
        let bytes = encode(&scene_set()).unwrap();
        assert_eq!(bytes.len(), 32 + 2 * (32 + 4));
        let decoded = decode(&bytes).unwrap();
        assert_eq!(decoded, scene_set());
    }

    #[test]
    fn global_is_padded_and_round_trips() {
        let chunk = Chunk {
            key: ChunkKey::new(FileModelKey::new("0830").unwrap(), ChunkKind::GlobalData),
            payload: Payload::Global(vec![9, 8, 7]),
        };
        let bytes = encode(&chunk).unwrap();
        assert_eq!(bytes.len(), 32 + 16);
        assert_eq!(decode(&bytes).unwrap(), chunk);
    }

    #[test]
    fn compact_key_round_trips() {
        let chunk = Chunk {
            key: ChunkKey::new(FileModelKey::new("14810").unwrap(), ChunkKind::SceneSet),
            payload: Payload::SceneSet(vec![vec![0u8; 256]]),
        };
        let bytes = encode(&chunk).unwrap();
        assert_eq!(bytes[5..8], *b"SnS");
        assert_eq!(bytes[32 + 5..32 + 8], *b"SnD");
        assert_eq!(decode(&bytes).unwrap(), chunk);
    }

    #[test]
    fn rejects_bad_header() {
        let mut bytes = encode(&scene_set()).unwrap();
        bytes[8] = 0;
        assert_eq!(decode(&bytes), Err(FormatError::HeaderSize(0)));
    }

    /// Opt-in: set KORG_PRODUCTS_DIR to the installed Products directory.
    #[test]
    fn round_trips_installed_files() {
        let Ok(root) = std::env::var("KORG_PRODUCTS_DIR") else {
            eprintln!("skip: set KORG_PRODUCTS_DIR to the installed Products directory");
            return;
        };
        let root = std::path::Path::new(&root);
        let mut checked = 0;
        for entry in std::fs::read_dir(root).unwrap() {
            let dir = entry.unwrap().path();
            if !dir.is_dir() {
                continue;
            }
            for file in std::fs::read_dir(dir).unwrap() {
                let path = file.unwrap().path();
                if !path.is_file() {
                    continue;
                }
                let bytes = std::fs::read(&path).unwrap();
                let decoded =
                    decode(&bytes).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
                let encoded =
                    encode(&decoded).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
                assert_eq!(encoded, bytes, "{} did not round-trip", path.display());
                checked += 1;
            }
        }
        assert_eq!(checked, 24);
    }
}
