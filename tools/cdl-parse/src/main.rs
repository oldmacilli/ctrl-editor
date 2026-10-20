//! Load and validate each CDL descriptor and run its test vectors. Prints a
//! summary, every finding as `file:line:col: severity: message` and each
//! vector's result; fails on errors and failed vectors.

use std::process::ExitCode;

fn main() -> ExitCode {
    let mut failed = false;
    for path in std::env::args().skip(1) {
        match ctrl_cdl::load(&path) {
            Ok(d) => {
                let report = ctrl_cdl::validate(&d);
                let verdict = if report.is_ok() { "ok  " } else { "FAIL" };
                failed |= !report.is_ok();
                println!(
                    "{verdict} {path}: {} ({} transports, {} blocks, {} messages, {} procedures, {} drivers)",
                    d.device.id,
                    d.transports.len(),
                    d.blocks.len(),
                    d.messages.len(),
                    d.procedures.len(),
                    d.drivers.len(),
                );
                for finding in &report.findings {
                    println!("  {finding}");
                }
                // Images resolve against the file that names them.
                let images = d
                    .device
                    .images
                    .iter()
                    .map(|i| (i.meta.pos, i.path.as_str()))
                    .chain(
                        d.panel
                            .iter()
                            .filter_map(|p| Some((p.meta.pos, p.image.as_deref()?))),
                    );
                for (pos, image) in images {
                    let declared = std::path::Path::new(&d.files[pos.file as usize]);
                    let file = declared
                        .parent()
                        .unwrap_or(std::path::Path::new(""))
                        .join(image);
                    if !file.is_file() {
                        failed = true;
                        println!(
                            "  {}: error: image {} not found",
                            d.error_at(pos, "").file,
                            file.display()
                        );
                    }
                }
                for v in ctrl_codec::Codec::new(&d).run_vectors() {
                    match v.outcome {
                        ctrl_codec::Outcome::Pass => println!("  vector {}: pass", v.name),
                        ctrl_codec::Outcome::Skipped(why) => {
                            println!("  vector {}: skipped ({why})", v.name)
                        }
                        ctrl_codec::Outcome::Fail(why) => {
                            failed = true;
                            println!("  vector {}: FAIL {why}", v.name);
                        }
                    }
                }
            }
            Err(errors) => {
                failed = true;
                eprintln!("FAIL {path}\n{errors}");
            }
        }
    }
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
