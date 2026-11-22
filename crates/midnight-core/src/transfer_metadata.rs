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
    /// Exact modified time, for at most `MAX_REQUIRED_METADATA_ROOTS` distinct native
    /// regular-file roots.
    RequireModifiedTime,
}

/// The most roots one transfer under `TransferMetadataPolicy::RequireModifiedTime` takes:
/// each is pinned and staged before anything is copied. The backend refuses more
/// itself, before any source is captured, whatever the dialog offered.
pub const MAX_REQUIRED_METADATA_ROOTS: usize = 64;

/// Why a Copy under `TransferMetadataPolicy::RequireModifiedTime` of more than
/// `MAX_REQUIRED_METADATA_ROOTS` roots does not start. A Move of as many gives
/// `MOVE_ROOTS_REFUSAL`, the sentence every Move entry point gives.
pub const REQUIRED_METADATA_ROOTS_REFUSAL: &str = "Required modified time accepts at most 64 \
     distinct native regular files; choose content-only Copy for larger selections.";

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
/// Required modified time accepts at most `MAX_REQUIRED_METADATA_ROOTS` native
/// regular-file roots. More roots, folders, links and virtual sources are refused;
/// there is no silent policy downgrade.
#[derive(Clone, Debug)]
pub struct MetadataTransferRequest {
    pub input: MetadataTransferInput,
    pub intent: TransferIntent,
    pub metadata: TransferMetadataPolicy,
}
