//! Immutable native checksum inputs from acknowledged inline-tree occurrences.
use crate::{
    Cancellation, ChecksumAlgorithms, ChecksumGeneration, ChecksumJobId, ChecksumLimits,
    ChecksumTarget, FsError, SelectionRevision, TreeProjection, TreeRequestId, TreeRowId,
    TreeSelectionSnapshot, TreeViewStamp,
};
use std::{path::PathBuf, sync::Arc};

#[derive(Clone, Debug)]
pub struct TreeChecksumRequest {
    pub view: Arc<TreeProjection>,
    pub request: TreeRequestId,
    pub selection_revision: SelectionRevision,
    pub selection: Arc<TreeSelectionSnapshot>,
    pub fallback: Option<TreeRowId>,
    pub cancel: Cancellation,
}

/// The directory and optional focused sidecar are backend-resolved draft defaults.
/// Only `sources` authorizes calculation; the presenter never resolves providers.
#[derive(Clone, Debug)]
pub struct TreeChecksumCapture {
    pub sources: Arc<dyn CapturedChecksumFiles>,
    pub directory: PathBuf,
    pub fallback: Option<PathBuf>,
}

/// Retain this owner through native reads and destroy it on a backend worker.
/// A captured source survives panel navigation, but is not a filesystem snapshot.
pub trait CapturedChecksumFiles: std::fmt::Debug + Send + Sync {
    fn stamp(&self) -> TreeViewStamp;
    fn request(&self) -> TreeRequestId;
    fn resolve(&self, cancel: &Cancellation) -> Result<Vec<ChecksumTarget>, FsError>;
}

#[derive(Clone, Debug)]
pub struct CapturedChecksumRequest {
    pub job: ChecksumJobId,
    pub generation: ChecksumGeneration,
    pub sources: Arc<dyn CapturedChecksumFiles>,
    pub algorithms: ChecksumAlgorithms,
    pub recursive: bool,
    pub limits: ChecksumLimits,
}
