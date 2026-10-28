fn main() {
    // What an editor's build.rs does to import "@ctrl-monitor/monitor.slint".
    let ui =
        std::env::var("DEP_CTRL_MONITOR_SLINT_UI").expect("ctrl-monitor-slint's links metadata");
    let config = slint_build::CompilerConfiguration::new()
        .with_library_paths([("ctrl-monitor".to_string(), ui.into())].into())
        .with_debug_info(true);
    slint_build::compile_with_config("ui/app.slint", config).expect("Slint build failed");
}
