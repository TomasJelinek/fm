//! Exact inline-tree Copy/Move capture. Provider ownership must be retired on backend workers.
use crate::*;
use std::{collections::VecDeque, path::PathBuf, sync::Arc};

#[derive(Clone, Debug)]
pub struct TreeTransferRequest {
    pub intent: TransferIntent,
    pub view: Arc<TreeProjection>,
    pub request: TreeRequestId,
    pub selection_revision: SelectionRevision,
    pub selection: Arc<TreeSelectionSnapshot>,
    pub fallback: Option<TreeRowId>,
    pub cancel: Cancellation,
}

/// Opaque retained provider roots. Call prepare and destroy the last owner only on
/// backend workers, including rejected sends, stale events, draft cancel and shutdown.
/// Cached accessors do no provider work. Native identity is pinned during preparation;
/// retaining a draft does not promise an immutable filesystem snapshot.
pub trait CapturedTransferSources: std::fmt::Debug + Send + Sync {
    fn intent(&self) -> TransferIntent;
    fn root_count(&self) -> usize;
    fn metadata_available(&self) -> bool;
    fn prepare(
        &self,
        service: &dyn TransferService,
        cancel: &Cancellation,
    ) -> Result<PreparedCapturedTransfer, FsError>;
    /// Optional explicit link-preserving Copy capture. Reject delegates exactly
    /// to legacy preparation; unimplemented Preserve fails before opening roots.
    /// Implementations retain the policy and original raw occurrence/pins through
    /// recursive children and retry, never recapturing a link's browsed referent.
    /// Preserve with a Move intent must be rejected before preparation or writes.
    fn prepare_with_links(
        &self,
        service: &dyn TransferService,
        links: CopyLinkPolicy,
        cancel: &Cancellation,
    ) -> Result<PreparedCapturedTransfer, FsError> {
        match links {
            CopyLinkPolicy::Reject => self.prepare(service, cancel),
            CopyLinkPolicy::Preserve => Err(FsError::from_io(
                "prepare captured transfer",
                std::path::Path::new(""),
                std::io::Error::new(
                    std::io::ErrorKind::Unsupported,
                    "link-preserving captured preparation is unsupported",
                ),
            )),
        }
    }
}

/// Prepared authority remains intent-specific; Copy roots cannot grant removal.
pub enum PreparedCapturedTransfer {
    Copy(VecDeque<Box<dyn TransferSource>>),
    Move(VecDeque<PreparedTransferMove>),
}

/// Retire sources on a backend worker when dismissing or rejecting this result.
#[derive(Clone, Debug)]
pub struct TreeTransferCapture {
    pub intent: TransferIntent,
    pub root_count: usize,
    pub metadata_available: bool,
    pub sources: Arc<dyn CapturedTransferSources>,
}

/// Confirmation uses normal FIFO admission; an open draft holds no queue slot.
#[derive(Clone, Debug)]
pub struct CapturedTransferRequest {
    pub verification: TransferVerification,
    pub id: OperationId,
    pub sources: Arc<dyn CapturedTransferSources>,
    pub destination: PathBuf,
    pub metadata: TransferMetadataPolicy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TreeCopyOccurrence {
    pub stamp: TreeViewStamp,
    pub row: TreeRowId,
}
