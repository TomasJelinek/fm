#[path = "../midnight-ui-bindings/build/tooltip.rs"]
mod tooltip;

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
    let input = manifest.join("ui/comparator.slint");
    let output =
        std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("comparator.rs");
    // Reuse the pinned structural adapter and compile once, with the same Cargo
    // input/resource/environment tracking as the main UI's generated bindings.
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
