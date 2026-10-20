//! The node table is complete, used, and printed into docs/cdl-schema.md.

use ctrl_cdl::schema::{markdown, SCHEMA};

const CONVERT: &str = include_str!("../src/convert.rs");

/// The paths the converter checks nodes against: `cx.node(n, "path")`.
fn used_paths() -> Vec<&'static str> {
    CONVERT
        .match_indices("cx.node(")
        .filter_map(|(i, _)| {
            let rest = &CONVERT[i..];
            let start = rest.find('"')? + 1;
            let end = start + rest[start..].find('"')?;
            Some(&rest[start..end])
        })
        .collect()
}

#[test]
fn every_entry_is_used_once_and_every_use_has_an_entry() {
    let used = used_paths();
    assert!(!used.is_empty());
    for path in &used {
        assert!(
            SCHEMA.iter().any(|s| s.path == *path),
            "no entry for `{path}`"
        );
    }
    for s in SCHEMA {
        let n = used.iter().filter(|p| **p == s.path).count();
        assert_eq!(n, 1, "`{}` is used {n} times by the converter", s.path);
        assert!(!s.doc.is_empty(), "`{}` has no doc", s.path);
        for p in s.props.unwrap_or(&[]) {
            assert!(
                !p.doc.is_empty(),
                "`{}` property `{}` has no doc",
                s.path,
                p.name
            );
        }
    }
}

#[test]
fn the_schema_document_is_current() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/cdl-schema.md");
    let on_disk = std::fs::read_to_string(path).unwrap_or_default();
    assert!(
        on_disk == markdown(),
        "docs/cdl-schema.md is stale: run `cargo run -p ctrl-cli -- schema > docs/cdl-schema.md`"
    );
}
