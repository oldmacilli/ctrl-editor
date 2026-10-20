//! Load each CDL descriptor with `ctrl-cdl` and print a summary, or every
//! error as `file:line:col: message`.

use std::process::ExitCode;

fn main() -> ExitCode {
    let mut failed = false;
    for path in std::env::args().skip(1) {
        match ctrl_cdl::load(&path) {
            Ok(d) => println!(
                "ok   {path}: {} ({} transports, {} blocks, {} messages, {} procedures, {} drivers)",
                d.device.id,
                d.transports.len(),
                d.blocks.len(),
                d.messages.len(),
                d.procedures.len(),
                d.drivers.len(),
            ),
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
