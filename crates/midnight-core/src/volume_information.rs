//! Measured filesystem details for an explicit, read-only volume information request.
use crate::{Cancellation, DesktopRequestId, FsError, NavigationId, PaneId, StorageKind};
use std::{ffi::OsString, path::PathBuf};

#[derive(Clone, Debug)]
pub struct VolumeInformationRequest {
    pub request: DesktopRequestId,
    pub pane: PaneId,
    pub navigation: NavigationId,
    pub path: PathBuf,
    pub cancel: Cancellation,
}

#[derive(Clone, Debug)]
pub struct VolumeInformationResult {
    pub request: DesktopRequestId,
    pub pane: PaneId,
    pub navigation: NavigationId,
    pub path: PathBuf,
    pub result: Result<VolumeInformation, FsError>,
}

/// Each value is independent. Caller quotas must never be mistaken for volume size.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VolumeCapacity {
    pub total: Option<u64>,
    pub free: Option<u64>,
    pub available: Option<u64>,
}
impl VolumeCapacity {
    /// Filesystem-used space, including filesystem reservations if counted as unavailable.
    pub fn used(&self) -> Option<u64> {
        self.total?.checked_sub(self.free?)
    }
}

/// A property positively reported by the operating system, never inferred from a suffix.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VolumeProperty {
    CasePreservedNames,
    CaseSensitiveLookup,
    UnicodeNames,
    PersistentAcls,
    FileCompression,
    SparseFiles,
    NamedStreams,
    HardLinks,
    ReparsePoints,
    Encryption,
    Quotas,
    NoExecute,
    NoSetId,
    NoDeviceFiles,
}

#[derive(Clone, Debug, Default)]
pub struct VolumeInformation {
    /// Resolved native mount/volume, when the OS can identify it reliably.
    pub root: Option<PathBuf>,
    pub label: Option<OsString>,
    pub filesystem: Option<OsString>,
    /// A kernel mount source (Linux) or volume serial identifier (Windows).
    pub source: Option<OsString>,
    pub capacity: VolumeCapacity,
    pub storage: StorageKind,
    pub read_only: Option<bool>,
    pub properties: Vec<VolumeProperty>,
    /// Partial failures do not discard other independently measured fields.
    pub unavailable: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn used_bytes_require_filesystem_total_and_free_never_caller_available() {
        let mut capacity = VolumeCapacity {
            total: Some(100),
            free: Some(40),
            available: Some(0),
        };
        assert_eq!(capacity.used(), Some(60));
        capacity.free = None;
        assert_eq!(capacity.used(), None);
        capacity.free = Some(101);
        assert_eq!(capacity.used(), None);
        capacity.total = None;
        assert_eq!(capacity.used(), None);
    }
}
