//! Preserve the exact CEF distribution's sandbox-sensitive delayed import list.
fn main() {
    println!("cargo:rerun-if-env-changed=CEF_PATH");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows")
        || std::env::var_os("CARGO_FEATURE_NATIVE_CEF").is_none()
    {
        return;
    }
    let path = std::env::var_os("CEF_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| panic!("CEF_PATH must select the verified pinned native SDK"));
    let file = path.join("cmake/cef_variables.cmake");
    println!("cargo:rerun-if-changed={}", file.display());
    let contents = std::fs::read_to_string(file)
        .unwrap_or_else(|e| panic!("CEF delay-load policy unavailable: {e}"));
    let mut inside = false;
    let mut count = 0;
    for line in contents.lines() {
        let line = line.trim();
        if line == "set(CEF_DELAYLOAD_FLAGS" {
            inside = true;
            continue;
        }
        if inside && line == ")" {
            break;
        }
        if inside && line.starts_with("/DELAYLOAD:") {
            let flag = line.split_whitespace().next().unwrap_or("");
            println!("cargo:rustc-cdylib-link-arg={flag}");
            count += 1;
        }
    }
    assert!(count > 10, "incomplete pinned CEF delay-load policy");
    println!("cargo:rustc-cdylib-link-arg=/DEFAULTLIB:delayimp.lib");
    // CEF deliberately lists DLLs unused by some clients; MSVC documents those as harmless.
    println!("cargo:rustc-cdylib-link-arg=/IGNORE:4199");
}
