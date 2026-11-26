#[cfg(windows)]
#[path = "build/windows.rs"]
mod windows;

fn main() {
    println!("cargo:rerun-if-changed=build/windows.rs");
    record_authorization_helper();
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        #[cfg(windows)]
        windows::configure();
        #[cfg(not(windows))]
        panic!("Build the supported Windows MSVC executable on a Windows host.");
    }
}

/// Record the SHA-256 of the authorization helper that this executable ships with, named by
/// `MIDNIGHT_ELEVATED_LISTING` (the built `midnight-elevated-listing` of the same build): only
/// a helper with exactly that content may then run with authorization from a folder the user
/// can change. Without it the recorded digest is empty: only an administrator-managed
/// installation offers authorization, and release packaging refuses the executable.
fn record_authorization_helper() {
    use sha2::{Digest, Sha256};
    println!("cargo:rerun-if-env-changed=MIDNIGHT_ELEVATED_LISTING");
    let Some(helper) = std::env::var_os("MIDNIGHT_ELEVATED_LISTING").filter(|v| !v.is_empty())
    else {
        println!("cargo:rustc-env=MIDNIGHT_ELEVATED_LISTING_SHA256=");
        return;
    };
    let helper = std::path::PathBuf::from(helper);
    println!("cargo:rerun-if-changed={}", helper.display());
    let bytes = std::fs::read(&helper)
        .unwrap_or_else(|error| panic!("MIDNIGHT_ELEVATED_LISTING {}: {error}", helper.display()));
    let digest: String = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    println!("cargo:rustc-env=MIDNIGHT_ELEVATED_LISTING_SHA256={digest}");
}
