//! Explicit Chromium broker profile. CEF isolates renderer/utility subprocesses; this
//! supervisor owns their process tree. This is deliberately not the decoder sandbox profile.
use std::{
    fmt,
    fs::File,
    io,
    path::{Path, PathBuf},
};

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

/// Chromium cannot establish its sandbox on this system (Linux: user namespaces denied
/// by AppArmor and no setuid-root `chrome-sandbox`). The message names the cause and the
/// one-time step that allows the sandbox; show it instead of a transport error. There is
/// no fallback without the sandbox.
#[derive(Debug)]
pub struct SandboxUnavailable {
    message: String,
    refusal: SandboxRefusal,
}
impl SandboxUnavailable {
    pub fn new(message: impl Into<String>, refusal: SandboxRefusal) -> Self {
        Self {
            message: message.into(),
            refusal,
        }
    }
    /// What the system refused, for a host that offers the one-time step itself.
    pub fn refusal(&self) -> &SandboxRefusal {
        &self.refusal
    }
}

/// Why the system refuses Chromium its sandbox. A typed reason, so that a host never
/// interprets the message text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SandboxRefusal {
    /// Linux: AppArmor denies unprivileged user namespaces to this browser executable,
    /// which runs without any profile (`kernel.apparmor_restrict_unprivileged_userns=1`).
    /// An AppArmor profile that grants `userns` to it allows the sandbox.
    UserNamespaces { executable: PathBuf },
    /// Linux: the kernel denies user namespaces to every program without administrator
    /// rights, because `setting` (such as `kernel.unprivileged_userns_clone`) is 0. No
    /// AppArmor profile allows them; only an administrator changing that setting, or a
    /// setuid-root `chrome-sandbox` in an installation that only root can change, does.
    NamespacesDisabled { setting: &'static str },
}
impl SandboxRefusal {
    /// The refusal as one sentence for the viewer, without the step that allows it.
    pub fn cause(&self) -> &'static str {
        match self {
            Self::UserNamespaces { .. } => {
                "The HTML viewer needs permission for Chromium's sandbox on this system \
                 (Ubuntu restricts unprivileged user namespaces)."
            }
            Self::NamespacesDisabled { .. } => {
                "The HTML viewer cannot use Chromium's sandbox on this system: it denies \
                 user namespaces to programs without administrator rights."
            }
        }
    }
}
impl fmt::Display for SandboxUnavailable {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}
impl std::error::Error for SandboxUnavailable {}

/// The browser could not start because the system lacks shared libraries that it needs
/// (Linux: the dynamic loader stopped it before its first message). The message names the
/// libraries and how to install them; show it instead of a transport error. Returned by
/// [`Process::startup_failure`].
#[derive(Debug)]
pub struct LibrariesMissing {
    libraries: Vec<String>,
    message: String,
}
impl LibrariesMissing {
    pub fn new(libraries: Vec<String>, message: impl Into<String>) -> Self {
        Self {
            libraries,
            message: message.into(),
        }
    }
    /// The missing libraries' names (sonames), in the order the loader needs them.
    pub fn libraries(&self) -> &[String] {
        &self.libraries
    }
}
impl fmt::Display for LibrariesMissing {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}
impl std::error::Error for LibrariesMissing {}

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
