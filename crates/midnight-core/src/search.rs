//! Immutable prefix-search requests and projections over an accepted directory listing.

use crate::{DirectoryIndex, Entry, NavigationId, PaneId};
use std::{path::PathBuf, sync::Arc};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SearchRevision(pub u64);

/// Source identity is the exact shared, unfiltered listing, never display text.
#[derive(Clone, Debug)]
pub struct SearchRequest {
    pub pane: PaneId,
    pub navigation: NavigationId,
    pub revision: SearchRevision,
    pub location: PathBuf,
    pub source: Arc<[Entry]>,
    /// Literal prefix with default Unicode case folding, without normalization.
    /// Non-Unicode native names match only the empty query; raw identities never change.
    pub query: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SearchMatch {
    /// Position in `SearchResult::source`, in that listing's original order.
    pub source_row: usize,
    /// UTF-8 byte boundary in the original `Entry::display_name`, rounded to
    /// the end of the containing extended grapheme. Zero means no highlight.
    pub display_prefix_end: usize,
}

/// A presentation/selection projection; it never replaces the filesystem snapshot.
#[derive(Clone, Debug)]
pub struct SearchResult {
    pub source: Arc<[Entry]>,
    /// Strictly increasing source positions permit local binary cursor lookup.
    pub matches: Arc<[SearchMatch]>,
    /// Positions are contiguous match-row positions, not source-row positions.
    pub index: Arc<DirectoryIndex>,
}
