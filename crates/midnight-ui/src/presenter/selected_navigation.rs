//! Selection-preserving cursor navigation in displayed source order, across every panel layout.
use super::*;

impl Presenter {
    pub(super) fn move_to_selected(&mut self, pane: PaneId, backwards: bool) {
        let p = &self.panes[pane.index()];
        if self.active != pane
            || p.loading
            || !p.search.query.is_empty()
            || p.search_view_waiting()
            || (p.tree_view().is_some() && !p.tree_input_ready())
        {
            return;
        }
        if let Some(row) = p.nearest_selected_row(backwards) {
            self.focus_display_row(pane, row);
        }
    }
}

impl Pane {
    fn nearest_selected_row(&self, backwards: bool) -> Option<usize> {
        let current = self.cursor_index()?;
        let offset = self.parent_offset();
        let (first, end) = if backwards {
            (0, current.saturating_sub(offset))
        } else {
            (
                current.saturating_add(1).saturating_sub(offset),
                self.display_len(),
            )
        };
        if first >= end {
            return None;
        }
        // Acknowledged bitsets share the exact displayed basis. Sparse jumps skip 64 rows
        // per word without hashing paths or expanding a selection into individual entries.
        if self.rectangle_preview.is_none() {
            if let Some(publication) = &self.tree.publication {
                if self.tree.preview.is_empty() {
                    return nearest_bit(publication.selection.bits(), first, end, backwards)
                        .map(|position| position + offset);
                }
            } else if self.selection_preview.is_empty() && !self.selection_deferred.any() {
                if let Some(archive) = &self.archive {
                    let positions = archive
                        .selected
                        .iter()
                        .filter_map(|id| self.display_index().by_id.get(id).copied())
                        .filter(|position| (first..end).contains(position));
                    return if backwards {
                        positions.max()
                    } else {
                        positions.min()
                    }
                    .map(|position| position + offset);
                }
                if self.selected.is_empty() {
                    return None;
                }
                if Arc::ptr_eq(self.selected.basis(), self.display_index()) {
                    return nearest_bit(self.selected.bits(), first, end, backwards)
                        .map(|position| position + offset);
                }
            }
        }
        // A filtered/reordered view or a pending gesture must use its current visible marks.
        // Tree context rows are excluded by selection_visible, and '..' has no source position.
        let selected = |position: &usize| {
            self.display_entry(*position)
                .is_some_and(|entry| self.selection_visible(entry))
        };
        if backwards {
            (first..end).rev().find(selected)
        } else {
            (first..end).find(selected)
        }
        .map(|position| position + offset)
    }
}

/// First set bit in a half-open range, without wrapping or examining individual empty rows.
fn nearest_bit(bits: &[u64], first: usize, end: usize, backwards: bool) -> Option<usize> {
    if first >= end {
        return None;
    }
    let first_word = first / 64;
    let last_word = (end - 1) / 64;
    let candidate = |word: usize| {
        let mut selected = *bits.get(word)?;
        if word == first_word {
            selected &= u64::MAX << (first % 64);
        }
        if word == last_word && !end.is_multiple_of(64) {
            selected &= (1 << (end % 64)) - 1;
        }
        (selected != 0).then(|| {
            word * 64
                + if backwards {
                    63 - selected.leading_zeros() as usize
                } else {
                    selected.trailing_zeros() as usize
                }
        })
    };
    if backwards {
        (first_word..=last_word).rev().find_map(candidate)
    } else {
        (first_word..=last_word).find_map(candidate)
    }
}

#[cfg(test)]
mod tests {
    use super::nearest_bit;

    #[test]
    fn selected_navigation_bitset_masks_current_row_and_word_boundaries() {
        let bits = [1 | (1 << 63), 1 | (1 << 63), 1];
        for (first, end, previous, next) in [
            (0, 129, Some(128), Some(0)),
            (1, 128, Some(127), Some(63)),
            (64, 127, Some(64), Some(64)),
            (65, 127, None, None),
            (128, 129, Some(128), Some(128)),
            (129, 129, None, None),
        ] {
            assert_eq!(nearest_bit(&bits, first, end, true), previous);
            assert_eq!(nearest_bit(&bits, first, end, false), next);
        }
        assert_eq!(nearest_bit(&[], 0, 1, false), None);
    }
}
