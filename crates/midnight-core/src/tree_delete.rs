//! Immutable inline-tree Delete capture; retain and retire provider ownership on workers.
use crate::*;
use std::{path::PathBuf, sync::Arc};

/// Matches the common tree-action capture count bound.
pub const MAX_CAPTURED_DELETE_ROOTS: usize = 4096;

#[derive(Clone, Debug)]
pub struct TreeDeleteRequest {
    pub mode: DeleteMode,
    /// A context-menu click on an unmarked row targets only that occurrence.
    pub scope: TreeClipboardScope,
    pub view: Arc<TreeProjection>,
    pub request: TreeRequestId,
    pub selection_revision: SelectionRevision,
    pub selection: Arc<TreeSelectionSnapshot>,
    pub fallback: Option<TreeRowId>,
    pub cancel: Cancellation,
}

/// Backend-owned native occurrences with overlaps removed by captured row ancestry.
/// Accessors are cached and perform no provider work. Preparation pins all native
/// identities before confirmation; capture itself is not a filesystem snapshot.
/// Call prepare and destroy the final owner only on backend workers. Prepared
/// targets must be destroyed before the source lease that supplied them.
pub trait CapturedDeleteSources: std::fmt::Debug + Send + Sync {
    fn stamp(&self) -> TreeViewStamp;
    fn request(&self) -> TreeRequestId;
    fn mode(&self) -> DeleteMode;
    fn root_count(&self) -> usize;
    fn prepare(
        &self,
        service: &dyn MutationService,
        cancel: &Cancellation,
    ) -> Result<Vec<Arc<dyn PreparedMutation>>, FsError>;
}

/// Retire successful captures on backend workers when dismissing or rejecting them.
#[derive(Clone, Debug)]
pub struct TreeDeleteCapture {
    pub mode: DeleteMode,
    pub root_count: usize,
    pub sources: Arc<dyn CapturedDeleteSources>,
}

/// Enters the ordinary bounded preparation/confirmation queue. Mode comes from
/// the opaque capture and cannot be changed by confirmation. source_directory
/// supplies display/refresh context only; it is never native mutation authority.
#[derive(Clone, Debug)]
pub struct PrepareCapturedDelete {
    pub id: OperationId,
    pub sources: Arc<dyn CapturedDeleteSources>,
    pub source_directory: PathBuf,
}
