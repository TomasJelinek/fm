//! Native clipboard publication from exact inline-tree occurrences.
use crate::{
    Cancellation, FileClipboard, FileClipboardIntent, FsError, SelectionRevision, TreeProjection,
    TreeRequestId, TreeRowId, TreeSelectionSnapshot,
};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TreeClipboardScope {
    #[default]
    SelectionOrFallback,
    /// Preserve marks, but target only this occurrence if it is not selected.
    Dragged(TreeRowId),
}

#[derive(Clone, Debug)]
pub struct TreeClipboardRequest {
    pub scope: TreeClipboardScope,
    pub intent: FileClipboardIntent,
    pub view: Arc<TreeProjection>,
    pub request: TreeRequestId,
    pub selection_revision: SelectionRevision,
    pub selection: Arc<TreeSelectionSnapshot>,
    pub fallback: Option<TreeRowId>,
    pub cancel: Cancellation,
}

/// Retained provider roots, not paths reconstructed from display text.
/// Resolve and release final ownership on backend workers. The native clipboard
/// worker retains this source through publication and revalidates immediately
/// before writing. Native paste establishes its own live operation authority.
pub trait CapturedClipboardFiles: std::fmt::Debug + Send + Sync {
    /// Backend-only consuming release. Ok confirms cleanup responsibility acquired
    /// by this reference; another live owner never requires a global wait. An
    /// unknown/deferred implementation must retain or explicitly acknowledge its
    /// actual native cleanup rather than infer completion from wrapper Drop.
    fn release_on_worker(self: Arc<Self>) -> Result<(), FsError> {
        std::mem::forget(self);
        Err(crate::transfer::unconfirmed_transfer_release(
            "release captured clipboard files",
        ))
    }
    /// Cached operation intent; performs no provider or filesystem work.
    fn intent(&self) -> FileClipboardIntent;
    /// Cached completion identity for this zero-based root in the immutable order
    /// returned by `resolve`. Backend consumers use it for completion feedback;
    /// it grants no filesystem authority and performs no provider or filesystem work.
    fn tree_copy_occurrence(&self, _root: usize) -> Option<crate::TreeCopyOccurrence> {
        None
    }
    fn resolve(&self, cancel: &Cancellation) -> Result<FileClipboard, FsError>;
}
