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
    /// A finite host composition promise, independent of native geometry. When
    /// passed a separately host-audited native service, all returned native
    /// sources, Move authorities, cleanup tickets and root/alias pins derive from
    /// that service. Additional retained provider owners use this capability's
    /// explicit consuming release; archive children use their archive bridge.
    /// Receiving an audited service alone never qualifies a foreign capture.
    fn prepares_audited_native_children(&self) -> bool {
        false
    }
    /// Consume this reference on a backend worker. Ok confirms any last-owner
    /// cleanup acquired by this release; shared tabs/operations need not finish.
    /// A deferred owner supplies its actual completion bridge. Unknown defaults
    /// retain ownership and fail rather than treating wrapper Drop as completion.
    fn release_on_worker(self: Arc<Self>) -> Result<(), FsError> {
        std::mem::forget(self);
        Err(crate::transfer::unconfirmed_transfer_release(
            "release captured transfer sources",
        ))
    }
    /// Give up this reference where its release cannot run, inside a retirement that
    /// is already quarantined. Never waits and destroys no native resource: a shared
    /// reference only gives up its count; the last owner passes the archive references
    /// it holds to their own quarantined release and keeps its registrations and
    /// permits retained. The default retains the incoming Arc.
    fn release_quarantined(self: Arc<Self>) {
        std::mem::forget(self);
    }
    fn intent(&self) -> TransferIntent;
    fn root_count(&self) -> usize;
    fn metadata_available(&self) -> bool;
    fn prepare(
        &self,
        service: &dyn TransferService,
        cancel: &Cancellation,
    ) -> Result<PreparedCapturedTransfer, FsError>;
    /// Prepare captured sources with an explicit copy-side link policy. Reject uses
    /// legacy preparation. Unimplemented policies fail before opening roots.
    /// Keep the exact original occurrence and its retained pins through recursion
    /// and retry. A browsed referent does not grant authority for the selected link.
    /// Preserve is available only for Copy.
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
            CopyLinkPolicy::FollowDirectoryContents | CopyLinkPolicy::EmptyDirectories => {
                Err(FsError::from_io(
                    "prepare captured transfer",
                    std::path::Path::new(""),
                    std::io::Error::new(
                        std::io::ErrorKind::Unsupported,
                        "captured directory link preparation is unsupported",
                    ),
                ))
            }
        }
    }
}

/// Prepared authority remains intent-specific; Copy roots cannot grant removal.
pub enum PreparedCapturedTransfer {
    Copy(VecDeque<Box<dyn TransferSource>>),
    Move(VecDeque<PreparedTransferMove>),
    MoveWithDirectoryLinks(VecDeque<crate::PreparedTransferMoveEntry>),
}

/// Retire sources on a backend worker when dismissing or rejecting this result.
#[derive(Clone, Debug)]
pub struct TreeTransferCapture {
    pub intent: TransferIntent,
    pub root_count: usize,
    pub metadata_available: bool,
    pub sources: Arc<dyn CapturedTransferSources>,
}

/// Confirmation admits the transfer; an open draft holds no queue slot.
#[derive(Clone, Debug)]
pub struct CapturedTransferRequest {
    pub start_policy: TransferStartPolicy,
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
