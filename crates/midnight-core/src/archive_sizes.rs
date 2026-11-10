//! Metadata-only archive sizes. Requests carry exact listing authority, never paths.
use crate::{
    EntryId, FolderSize, FolderSizeScanId, NavigationId, PaneId, SelectedFolderSizes,
    SelectionRevision, SelectionSizeId, TreeRootSnapshot, VirtualEntryId,
};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

#[derive(Clone, Debug)]
pub enum ArchiveSizeCommand {
    /// Acknowledge every received snapshot, including stale/retired mounts.
    /// Only the exact in-flight mount/version releases the next publication.
    Acknowledge {
        pane: PaneId,
        mount: u64,
        version: u64,
    },
    Compute {
        root: TreeRootSnapshot,
        scan: FolderSizeScanId,
        /// None means every immediate ordinary directory. Some(empty/files-only)
        /// means no directories; it must never fall back to all folders.
        selection: Option<Arc<HashSet<EntryId>>>,
    },
    Entry {
        root: TreeRootSnapshot,
        scan: FolderSizeScanId,
        entry: EntryId,
    },
    Observe {
        root: TreeRootSnapshot,
        request: SelectionSizeId,
        revision: SelectionRevision,
        selected: Arc<HashSet<EntryId>>,
    },
    Cancel {
        pane: PaneId,
        scan: FolderSizeScanId,
    },
}

/// Immutable backend-produced lookup. Replacing a large snapshot must transfer
/// its old Arc to backend reclamation, never walk/drop its map on the UI thread.
#[derive(Clone, Debug, Default)]
pub struct ArchiveSizeSnapshot {
    pub mount: u64,
    pub version: u64,
    values: HashMap<VirtualEntryId, FolderSize>,
}
impl ArchiveSizeSnapshot {
    pub fn new(mount: u64, version: u64, values: HashMap<VirtualEntryId, FolderSize>) -> Self {
        Self {
            mount,
            version,
            values,
        }
    }
    pub fn get(&self, id: VirtualEntryId) -> Option<&FolderSize> {
        self.values.get(&id)
    }
    pub fn len(&self) -> usize {
        self.values.len()
    }
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

#[derive(Clone, Debug)]
pub enum ArchiveSizeEvent {
    /// Retire only this mount's sizes and selected summary. Individual requests
    /// still finish by scan ID; a newer mount is unaffected.
    Invalidated { pane: PaneId, mount: u64 },
    /// Mount/entry identity survives a resort or navigation within that mount.
    /// Consumers accept only their current mount and increasing snapshot version.
    Snapshot {
        pane: PaneId,
        scan: Option<FolderSizeScanId>,
        sizes: Arc<ArchiveSizeSnapshot>,
    },
    Finished {
        pane: PaneId,
        scan: FolderSizeScanId,
        cancelled: bool,
    },
    Selected {
        request: SelectionSizeId,
        pane: PaneId,
        navigation: NavigationId,
        revision: SelectionRevision,
        version: u64,
        summary: SelectedFolderSizes,
    },
    /// Explicit rejection preserves normal interaction and never scans a fallback.
    Rejected {
        pane: PaneId,
        scan: Option<FolderSizeScanId>,
        message: String,
    },
}
