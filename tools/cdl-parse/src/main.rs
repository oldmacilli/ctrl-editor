//! Parse each CDL file as KDL 2.0 and print its top-level nodes.

use std::process::ExitCode;

fn main() -> ExitCode {
    let mut failed = false;
    for path in std::env::args().skip(1) {
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) => {
                eprintln!("{path}: {error}");
                failed = true;
                continue;
            }
        };
        match text.parse::<kdl::KdlDocument>() {
            Ok(doc) => {
                let names: Vec<_> = doc.nodes().iter().map(|n| n.name().value()).collect();
                println!("ok   {path}: {} nodes ({})", names.len(), names.join(" "));
            }
            Err(error) => {
                failed = true;
                eprintln!("FAIL {path}: {error:?}");
            }
        }
    }
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
