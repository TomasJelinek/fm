//! Optional transfer metadata: required modified time for bounded native regular-file
//! transfers, and best-effort preserved times and attributes for any transfer.
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
    /// The Copy/Move dialog's "Preserve times and attributes": every copied file and
    /// every folder the job creates gets its source's times and attributes
    /// (`TransferPreservedMetadata`), for any number of items. Best effort: what a
    /// destination cannot take is a warning of the job
    /// (`CopyErrorStage::PreservingMetadata`), never a failure of the copy, and the
    /// content is verified as without it.
    PreserveTimesAndAttributes,
}

/// The Windows attributes that `PreserveTimesAndAttributes` copies: Read-only, Hidden,
/// System, Archive and Not content indexed. Compressed, Encrypted and Sparse are how the
/// destination stores a file (its folder decides), and are never copied.
pub const PRESERVED_WINDOWS_ATTRIBUTES: u32 = 0x1 | 0x2 | 0x4 | 0x20 | 0x2000;

/// The POSIX mode bits that `PreserveTimesAndAttributes` copies: read, write and execute
/// for user, group and other, without the process umask. Set-user-ID, set-group-ID and
/// sticky are never copied: the copy belongs to the user who makes it.
pub const PRESERVED_UNIX_MODE_BITS: u32 = 0o777;

/// A source's times and attributes, read before its first byte (or, for a folder, before
/// its listing) so that the copy's own reading cannot change the access time it keeps.
/// Each field is what the source carries; None is unknown and leaves the copy's own
/// value. Destinations apply what they can store (`StagedTransferFile::
/// apply_preserved_metadata`, `TransferDirectory::apply_preserved_metadata`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TransferPreservedMetadata {
    pub created: Option<SystemTime>,
    pub accessed: Option<SystemTime>,
    pub modified: Option<SystemTime>,
    /// Raw Windows attributes; only `PRESERVED_WINDOWS_ATTRIBUTES` are applied.
    pub windows_attributes: Option<u32>,
    /// POSIX mode; only `PRESERVED_UNIX_MODE_BITS` are applied.
    pub unix_mode: Option<u32>,
}

impl TransferPreservedMetadata {
    /// What a source without native metadata carries: its entry's modified time, such
    /// as an archive member's. Its other times and its attributes are unknown.
    pub fn modified_only(modified: Option<SystemTime>) -> Self {
        Self {
            modified,
            ..Self::default()
        }
    }
}

/// The most roots one transfer under `TransferMetadataPolicy::RequireModifiedTime` takes:
/// each is pinned and staged before anything is copied. The backend refuses more
/// itself, before any source is captured, whatever the dialog offered.
pub const MAX_REQUIRED_METADATA_ROOTS: usize = 64;

/// Why a Copy or Move under `TransferMetadataPolicy::RequireModifiedTime` of more than
/// `MAX_REQUIRED_METADATA_ROOTS` roots does not start.
pub const REQUIRED_METADATA_ROOTS_REFUSAL: &str = "Required modified time accepts at most 64 \
     distinct native regular files; transfer larger selections without it.";

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
/// there is no silent policy downgrade. `PreserveTimesAndAttributes` has no such bound.
#[derive(Clone, Debug)]
pub struct MetadataTransferRequest {
    pub input: MetadataTransferInput,
    pub intent: TransferIntent,
    pub metadata: TransferMetadataPolicy,
}
