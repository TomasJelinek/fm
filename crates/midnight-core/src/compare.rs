//! Read-only comparison of captured visible directory entries, never content verification.
use crate::{DirectoryIndex, SelectionRevision, SelectionSnapshot, VisibleDirectory};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct CompareId(pub u64);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompareOptions {
    pub by_size: bool,
    pub by_modified: bool,
    pub case_sensitive: bool,
    pub time_tolerance_seconds: u32,
    pub unmatched_directories: bool,
}

impl Default for CompareOptions {
    fn default() -> Self {
        Self {
            by_size: true,
            by_modified: true,
            case_sensitive: true,
            time_tolerance_seconds: 2,
            unmatched_directories: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct CompareSide {
    pub source: VisibleDirectory,
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

#[derive(Clone, Debug)]
pub struct CompareResult {
    pub request: Arc<CompareRequest>,
    pub selections: [Arc<SelectionSnapshot>; 2],
    /// Pairs/items whose requested metadata is unavailable; these are selected.
    pub unknown: usize,
    /// Case-insensitive name groups that cannot be paired uniquely.
    pub ambiguous: usize,
}
