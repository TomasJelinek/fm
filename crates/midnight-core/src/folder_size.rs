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
