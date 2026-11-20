//! Read-only comparison of captured visible directory entries. Metadata criteria
//! never read file contents; the optional content criterion reads both files of a
//! pair byte by byte on a backend worker.
use crate::{DirectoryIndex, FsError, SelectionRevision, SelectionSnapshot, VisibleDirectory};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct CompareId(pub u64);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompareOptions {
    pub by_size: bool,
    pub by_modified: bool,
    /// Same-named files of equal known size that the enabled metadata criteria
    /// found equal are read byte by byte; different bytes select both files. A
    /// known size difference selects both files when size is not a criterion.
    pub by_content: bool,
    /// Same-named folders are compared recursively. A folder that contains a
    /// difference is selected on each side the difference selects; folders and
    /// links present on one side only count as differences at every depth.
    pub recursive: bool,
    pub case_sensitive: bool,
    pub time_tolerance_seconds: u32,
    pub unmatched_directories: bool,
}

impl Default for CompareOptions {
    fn default() -> Self {
        Self {
            by_size: true,
            by_modified: true,
            by_content: false,
            recursive: false,
            case_sensitive: true,
            time_tolerance_seconds: 2,
            unmatched_directories: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct CompareSide {
    pub source: VisibleDirectory,
    /// Optional exact displayed tree; comparison includes selectable root rows only.
    pub tree: Option<Arc<crate::TreeProjection>>,
    pub view: Arc<DirectoryIndex>,
    pub view_revision: u64,
    pub selection_revision: SelectionRevision,
}

#[derive(Clone, Debug)]
pub struct CompareRequest {
    pub id: CompareId,
    pub sides: [CompareSide; 2],
    pub options: CompareOptions,
}

/// What a running comparison is doing; sampled at most four times per second.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ComparePhase {
    /// Listing same-named folders (Include subfolders) and comparing metadata.
    #[default]
    Scanning,
    /// Reading file pairs of equal size byte by byte.
    Contents,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CompareProgress {
    pub phase: ComparePhase,
    /// Folder pairs listed inside the compared directories.
    pub folders: u64,
    /// Entries examined inside those folders (displayed rows not included).
    pub entries: u64,
    /// File pairs read completely so far, and how many need reading.
    pub content_files: u64,
    pub content_files_total: u64,
    /// Bytes read from both sides together, and the total both sides need.
    pub content_bytes: u64,
    pub content_bytes_total: u64,
}

/// At most this many diagnostics are retained; the counts include the rest.
pub const MAX_COMPARE_DIAGNOSTICS: usize = 8;

#[derive(Clone, Debug)]
pub struct CompareResult {
    pub request: Arc<CompareRequest>,
    pub selections: [Arc<SelectionSnapshot>; 2],
    /// Worker-built occurrence selections; these never authorize native operations.
    pub tree_replacements: [Option<Arc<crate::TreeSelectionReplacement>>; 2],
    /// Pairs/items whose requested metadata is unavailable; these are selected.
    pub unknown: usize,
    /// Case-insensitive name groups that cannot be paired uniquely.
    pub ambiguous: usize,
    /// Same-named displayed folder pairs that contain differences (Include subfolders).
    pub differing_folders: usize,
    /// File pairs of equal size whose bytes differ (Compare contents).
    pub content_differences: usize,
    /// File pairs read completely, and the bytes read from both sides together.
    pub content_files: u64,
    pub content_bytes: u64,
    /// Links, special files and mount boundaries inside folders that exist on
    /// both sides; they are matched by name only and never followed.
    pub skipped: u64,
    /// Entries or folders that could not be read; their folders count as different.
    pub errors: u64,
    /// A resource limit ended the folder walk early; unfinished folders count as different.
    pub limited: bool,
    /// At most MAX_COMPARE_DIAGNOSTICS samples of the errors above.
    pub diagnostics: Vec<FsError>,
}
