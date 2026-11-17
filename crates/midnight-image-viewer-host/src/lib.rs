//! Backend-only discovery and supervision of persistent restricted image-viewer processes.
#![forbid(unsafe_code)]

pub use midnight_image_viewer_api::{DocumentMetadata, Frame, Metadata, Viewport};
mod providers;
pub use providers::{Discovery, Provider, discover, discover_with_diagnostics};
mod raster_reply;
mod session;
pub use session::{ImageSession, Source};
mod startup;
pub use startup::{CleanupStatus, PreparedImageWorker, cleanup_status};

#[cfg(windows)]
#[path = "paths_windows.rs"]
mod paths;
#[cfg(target_os = "linux")]
#[path = "paths_linux.rs"]
mod paths;

/// Discovery validates a plugin installation and every ancestor of it. A fixture root
/// below a checkout whose ancestors other accounts may change (for example a secondary
/// drive with the default ACL) can never pass, so say how to provide a private root
/// instead of failing later inside discovery.
#[cfg(test)]
fn require_private_fixture_root(root: &std::path::Path) {
    if let Err(error) = midnight_plugin_host::validate_plugin_path(root) {
        panic!(
            "plugin fixture root {} is not private ({error}); set HOST_TEST_INSTALL_ROOT to a \
             directory that only your account, SYSTEM and Administrators (Linux: only your \
             account or root) can change, with private ancestors; see docs/developer/TEST_EXECUTION.md",
            root.display()
        );
    }
}
