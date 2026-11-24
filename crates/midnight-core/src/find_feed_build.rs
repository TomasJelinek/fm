//! Search-result-to-panel conversion. `prepare` belongs on a backend worker.
use crate::{
    Cancellation, DirectoryIndex, Entry, EntryId, EntryKind, FindGeneration, FindLimits, FindOrder,
    FindResult, FindResultId, FindSessionId, FsError, PaneId,
};
use std::{collections::HashMap, io, path::Path, sync::Arc};

#[derive(Clone, Debug)]
pub struct FindFeedBuildRequest {
    pub id: u64,
    pub pane: PaneId,
    pub session: FindSessionId,
    pub generation: FindGeneration,
    pub results: Arc<[Arc<FindResult>]>,
    pub order: Option<Arc<FindOrder>>,
    pub selection_bits: Arc<[u64]>,
    pub explicit_marks: bool,
    pub cursor: Option<FindResultId>,
    pub cancel: Cancellation,
}
#[derive(Clone, Copy, Debug)]
pub struct FindFeedBuildStamp {
    pub id: u64,
    pub pane: PaneId,
    pub session: FindSessionId,
    pub generation: FindGeneration,
}
#[derive(Clone, Debug)]
pub struct FindFeedBuilt {
    pub entries: Arc<[Entry]>,
    pub index: Arc<DirectoryIndex>,
    pub selected: Arc<Vec<bool>>,
    pub selected_count: usize,
    pub selected_bytes: u64,
    pub cursor: usize,
    pub omitted: usize,
}
#[derive(Clone, Debug)]
pub struct FindFeedBuildResult {
    pub request: FindFeedBuildStamp,
    pub result: Result<FindFeedBuilt, FsError>,
}
fn invalid(message: &str) -> FsError {
    FsError::from_io(
        "feed search results",
        Path::new(""),
        io::Error::new(io::ErrorKind::InvalidInput, message),
    )
}
impl FindFeedBuildRequest {
    pub fn stamp(&self) -> FindFeedBuildStamp {
        FindFeedBuildStamp {
            id: self.id,
            pane: self.pane,
            session: self.session,
            generation: self.generation,
        }
    }
    fn check(&self) -> Result<(), FsError> {
        if self.cancel.is_cancelled() {
            Err(FsError::from_io(
                "feed search results",
                Path::new(""),
                io::Error::new(
                    io::ErrorKind::Interrupted,
                    "Preparing panel results was cancelled.",
                ),
            ))
        } else {
            Ok(())
        }
    }
    /// Deduplicate native paths, build presentation entries and their complete
    /// immutable index, and transfer explicit marks. This enumerates up to the
    /// Find budget and allocates paths; call only on a backend worker.
    pub fn prepare(&self) -> Result<FindFeedBuilt, FsError> {
        self.check()?;
        let limits = FindLimits::default();
        if self.results.len() > limits.max_results
            || self.selection_bits.len() > self.results.len().div_ceil(64)
        {
            return Err(invalid(
                "Search results exceed the feed row or selection budget.",
            ));
        }
        let mut bytes = 0usize;
        for (position, result) in self.results.iter().enumerate() {
            self.check()?;
            bytes = bytes
                .saturating_add(std::mem::size_of::<FindResult>())
                .saturating_add(
                    result
                        .entry
                        .path
                        .as_os_str()
                        .as_encoded_bytes()
                        .len()
                        .saturating_mul(4),
                );
            if result.id.0 != position as u64 + 1 || bytes > limits.max_retained_bytes {
                return Err(invalid(
                    "Search result identities or retained bytes exceed the feed budget.",
                ));
            }
        }
        let order = self.order.as_ref();
        if order.is_some_and(|order| {
            order.ids.len() > self.results.len() || order.positions.len() != order.ids.len()
        }) {
            return Err(invalid("Search result order exceeds its captured prefix."));
        }
        let mut used = vec![false; self.results.len()];
        let mut seen = HashMap::with_capacity(self.results.len());
        let mut entries: Vec<Entry> = Vec::with_capacity(self.results.len());
        let mut selected: Vec<bool> = Vec::with_capacity(self.results.len());
        let mut cursor = 0;
        let mut omitted = 0;
        for position in 0..self.results.len() {
            self.check()?;
            let slot = match order.filter(|order| position < order.ids.len()) {
                Some(order) => {
                    let slot = order.ids[position]
                        .0
                        .checked_sub(1)
                        .and_then(|slot| usize::try_from(slot).ok())
                        .filter(|slot| *slot < order.ids.len())
                        .ok_or_else(|| {
                            invalid("Search result order contains an unknown prefix identity.")
                        })?;
                    if order.positions[slot] != position {
                        return Err(invalid(
                            "Search result order positions disagree with its identities.",
                        ));
                    }
                    slot
                }
                None => position,
            };
            if std::mem::replace(&mut used[slot], true) {
                return Err(invalid(
                    "Search result order contains duplicate identities.",
                ));
            }
            let result = &self.results[slot];
            if result.archive.is_some()
                || !matches!(result.entry.kind, EntryKind::File | EntryKind::Directory)
            {
                omitted += 1;
                continue;
            }
            let row = if let Some(row) = seen.get(result.entry.path.as_path()) {
                *row
            } else {
                let row = entries.len();
                let mut entry = Entry::new(
                    EntryId(row as u64 + 1),
                    result.entry.path.clone(),
                    result.entry.kind,
                );
                entry.metadata = result.entry.metadata.clone();
                seen.insert(result.entry.path.as_path(), row);
                entries.push(entry);
                selected.push(false);
                row
            };
            if self.explicit_marks
                && self
                    .selection_bits
                    .get(slot / 64)
                    .is_some_and(|word| word & (1u64 << (slot % 64)) != 0)
            {
                selected[row] = true;
            }
            if self.cursor == Some(result.id) {
                cursor = row;
            }
        }
        if entries.is_empty() {
            return Err(invalid(if omitted > 0 {
                "Only archive members were found; open them with Go to file."
            } else {
                "There are no results to feed to a panel."
            }));
        }
        let mut selected_count = 0usize;
        let mut selected_bytes = 0u64;
        for (entry, selected) in entries.iter().zip(&selected) {
            self.check()?;
            if *selected {
                selected_count += 1;
                if entry.kind == EntryKind::File {
                    selected_bytes =
                        selected_bytes.saturating_add(entry.metadata.size.unwrap_or(0));
                }
            }
        }
        self.check()?;
        let index = Arc::new(DirectoryIndex::from_entries(&entries));
        self.check()?;
        Ok(FindFeedBuilt {
            entries: entries.into(),
            index,
            selected: Arc::new(selected),
            selected_count,
            selected_bytes,
            cursor,
            omitted,
        })
    }
}
#[cfg(test)]
#[path = "find_feed_build_tests.rs"]
mod tests;
