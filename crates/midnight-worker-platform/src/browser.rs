//! Explicit Chromium broker profile. CEF isolates renderer/utility subprocesses; this
//! supervisor owns their process tree. This is deliberately not the decoder sandbox profile.
use std::{fs::File, io, path::Path};

#[cfg(windows)]
#[path = "browser_windows.rs"]
mod native;
#[cfg(target_os = "linux")]
#[path = "browser_linux.rs"]
mod native;
pub use native::Process;
#[path = "browser_profile.rs"]
mod profile;

pub struct Browser {
    pub input: File,
    pub output: File,
    pub process: Process,
    // Retained until every Chromium process is reaped, then deleted on the backend.
    _profile: profile::Profile,
}

/// Only a host-validated, private installed CEF bootstrap may be launched here.
/// The plugin must use Chromium's sandbox; this profile does not grant renderer authority.
pub fn spawn(executable: &Path) -> io::Result<Browser> {
    spawn_configured(executable, false)
}

/// Launch a host that accepts the negotiated physical-v1 web shortcut event extension.
/// The fixed capability is added explicitly; ambient process values are never forwarded.
pub fn spawn_with_host_shortcuts(executable: &Path) -> io::Result<Browser> {
    spawn_configured(executable, true)
}

/// Public web-viewer API launch contract; kept independent of the protocol crate.
pub const HOST_SHORTCUTS_ENV: &str = "MIDNIGHT_WEB_VIEWER_HOST_SHORTCUTS";
/// Exact capability value accepted by web-viewer API 0.2 providers.
pub const HOST_SHORTCUTS_PHYSICAL_V1: &str = "physical-v1";

fn spawn_configured(executable: &Path, host_shortcuts: bool) -> io::Result<Browser> {
    if !executable.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Browser path must be absolute",
        ));
    }
    let profile = profile::Profile::new(&native::profile_parent()?)?;
    native::spawn(executable, profile, host_shortcuts)
}
