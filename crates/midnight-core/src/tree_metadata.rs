//! Exact native metadata authority captured from acknowledged inline-tree occurrences.
use crate::{
    Cancellation, FsError, MutationService, OperationId, PreparedMutation, SelectionRevision,
    TreeProjection, TreeRequestId, TreeRowId, TreeSelectionSnapshot, TreeViewStamp,
};
use std::{path::PathBuf, sync::Arc};

#[derive(Clone, Debug)]
pub struct TreeMetadataRequest {
    pub view: Arc<TreeProjection>,
    pub request: TreeRequestId,
    pub selection_revision: SelectionRevision,
    pub selection: Arc<TreeSelectionSnapshot>,
    pub fallback: Option<TreeRowId>,
    pub cancel: Cancellation,
}

/// Native roots in deepest-first occurrence order, retained with their provider registrations.
/// Accessors are cached. Preparation and final native/provider cleanup run on backend workers.
/// Every root remains an independent nonrecursive target. For recursive edits, covered roots
/// are reached through their selected ancestor and must not execute independently a second time.
pub trait CapturedMetadataSources: std::fmt::Debug + Send + Sync {
    fn stamp(&self) -> TreeViewStamp;
    fn request(&self) -> TreeRequestId;
    fn root_count(&self) -> usize;
    fn covered_roots(&self) -> &[bool];
    fn prepare(
        &self,
        service: &dyn MutationService,
        cancel: &Cancellation,
    ) -> Result<Vec<Arc<dyn PreparedMutation>>, FsError>;
}

/// Joins the existing bounded metadata preparation/preview/versioned-confirmation pipeline.
/// source_directory provides presentation/refresh context only, never mutation authority.
#[derive(Clone, Debug)]
pub struct PrepareCapturedMetadata {
    pub id: OperationId,
    pub sources: Arc<dyn CapturedMetadataSources>,
    pub source_directory: PathBuf,
}
