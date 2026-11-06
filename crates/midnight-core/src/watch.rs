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
    /// Watch immediate directory contents; implementations must not read file contents.
    /// The callback must stay short and bounded. Errors use the same rescan hint.
    fn watch_directory(
        &self,
        path: &Path,
        changed: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Box<dyn WatchSubscription>, FsError>;
}
