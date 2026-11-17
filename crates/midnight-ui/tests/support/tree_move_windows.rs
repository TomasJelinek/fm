//! Private inheritable ACL for this owned Windows fixture only.
//!
//! The result is the protected DACL `D:PAI(A;OICI;FA;;;<current user>)`, which the
//! Windows transfer recovery requires of a source parent. The System32 tools are run
//! by absolute path (never resolved through PATH), and the user SID is resolved once
//! per process: one `icacls` start costs tens of milliseconds, a PowerShell start
//! several hundred.
use std::{
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn system32(tool: &str) -> PathBuf {
    std::env::var_os("SystemRoot")
        .map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from)
        .join("System32")
        .join(tool)
}

/// The SID of the account running the tests, e.g. `S-1-5-21-…`.
fn current_user_sid() -> &'static str {
    static SID: OnceLock<String> = OnceLock::new();
    SID.get_or_init(|| {
        let output = Command::new(system32("whoami.exe"))
            .args(["/user", "/fo", "csv", "/nh"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "whoami: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        // One CSV row, "DOMAIN\user","S-1-…", independent of the display language.
        let row = String::from_utf8_lossy(&output.stdout);
        row.trim()
            .rsplit(',')
            .next()
            .map(|sid| sid.trim_matches('"').to_owned())
            .filter(|sid| sid.starts_with("S-1-"))
            .unwrap_or_else(|| panic!("no user SID in whoami output: {row}"))
    })
}

pub fn secure(path: &Path) {
    // /inheritance:r removes the inherited entries and protects the DACL; /grant:r
    // leaves the user's full control as the only entry, inherited by the contents.
    let output = Command::new(system32("icacls.exe"))
        .arg(path)
        .args(["/inheritance:r", "/grant:r"])
        .arg(format!("*{}:(OI)(CI)F", current_user_sid()))
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "private fixture ACL: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}
