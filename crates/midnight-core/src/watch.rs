//! Native event sources report hints; backend debounce and polling establish refresh policy.

use crate::{Entry, FsError, NavigationId, PaneId};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

/// UI-accepted immutable listing, shared without copying the directory's rows.
#[derive(Clone, Debug)]
pub struct VisibleDirectory {
    pub pane: PaneId,
    pub navigation: NavigationId,
    pub path: PathBuf,
    pub entries: Arc<[Entry]>,
}

/// Dropping a subscription may stop/join native workers; do so on a backend worker.
pub trait WatchSubscription: Send + 'static {}

/// An event/overflow/error is a hint to rescan, never an authoritative mutation.
pub trait WatchService: Send + Sync + 'static {
    /// Watch 1..=32 unique native directories with one subscription.
    /// Bit `i` of each callback mask identifies `paths[i]`; errors, overflow and
    /// unrecognized event paths set all active bits. Ordinary access is ignored.
    /// Registration and subscription destruction belong on backend workers;
    /// callbacks must stay short and bounded. Unsupported services may poll instead.
    fn watch_directories(
        &self,
        paths: &[PathBuf],
        _changed: Arc<dyn Fn(u32) + Send + Sync>,
    ) -> Result<Box<dyn WatchSubscription>, FsError> {
        Err(FsError::from_io(
            "watch directories",
            paths.first().map_or(Path::new(""), PathBuf::as_path),
            std::io::ErrorKind::Unsupported.into(),
        ))
    }

    /// Watch immediate directory contents; implementations must not read file contents.
    /// The callback must stay short and bounded. Errors use the same rescan hint.
    fn watch_directory(
        &self,
        path: &Path,
        changed: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Box<dyn WatchSubscription>, FsError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    struct SingleDirectoryOnly;

    impl WatchService for SingleDirectoryOnly {
        fn watch_directory(
            &self,
            path: &Path,
            _changed: Arc<dyn Fn() + Send + Sync>,
        ) -> Result<Box<dyn WatchSubscription>, FsError> {
            Err(FsError::from_io(
                "watch directory",
                path,
                io::ErrorKind::Unsupported.into(),
            ))
        }
    }

    #[test]
    fn batch_defaults_to_unsupported_without_breaking_single_directory_implementors() {
        let result = SingleDirectoryOnly.watch_directories(
            &[PathBuf::from("directory")],
            Arc::new(|_| panic!("unsupported watch invoked callback")),
        );
        assert!(matches!(result, Err(error) if error.kind == io::ErrorKind::Unsupported));
    }
}
