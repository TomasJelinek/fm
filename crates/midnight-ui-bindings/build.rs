#[path = "build/tooltip.rs"]
mod tooltip;

fn main() {
    let manifest = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let input = manifest.join("../midnight-ui/ui/main.slint");
    // Keep Slint's normal resource, imported-file and environment tracking.
    slint_build::compile_with_config(
        &input,
        slint_build::CompilerConfiguration::new().with_style("fluent".into()),
    )
    .unwrap();
    let output = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("main.rs");
    tooltip::compile(&input, &output).unwrap();
}
