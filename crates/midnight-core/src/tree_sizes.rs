//! Exact occurrence directory-size requests and bounded backend publications.
use crate::{
    FolderSize, SelectedFolderSizes, SelectionRevision, TreeDiagnostic, TreeProjection,
    TreeRequestId, TreeRowId, TreeSelectionSnapshot, TreeViewStamp,
};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

#[derive(Clone, Copy, Debug)]
pub enum TreeSizeScope {
    /// Selected directories, or all eligible directory rows in the acknowledged
    /// projection when unselected, including expanded descendants outside the viewport.
    SelectionOrVisible,
    Single(TreeRowId),
}

#[derive(Clone, Debug)]
pub struct TreeSizeRequest {
    pub view: Arc<TreeProjection>,
    pub request: TreeRequestId,
    pub selection_revision: SelectionRevision,
    pub selection: Arc<TreeSelectionSnapshot>,
    pub scope: TreeSizeScope,
}

#[derive(Clone, Debug)]
pub enum TreeSizeCommand {
    Compute(TreeSizeRequest),
    Acknowledge {
        stamp: TreeViewStamp,
        version: u64,
    },
    Cancel {
        stamp: TreeViewStamp,
        request: TreeRequestId,
    },
}

#[derive(Clone, Debug)]
pub struct TreeSizeSnapshot {
    pub stamp: TreeViewStamp,
    pub version: u64,
    values: HashMap<TreeRowId, FolderSize>,
    running: HashSet<TreeRowId>,
}
impl TreeSizeSnapshot {
    /// Called on backend workers; the bounded maps contain occurrence identities only.
    pub fn new(
        stamp: TreeViewStamp,
        version: u64,
        values: HashMap<TreeRowId, FolderSize>,
        running: HashSet<TreeRowId>,
    ) -> Result<Self, &'static str> {
        if values.len() > TreeProjection::MAX_ROWS || running.len() > TreeProjection::MAX_ROWS {
            return Err("Tree size snapshot exceeds its row budget.");
        }
        Ok(Self {
            stamp,
            version,
            values,
            running,
        })
    }
    pub fn get(&self, row: TreeRowId) -> Option<&FolderSize> {
        self.values.get(&row)
    }
    pub fn running(&self, row: TreeRowId) -> bool {
        self.running.contains(&row)
    }
    pub fn len(&self) -> usize {
        self.values.len()
    }
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

#[derive(Clone, Debug)]
pub enum TreeSizeEvent {
    Snapshot {
        sizes: Arc<TreeSizeSnapshot>,
    },
    Selected {
        view: Arc<TreeProjection>,
        selection_revision: SelectionRevision,
        selection: Arc<TreeSelectionSnapshot>,
        version: u64,
        summary: SelectedFolderSizes,
    },
    Finished {
        stamp: TreeViewStamp,
        request: TreeRequestId,
        cancelled: bool,
    },
    Rejected {
        stamp: TreeViewStamp,
        request: TreeRequestId,
        error: TreeDiagnostic,
    },
}
