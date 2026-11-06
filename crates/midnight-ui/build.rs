#[path = "build/tooltip.rs"]
mod tooltip;

fn main() {
    // Keep Slint's normal resource, imported-file and environment tracking.
    slint_build::compile_with_config(
        "ui/main.slint",
        slint_build::CompilerConfiguration::new().with_style("fluent".into()),
    )
    .unwrap();
    let manifest = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let output = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("main.rs");
    tooltip::compile(&manifest.join("ui/main.slint"), &output).unwrap();
}
