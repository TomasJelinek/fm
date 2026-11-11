//! Display-only subfolder hints for an exact flat listing; never tree authority.
use crate::{Cancellation, EntryId, TreeDiagnostic, TreeRootSnapshot};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct FolderPresenceRevision(pub u64);

#[derive(Clone, Debug)]
pub struct FolderPresenceRequest {
    pub root: TreeRootSnapshot,
    pub revision: FolderPresenceRevision,
    /// Only ordinary directory IDs in the current viewport, excluding parent rows.
    pub visible: Vec<EntryId>,
    pub cancel: Cancellation,
}
impl FolderPresenceRequest {
    pub const MAX_VISIBLE: usize = 256;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FolderPresence {
    Empty,
    HasFolders,
    Unavailable(TreeDiagnostic),
}

#[derive(Clone, Debug)]
pub struct FolderPresenceSnapshot {
    pub root: TreeRootSnapshot,
    pub revision: FolderPresenceRevision,
    pub values: Arc<[(EntryId, FolderPresence)]>,
}
