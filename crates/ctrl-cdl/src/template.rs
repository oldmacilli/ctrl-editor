//! Message templates (CDL §6): bytes and placeholders, parsed but not encoded.

use std::fmt;

use serde::{Deserialize, Serialize};

/// A stored or wire value type (CDL §4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Storage {
    U7,
    U8,
    U14le,
    U14be,
    S7,
    U16le,
    U16be,
    Bool,
    /// `bits(n)` inside a byte, 1..=8.
    Bits(u8),
    /// `ascii(n)`, space padded.
    Ascii(u16),
}

impl Storage {
    pub fn parse(s: &str) -> Result<Self, String> {
        let simple = match s {
            "u7" => Some(Self::U7),
            "u8" => Some(Self::U8),
            "u14le" => Some(Self::U14le),
            "u14be" => Some(Self::U14be),
            "s7" => Some(Self::S7),
            "u16le" => Some(Self::U16le),
            "u16be" => Some(Self::U16be),
            "bool" => Some(Self::Bool),
            _ => None,
        };
        if let Some(storage) = simple {
            return Ok(storage);
        }
        if let Some(n) = call_arg(s, "bits") {
            return match n.parse::<u8>() {
                Ok(n @ 1..=8) => Ok(Self::Bits(n)),
                _ => Err(format!("`{s}`: bits(n) takes 1..=8")),
            };
        }
        if let Some(n) = call_arg(s, "ascii") {
            return match n.parse::<u16>() {
                Ok(n) if n > 0 => Ok(Self::Ascii(n)),
                _ => Err(format!("`{s}`: ascii(n) takes a length above 0")),
            };
        }
        Err(format!("`{s}` is not a storage type (u7, u8, u14le, u14be, s7, u16le, u16be, bool, bits(n), ascii(n))"))
    }
}

impl fmt::Display for Storage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::U7 => f.write_str("u7"),
            Self::U8 => f.write_str("u8"),
            Self::U14le => f.write_str("u14le"),
            Self::U14be => f.write_str("u14be"),
            Self::S7 => f.write_str("s7"),
            Self::U16le => f.write_str("u16le"),
            Self::U16be => f.write_str("u16be"),
            Self::Bool => f.write_str("bool"),
            Self::Bits(n) => write!(f, "bits({n})"),
            Self::Ascii(n) => write!(f, "ascii({n})"),
        }
    }
}

/// How a `{name:…}` placeholder encodes its value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Encoding {
    Storage(Storage),
    /// The whole block, 8 to 7 bit packed (KORG).
    Packed7,
    /// The whole block as is.
    Raw,
    /// A device pack type such as `korg-model` (CDL §8).
    Vendor(String),
}

impl Encoding {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "packed7" => Ok(Self::Packed7),
            "raw" => Ok(Self::Raw),
            _ => match Storage::parse(s) {
                Ok(storage) => Ok(Self::Storage(storage)),
                Err(_) if is_ident(s) => Ok(Self::Vendor(s.to_string())),
                Err(error) => Err(error),
            },
        }
    }
}

impl fmt::Display for Encoding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Storage(storage) => storage.fmt(f),
            Self::Packed7 => f.write_str("packed7"),
            Self::Raw => f.write_str("raw"),
            Self::Vendor(name) => f.write_str(name),
        }
    }
}

/// Built-in checksums (CDL §6.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ChecksumAlgorithm {
    Sum7,
    Roland,
    Xor7,
    Fnv1a32,
}

/// `{sum:algorithm(from,to)}`: byte indexes into the message; negative ones
/// count from the end.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checksum {
    pub algorithm: ChecksumAlgorithm,
    pub from: i64,
    pub to: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "piece", rename_all = "kebab-case")]
pub enum Piece {
    /// A literal byte, `F0`.
    Byte {
        value: u8,
    },
    /// `..`: any one byte (reply patterns only).
    Any,
    /// `{name}` or `{name:encoding}`.
    Field {
        name: String,
        encoding: Option<Encoding>,
    },
    /// `{name*}`: the rest up to the terminator (reply patterns only).
    Rest {
        name: String,
    },
    /// `{0x20 + s}` or `{=expr}`: a computed byte, an LBMT expression.
    Expr {
        expr: String,
    },
    Checksum(Checksum),
}

impl Piece {
    /// True for pieces that only make sense when matching a reply.
    pub fn is_pattern_only(&self) -> bool {
        matches!(self, Self::Any | Self::Rest { .. })
    }
}

impl fmt::Display for Piece {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Byte { value } => write!(f, "{value:02X}"),
            Self::Any => f.write_str(".."),
            Self::Field {
                name,
                encoding: None,
            } => write!(f, "{{{name}}}"),
            Self::Field {
                name,
                encoding: Some(encoding),
            } => write!(f, "{{{name}:{encoding}}}"),
            Self::Rest { name } => write!(f, "{{{name}*}}"),
            Self::Expr { expr } => write!(f, "{{={expr}}}"),
            Self::Checksum(c) => {
                let algorithm = match c.algorithm {
                    ChecksumAlgorithm::Sum7 => "sum7",
                    ChecksumAlgorithm::Roland => "roland",
                    ChecksumAlgorithm::Xor7 => "xor7",
                    ChecksumAlgorithm::Fnv1a32 => "fnv1a32",
                };
                write!(f, "{{sum:{algorithm}({},{})}}", c.from, c.to)
            }
        }
    }
}

/// A parsed template string such as `F0 09 00 00 00 02 03 {index} F7`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Template {
    pub pieces: Vec<Piece>,
}

impl Template {
    pub fn parse(s: &str) -> Result<Self, String> {
        let mut pieces = Vec::new();
        let mut rest = s.trim_start();
        while !rest.is_empty() {
            if let Some(after) = rest.strip_prefix('{') {
                let end = after
                    .find('}')
                    .ok_or_else(|| format!("unclosed `{{` in `{rest}`"))?;
                pieces.push(placeholder(after[..end].trim())?);
                rest = &after[end + 1..];
            } else {
                let end = rest
                    .find(|c: char| c.is_whitespace() || c == '{')
                    .unwrap_or(rest.len());
                let token = &rest[..end];
                pieces.push(match token {
                    ".." => Piece::Any,
                    _ if token.len() == 2 && token.bytes().all(|b| b.is_ascii_hexdigit()) => {
                        Piece::Byte {
                            value: u8::from_str_radix(token, 16).expect("two hex digits"),
                        }
                    }
                    _ => {
                        return Err(format!(
                        "`{token}` is not a byte (two hex digits), `..` or a `{{…}}` placeholder"
                    ))
                    }
                });
                rest = &rest[end..];
            }
            rest = rest.trim_start();
        }
        if pieces.is_empty() {
            return Err("empty template".into());
        }
        Ok(Self { pieces })
    }
}

impl fmt::Display for Template {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, piece) in self.pieces.iter().enumerate() {
            if i > 0 {
                f.write_str(" ")?;
            }
            piece.fmt(f)?;
        }
        Ok(())
    }
}

fn placeholder(inner: &str) -> Result<Piece, String> {
    if inner.is_empty() {
        return Err("empty `{}` placeholder".into());
    }
    if let Some(expr) = inner.strip_prefix('=') {
        let expr = expr.trim();
        if expr.is_empty() {
            return Err("empty `{=}` expression".into());
        }
        return Ok(Piece::Expr { expr: expr.into() });
    }
    if let Some(name) = inner.strip_suffix('*') {
        if is_ident(name) {
            return Ok(Piece::Rest { name: name.into() });
        }
    }
    if let Some((name, encoding)) = inner.split_once(':') {
        let (name, encoding) = (name.trim(), encoding.trim());
        if name == "sum" {
            return checksum(encoding).map(Piece::Checksum);
        }
        if is_ident(name) {
            return Ok(Piece::Field {
                name: name.into(),
                encoding: Some(Encoding::parse(encoding)?),
            });
        }
    }
    if is_ident(inner) {
        return Ok(Piece::Field {
            name: inner.into(),
            encoding: None,
        });
    }
    Ok(Piece::Expr { expr: inner.into() })
}

fn checksum(s: &str) -> Result<Checksum, String> {
    let bad = || format!("`{s}`: a checksum is algorithm(from,to), e.g. roland(5,-3)");
    let open = s.find('(').ok_or_else(bad)?;
    let args = s[open..]
        .strip_prefix('(')
        .and_then(|a| a.strip_suffix(')'))
        .ok_or_else(bad)?;
    let algorithm = match &s[..open] {
        "sum7" => ChecksumAlgorithm::Sum7,
        "roland" => ChecksumAlgorithm::Roland,
        "xor7" => ChecksumAlgorithm::Xor7,
        "fnv1a32" => ChecksumAlgorithm::Fnv1a32,
        other => {
            return Err(format!(
                "`{other}` is not a built-in checksum (sum7, roland, xor7, fnv1a32)"
            ))
        }
    };
    let (from, to) = args.split_once(',').ok_or_else(bad)?;
    Ok(Checksum {
        algorithm,
        from: from.trim().parse().map_err(|_| bad())?,
        to: to.trim().parse().map_err(|_| bad())?,
    })
}

/// A name in CDL: a letter, `_` or `$` first, then letters, digits, `_`,
/// `-`, `.` or `$` (`device-id`, `cmd.scene_data_request`, `drawbar.$i.cc`).
pub(crate) fn is_ident(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_' || c == '$')
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '$'))
}

fn call_arg<'a>(s: &'a str, function: &str) -> Option<&'a str> {
    s.strip_prefix(function)?
        .strip_prefix('(')?
        .strip_suffix(')')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(name: &str) -> Piece {
        Piece::Field {
            name: name.into(),
            encoding: None,
        }
    }

    #[test]
    fn bytes_fields_and_expressions() {
        let t = Template::parse("F0 42 {0x40 | device-id} {model:korg-model} {index} f7").unwrap();
        assert_eq!(
            t.pieces,
            vec![
                Piece::Byte { value: 0xF0 },
                Piece::Byte { value: 0x42 },
                Piece::Expr {
                    expr: "0x40 | device-id".into()
                },
                Piece::Field {
                    name: "model".into(),
                    encoding: Some(Encoding::Vendor("korg-model".into()))
                },
                field("index"),
                Piece::Byte { value: 0xF7 },
            ]
        );
    }

    #[test]
    fn pattern_pieces_and_checksums() {
        let t =
            Template::parse("F0 7E {ch} .. {family:u14le} {version*} {sum:roland(5,-2)} {=v + 1}")
                .unwrap();
        assert_eq!(t.pieces[3], Piece::Any);
        assert_eq!(
            t.pieces[4],
            Piece::Field {
                name: "family".into(),
                encoding: Some(Encoding::Storage(Storage::U14le))
            }
        );
        assert_eq!(
            t.pieces[5],
            Piece::Rest {
                name: "version".into()
            }
        );
        assert_eq!(
            t.pieces[6],
            Piece::Checksum(Checksum {
                algorithm: ChecksumAlgorithm::Roland,
                from: 5,
                to: -2
            })
        );
        assert_eq!(
            t.pieces[7],
            Piece::Expr {
                expr: "v + 1".into()
            }
        );
        assert!(t.pieces[3].is_pattern_only() && t.pieces[5].is_pattern_only());
    }

    #[test]
    fn placeholders_need_no_spaces_around_them() {
        let t = Template::parse("{0x10 + s}00{cc}").unwrap();
        assert_eq!(t.pieces.len(), 3);
        assert_eq!(t.to_string(), "{=0x10 + s} 00 {cc}");
    }

    #[test]
    fn display_round_trips() {
        let src = "F0 09 {index} {block:packed7} {name:ascii(16)} {rest*} .. {sum:sum7(1,-2)} F7";
        let t = Template::parse(src).unwrap();
        assert_eq!(Template::parse(&t.to_string()).unwrap(), t);
    }

    #[test]
    fn rejects_bad_tokens() {
        assert!(Template::parse("F0 7G").unwrap_err().contains("`7G`"));
        assert!(Template::parse("F0 0x7F").is_err());
        assert!(Template::parse("F0 {index")
            .unwrap_err()
            .contains("unclosed"));
        assert!(Template::parse("{}").is_err());
        assert!(Template::parse("  ").is_err());
        assert!(Template::parse("{sum:crc(1,2)}").is_err());
        assert!(Template::parse("{x:bits(9)}").is_err());
    }

    #[test]
    fn storage_types() {
        assert_eq!(Storage::parse("ascii(16)"), Ok(Storage::Ascii(16)));
        assert_eq!(Storage::parse("bits(3)"), Ok(Storage::Bits(3)));
        assert!(Storage::parse("u9").is_err());
        assert!(Storage::parse("ascii(0)").is_err());
    }
}
