//! Optional required metadata for bounded native regular-file transfers.
use crate::{
    Cancellation, ClipboardMoveRequest, CopyRequest, FsError, SelectedCopyRequest, TransferIntent,
};
use std::time::SystemTime;

/// Existing transfer requests remain content-only. Required fields never fall back.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TransferMetadataPolicy {
    #[default]
    ContentOnly,
    /// Exact modified time; first slice accepts at most64 native regular-file roots.
    RequireModifiedTime,
}

/// Provider-owned source handle and pre-read revision. No pathname reopening.
pub trait TransferModifiedTimePin: Send {
    fn modified(&self) -> SystemTime;
    /// Read the same retained object, including after recoverable source capture.
    fn current_modified(&self, cancel: &Cancellation) -> Result<SystemTime, FsError>;
    /// Original revision check before publication, never after our own source rename.
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError>;
}

#[derive(Clone, Debug)]
pub enum MetadataTransferInput {
    Paths(CopyRequest),
    Selected(SelectedCopyRequest),
    Clipboard(ClipboardMoveRequest),
}

/// Explicit optional policy without changing existing request constructors or defaults.
/// Required modified time accepts at most64 native regular-file roots. Folders,
/// links and virtual sources are refused; there is no silent policy downgrade.
#[derive(Clone, Debug)]
pub struct MetadataTransferRequest {
    pub input: MetadataTransferInput,
    pub intent: TransferIntent,
    pub metadata: TransferMetadataPolicy,
}
