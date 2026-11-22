//! The Find window's multi-selection of results: the shared list model
//! ([`crate::list_selection::ListSelection`]) kept by result identity over the order the
//! window shows, and the copy actions that act on it. A result's slot is its dense ID
//! minus one, which is also its index in `FindSession::results`; sorting and streamed
//! results only change where a slot is shown.
use super::*;
use crate::list_selection::ListOrder;

/// What a click, a key or the context menu asks of a Find window's selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FindSelect {
    /// A click on a result: it alone, toggled (`toggle`, Ctrl) or the range from the
    /// anchor (`range`, Shift; added to the selection with Ctrl).
    Click {
        result: FindResultId,
        toggle: bool,
        range: bool,
    },
    /// A cursor key reached a result: it alone, the range from the anchor (`extend`,
    /// Shift) or only the cursor (`keep`, Ctrl).
    Move {
        result: FindResultId,
        extend: bool,
        keep: bool,
    },
    /// Space (`advance` false) or Insert: toggle the cursor's result.
    Toggle { advance: bool },
    /// The context menu opens on a result: a selection that holds it stays, otherwise
    /// the result is selected alone, so the menu acts on what is shown selected.
    Menu { result: FindResultId },
    /// Ctrl+A: every result shown.
    All,
    /// Ctrl+D: no result; the cursor stays.
    Clear,
}

/// What a copy action puts on the clipboard, one line per result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FindCopy {
    /// File and folder names.
    Names,
    /// Full paths, as the search found them.
    Paths,
    /// The row's columns, separated by tabs: name, extension, folder, size and modified
    /// time (with the identical-files group first when the search found duplicates).
    Details,
}

/// The order a Find window shows its results in: the backend's sort of the results it
/// had sorted (`FindOrder`), then the results streamed in after that sort, in the order
/// they were found. `find_windows::Rows` shows rows by the same rule.
pub(crate) struct Shown<'a> {
    results: usize,
    order: Option<&'a FindOrder>,
}

impl<'a> Shown<'a> {
    pub(crate) fn of(session: &'a FindSession) -> Self {
        Self {
            results: session.results.len(),
            order: session.order.as_deref(),
        }
    }
}

impl ListOrder for Shown<'_> {
    fn len(&self) -> usize {
        self.results
    }
    fn slot_at(&self, index: usize) -> Option<usize> {
        if index >= self.results {
            return None;
        }
        let id = self
            .order
            .and_then(|order| order.ids.get(index))
            .map_or(index as u64 + 1, |id| id.0);
        usize::try_from(id.checked_sub(1)?)
            .ok()
            .filter(|slot| *slot < self.results)
    }
    fn index_of(&self, slot: usize) -> Option<usize> {
        if slot >= self.results {
            return None;
        }
        let index = self
            .order
            .and_then(|order| order.positions.get(slot))
            .copied()
            .unwrap_or(slot);
        (index < self.results).then_some(index)
    }
}

/// The slot of a result ID.
pub(crate) fn slot(result: FindResultId) -> Option<usize> {
    usize::try_from(result.0.checked_sub(1)?).ok()
}

/// The result a window row token (`generation:id`) names in this session's current
/// generation, as its slot.
fn token_slot(s: &FindSession, token: &str) -> Option<usize> {
    let (generation, result) = token.split_once(':')?;
    if generation.parse::<u64>().ok()? != s.generation.0 {
        return None;
    }
    let slot = slot(FindResultId(result.parse().ok()?))?;
    s.results
        .get(slot)
        .is_some_and(|found| found.id.0 == slot as u64 + 1)
        .then_some(slot)
}

impl FindSession {
    /// Forget the selection: the window shows another search's results now.
    pub(super) fn reset_selection(&mut self) {
        self.selection.reset();
        self.selection_bytes = 0;
        self.selection_revision = self.selection_revision.wrapping_add(1);
    }

    /// The bytes of the selected files, for the status line; folders have no size here.
    fn summarize_selection(&mut self) {
        let results = &self.results;
        self.selection_bytes = self
            .selection
            .slots()
            .filter_map(|slot| results.get(slot))
            .filter(|result| result.entry.kind == EntryKind::File)
            .filter_map(|result| result.entry.metadata.size)
            .fold(0u64, u64::saturating_add);
    }

    /// Whether the selection is more than the cursor's result: Feed to panel then marks
    /// the selected results in the panel. A selection that only follows the cursor marks
    /// nothing there, so the panel's own cursor keeps acting on what it focuses.
    pub(crate) fn selection_is_explicit(&self) -> bool {
        self.selection.len() > 1
            || self
                .selection
                .slots()
                .next()
                .is_some_and(|slot| Some(slot) != self.selection.cursor())
    }

    /// The results an action on the result in `slot` acts on, in the order shown: the
    /// whole selection when that result is part of it, otherwise that result alone.
    pub(crate) fn targets(&self, slot: usize) -> Vec<&FindResult> {
        if !self.selection.is_selected(slot) {
            return self.results.get(slot).map(|r| &**r).into_iter().collect();
        }
        let shown = Shown::of(self);
        let mut targets = Vec::with_capacity(self.selection.len());
        // One pass over the shown order, with an O(1) lookup per row.
        for index in 0..shown.len() {
            if let Some(slot) = shown.slot_at(index)
                && self.selection.is_selected(slot)
                && let Some(result) = self.results.get(slot)
            {
                targets.push(&**result);
            }
        }
        targets
    }
}

/// The size column's text, as the result list shows it.
pub(crate) fn size_text(entry: &FindEntry, locale: &NumberLocale) -> String {
    if entry.kind == EntryKind::Directory {
        "<DIR>".into()
    } else {
        entry.metadata.size.map_or_else(
            || "Unknown size".into(),
            |n| format!("{} B", locale.format_u64(n)),
        )
    }
}

/// The modified column's text, as the result list shows it.
pub(crate) fn modified_text(entry: &FindEntry) -> String {
    crate::time_presentation::modified(&entry.metadata)
        .map(|time| format!("{} {}", time.date, time.time))
        .unwrap_or_default()
}

/// The identical-files group column's text.
pub(crate) fn group_text(result: &FindResult) -> String {
    result
        .duplicate_group
        .map(|group| format!("Group {group} · {} files", result.duplicate_group_size))
        .unwrap_or_default()
}

/// The clipboard text of `copy` for `targets`, built in one pass. Names and paths are
/// the raw ones the search found, never the escaped text a row shows.
fn copy_text(
    targets: &[&FindResult],
    copy: FindCopy,
    duplicates: bool,
    locale: &NumberLocale,
) -> String {
    let mut text = String::new();
    for (line, result) in targets.iter().enumerate() {
        let entry = &result.entry;
        if line > 0 {
            text.push('\n');
        }
        let name = || {
            entry
                .path
                .file_name()
                .unwrap_or(entry.path.as_os_str())
                .to_string_lossy()
        };
        match copy {
            FindCopy::Names => text.push_str(&name()),
            FindCopy::Paths => text.push_str(&entry.path.to_string_lossy()),
            FindCopy::Details => {
                if duplicates {
                    text.push_str(&group_text(result));
                    text.push('\t');
                }
                text.push_str(&name());
                text.push('\t');
                if let Some(extension) = entry.path.extension() {
                    text.push_str(&extension.to_string_lossy());
                }
                text.push('\t');
                text.push_str(&entry.path.parent().unwrap_or(&entry.path).to_string_lossy());
                text.push('\t');
                text.push_str(&size_text(entry, locale));
                text.push('\t');
                text.push_str(&modified_text(entry));
            }
        }
    }
    text
}

impl Presenter {
    /// Apply a click, key or menu request to a Find window's selection. Returns whether
    /// the selection, its anchor or the cursor may have changed.
    pub(crate) fn find_select(&mut self, id: FindSessionId, select: FindSelect) -> bool {
        let Some(s) = self.finds.sessions.get_mut(&id) else {
            return false;
        };
        let shown = Shown {
            results: s.results.len(),
            order: s.order.as_deref(),
        };
        let index = |result: FindResultId| slot(result).and_then(|slot| shown.index_of(slot));
        let changed = match select {
            FindSelect::Click {
                result,
                toggle,
                range,
            } => index(result)
                .and_then(|index| s.selection.click(&shown, index, toggle, range))
                .is_some(),
            FindSelect::Move {
                result,
                extend,
                keep,
            } => index(result)
                .and_then(|index| s.selection.move_to(&shown, index, extend, keep))
                .is_some(),
            FindSelect::Toggle { advance } => s.selection.toggle_cursor(&shown, advance).is_some(),
            FindSelect::Menu { result } => match (slot(result), index(result)) {
                (Some(slot), Some(index)) if s.selection.is_selected(slot) => {
                    s.selection.move_to(&shown, index, false, true).is_some()
                }
                (_, Some(index)) => s.selection.click(&shown, index, false, false).is_some(),
                _ => false,
            },
            FindSelect::All => {
                s.selection.select_all(&shown);
                true
            }
            FindSelect::Clear => {
                s.selection.clear(&shown);
                true
            }
        };
        if changed {
            s.summarize_selection();
            s.selection_revision = s.selection_revision.wrapping_add(1);
        }
        changed
    }

    /// Copy the names, paths or row details of the results an action on the row
    /// `row_token` acts on (the selection when that row is part of it, else the row).
    pub(crate) fn find_copy(
        &mut self,
        id: FindSessionId,
        row_token: &str,
        copy: FindCopy,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let Some(s) = self.finds.sessions.get(&id) else {
            return false;
        };
        let Some(slot) = token_slot(s, row_token) else {
            return false;
        };
        let text = copy_text(&s.targets(slot), copy, s.duplicates, &self.number_locale);
        if text.is_empty() {
            return false;
        }
        self.queue_clipboard_text(text, send)
    }
}

#[cfg(test)]
#[path = "find_selection_tests.rs"]
mod tests;
