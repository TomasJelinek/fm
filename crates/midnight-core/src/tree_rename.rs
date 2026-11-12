//! Exact focused native Rename authority for an inline-tree occurrence.
use crate::{
    Cancellation, FsError, MutationService, PreparedMutation, TreeProjection, TreeRequestId,
    TreeRowId, TreeViewStamp,
};
use std::{path::PathBuf, sync::Arc};

/// F2 captures focus independently of marked selection. Only the exact acknowledged
/// projection and a selectable native occurrence may enter the Rename workflow.
#[derive(Clone, Debug)]
pub struct TreeRenameRequest {
    pub view: Arc<TreeProjection>,
    pub request: TreeRequestId,
    pub row: TreeRowId,
    pub cancel: Cancellation,
}

/// Backend-owned provider authority for one native file, directory or link. Cached
/// accessors do no I/O. Preparation belongs to backend workers. The final owner safely
/// retires native/provider resources independently of its caller or runtime; returned
/// native pins must close before this retained source is released.
pub trait CapturedRenameTarget: std::fmt::Debug + Send + Sync {
    fn stamp(&self) -> TreeViewStamp;
    fn request(&self) -> TreeRequestId;
    fn row(&self) -> TreeRowId;
    fn prepare(
        &self,
        service: &dyn MutationService,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn PreparedMutation>, FsError>;
}

/// Uses the ordinary bounded preparation and explicit confirmation flow. The
/// source_directory is display/refresh context only, never mutation authority.
#[derive(Clone, Debug)]
pub struct PrepareCapturedRename {
    pub id: crate::OperationId,
    pub source: Arc<dyn CapturedRenameTarget>,
    pub source_directory: PathBuf,
}
