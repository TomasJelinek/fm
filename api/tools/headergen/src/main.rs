//! Maintainer tool; plugin builds consume the checked-in generated C header.
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let api = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../midnight-plugin-api");
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| api.join("include/midnight_plugin_v1.h"));
    let config = cbindgen::Config::from_file(api.join("cbindgen.toml"))?;
    let bindings = cbindgen::Builder::new()
        .with_src(api.join("src/abi.rs"))
        .with_config(config)
        .generate()?;
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    bindings.write_to_file(output);
    Ok(())
}
