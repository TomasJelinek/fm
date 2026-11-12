//! Small, revision-scoped selected-folder totals prepared by backend workers.
use crate::{NavigationId, PaneId, SelectionRevision, SelectionSnapshot};
use std::{path::PathBuf, sync::Arc};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct SelectionSizeId(pub u64);

#[derive(Clone, Debug)]
pub struct SelectionSizeRequest {
    pub id: SelectionSizeId,
    pub pane: PaneId,
    pub navigation: NavigationId,
    /// The displayed native cache can predate a failed navigation attempt.
    pub source_navigation: NavigationId,
    pub revision: SelectionRevision,
    pub location: PathBuf,
    pub selection: Arc<SelectionSnapshot>,
}

/// Completed, error-free bytes remain separate from unfinished/error estimates.
/// A missing byte total means the exact sum exceeds u64, never a wrapped value.
/// Unknown folders are `folders - complete - partial`, including skipped links.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SelectedFolderSizes {
    pub folders: usize,
    pub complete: usize,
    pub complete_bytes: Option<u64>,
    pub partial: usize,
    pub partial_bytes: Option<u64>,
}
