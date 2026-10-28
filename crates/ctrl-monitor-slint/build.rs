fn main() {
    // For editors that embed the pane: DEP_CTRL_MONITOR_SLINT_UI.
    let dir = std::env::var("CARGO_MANIFEST_DIR").expect("set by cargo");
    println!("cargo:ui={dir}/ui");
    // Debug builds carry element ids, so the headless test can find
    // elements (i-slint-backend-testing's ElementHandle).
    let debug = std::env::var("PROFILE").is_ok_and(|p| p == "debug");
    let config = slint_build::CompilerConfiguration::new().with_debug_info(debug);
    slint_build::compile_with_config("ui/main.slint", config).expect("Slint build failed");
}
