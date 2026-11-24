//! The restricted workers' read and execute grant on the installed files they load.
//!
//! Windows checks the workers' AppContainer's access to every library their loader opens.
//! The build and deployment scripts grant it on the shared FFmpeg libraries and the playback
//! helpers, but a release ZIP carries no permissions: after extraction the files inherit only
//! their folder's, which grant AppContainers nothing in a user's profile. The hosts therefore
//! add the grant before they start a worker, the way the scripts do, where the installation
//! is private. A shared installation is left as it is and reported as before.
use std::{
    io,
    path::{Path, PathBuf},
};

/// Before starting the restricted worker `executable`, let the workers load `files`: the
/// installed files its package needs them to open (the libraries beside it, for the video
/// playback helpers also the helper itself). A file the workers may not load yet gets one
/// explicit read and execute entry for their AppContainer
/// ([`midnight_worker_platform::grant_worker_load`]), only when the file and every folder
/// above it are private by the plugin trust rules: no other account can change, rename or
/// replace them. Nothing else changes, and nothing changes in a shared installation.
/// Outcomes are kept for the run ([`midnight_worker_platform::prepare_worker`]); the first
/// failure is returned, and the worker may still be started, as its failure is diagnosed.
/// Call on a backend worker.
pub fn prepare_restricted_worker(
    executable: &Path,
    files: impl FnOnce() -> Vec<PathBuf>,
) -> io::Result<()> {
    midnight_worker_platform::prepare_worker(executable, files, private)
}

/// Whether the plugin trust rules let the application change `path`'s permissions.
fn private(path: &Path) -> bool {
    #[cfg(windows)]
    {
        crate::validate_private_path(path).is_ok()
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        false
    }
}
