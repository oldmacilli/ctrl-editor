//! An editor crate in miniature: its own window around `MonitorPane`,
//! compiled here, with the glue expanded here (`monitor_glue!`).

slint::include_modules!();
ctrl_monitor_slint::monitor_glue!();
