//! A re-read of the folder whose rows a pane shows: a Refresh after a watch
//! hint, a finished operation or an explicit reload. The UI keeps showing its
//! rows until the listing commits, so no row batches stream to it. Entries of
//! a shown path and kind keep their identity, sort keys and localized
//! metadata, and the new rows are described as edits of the shown rows
//! (`Event::ListingChanges`), so the UI updates only the rows that changed.

use crate::find_worker::localize_metadata;
use midnight_core::{Entry, EntryId, EntryKind, EntryMetadata, LocaleService, RowEdit};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
};

/// Edits that touch more rows than this replace the rows instead: one model
/// reset is then cheaper than notifying each row, and the change is no longer
/// "a few rows".
const MAX_EDITED_ROWS: usize = 256;

/// The rows a pane shows for the folder that is listed again.
pub(crate) struct Shown {
    rows: Arc<[Entry]>,
    by_path: HashMap<PathBuf, usize>,
}

impl Shown {
    pub(crate) fn new(rows: Arc<[Entry]>) -> Self {
        let by_path = rows
            .iter()
            .enumerate()
            .map(|(row, entry)| (entry.path.clone(), row))
            .collect();
        Self { rows, by_path }
    }

    pub(crate) fn into_rows(self) -> Arc<[Entry]> {
        self.rows
    }

    /// The shown entry that `entry` continues: the same path and kind.
    fn entry(&self, entry: &Entry) -> Option<&Entry> {
        self.by_path
            .get(&entry.path)
            .map(|row| &self.rows[*row])
            .filter(|shown| shown.kind == entry.kind)
    }
}

/// Gives each entry a re-read finds the identity of the shown entry it
/// continues, and remembers the identities the listing read, which are
/// restored when an adopted identity is not unique (a provider may number
/// entries independently of earlier listings).
#[derive(Default)]
pub(crate) struct Adoption {
    read: Vec<EntryId>,
    adopted: HashSet<EntryId>,
    new: Vec<EntryId>,
}

impl Adoption {
    /// Completes `entry` with `metadata` read now (or keeps what the listing
    /// read when it is `None`). An entry continuing a shown one with the same
    /// metadata becomes that shown entry, keeping its localized time; one with
    /// other metadata keeps the shown identity, name and sort keys.
    pub(crate) fn adopt(
        &mut self,
        shown: &Shown,
        entry: &mut Entry,
        metadata: Option<EntryMetadata>,
        locale: Option<&dyn LocaleService>,
    ) {
        self.read.push(entry.id);
        let mut metadata = metadata.unwrap_or_else(|| entry.metadata.clone());
        match shown.entry(entry) {
            Some(previous) if same_metadata(previous, &metadata) => *entry = previous.clone(),
            Some(previous) => {
                localize_metadata(&mut metadata, locale);
                *entry = previous.clone();
                entry.metadata = metadata;
            }
            None => {
                localize_metadata(&mut metadata, locale);
                entry.metadata = metadata;
                self.new.push(entry.id);
                return;
            }
        }
        self.adopted.insert(entry.id);
    }

    /// Whether `entries` (adopted in order) keep the adopted identities.
    /// Otherwise every entry gets back the identity the listing read.
    pub(crate) fn finish(self, entries: &mut [Entry]) -> bool {
        if self.new.iter().all(|id| !self.adopted.contains(id)) {
            return true;
        }
        for (entry, id) in entries.iter_mut().zip(self.read) {
            entry.id = id;
        }
        false
    }
}

/// Whether `metadata` read now shows what `shown` shows. Directory lengths are
/// not compared: a listing replaces them with computed folder totals.
fn same_metadata(shown: &Entry, metadata: &EntryMetadata) -> bool {
    let directory = matches!(shown.kind, EntryKind::Directory | EntryKind::DirectoryLink);
    let old = &shown.metadata;
    (directory || old.size == metadata.size)
        && old.modified == metadata.modified
        && old.compression == metadata.compression
        && old.native_attributes == metadata.native_attributes
        && old.unix_mode == metadata.unix_mode
}

/// The rows `before` and `after` show the same entry the same way.
fn same_row(before: &Entry, after: &Entry) -> bool {
    before.kind == after.kind
        && before.metadata == after.metadata
        && before.display_name == after.display_name
}

/// Edits that turn the sorted rows `before` into the sorted rows `after`,
/// matching rows by identity and path; `None` beyond `MAX_EDITED_ROWS`. A row that
/// moved is removed and added again, so a move costs two edits in either
/// direction.
pub(crate) fn row_edits(before: &[Entry], after: &[Entry]) -> Option<Vec<RowEdit>> {
    type Key<'a> = (EntryId, &'a Path);
    fn key(entry: &Entry) -> Key<'_> {
        (entry.id, entry.path.as_path())
    }
    fn rows(entries: &[Entry]) -> HashMap<Key<'_>, usize> {
        entries
            .iter()
            .enumerate()
            .map(|(row, entry)| (key(entry), row))
            .collect()
    }
    let (before_rows, after_rows) = (rows(before), rows(after));
    // Rows that moved and were already edited on one side: removed where they
    // were shown and still to be added where they are listed, or the reverse.
    let mut moved: HashSet<Key> = HashSet::new();
    let mut edits = Edits::default();
    let (mut old, mut new) = (0, 0);
    loop {
        match (before.get(old), after.get(new)) {
            (None, None) => break,
            (Some(shown), Some(listed)) if key(shown) == key(listed) => {
                if !same_row(shown, listed) {
                    edits.push(RowEdit::Changed { row: new });
                }
                old += 1;
                new += 1;
            }
            (Some(_), None) => {
                edits.remove(new);
                old += 1;
            }
            (None, Some(_)) => {
                edits.add(new);
                new += 1;
            }
            // Gone, or a mover already added where it is listed.
            (Some(shown), Some(_))
                if !after_rows.contains_key(&key(shown)) || moved.contains(&key(shown)) =>
            {
                edits.remove(new);
                old += 1;
            }
            // New, or a mover already removed where it was shown.
            (Some(_), Some(listed))
                if !before_rows.contains_key(&key(listed)) || moved.contains(&key(listed)) =>
            {
                edits.add(new);
                new += 1;
            }
            // Both rows are on either side, ahead of both cursors: the one that
            // moved further is the mover, so a row that jumps far up costs as
            // little as one that falls far down. A tie keeps the shown row.
            (Some(shown), Some(listed)) => {
                let down = after_rows
                    .get(&key(shown))
                    .map_or(0, |row| row.saturating_sub(new));
                let up = before_rows
                    .get(&key(listed))
                    .map_or(0, |row| row.saturating_sub(old));
                if down >= up {
                    moved.insert(key(shown));
                    edits.remove(new);
                    old += 1;
                } else {
                    moved.insert(key(listed));
                    edits.add(new);
                    new += 1;
                }
            }
        }
        if edits.rows > MAX_EDITED_ROWS {
            return None;
        }
    }
    Some(edits.edits)
}

/// Edits with consecutive removals and additions merged, and how many rows
/// they touch.
#[derive(Default)]
struct Edits {
    edits: Vec<RowEdit>,
    rows: usize,
}

impl Edits {
    fn push(&mut self, edit: RowEdit) {
        self.rows += 1;
        self.edits.push(edit);
    }

    fn remove(&mut self, at: usize) {
        self.rows += 1;
        if let Some(RowEdit::Removed { row, count }) = self.edits.last_mut()
            && *row == at
        {
            *count += 1;
        } else {
            self.edits.push(RowEdit::Removed { row: at, count: 1 });
        }
    }

    fn add(&mut self, at: usize) {
        self.rows += 1;
        if let Some(RowEdit::Added { row, count }) = self.edits.last_mut()
            && *row + *count == at
        {
            *count += 1;
        } else {
            self.edits.push(RowEdit::Added { row: at, count: 1 });
        }
    }
}

#[cfg(test)]
#[path = "listing_refresh_tests.rs"]
mod tests;
