#[path = "build/arms.rs"]
mod arms;
#[path = "build/partition.rs"]
mod partition;
#[path = "build/source.rs"]
mod source;
#[path = "build/tooltip.rs"]
mod tooltip;

/// Environment that changes Slint's compilation, as tracked by `slint_build`.
const SLINT_ENVIRONMENT: [&str; 9] = [
    "SLINT_STYLE",
    "SLINT_FONT_SIZES",
    "SLINT_SCALE_FACTOR",
    "SLINT_ASSET_SECTION",
    "SLINT_EMBED_RESOURCES",
    "SLINT_EMIT_DEBUG_INFO",
    "SLINT_LIVE_PREVIEW",
    "SLINT_BUNDLE_TRANSLATIONS",
    "SLINT_COMPILER_DENY_WARNINGS",
];

fn main() {
    let manifest = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let input = manifest.join("../midnight-ui/ui/main.slint");
    let output = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("main.rs");
    // The UI is compiled once: the tooltip adapter emits slint_build's dependency,
    // environment and include tracking itself instead of a second full compile.
    // Slint diagnostics are already printed with their snippets; panic with the plain
    // messages instead of a Debug-escaped single line.
    let dependencies = tooltip::compile(&input, &output).unwrap_or_else(|error| panic!("{error}"));
    for dependency in dependencies {
        println!("cargo:rerun-if-changed={}", dependency.display());
    }
    for variable in SLINT_ENVIRONMENT {
        println!("cargo:rerun-if-env-changed={variable}");
    }
    println!(
        "cargo:rustc-env=SLINT_INCLUDE_GENERATED={}",
        output.display()
    );
}
