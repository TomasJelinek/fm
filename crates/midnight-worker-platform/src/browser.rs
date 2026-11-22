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
    spawn_with_extensions(executable, Extensions::default())
}

/// Launch a host that accepts the negotiated physical-v1 web shortcut event extension.
/// The fixed capability is added explicitly; ambient process values are never forwarded.
pub fn spawn_with_host_shortcuts(executable: &Path) -> io::Result<Browser> {
    spawn_with_extensions(
        executable,
        Extensions {
            host_shortcuts: true,
            ..Extensions::default()
        },
    )
}

/// Launch a browser for a host that accepts the given negotiated web-viewer API
/// extensions. Each adds its one fixed value to the cleared environment; ambient
/// process values are never forwarded.
pub fn spawn_with_extensions(executable: &Path, extensions: Extensions) -> io::Result<Browser> {
    if !executable.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Browser path must be absolute",
        ));
    }
    let profile = profile::Profile::new(&native::profile_parent()?)?;
    native::spawn(executable, profile, extensions)
}

/// Public web-viewer API launch contract; kept independent of the protocol crate.
pub const HOST_SHORTCUTS_ENV: &str = "MIDNIGHT_WEB_VIEWER_HOST_SHORTCUTS";
/// Exact capability value accepted by web-viewer API 0.2 providers.
pub const HOST_SHORTCUTS_PHYSICAL_V1: &str = "physical-v1";
/// Public web-viewer API launch contract of host link routing (API 0.3).
pub const LINKS_ENV: &str = "MIDNIGHT_WEB_VIEWER_LINKS";
/// Exact capability value accepted by web-viewer API 0.3 providers.
pub const LINKS_HOST_V1: &str = "host-v1";

/// The web-viewer API extensions a launching host accepts. A provider uses an extension
/// only when its exact value is in the environment the host launches it with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Extensions {
    /// Physical Backspace/Alt+Left/Alt+Right shortcuts (API 0.2): [`HOST_SHORTCUTS_ENV`].
    pub host_shortcuts: bool,
    /// Link hover and activation events instead of in-browser navigation (API 0.3):
    /// [`LINKS_ENV`].
    pub links: bool,
}

impl Extensions {
    /// The fixed value of each accepted extension, keyed by the environment variable that
    /// carries it.
    pub fn environment(self) -> impl Iterator<Item = (&'static str, &'static str)> {
        [
            (
                self.host_shortcuts,
                HOST_SHORTCUTS_ENV,
                HOST_SHORTCUTS_PHYSICAL_V1,
            ),
            (self.links, LINKS_ENV, LINKS_HOST_V1),
        ]
        .into_iter()
        .filter_map(|(accepted, key, value)| accepted.then_some((key, value)))
    }
    /// The fixed value of `key`, when it names an accepted extension.
    #[cfg(any(windows, test))]
    fn value(self, key: &str) -> Option<&'static str> {
        self.environment()
            .find(|(name, _)| *name == key)
            .map(|(_, value)| value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_accepted_extension_adds_exactly_its_fixed_launch_value() {
        let none = Extensions::default();
        assert_eq!(none.environment().count(), 0);
        let shortcuts = Extensions {
            host_shortcuts: true,
            links: false,
        };
        assert_eq!(
            shortcuts.environment().collect::<Vec<_>>(),
            [(HOST_SHORTCUTS_ENV, HOST_SHORTCUTS_PHYSICAL_V1)]
        );
        let links = Extensions {
            host_shortcuts: false,
            links: true,
        };
        assert_eq!(
            links.environment().collect::<Vec<_>>(),
            [(LINKS_ENV, LINKS_HOST_V1)]
        );
        let both = Extensions {
            host_shortcuts: true,
            links: true,
        };
        assert_eq!(
            both.environment().collect::<Vec<_>>(),
            [
                (HOST_SHORTCUTS_ENV, HOST_SHORTCUTS_PHYSICAL_V1),
                (LINKS_ENV, LINKS_HOST_V1)
            ]
        );
        assert_eq!(both.value(LINKS_ENV), Some("host-v1"));
        assert_eq!(shortcuts.value(LINKS_ENV), None);
        assert_eq!(both.value("MIDNIGHT_WEB_VIEWER_PROFILE"), None);
        assert_eq!(
            (LINKS_ENV, LINKS_HOST_V1),
            ("MIDNIGHT_WEB_VIEWER_LINKS", "host-v1")
        );
    }
}
