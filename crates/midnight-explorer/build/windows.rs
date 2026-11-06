//! Windows linker configuration stays separate from application composition.
pub fn configure() {
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc") {
        panic!("The supported Windows build uses the MSVC toolchain.");
    }
    let manifest = std::path::PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR").expect("Cargo supplies the package directory"),
    )
    .join("build/windows/application.manifest");
    println!("cargo:rerun-if-changed={}", manifest.display());
    println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
    compile_icon();
}

fn compile_icon() {
    let package = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let assets = package.join("../../assets/branding");
    let resource = package.join("build/windows/application.rc");
    let output =
        std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("application.res");
    println!("cargo:rerun-if-changed={}", resource.display());
    println!(
        "cargo:rerun-if-changed={}",
        assets.join("midnight-explorer.ico").display()
    );
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap();
    let sdk = find_msvc_tools::find_windows_sdk(&arch)
        .expect("Install the Windows SDK with Visual Studio C++ Build Tools.");
    let compiler = sdk
        .path()
        .map(|directory| directory.join("rc.exe"))
        .find(|path| path.is_file())
        .expect("The Windows SDK must include its resource compiler (rc.exe).");
    let status = std::process::Command::new(compiler)
        .arg("/nologo")
        .arg("/fo")
        .arg(&output)
        .arg("/i")
        .arg(assets)
        .arg(resource)
        .status()
        .expect("Run the Windows resource compiler.");
    assert!(
        status.success(),
        "Compiling the application icon failed: {status}"
    );
    println!("cargo:rustc-link-arg-bin=me={}", output.display());
}
