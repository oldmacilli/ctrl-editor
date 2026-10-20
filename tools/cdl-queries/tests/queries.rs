//! The CDL tree-sitter queries compile against tree-sitter-kdl, capture what
//! they claim on the repository's descriptors, and name every CDL node.

use std::collections::BTreeSet;
use std::path::Path;

use tree_sitter::{Language, Parser, Query, QueryCursor, StreamingIterator};

const HIGHLIGHTS: &str = include_str!("../../../editors/tree-sitter/queries/cdl/highlights.scm");
const INJECTIONS: &str = include_str!("../../../editors/tree-sitter/queries/cdl/injections.scm");

fn language() -> Language {
    tree_sitter_kdl::LANGUAGE.into()
}

/// KDL's own queries first, then CDL's, as `; inherits: kdl` does.
fn query(kdl: &str, cdl: &str) -> Query {
    Query::new(&language(), &format!("{kdl}\n{cdl}"))
        .unwrap_or_else(|e| panic!("query does not compile: {e}"))
}

fn descriptor(rel: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../descriptors")
        .join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// (capture name, text) for every capture of `q` in `src`.
fn captures(q: &Query, src: &str) -> Vec<(String, String)> {
    let mut parser = Parser::new();
    parser.set_language(&language()).unwrap();
    let tree = parser.parse(src, None).unwrap();
    assert!(
        !tree.root_node().has_error(),
        "the KDL grammar rejects the file"
    );
    let mut cursor = QueryCursor::new();
    let mut out = Vec::new();
    let mut matches = cursor.matches(q, tree.root_node(), src.as_bytes());
    while let Some(m) = matches.next() {
        for c in m.captures() {
            let name = q.capture_names()[c.index as usize].to_string();
            let text = c.node.utf8_text(src.as_bytes()).unwrap().to_string();
            out.push((name, text));
        }
    }
    out
}

fn has(found: &[(String, String)], capture: &str, text: &str) -> bool {
    found.iter().any(|(c, t)| c == capture && t == text)
}

#[test]
fn highlights_mark_cdl_nodes_status_and_templates() {
    let q = query(tree_sitter_kdl::HIGHLIGHTS_QUERY, HIGHLIGHTS);
    let found = captures(&q, &descriptor("korg/nanokontrol-studio.ctrl.kdl"));
    assert!(has(&found, "keyword", "message"), "{found:?}");
    assert!(has(&found, "keyword", "procedure"));
    assert!(has(&found, "keyword.function", "exchange"));
    assert!(has(&found, "keyword.function", "gate"));
    assert!(has(&found, "tag.attribute", "field"));
    assert!(has(&found, "constant.builtin", "measured"));
    assert!(has(&found, "comment.warning", "inferred"));
    assert!(has(&found, "comment.warning", "unknown"));
    assert!(has(&found, "string.special", "01 05 00 40 {data:packed7}"));
    assert!(found
        .iter()
        .any(|(c, t)| c == "string.documentation" && t.contains("korg-sysex")));

    let tec = captures(&q, &descriptor("tecontrol/breath.ctrl.kdl"));
    assert!(has(&tec, "tag.attribute", "arg"));
    assert!(has(&tec, "string.special", "{0x10 + s}"));
    // Evidence text is not a template.
    assert!(!tec
        .iter()
        .any(|(c, t)| c == "string.special" && t.starts_with("TECcontrol findings")));

    let db1 = captures(&q, &descriptor("oceanbeach/db1.ctrl.kdl"));
    assert!(has(&db1, "string.special", "\"product == 0\""), "{db1:?}");
}

#[test]
fn the_queries_compile_quickly() {
    // A pattern that puts `node_field` under `node` takes minutes to
    // compile against tree-sitter-kdl, here and in every editor.
    for (name, cdl) in [("highlights", HIGHLIGHTS), ("injections", INJECTIONS)] {
        let t = std::time::Instant::now();
        Query::new(&language(), cdl).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(
            t.elapsed() < std::time::Duration::from_secs(2),
            "{name}: {:?}",
            t.elapsed()
        );
    }
}

#[test]
fn the_lbmt_source_is_injected() {
    let q = query(tree_sitter_kdl::INJECTIONS_QUERY, INJECTIONS);
    let found = captures(&q, &descriptor("oceanbeach/db1.ctrl.kdl"));
    let lbmt: Vec<_> = found
        .iter()
        .filter(|(c, _)| c == "injection.content")
        .map(|(_, t)| t)
        .collect();
    assert_eq!(lbmt.len(), 1, "{found:?}");
    assert!(lbmt[0].contains("@translator"), "{}", lbmt[0]);
    // The node's first argument, its name, is not LBMT.
    assert!(!lbmt[0].contains("organ-remap"));
    assert!((0..q.pattern_count())
        .flat_map(|i| q.property_settings(i).iter())
        .any(|p| &*p.key == "injection.language" && p.value.as_deref() == Some("lbmt")));
}

/// Node names from the converter's table: `message/reply` → message, reply;
/// `step send|exchange` → send, exchange; `block … field` → field.
fn schema_names() -> BTreeSet<&'static str> {
    let mut out = BTreeSet::new();
    for s in ctrl_cdl::schema::SCHEMA {
        let path = s.path.strip_prefix("step ").unwrap_or(s.path);
        let last = path.rsplit(['/', ' ']).next().unwrap();
        out.extend(last.split('|'));
    }
    out
}

#[test]
fn every_cdl_node_is_highlighted() {
    let missing: Vec<_> = schema_names()
        .into_iter()
        .filter(|name| !HIGHLIGHTS.contains(&format!("\"{name}\"")))
        .collect();
    assert!(missing.is_empty(), "not in highlights.scm: {missing:?}");
}

#[test]
fn every_descriptor_parses_with_the_kdl_grammar() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../descriptors");
    let mut n = 0;
    let mut parser = Parser::new();
    parser.set_language(&language()).unwrap();
    for vendor in std::fs::read_dir(dir).unwrap() {
        for file in std::fs::read_dir(vendor.unwrap().path()).unwrap() {
            let path = file.unwrap().path();
            if path.extension().is_some_and(|e| e == "kdl") {
                let src = std::fs::read_to_string(&path).unwrap();
                let tree = parser.parse(&src, None).unwrap();
                assert!(!tree.root_node().has_error(), "{}", path.display());
                n += 1;
            }
        }
    }
    assert!(n > 30, "{n} files");
}
