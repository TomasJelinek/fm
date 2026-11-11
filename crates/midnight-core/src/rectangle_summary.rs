//! Read-only summary of a transient rubber-band selection, without committing marks.
use crate::*;
use std::{collections::HashSet, sync::Arc};

#[derive(Clone, Debug)]
pub enum RectangleSelectionSeed {
    Native(Arc<SelectionSnapshot>),
    Archive {
        selected: Arc<HashSet<EntryId>>,
        full: Arc<DirectoryIndex>,
    },
    Tree(Arc<TreeSelectionSnapshot>),
}

#[derive(Clone, Debug)]
pub struct RectangleSummaryRequest {
    pub pane: PaneId,
    /// Allocation identity; a result cannot attach to a later gesture or range.
    pub token: Arc<()>,
    pub view: Arc<DirectoryIndex>,
    pub seed: RectangleSelectionSeed,
    pub ranges: SelectionRanges,
    pub mode: SelectionMode,
    pub cancel: Cancellation,
}
