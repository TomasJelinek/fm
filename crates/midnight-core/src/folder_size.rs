//! Progress from a cancellable, read-only directory-size scan.

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct FolderSizeScanId(pub u64);

/// Logical file lengths, not allocated disk usage or a filesystem snapshot.
/// A scanned root that is itself a native directory link is measured through its
/// target; directory and file links met inside the measured tree, and ancestry
/// cycles, are never followed and are counted in `skipped_links`.
/// Special files are excluded; errors make a result explicitly partial.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FolderSize {
    pub bytes: u64,
    pub files: u64,
    pub skipped_links: u64,
    pub errors: u64,
    pub finished: bool,
    /// Immutable diagnostic prefix for this publication; retained until backend reclamation.
    pub warnings: Option<crate::SizeWarningReport>,
}

impl FolderSize {
    /// The total a Size sort orders this folder by: that of a finished calculation,
    /// including one that skipped unreadable entries (a partial total shown with
    /// `≈`). A running, cancelled or otherwise unfinished total gives none, so the
    /// folder sorts with the folders whose size is unknown.
    pub fn sort_bytes(&self) -> Option<u64> {
        self.finished.then_some(self.bytes)
    }
}
