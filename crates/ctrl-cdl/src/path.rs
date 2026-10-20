//! Data paths (CDL §4.4): `block [ "[" index "]" ] { "." name [ "[" index "]" ] }`.

use std::fmt;

use serde::{Deserialize, Serialize};

/// An index inside `[…]`. Expressions stay LBMT source text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Index {
    At(String),
    /// Half-open: `start .. end`.
    Range {
        start: String,
        end: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Segment {
    pub name: String,
    pub index: Option<Index>,
}

/// `scene[scene].jog.channel`, `sensor[s].curve[p * 6 .. p * 6 + 6]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Path {
    pub segments: Vec<Segment>,
}

impl Path {
    pub fn parse(s: &str) -> Result<Self, String> {
        let mut segments = Vec::new();
        let mut rest = s.trim();
        loop {
            let end = rest
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
                .unwrap_or(rest.len());
            let name = &rest[..end];
            if !name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') {
                return Err(format!("`{s}`: expected a name at `{rest}`"));
            }
            rest = &rest[end..];
            let mut index = None;
            if let Some(after) = rest.strip_prefix('[') {
                let close = after
                    .find(']')
                    .ok_or_else(|| format!("`{s}`: unclosed `[`"))?;
                index = Some(parse_index(s, &after[..close])?);
                rest = &after[close + 1..];
            }
            segments.push(Segment {
                name: name.into(),
                index,
            });
            if rest.is_empty() {
                return Ok(Self { segments });
            }
            rest = rest
                .strip_prefix('.')
                .ok_or_else(|| format!("`{s}`: expected `.` or the end at `{rest}`"))?;
        }
    }

    /// Paths separated by whitespace outside brackets, as in `compare=`.
    pub fn parse_list(s: &str) -> Result<Vec<Self>, String> {
        let mut paths = Vec::new();
        let mut depth = 0usize;
        let mut start = None;
        for (i, c) in s.char_indices() {
            match c {
                '[' => depth += 1,
                ']' => depth = depth.saturating_sub(1),
                _ => {}
            }
            if c.is_whitespace() && depth == 0 {
                if let Some(from) = start.take() {
                    paths.push(Self::parse(&s[from..i])?);
                }
            } else if start.is_none() {
                start = Some(i);
            }
        }
        if let Some(from) = start {
            paths.push(Self::parse(&s[from..])?);
        }
        if paths.is_empty() {
            return Err("expected at least one path".into());
        }
        Ok(paths)
    }

    /// The block this path starts at.
    pub fn block(&self) -> &str {
        &self.segments[0].name
    }
}

fn parse_index(path: &str, inner: &str) -> Result<Index, String> {
    let inner = inner.trim();
    if let Some((start, end)) = inner.split_once("..") {
        let (start, end) = (start.trim(), end.trim());
        if end.starts_with('=') {
            return Err(format!(
                "`{path}`: a path range is half-open; write `a .. b`, not `a ..= b`"
            ));
        }
        if start.is_empty() || end.is_empty() {
            return Err(format!("`{path}`: a path range needs both ends (`a .. b`)"));
        }
        return Ok(Index::Range {
            start: start.into(),
            end: end.into(),
        });
    }
    if inner.is_empty() {
        return Err(format!("`{path}`: empty index"));
    }
    Ok(Index::At(inner.into()))
}

impl fmt::Display for Path {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, segment) in self.segments.iter().enumerate() {
            if i > 0 {
                f.write_str(".")?;
            }
            f.write_str(&segment.name)?;
            match &segment.index {
                None => {}
                Some(Index::At(at)) => write!(f, "[{at}]")?,
                Some(Index::Range { start, end }) => write!(f, "[{start} .. {end}]")?,
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_and_indexes() {
        let p = Path::parse("sensor[s].curve[p * 6 .. p * 6 + 6]").unwrap();
        assert_eq!(p.block(), "sensor");
        assert_eq!(p.segments[0].index, Some(Index::At("s".into())));
        assert_eq!(
            p.segments[1].index,
            Some(Index::Range {
                start: "p * 6".into(),
                end: "p * 6 + 6".into()
            })
        );
        assert_eq!(p.to_string(), "sensor[s].curve[p * 6 .. p * 6 + 6]");
        let p = Path::parse("scene[scene].jog.sign-cc").unwrap();
        assert_eq!(p.segments.len(), 3);
        assert_eq!(p.segments[2].name, "sign-cc");
    }

    #[test]
    fn lists_split_outside_brackets() {
        let list = Path::parse_list("sensor[s].meta  sensor[p * 6 .. 8].shaping").unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[1].to_string(), "sensor[p * 6 .. 8].shaping");
    }

    #[test]
    fn rejects_bad_paths() {
        assert!(Path::parse("curve[p * 6 ..]")
            .unwrap_err()
            .contains("both ends"));
        assert!(Path::parse("curve[0 ..= 5]")
            .unwrap_err()
            .contains("half-open"));
        assert!(Path::parse("scene[").is_err());
        assert!(Path::parse("scene..jog").is_err());
        assert!(Path::parse("scene.").is_err());
        assert!(Path::parse("1scene").is_err());
        assert!(Path::parse("scene[]").is_err());
        assert!(Path::parse_list(" ").is_err());
    }
}
