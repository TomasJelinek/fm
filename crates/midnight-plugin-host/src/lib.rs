//! Backend-only host for trusted native viewer plugins.
#![deny(unsafe_op_in_unsafe_fn)]

#[cfg(test)]
mod tests;
#[cfg(all(test, target_os = "linux"))]
mod tests_linux;
#[cfg(all(test, windows))]
mod tests_windows;

pub use midnight_plugin_api as api;
mod discovery;
pub use discovery::{Diagnostic, PluginInfo, host_target};
mod content;
mod text_detection;
pub use content::{CapturedFile, ContentResources, ContentSource};
#[cfg(windows)]
#[path = "installation_windows.rs"]
mod installation;
#[cfg(not(windows))]
mod installation {
    /// Executable-relative discovery. Linux reports the kernel-resolved path
    /// (`/proc/self/exe`), so no launch-spelling normalization is needed there.
    pub(crate) fn resolve() -> Result<std::path::PathBuf, crate::Error> {
        Ok(std::env::current_exe()?)
    }
}
/// Resolve the installation used by every plugin host on a backend worker.
///
/// Windows first uses a protected executable directory. Otherwise, a protected
/// pointer scoped to the launch path selects an explicitly deployed legacy
/// schema-1 registration. Default matching uses source executable length and
/// native LastWriteTime metadata; it does not establish executable byte equality.
/// `MIDNIGHT_VERIFY_RUNTIME_SHA256` absent or `0` leaves strict verification off;
/// `1` enables the previous strict executable hash and snapshot checks. Other
/// values are rejected. Using a deployment requires updating a missing or stale
/// pointer. A standalone executable without adjacent plugin directories remains
/// allowed. Malformed or unsafe registrations return an error.
/// Other platforms retain executable-relative discovery.
///
/// The result and environment choice are retained until restart. Package ACL,
/// ABI and loading checks still apply; resolving a location does not authorize
/// its contents.
pub fn installed_executable() -> Result<std::path::PathBuf, Error> {
    static INSTALLATION: std::sync::OnceLock<Result<std::path::PathBuf, Error>> =
        std::sync::OnceLock::new();
    INSTALLATION.get_or_init(installation::resolve).clone()
}
/// Shared installation trust check for host-owned executable plugin discovery.
pub fn validate_plugin_path(path: &std::path::Path) -> Result<(), Error> {
    trust::check(path)?;
    if path.canonicalize()? != path {
        return Err(Error(
            "Plugin path redirects outside its installation".into(),
        ));
    }
    Ok(())
}
/// Validate every local dependency of an installed executable package before launch.
/// No filename extension exemption is made for resources or dependent native libraries.
pub fn validate_plugin_package(directory: &std::path::Path) -> Result<(), Error> {
    package_trust::check(directory)
}
mod events;
pub use events::{EventKind, HostEvent};
mod session;
mod wake;
pub use session::{OPEN_REQUEST_ID, Session};
mod callbacks;
mod dispatch;
mod loaded;
#[cfg(windows)]
#[path = "native_windows.rs"]
mod native;
#[cfg(target_os = "linux")]
#[path = "native_linux.rs"]
mod native;
#[cfg(not(any(windows, target_os = "linux")))]
#[path = "native_unsupported.rs"]
mod native;
mod package_trust;
mod trusted_library;
pub use trusted_library::TrustedLibrary;
mod trusted_executable;
pub use trusted_executable::TrustedExecutablePin;
mod replies;
#[cfg(windows)]
#[path = "trust_windows.rs"]
mod trust;
#[cfg(target_os = "linux")]
#[path = "trust_linux.rs"]
mod trust;
#[cfg(not(any(windows, target_os = "linux")))]
mod trust {
    pub fn check(_: &std::path::Path) -> Result<(), crate::Error> {
        Err(crate::Error(
            "Native plugin permission validation unsupported on this platform".into(),
        ))
    }
}

use std::{
    path::Path,
    sync::{Arc, OnceLock, atomic::AtomicUsize},
};
static SESSIONS: OnceLock<Arc<AtomicUsize>> = OnceLock::new();

#[derive(Debug, Clone)]
pub struct Error(pub String);
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for Error {}
impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        Self(value.to_string())
    }
}

#[derive(Debug)]
pub struct Runtime {
    catalog: Vec<PluginInfo>,
    diagnostics: Vec<Diagnostic>,
    sessions: Arc<AtomicUsize>,
}
impl Runtime {
    pub fn discover(executable_directory: &Path) -> Result<Self, Error> {
        let (catalog, diagnostics) = discovery::discover(executable_directory)?;
        Ok(Self {
            catalog,
            diagnostics,
            sessions: SESSIONS
                .get_or_init(|| Arc::new(AtomicUsize::new(0)))
                .clone(),
        })
    }
    pub fn catalog(&self) -> &[PluginInfo] {
        &self.catalog
    }
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }
    pub fn is_loaded(&self, plugin_id: &str) -> bool {
        self.catalog
            .iter()
            .find(|info| info.id == plugin_id)
            .is_some_and(loaded::Loaded::is_loaded)
    }
    pub fn choose(&self, path: &Path, basic: bool) -> Option<(&PluginInfo, u16)> {
        self.choose_enabled(path, basic, &[])
    }
    pub fn choose_enabled(
        &self,
        path: &Path,
        basic: bool,
        disabled: &[String],
    ) -> Option<(&PluginInfo, u16)> {
        discovery::choose(&self.catalog, path, basic, disabled)
    }
    pub fn open(
        &self,
        plugin_id: &str,
        file: CapturedFile,
        generation: u64,
        mode: u16,
    ) -> Result<Session, Error> {
        let info = self
            .catalog
            .iter()
            .find(|info| info.id == plugin_id)
            .ok_or_else(|| Error("Viewer plugin is unavailable".into()))?;
        session::Session::open(info, file, generation, mode, self.sessions.clone())
    }
}
