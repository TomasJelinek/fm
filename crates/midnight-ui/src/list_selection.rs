//! Multi-selection for result lists (Checksums, Find results): rows are kept by their
//! identity, never by their place, so a selection survives rows that change, arrive or
//! are sorted again. A cursor marks the keyboard's row and an anchor starts Shift ranges.
//!
//! The list says how it shows its rows ([`ListOrder`]); each operation returns the shown
//! rows whose mark may have changed ([`Changed`]), which the list model notifies, so
//! Select all and a range touch the model once per row, never rebuild it.
//!
//! Conventions, the same in every list that uses this module:
//! - A click selects the row alone and makes it the cursor and the anchor; Ctrl+click
//!   toggles the row; Shift+click marks the range from the anchor to the row, replacing
//!   the selection (Ctrl+Shift+click adds the range to it).
//! - Up, Down, Page Up, Page Down, Home and End select the row they reach (as a click);
//!   with Shift they mark the range from the anchor; with Ctrl they move the cursor alone.
//! - Space toggles the cursor's row; Insert toggles it and moves the cursor down.
//! - Ctrl+A selects every shown row; Ctrl+D deselects every row.
use std::ops::Range;

/// How a list shows its rows now. Each row's identity is a dense number, its *slot*: the
/// selection keeps one bit per slot, so asking whether a row is selected costs O(1).
pub(crate) trait ListOrder {
    /// How many rows are shown.
    fn len(&self) -> usize;
    /// The slot of the row shown at `index`.
    fn slot_at(&self, index: usize) -> Option<usize>;
    /// Where the row with this slot is shown now, when it is.
    fn index_of(&self, slot: usize) -> Option<usize>;
    /// Calls `each` with the slot of every row shown in `range`, in order. A list whose
    /// `slot_at` is not O(1) overrides this with one pass over its rows.
    fn slots_in(&self, range: Range<usize>, each: &mut dyn FnMut(usize)) {
        for index in range {
            if let Some(slot) = self.slot_at(index) {
                each(slot);
            }
        }
    }
}

/// Shown rows (display indices) whose mark may have changed; the list model notifies
/// each of them. Overlapping ranges are harmless.
pub(crate) type Changed = Vec<Range<usize>>;

/// One range of shown rows to notify.
pub(crate) fn one(rows: Range<usize>) -> Changed {
    vec![rows]
}

/// Above this many previously selected rows, a change that drops them reports every
/// shown row instead of finding each one.
const LISTED_CHANGES: usize = 4096;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ListSelection {
    /// One bit per slot.
    bits: Vec<u64>,
    /// Selected slots.
    count: usize,
    cursor: Option<usize>,
    anchor: Option<usize>,
}

impl ListSelection {
    pub(crate) fn is_selected(&self, slot: usize) -> bool {
        self.bits
            .get(slot / 64)
            .is_some_and(|word| word & (1 << (slot % 64)) != 0)
    }
    /// How many rows are selected.
    pub(crate) fn len(&self) -> usize {
        self.count
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.count == 0
    }
    /// The cursor's slot.
    pub(crate) fn cursor(&self) -> Option<usize> {
        self.cursor
    }
    /// The anchor's slot.
    #[allow(dead_code, reason = "the checksum results use it; Find does not")]
    pub(crate) fn anchor(&self) -> Option<usize> {
        self.anchor
    }
    /// Where the cursor's row is shown now.
    pub(crate) fn cursor_index(&self, order: &impl ListOrder) -> Option<usize> {
        self.cursor.and_then(|slot| order.index_of(slot))
    }
    /// The bits of the selection, one per slot from slot 0 (a snapshot for a copy).
    pub(crate) fn words(&self) -> &[u64] {
        &self.bits
    }
    /// Bounded copy for backend requests (Find holds at most 100,000 slots).
    pub(crate) fn bits_snapshot(&self) -> std::sync::Arc<[u64]> {
        self.bits.clone().into()
    }
    /// How many selected slots `mask` (bits by slot) also has: word operations only.
    pub(crate) fn count_in(&self, mask: &[u64]) -> usize {
        self.bits
            .iter()
            .zip(mask)
            .map(|(bits, mask)| (bits & mask).count_ones() as usize)
            .sum()
    }
    /// The selected slots, in ascending order.
    pub(crate) fn slots(&self) -> impl Iterator<Item = usize> + '_ {
        self.bits.iter().enumerate().flat_map(|(word, bits)| {
            let mut remaining = *bits;
            std::iter::from_fn(move || {
                let bit = (remaining != 0).then(|| remaining.trailing_zeros() as usize)?;
                remaining &= remaining - 1;
                Some(word * 64 + bit)
            })
        })
    }
    /// Mark or unmark one slot; returns whether its mark changed.
    pub(crate) fn set(&mut self, slot: usize, selected: bool) -> bool {
        let (word, bit) = (slot / 64, 1u64 << (slot % 64));
        if word >= self.bits.len() {
            if !selected {
                return false;
            }
            self.bits.resize(word + 1, 0);
        }
        let was = self.bits[word] & bit != 0;
        if was == selected {
            return false;
        }
        if selected {
            self.bits[word] |= bit;
            self.count += 1;
        } else {
            self.bits[word] &= !bit;
            self.count -= 1;
        }
        true
    }
    /// A row's identity changed (a listed file got its row): its mark, and the cursor
    /// and anchor on it, follow it.
    #[allow(dead_code, reason = "the checksum results use it; Find does not")]
    pub(crate) fn rename(&mut self, from: usize, to: usize) {
        if self.set(from, false) {
            self.set(to, true);
        }
        for slot in [&mut self.cursor, &mut self.anchor] {
            if *slot == Some(from) {
                *slot = Some(to);
            }
        }
    }
    /// Put the cursor on a row by its slot (None: no cursor), without changing marks.
    #[allow(dead_code, reason = "the checksum results use it; Find does not")]
    pub(crate) fn set_cursor(&mut self, slot: Option<usize>) {
        self.cursor = slot;
    }
    /// Put the anchor on a row by its slot, without changing marks.
    #[allow(dead_code, reason = "the checksum results use it; Find does not")]
    pub(crate) fn set_anchor(&mut self, slot: Option<usize>) {
        self.anchor = slot;
    }
    /// Forget the selection, the cursor and the anchor: the list shows other rows now.
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }

    /// A click on the row shown at `index`: the row alone, or toggled (`toggle`, Ctrl), or
    /// the range from the anchor (`range`, Shift), added to the selection with Ctrl. The
    /// row becomes the cursor; it becomes the anchor unless it ends a range.
    pub(crate) fn click(
        &mut self,
        order: &impl ListOrder,
        index: usize,
        toggle: bool,
        range: bool,
    ) -> Option<Changed> {
        let slot = order.slot_at(index)?;
        let changed = if range {
            let anchor = self.anchor.and_then(|anchor| order.index_of(anchor));
            if anchor.is_none() {
                self.anchor = Some(slot);
            }
            self.mark_range(order, anchor.unwrap_or(index), index, toggle)
        } else if toggle {
            let selected = !self.is_selected(slot);
            self.set(slot, selected);
            self.anchor = Some(slot);
            one(index..index + 1)
        } else {
            self.anchor = Some(slot);
            self.only(order, index, slot)
        };
        self.cursor = Some(slot);
        Some(changed)
    }

    /// A cursor key reached the row shown at `index`: select it alone, or mark the range
    /// from the anchor (`extend`, Shift; added to the selection with `keep`), or move the
    /// cursor alone (`keep`, Ctrl).
    pub(crate) fn move_to(
        &mut self,
        order: &impl ListOrder,
        index: usize,
        extend: bool,
        keep: bool,
    ) -> Option<Changed> {
        let slot = order.slot_at(index)?;
        let changed = if extend {
            // Without a shown anchor the range starts at the cursor's row, which becomes
            // the anchor.
            let shown = |slot: Option<usize>| slot.and_then(|slot| order.index_of(slot));
            let anchor = shown(self.anchor).or(shown(self.cursor)).unwrap_or(index);
            if shown(self.anchor).is_none() {
                self.anchor = order.slot_at(anchor);
            }
            self.mark_range(order, anchor, index, keep)
        } else if keep {
            Vec::new()
        } else {
            self.anchor = Some(slot);
            self.only(order, index, slot)
        };
        self.cursor = Some(slot);
        Some(changed)
    }

    /// Space (`advance` false) or Insert (`advance` true): toggle the cursor's row, the
    /// first row when there is no cursor; Insert then moves the cursor one row down.
    pub(crate) fn toggle_cursor(
        &mut self,
        order: &impl ListOrder,
        advance: bool,
    ) -> Option<Changed> {
        let index = self.cursor_index(order).unwrap_or(0);
        let slot = order.slot_at(index)?;
        let selected = !self.is_selected(slot);
        self.set(slot, selected);
        self.anchor = Some(slot);
        self.cursor = Some(slot);
        if advance && let Some(next) = order.slot_at(index + 1) {
            self.cursor = Some(next);
        }
        Some(one(index..index + 1))
    }

    /// Ctrl+A: every shown row.
    pub(crate) fn select_all(&mut self, order: &impl ListOrder) -> Changed {
        let shown = order.len();
        order.slots_in(0..shown, &mut |slot| {
            self.set(slot, true);
        });
        one(0..shown)
    }

    /// Ctrl+D: no row. The cursor and the anchor stay.
    pub(crate) fn clear(&mut self, order: &impl ListOrder) -> Changed {
        let changed = self.marked_rows(order);
        self.bits.clear();
        self.count = 0;
        changed
    }

    /// Select only the row `slot`, shown at `index`.
    fn only(&mut self, order: &impl ListOrder, index: usize, slot: usize) -> Changed {
        let mut changed = self.marked_rows(order);
        self.bits.clear();
        self.count = 0;
        self.set(slot, true);
        changed.push(index..index + 1);
        changed
    }

    /// Mark the rows shown from `from` to `to` (either order), adding them to the
    /// selection or (without `add`) replacing it.
    fn mark_range(&mut self, order: &impl ListOrder, from: usize, to: usize, add: bool) -> Changed {
        let range = from.min(to)..from.max(to) + 1;
        let mut changed = if add {
            Vec::new()
        } else {
            let old = self.marked_rows(order);
            self.bits.clear();
            self.count = 0;
            old
        };
        order.slots_in(range.clone(), &mut |slot| {
            self.set(slot, true);
        });
        changed.push(range);
        changed
    }

    /// The shown rows that are marked now, as ranges; every shown row when there are many.
    fn marked_rows(&self, order: &impl ListOrder) -> Changed {
        if self.count > LISTED_CHANGES {
            return one(0..order.len());
        }
        let mut indices: Vec<usize> = self
            .slots()
            .filter_map(|slot| order.index_of(slot))
            .collect();
        indices.sort_unstable();
        let mut changed: Changed = Vec::new();
        for index in indices {
            match changed.last_mut() {
                Some(last) if last.end == index => last.end += 1,
                _ => changed.push(index..index + 1),
            }
        }
        changed
    }
}

#[cfg(test)]
#[path = "list_selection_tests.rs"]
mod tests;
