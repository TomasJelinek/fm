//! Immutable inputs and worker-built views for search results fed to a panel.
use crate::{
    Cancellation, DirectoryIndex, Entry, EntryId, FindSortSpec, FsError, MaskSelection, PaneId,
    PanelFilter,
};
use std::{collections::HashMap, path::PathBuf, sync::Arc};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FindFeedOptions {
    pub sort: FindSortSpec,
    pub filter: PanelFilter,
    pub show_hidden: bool,
}

/// Two coalescing backend slots (one per pane) project at most the Find result
/// budget. All arrays are immutable snapshots; matching, indexing and sorting
/// execute on the worker. Row identities and paths remain unchanged.
#[derive(Clone, Debug)]
pub struct FindFeedProjectionRequest {
    pub id: u64,
    pub pane: PaneId,
    pub rows_serial: u64,
    pub selection_revision: u64,
    pub entries: Arc<[Entry]>,
    pub visible: Arc<[Entry]>,
    pub selected: Arc<Vec<bool>>,
    pub cursor: Option<EntryId>,
    pub options: FindFeedOptions,
    /// Applies only to members of `visible`; never to filtered-out source rows.
    pub mask: Option<Arc<MaskSelection>>,
    /// At most 64 completed mutation batches (100,000 paths in total); a removed directory also removes
    /// descendants. The worker returns an original source with these paths pruned.
    pub removed: Arc<[Arc<[PathBuf]>]>,
    /// Calculated totals that a Size sort orders folder rows by
    /// (`FolderSize::sort_bytes`), by row identity; empty for other columns.
    pub folder_sizes: Arc<HashMap<EntryId, u64>>,
    pub cancel: Cancellation,
}

#[derive(Clone, Debug)]
pub struct FindFeedProjection {
    pub all_entries: Arc<[Entry]>,
    pub entries: Arc<[Entry]>,
    pub index: Arc<DirectoryIndex>,
    pub selected: Arc<Vec<bool>>,
    pub selected_count: usize,
    pub selected_bytes: u64,
    pub cursor: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct FindFeedProjectionStamp {
    pub id: u64,
    pub pane: PaneId,
    pub rows_serial: u64,
    pub selection_revision: u64,
}

#[derive(Clone, Debug)]
pub struct FindFeedProjectionResult {
    pub request: FindFeedProjectionStamp,
    pub result: Result<FindFeedProjection, FsError>,
}
