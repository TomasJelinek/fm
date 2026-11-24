//! Immutable, explicitly captured roots for measuring folders in Find result panels.
use crate::{
    Cancellation, Entry, EntryId, EntryKind, FindLimits, FolderSize, FolderSizeScanId, FsError,
    PaneId,
};
use std::{collections::HashSet, sync::Arc};

#[derive(Clone, Debug)]
pub struct FindFeedSizeRequest {
    pub pane: PaneId,
    pub id: FolderSizeScanId,
    pub rows_serial: u64,
    pub entries: Arc<[Entry]>,
    pub selected: Arc<Vec<bool>>,
    pub cancel: Cancellation,
}

impl FindFeedSizeRequest {
    /// Validate and select roots on a backend worker. An unmarked list measures
    /// every shown folder; an explicit selection of only files measures nothing.
    pub fn roots(&self) -> Result<Vec<Entry>, &'static str> {
        if self.id.0 == 0
            || self.rows_serial == 0
            || self.entries.len() > FindLimits::default().max_results
            || self.selected.len() != self.entries.len()
        {
            return Err("Invalid Find folder-size request shape.");
        }
        let mut ids = HashSet::with_capacity(self.entries.len());
        for entry in self.entries.iter() {
            if entry.id.0 == 0
                || !ids.insert(entry.id)
                || entry.native_path().is_none()
                || !entry.path.is_absolute()
                || entry.path.as_os_str().as_encoded_bytes().contains(&0)
            {
                return Err("Find folder-size roots require distinct native entries.");
            }
        }
        let marked = self.selected.iter().any(|selected| *selected);
        Ok(self
            .entries
            .iter()
            .zip(self.selected.iter())
            .filter(|(entry, selected)| {
                (!marked || **selected)
                    && matches!(entry.kind, EntryKind::Directory | EntryKind::DirectoryLink)
            })
            .map(|(entry, _)| entry.clone())
            .collect())
    }
}

#[derive(Clone, Debug)]
pub enum FindFeedSizeUpdate {
    Updated {
        entry: EntryId,
        size: FolderSize,
    },
    Finished {
        cancelled: bool,
        error: Option<FsError>,
    },
}

#[derive(Clone, Debug)]
pub struct FindFeedSizeEvent {
    pub pane: PaneId,
    pub id: FolderSizeScanId,
    pub rows_serial: u64,
    pub update: FindFeedSizeUpdate,
}

#[cfg(test)]
#[path = "find_feed_sizes_tests.rs"]
mod tests;
