//! The quick (filename prefix) search only moves the cursor to matches. It never
//! blocks or delays a command: a command closes it and acts on the selection or the
//! focused row that the panel shows, as if no search had been open. Enter differs:
//! it does not open a focused row that the typed name does not match.
use super::*;

/// A short non-modal note in a pane's status row about something Enter or a command
/// did not do. It lasts while the pane keeps its folder, query and cursor.
pub(crate) struct SearchNote {
    text: String,
    generation: u64,
    query: String,
    cursor: Option<EntryId>,
    parent_cursor: bool,
    /// Shown only while the search for `query` is still computing.
    while_pending: bool,
}

impl Pane {
    fn search_note(&self) -> Option<&str> {
        let note = self.search.note.as_ref()?;
        (note.generation == self.generation
            && note.query == self.search.query
            && note.cursor == self.cursor
            && note.parent_cursor == self.parent_cursor
            && (!note.while_pending || self.search.pending))
            .then_some(note.text.as_str())
    }

    /// Whether the shown result's persistent-filter rows were computed with the current
    /// filter for these entries, so they stay valid without the query.
    pub(super) fn keeps_filter_rows(&self) -> bool {
        self.search.filter_rows_current && self.search_projection().is_some()
    }

    /// What the backend computes for the persistent filter with an empty query: the
    /// filter's rows, each a match without a highlighted prefix.
    fn kept_filter_rows(&self) -> Option<SearchResult> {
        let result = self
            .search_projection()
            .filter(|_| self.search.filter_rows_current)?;
        Some(SearchResult {
            source: result.source.clone(),
            matches: result.filtered.clone(),
            index: result.filtered_index.clone(),
            filtered: result.filtered.clone(),
            filtered_index: result.filtered_index.clone(),
        })
    }
}

impl Presenter {
    /// Run a command on `pane`'s selection or focused row. A quick search that is still
    /// computing is cancelled first, so its late result cannot move the cursor and the
    /// command acts on the row focused when it was invoked. The command then acts on
    /// what the panel shows, and an open quick search closes, keeping the cursor.
    ///
    /// An inline tree keeps its projection until the command captured its targets:
    /// closing its search reprojects the tree, which would replace the captured view.
    pub fn panel_command<R>(
        &mut self,
        pane: PaneId,
        send: &dyn Fn(Command) -> bool,
        command: impl FnOnce(&mut Self) -> R,
    ) -> R {
        self.clear_search_note(pane);
        self.cancel_computing_search(pane, send);
        let result = command(self);
        self.request_quick_search_close(pane, send);
        result
    }

    /// Run a command on a clicked row (double-click, context menu, drag). The row index
    /// names a row of the shown list, so nothing may change that list first.
    pub fn panel_row_command<R>(
        &mut self,
        pane: PaneId,
        send: &dyn Fn(Command) -> bool,
        command: impl FnOnce(&mut Self) -> R,
    ) -> R {
        self.clear_search_note(pane);
        let result = command(self);
        self.request_quick_search_close(pane, send);
        result
    }

    /// F3 and Alt+F3: the focused file's viewer launch, or why there is none. `None`
    /// declines silently (a focused folder or the parent row). After an archive's
    /// password, it is the member that F3 named when it asked (`take_archive_view`).
    pub fn view_command(
        &mut self,
        pane: PaneId,
        basic: bool,
        send: &dyn Fn(Command) -> bool,
    ) -> Result<ViewerLaunch, Option<String>> {
        self.panel_command(pane, send, |p| {
            if let Some(launch) = p.take_unlocked_view(pane, basic) {
                return Ok(launch);
            }
            p.viewer_launch(pane, basic)
                .ok_or_else(|| p.viewer_unavailable_reason(pane))
        })
    }

    /// Compare two files, starting from the active pane.
    pub fn compare_command(
        &mut self,
        active: PaneId,
        send: &dyn Fn(Command) -> bool,
    ) -> Result<FileCompareLaunch, String> {
        self.panel_command(active, send, |p| p.file_compare_launch(active))
    }

    /// Enter: open the focused folder or file. A query that does not match the focused
    /// row closes the search and opens nothing: the user meant to reach a match, and
    /// the old row could start a program they did not mean.
    pub(crate) fn activate_focused(
        &mut self,
        pane: PaneId,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        if !self.focused_row_entered(pane, send) {
            return false;
        }
        let Some(row) = self.panes[pane.index()].cursor_index() else {
            return false;
        };
        self.open(pane, row, send)
    }

    /// Ctrl+PageDown: enter the focused folder, never a file. It follows Enter's rule.
    pub(crate) fn enter_focused_folder(
        &mut self,
        pane: PaneId,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        self.focused_row_entered(pane, send) && self.open_folder(pane, send)
    }

    fn focused_row_entered(&mut self, pane: PaneId, send: &dyn Fn(Command) -> bool) -> bool {
        self.clear_search_note(pane);
        let p = &self.panes[pane.index()];
        if !p.search.query.is_empty() {
            let query = midnight_core::visible_name(&p.search.query).into_owned();
            if p.search.pending {
                self.note_search(pane, format!("Still searching for “{query}”…"), true);
                return false;
            }
            if !p.focus_matches_search() {
                // A tree that is still loading keeps its own state.
                if p.tree_view().is_some() && !p.tree_input_ready() {
                    return false;
                }
                self.close_quick_search(pane, send);
                self.note_search(pane, format!("No name matches “{query}”"), false);
                return false;
            }
        }
        let p = &self.panes[pane.index()];
        if p.focus_actionable() {
            return true;
        }
        if p.tree_view().is_none() && p.rows_settled() && p.search.panel_filter.enabled {
            self.note_search(pane, FILTER_HIDES_FOCUS.into(), false);
        }
        false
    }

    /// Close `pane`'s quick search now, keeping the cursor where it is shown.
    pub(crate) fn close_quick_search(&mut self, pane: PaneId, send: &dyn Fn(Command) -> bool) {
        let p = &mut self.panes[pane.index()];
        p.search.close_requested = false;
        if p.search.query.is_empty() {
            return;
        }
        if p.tree_view().is_some() {
            // The reprojection keeps the focused occurrence (`tree_search_applied`).
            self.change_tree_search_query(pane, "", send);
        } else {
            self.close_flat_quick_search(pane, send);
        }
    }

    /// A pane's note for its status row, shown while it still applies.
    pub(crate) fn search_note(&self, pane: PaneId) -> Option<&str> {
        self.panes[pane.index()].search_note()
    }

    /// Explain that the panel filter hides the focused row; commands then have no row
    /// to act on. Returns whether it does.
    pub(super) fn note_filter_hides_focus(&mut self, pane: PaneId) -> bool {
        let p = &self.panes[pane.index()];
        let hidden = p.tree_view().is_none()
            && p.search.panel_filter.enabled
            && p.rows_settled()
            && !p.focus_actionable();
        if hidden {
            self.note_search(pane, FILTER_HIDES_FOCUS.into(), false);
        }
        hidden
    }

    fn clear_search_note(&mut self, pane: PaneId) {
        if let Some(note) = self.panes[pane.index()].search.note.take() {
            self.retired.push(Box::new(note));
        }
    }

    fn note_search(&mut self, pane: PaneId, text: String, while_pending: bool) {
        let p = &mut self.panes[pane.index()];
        let note = SearchNote {
            text,
            generation: p.generation,
            query: p.search.query.clone(),
            cursor: p.cursor,
            parent_cursor: p.parent_cursor,
            while_pending,
        };
        if let Some(old) = p.search.note.replace(note) {
            self.retired.push(Box::new(old));
        }
    }

    /// A flat panel's search still computing (or failed) has no settled matches: stop
    /// it before a command looks at the panel. An inline tree's search keeps the tree
    /// busy until its reprojection arrives; the tree reports that itself.
    fn cancel_computing_search(&mut self, pane: PaneId, send: &dyn Fn(Command) -> bool) {
        let p = &self.panes[pane.index()];
        if self.feed(pane).is_none()
            && p.tree_view().is_none()
            && !p.search.query.is_empty()
            && (p.search.pending || p.search.failed || p.search_view_waiting())
        {
            self.close_quick_search(pane, send);
        }
    }

    fn request_quick_search_close(&mut self, pane: PaneId, send: &dyn Fn(Command) -> bool) {
        // Search results fed to the panel cover it and its own search.
        if self.feed(pane).is_some() || self.panes[pane.index()].search.query.is_empty() {
            return;
        }
        self.panes[pane.index()].search.close_requested = true;
        self.flush_quick_search_closes(send);
    }

    /// Close the quick searches that commands asked to close. A context menu or a drag
    /// still uses the view it captured: an inline tree's reprojection or a quick-filtered
    /// list that shows every row again would cancel it, so those wait until it ends.
    pub(super) fn flush_quick_search_closes(&mut self, send: &dyn Fn(Command) -> bool) {
        if self.menu_pending()
            || self.drag_drop.lifecycle.active().is_some()
            || self.drag_drop.tree.source.is_some()
        {
            return;
        }
        for pane in [PaneId::Left, PaneId::Right] {
            if self.panes[pane.index()].search.close_requested {
                self.close_quick_search(pane, send);
            }
        }
    }

    fn close_flat_quick_search(&mut self, pane: PaneId, send: &dyn Fn(Command) -> bool) {
        let Some(revision) = self.next_search_revision(pane) else {
            return;
        };
        let slot = pane.index();
        let p = &mut self.panes[slot];
        let shown = Arc::as_ptr(p.display_index());
        if p.search.failed {
            p.set_error("", &mut self.retired);
            p.search.failed = false;
        }
        self.retired.push(Box::new(mem::take(&mut p.search.query)));
        p.search.revision = revision;
        p.search.choose_first = false;
        p.search.filter = self.preferences.values.quick_search_filter;
        // The persistent filter keeps showing its rows; nothing waits for them.
        let kept = if p.search.panel_filter.enabled {
            p.kept_filter_rows()
        } else {
            None
        };
        if let Some(kept) = kept {
            self.retired
                .push(Box::new(p.search.result.replace(Arc::new(kept))));
            p.search.applied = revision;
            p.search.pending = false;
        } else {
            if !p.search.panel_filter.enabled {
                self.retired.push(Box::new(p.search.result.take()));
            }
            p.search.pending = p.search.panel_filter.enabled;
        }
        p.search.needs_request = p.search.pending;
        // Whatever the backend still computes for the closed query is superseded; its
        // late result cannot move the cursor (`apply_search_event` checks the revision).
        self.search_cancellations[slot] = Some(revision);
        if !std::ptr::eq(shown, Arc::as_ptr(p.display_index())) {
            // A quick-filtered list shows every row again; marks follow the new rows.
            p.search.view_revision = p.search.view_revision.saturating_add(1);
            p.selection_rebase_pending |= p.archive.is_none()
                && !Arc::ptr_eq(p.selected.basis(), p.display_index())
                && !(p.selected.is_empty()
                    && p.selection_preview.is_empty()
                    && !p.selection_deferred.any());
        }
        self.flush_search(send);
        self.flush_selection(send);
    }
}

const FILTER_HIDES_FOCUS: &str =
    "The panel filter hides the focused item. Focus a shown item or change the filter.";
