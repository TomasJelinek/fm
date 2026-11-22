//! The result list's selection (`crate::list_selection`), kept by row identity: a finished
//! row by its ID, a listed file by its place in the listing while it waits for its row
//! (its mark then follows it to the row). A new calculation of the same files selects
//! the same files again; any other new run starts with no selection.
use super::*;
use crate::list_selection::{Changed, ListOrder, ListSelection};

/// A finished row's slot: twice its place in the order found (its ID less one).
pub(crate) fn row_slot(id: u64) -> Option<usize> {
    usize::try_from(id.checked_sub(1)?).ok()?.checked_mul(2)
}
/// A listed file's slot while it waits: odd, beside the finished rows' even slots.
pub(crate) fn listed_slot(index: usize) -> Option<usize> {
    index.checked_mul(2)?.checked_add(1)
}

/// The previous run's selected files, cursor and anchor, by name below its base folder,
/// while a run of the same files lists and reads them again.
pub(crate) struct Carried {
    base: Arc<Path>,
    names: HashSet<Box<Path>>,
    cursor: Option<Box<Path>>,
    anchor: Option<Box<Path>>,
}

/// How a session shows its rows: the rows so far in the order found, then the listed
/// files without a row yet.
pub(crate) struct Shown<'a> {
    session: &'a ChecksumSession,
    finished: usize,
    /// Rows numbered from one in the order found (as the worker numbers them): a row's
    /// slot is then found from its place alone.
    dense: bool,
}
impl<'a> Shown<'a> {
    pub(crate) fn new(session: &'a ChecksumSession) -> Self {
        let rows = session.rows();
        let finished = rows.len();
        let dense = finished == 0
            || rows
                .get(finished - 1)
                .is_some_and(|row| row.id.0 == finished as u64);
        Self {
            session,
            finished,
            dense,
        }
    }
}
impl ListOrder for Shown<'_> {
    fn len(&self) -> usize {
        self.finished + self.session.pending_count()
    }
    fn slot_at(&self, index: usize) -> Option<usize> {
        if index < self.finished {
            if self.dense {
                return index.checked_mul(2);
            }
            return row_slot(self.session.rows().get(index)?.id.0);
        }
        listed_slot(self.session.planned.waiting_index(index - self.finished)?)
    }
    fn index_of(&self, slot: usize) -> Option<usize> {
        let place = slot / 2;
        if slot % 2 == 1 {
            return Some(self.finished + self.session.planned.waiting_position(place)?);
        }
        if self.dense {
            return (place < self.finished).then_some(place);
        }
        // IDs grow in the order found: search for this one.
        let rows = self.session.rows();
        let id = place as u64 + 1;
        let (mut low, mut high) = (0, self.finished);
        while low < high {
            let middle = low + (high - low) / 2;
            if rows.get(middle)?.id.0 < id {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        rows.get(low).filter(|row| row.id.0 == id).map(|_| low)
    }
    fn slots_in(&self, range: std::ops::Range<usize>, each: &mut dyn FnMut(usize)) {
        let finished_end = range.end.min(self.finished);
        for index in range.start..finished_end {
            let slot = if self.dense {
                Some(index * 2)
            } else {
                self.slot_at(index)
            };
            if let Some(slot) = slot {
                each(slot);
            }
        }
        if range.end > self.finished {
            let from = range.start.saturating_sub(self.finished);
            let count = range.end - range.start.max(self.finished);
            for index in self.session.planned.waiting_from(from).take(count) {
                if let Some(slot) = listed_slot(index) {
                    each(slot);
                }
            }
        }
    }
}

impl ChecksumSession {
    pub(crate) fn shown(&self) -> Shown<'_> {
        Shown::new(self)
    }

    /// Change the selection against the rows shown now.
    fn select<T>(&mut self, change: impl FnOnce(&mut ListSelection, &Shown<'_>) -> T) -> T {
        let mut selection = mem::take(&mut self.selection);
        let changed = change(&mut selection, &Shown::new(self));
        self.selection = selection;
        changed
    }
    /// A click on the row shown at `index` (Ctrl `toggle`, Shift `range`).
    pub(crate) fn click_row(&mut self, index: usize, toggle: bool, range: bool) -> Option<Changed> {
        self.select(|selection, shown| selection.click(shown, index, toggle, range))
    }
    /// A cursor key reached the row shown at `index` (Shift `extend`, Ctrl `keep`).
    pub(crate) fn move_cursor(
        &mut self,
        index: usize,
        extend: bool,
        keep: bool,
    ) -> Option<Changed> {
        self.select(|selection, shown| selection.move_to(shown, index, extend, keep))
    }
    /// Space (`advance` false) or Insert (`advance` true).
    pub(crate) fn toggle_cursor(&mut self, advance: bool) -> Option<Changed> {
        self.select(|selection, shown| selection.toggle_cursor(shown, advance))
    }
    /// Ctrl+A.
    pub(crate) fn select_all_rows(&mut self) -> Changed {
        self.select(|selection, shown| selection.select_all(shown))
    }
    /// Ctrl+D.
    pub(crate) fn deselect_all_rows(&mut self) -> Changed {
        self.select(|selection, shown| selection.clear(shown))
    }
    /// Where the cursor's row is shown now.
    pub(crate) fn cursor_row(&self) -> Option<usize> {
        self.selection.cursor_index(&self.shown())
    }
    /// The slot of the row a list callback names by its token: a finished row
    /// ("generation:id") or a listed file ("generation:listed:place") of the shown run.
    pub(crate) fn token_slot(&self, token: &str) -> Option<usize> {
        let (generation, rest) = token.split_once(':')?;
        if generation.parse::<u64>().ok()? != self.generation.0 {
            return None;
        }
        if let Some(place) = rest.strip_prefix("listed:") {
            let place = place.parse::<usize>().ok()?;
            self.planned.waiting_position(place)?;
            return listed_slot(place);
        }
        let id = rest.parse::<u64>().ok()?;
        self.rows().by_id(id)?;
        row_slot(id)
    }
    /// The selected rows in the order shown, then how many selected files still wait.
    pub(crate) fn selected_rows(&self) -> (Vec<ChecksumRowRef<'_>>, u64) {
        let rows = self.rows();
        let mut found = Vec::with_capacity(self.selection.len());
        let mut waiting = 0u64;
        for slot in self.selection.slots() {
            if slot.is_multiple_of(2) {
                if let Some(row) = rows.by_id(slot as u64 / 2 + 1) {
                    found.push(row);
                }
            } else if self.planned.waiting_position(slot / 2).is_some() {
                waiting += 1;
            }
        }
        (found, waiting)
    }
    /// How many selected rows a checksum file takes (calculated ones), and how many have
    /// a checksum to copy (calculated or verified ones).
    pub(crate) fn selection_counts(&self) -> (u64, u64) {
        let rows = self.rows();
        let (mut lines, mut hashes) = (0, 0);
        for slot in self.selection.slots().filter(|slot| slot.is_multiple_of(2)) {
            let Some(row) = rows.by_id(slot as u64 / 2 + 1) else {
                continue;
            };
            match row.state {
                ChecksumRowState::Calculated if !row.digests.is_empty() => {
                    lines += 1;
                    hashes += 1;
                }
                ChecksumRowState::Verified | ChecksumRowState::Mismatch
                    if !row.digests.is_empty() =>
                {
                    hashes += 1;
                }
                _ => {}
            }
        }
        (lines, hashes)
    }

    /// A new run starts. Of the same files it selects the same files again, by name, as
    /// it lists and reads them; otherwise the selection, cursor and anchor go.
    pub(super) fn restart_selection(&mut self, same_files: bool) {
        self.carried = same_files.then(|| self.carry()).flatten();
        self.selection.reset();
    }
    fn carry(&self) -> Option<Carried> {
        let rows = self.rows();
        let base: Arc<Path> = rows
            .chunks()
            .first()
            .map(|chunk| chunk.base.clone())
            .or_else(|| self.planned.base().map(Arc::from))?;
        let name_of = |slot: usize| -> Option<Box<Path>> {
            if slot.is_multiple_of(2) {
                let row = rows.by_id(slot as u64 / 2 + 1)?;
                (*row.base == *base).then(|| row.name.clone())
            } else {
                self.planned.file(slot / 2).map(|file| file.name.clone())
            }
        };
        let names: HashSet<_> = self.selection.slots().filter_map(name_of).collect();
        let cursor = self.selection.cursor().and_then(name_of);
        let anchor = self.selection.anchor().and_then(name_of);
        (!names.is_empty() || cursor.is_some()).then_some(Carried {
            base,
            names,
            cursor,
            anchor,
        })
    }
    /// Files just listed: those the previous run had selected are selected again.
    pub(super) fn select_carried_listed(&mut self, added: std::ops::Range<usize>) {
        let Some(carried) = self.carried.as_mut() else {
            return;
        };
        if self.planned.base() != Some(&*carried.base) {
            return;
        }
        for index in added {
            let (Some(file), Some(slot)) = (self.planned.file(index), listed_slot(index)) else {
                continue;
            };
            if carried.names.remove(&file.name) {
                self.selection.set(slot, true);
            }
            if carried.cursor.as_ref() == Some(&file.name) {
                self.selection.set_cursor(Some(slot));
            }
            if carried.anchor.as_ref() == Some(&file.name) {
                self.selection.set_anchor(Some(slot));
            }
        }
    }
    /// A row arrived: a listed file's mark, cursor and anchor follow it to its row; a
    /// row for an entry the listing left out is selected again if it was before.
    pub(super) fn row_arrived(&mut self, base: &Path, row: &ChecksumRow, listed: Option<usize>) {
        let Some(slot) = row_slot(row.id.0) else {
            return;
        };
        if let Some(from) = listed.and_then(listed_slot) {
            self.selection.rename(from, slot);
            return;
        }
        let Some(carried) = self
            .carried
            .as_mut()
            .filter(|carried| *carried.base == *base)
        else {
            return;
        };
        if carried.names.remove(&row.name) {
            self.selection.set(slot, true);
        }
        if carried.cursor.as_ref() == Some(&row.name) {
            self.selection.set_cursor(Some(slot));
        }
        if carried.anchor.as_ref() == Some(&row.name) {
            self.selection.set_anchor(Some(slot));
        }
    }
    /// The run finished: listed files that got no row leave the selection, and what the
    /// previous run had selected is no longer looked for.
    pub(super) fn finish_selection(&mut self) -> Option<Carried> {
        let waiting: Vec<_> = self
            .selection
            .slots()
            .filter(|slot| slot % 2 == 1)
            .collect();
        for slot in waiting {
            self.selection.set(slot, false);
        }
        self.carried.take()
    }
}

#[cfg(test)]
#[path = "selection_tests.rs"]
mod tests;
