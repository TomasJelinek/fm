fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows")
        || std::env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc")
    {
        return;
    }
    let manifest = std::path::PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR").expect("Cargo provides its manifest directory"),
    )
    .join("tests")
    .join("console.manifest");
    println!("cargo:rerun-if-changed={}", manifest.display());
    // Applies only to this package's linked test executables; library dependencies
    // do not propagate rustc-link-arg into the application's own manifest policy.
    println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
}
