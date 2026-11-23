//! The one-time notice for an installation that other accounts can change, and the backend
//! actions it offers. Plugins load from such an installation; the notice explains it once.
use std::path::PathBuf;

/// What other accounts can change in the running installation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InstallationStatus {
    /// The installation folder.
    pub folder: PathBuf,
    /// The first folder above it that other accounts can rename or replace.
    pub shared_ancestor: Option<PathBuf>,
    /// The folder itself and objects in it that other accounts can change (first few).
    pub shared: Vec<PathBuf>,
    /// All such objects in the folder, the folder included.
    pub shared_count: usize,
    /// Objects that refuse plugins outright, with the reason (first few).
    pub refused: Vec<String>,
}

impl InstallationStatus {
    pub fn is_private(&self) -> bool {
        self.shared_ancestor.is_none() && self.shared_count == 0 && self.refused.is_empty()
    }
}

/// A notice button's backend work; each runs on a backend worker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallationRequest {
    /// Make the installation folder private (it must be the user's own).
    Secure,
    /// Copy the installation into the user's profile; `replace` replaces an earlier copy.
    CopyToProfile { replace: bool },
    /// Remember that the notice was dismissed for this installation folder.
    Dismiss,
    /// Start the copy made by [`InstallationRequest::CopyToProfile`]; this session then closes.
    RestartFromCopy,
}

/// Backend answers for the installation notice.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallationEvent {
    /// Startup check: `Some` when other accounts can change the installation and the notice
    /// was not dismissed for it; `None` otherwise.
    Checked(Option<InstallationStatus>),
    Secured {
        status: InstallationStatus,
        /// Objects whose permissions could not be changed, with the reason.
        failures: Vec<String>,
    },
    SecureFailed(String),
    Copied {
        folder: PathBuf,
        files: u64,
        bytes: u64,
        status: InstallationStatus,
    },
    /// The profile already holds a copy; nothing was changed.
    CopyExists(PathBuf),
    CopyFailed(String),
    /// The copy has started; close this session.
    Restarting,
    RestartFailed(String),
}
