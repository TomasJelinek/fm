//! Worker-resolved Custom Command arguments from acknowledged tree occurrences.
use crate::{
    Cancellation, Entry, EntryId, FsError, SelectionRevision, TreeProjection, TreeRequestId,
    TreeRowId, TreeSelectionSnapshot, TreeViewStamp,
};
use std::{path::PathBuf, sync::Arc};

#[derive(Clone, Debug)]
pub struct TreeUserCommandRequest {
    pub view: Arc<TreeProjection>,
    pub request: TreeRequestId,
    pub selection_revision: SelectionRevision,
    pub selection: Arc<TreeSelectionSnapshot>,
    /// Independent of marks: Once commands retain the actual focused occurrence.
    pub focus: Option<TreeRowId>,
    pub cancel: Cancellation,
}

/// Owned native values resolved on a worker. An empty selection remains empty;
/// command expansion applies its existing focused fallback only when required.
#[derive(Debug)]
pub struct ResolvedUserCommandSources {
    pub directory: PathBuf,
    pub selected: Arc<[Entry]>,
    pub focus: Option<Entry>,
}

/// Retain through native preparation and process launch; destroy on a worker.
/// Captures survive panel navigation but do not promise a filesystem snapshot.
pub trait CapturedUserCommandSources: std::fmt::Debug + Send + Sync {
    fn stamp(&self) -> TreeViewStamp;
    fn request(&self) -> TreeRequestId;
    fn resolve(&self, cancel: &Cancellation) -> Result<ResolvedUserCommandSources, FsError>;
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError>;
    /// Revalidate the root and only the immutable entries used by one process.
    /// Unknown entry IDs must fail. The default preserves full validation.
    fn revalidate_for(&self, entries: &[EntryId], cancel: &Cancellation) -> Result<(), FsError> {
        let _ = entries;
        self.revalidate(cancel)
    }
}
