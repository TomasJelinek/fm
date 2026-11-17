//! Windows linker configuration stays separate from application composition.
const MANIFEST_VERSION: &str = "@MIDNIGHT_VERSION_NUMBERS@";

pub fn configure() {
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc") {
        panic!("The supported Windows build uses the MSVC toolchain.");
    }
    let template = std::path::PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR").expect("Cargo supplies the package directory"),
    )
    .join("build/windows/application.manifest");
    println!("cargo:rerun-if-changed={}", template.display());
    let manifest = write_manifest(&template);
    println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
    compile_resources();
}

/// The assembly identity carries the same Cargo-derived version as the resources.
fn write_manifest(template: &std::path::Path) -> std::path::PathBuf {
    let text = std::fs::read_to_string(template).expect("Read the Windows manifest template.");
    assert_eq!(
        text.matches(MANIFEST_VERSION).count(),
        1,
        "The Windows manifest template must name its version as {MANIFEST_VERSION} once."
    );
    let [major, minor, patch] = version_numbers();
    let output =
        std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("application.manifest");
    std::fs::write(
        &output,
        text.replace(MANIFEST_VERSION, &format!("{major}.{minor}.{patch}.0")),
    )
    .expect("Write the generated Windows manifest.");
    output
}

fn version_numbers() -> [u16; 3] {
    [
        version_component("major", env!("CARGO_PKG_VERSION_MAJOR")),
        version_component("minor", env!("CARGO_PKG_VERSION_MINOR")),
        version_component("patch", env!("CARGO_PKG_VERSION_PATCH")),
    ]
}

fn compile_resources() {
    let package = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let assets = package.join("../../assets/branding");
    let resource = package.join("build/windows/application.rc");
    let output_dir = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let output = output_dir.join("application.res");
    write_version_header(&output_dir);
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
        .arg(&output_dir)
        .arg("/i")
        .arg(assets)
        .arg(resource)
        .status()
        .expect("Run the Windows resource compiler.");
    assert!(
        status.success(),
        "Compiling the application resources failed: {status}"
    );
    println!("cargo:rustc-link-arg-bin=me={}", output.display());
}

fn write_version_header(output_dir: &std::path::Path) {
    let [major, minor, patch] = version_numbers();
    let version = env!("CARGO_PKG_VERSION");
    let header = format!(
        "#define MIDNIGHT_VERSION_NUMBERS {major},{minor},{patch},0\n\
         #define MIDNIGHT_VERSION_STRING \"{version}\\0\"\n"
    );
    std::fs::write(output_dir.join("application-version.h"), header)
        .expect("Write the generated Windows version header.");
}

fn version_component(name: &str, value: &str) -> u16 {
    value.parse::<u16>().unwrap_or_else(|_| {
        panic!("Cargo package version {name} must fit a Windows u16 component: {value}")
    })
}
