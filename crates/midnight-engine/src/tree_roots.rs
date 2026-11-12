//! Backend-only ownership of the exact completed root listing and its source.
//!
//! UI-supplied display indexes, restored paths and equal-content allocations are
//! not source authority. Captures retain an existing archive mount across later
//! navigation; their final destruction belongs to a backend worker.
use crate::{
    SnapshotState,
    providers::{NativeProvider, OpenedLocation, ProviderError},
};
use midnight_core::{
    ArchiveListing, ArchiveLocation, Cancellation, NavigationId, PaneId, TreeRootSnapshot,
};
use std::{path::PathBuf, sync::Arc};

pub(crate) struct ReadyRoot {
    snapshot: TreeRootSnapshot,
    source: RootSource,
}

enum RootSource {
    Native(PathBuf),
    Archive(ArchiveLocation),
}

impl SnapshotState {
    /// Caller drops the retired root outside the snapshots lock on a blocking worker.
    pub(crate) fn remember_native_tree_root(
        &mut self,
        snapshot: TreeRootSnapshot,
        path: PathBuf,
    ) -> Option<Arc<ReadyRoot>> {
        let navigation = snapshot.navigation;
        let retired = self.tree_root.replace(Arc::new(ReadyRoot {
            snapshot,
            source: RootSource::Native(path),
        }));
        if let Some(notify) = &self.tree_notify {
            notify.notify(navigation);
        }
        retired
    }

    pub(crate) fn remember_archive_tree_root(
        &mut self,
        pane: PaneId,
        navigation: NavigationId,
        listing: &ArchiveListing,
    ) -> Option<Arc<ReadyRoot>> {
        let retired = self.tree_root.replace(Arc::new(ReadyRoot {
            snapshot: TreeRootSnapshot {
                pane,
                navigation,
                entries: listing.entries.clone(),
                index: listing.index.clone(),
            },
            source: RootSource::Archive(listing.location.clone()),
        }));
        if let Some(notify) = &self.tree_notify {
            notify.notify(navigation);
        }
        retired
    }

    pub(crate) fn capture_tree_root(
        &self,
        requested: &TreeRootSnapshot,
    ) -> Result<Arc<ReadyRoot>, ProviderError> {
        let ready = self
            .tree_root
            .as_ref()
            .ok_or(ProviderError::StaleReference)?;
        if self.pending.is_some()
            || self
                .network_navigation
                .is_some_and(|navigation| navigation >= requested.navigation)
            || !ready.snapshot.same_snapshot(requested)
        {
            return Err(ProviderError::StaleReference);
        }
        let current = match &ready.source {
            RootSource::Native(_) => self.has_native_view(),
            RootSource::Archive(_) => {
                self.archive.as_ref().is_some_and(|(navigation, entries)| {
                    *navigation == requested.navigation && Arc::ptr_eq(entries, &requested.entries)
                }) && self
                    .archive_navigation
                    .is_none_or(|navigation| navigation == requested.navigation)
            }
        };
        if !current {
            return Err(ProviderError::StaleReference);
        }
        Ok(ready.clone())
    }
}

/// Run after navigation has been cancelled; detached publishers must not refill
/// the cache. A bounded two-root drain also releases final provider handles away
/// from the async executor and outside the shared snapshots mutex.
pub(crate) async fn shutdown(snapshots: crate::Snapshots) {
    let _ = tokio::task::spawn_blocking(move || {
        let retired: [_; 2] = {
            let mut states = snapshots.lock().unwrap_or_else(|error| error.into_inner());
            std::array::from_fn(|pane| states[pane].tree_root.take())
        };
        drop(retired);
    })
    .await;
}

impl ReadyRoot {
    /// Backend-only archive source retained by an exact completed-listing capture.
    /// Callers still validate its live source before metadata reads/publication.
    pub(crate) fn archive(&self) -> Option<&ArchiveLocation> {
        match &self.source {
            RootSource::Archive(location) => Some(location),
            RootSource::Native(_) => None,
        }
    }

    pub(crate) fn snapshot(&self) -> &TreeRootSnapshot {
        &self.snapshot
    }

    /// Only a backend worker may create provider adapters or retain/drop mounts.
    pub(crate) fn open(
        &self,
        native: &NativeProvider,
        cancel: &Cancellation,
    ) -> Result<OpenedLocation, ProviderError> {
        crate::tree_actor::preflight_root(&self.snapshot, cancel)?;
        match &self.source {
            RootSource::Native(path) => native.capture_root(path, &self.snapshot.entries, cancel),
            RootSource::Archive(location) => {
                crate::providers::captured_archive_root(location, &self.snapshot.entries, cancel)
            }
        }
    }
}

#[cfg(test)]
#[path = "tree_roots_tests.rs"]
mod tests;
