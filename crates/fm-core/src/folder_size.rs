//! Progress from a cancellable, read-only directory-size scan.

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct FolderSizeScanId(pub u64);

/// Logical file lengths, not allocated disk usage or a filesystem snapshot.
/// Links and special files are excluded; errors make a result explicitly partial.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FolderSize {
    pub bytes: u64,
    pub files: u64,
    pub skipped_links: u64,
    pub errors: u64,
    pub finished: bool,
}
