//! Load and validate each CDL descriptor with `ctrl-cdl`. Prints a summary
//! and every finding as `file:line:col: severity: message`; fails on errors.

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
