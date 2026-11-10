//! Latest desired tree query; matching and ordered match positions belong to the backend.
use super::*;

/// One Insert gesture over the exact matching occurrence, surviving only its clear transaction.
#[derive(Clone, Copy)]
pub(super) struct Insert {
    stamp: TreeViewStamp,
    row: TreeRowId,
    requested: bool,
}

impl Insert {
    pub(super) fn visible_in(self, view: &TreeProjection) -> bool {
        self.stamp.pane == view.stamp.pane
            && self.stamp.navigation == view.stamp.navigation
            && self.stamp.session == view.stamp.session
            && view
                .index
                .row_positions
                .get(&self.row)
                .and_then(|position| view.rows.get(*position))
                .is_some_and(|row| row.selectable && !row.context_only)
    }
}

impl Pane {
    pub(super) fn tree_search_options(&self) -> TreeSearchOptions {
        TreeSearchOptions {
            query: self.search.query.clone(),
            filter_matches: self.search.filter,
        }
    }

    pub(super) fn tree_search_current(&self) -> bool {
        let applied = &self.tree.options.applied_search;
        !self.search.pending
            && !self.search.failed
            && applied.query == self.search.query
            && (applied.query.is_empty() || applied.filter_matches == self.search.filter)
    }

    pub(super) fn tree_search_cursor_matches(&self) -> bool {
        self.tree_cursor_row()
            .is_some_and(|row| row.search_match && row.selectable && !row.context_only)
    }

    pub(super) fn tree_cursor_row(&self) -> Option<&TreeDisplayRow> {
        if self.parent_cursor {
            return None;
        }
        let view = self.tree_view()?;
        let row = view.index.directory.by_id.get(&self.cursor?)?;
        view.rows.get(*row)
    }
}

impl Presenter {
    pub(super) fn change_tree_search_query(
        &mut self,
        pane: PaneId,
        query: &str,
        send: &dyn Fn(Command) -> bool,
    ) {
        if query.len() > TreeSearchOptions::MAX_QUERY_BYTES || query.contains('\0') {
            self.panes[pane.index()].set_error(
                "Search text exceeds its budget or contains NUL.",
                &mut self.retired,
            );
            return;
        }
        let Some(revision) = self.next_search_revision(pane) else {
            return;
        };
        self.cancel_selection_actions(pane);
        let p = &mut self.panes[pane.index()];
        p.tree.search_insert = if query.is_empty() && p.tree_search_current() {
            p.tree_cursor_row()
                .filter(|row| row.search_match && row.selectable && !row.context_only)
                .zip(p.tree_view())
                .map(|(row, view)| Insert {
                    stamp: view.stamp,
                    row: row.id,
                    requested: false,
                })
        } else {
            None
        };
        if p.search.failed {
            p.set_error("", &mut self.retired);
        }
        self.retired.push(Box::new(mem::replace(
            &mut p.search.query,
            query.to_owned(),
        )));
        p.search.revision = revision;
        p.search.filter = self.preferences.values.quick_search_filter;
        p.search.pending = true;
        p.search.needs_request = true;
        p.search.choose_first = !query.is_empty();
        p.search.failed = false;
        if query.is_empty() && p.tree.options.applied_search.query.is_empty() {
            // A never-admitted query has no matching Insert target. Keep its
            // local clear fenced until the next pump, so Slint's immediate
            // key9 cannot toggle the old, nonmatching caret.
            return;
        }
        self.flush_search(send);
    }

    /// Settle only the latest desired query; an older receipt must not move its caret.
    pub(super) fn tree_search_applied(&mut self, pane: PaneId, applied: TreeSearchOptions) {
        let p = &mut self.panes[pane.index()];
        let current = applied.query == p.search.query
            && (applied.query.is_empty() || applied.filter_matches == p.search.filter);
        self.retired.push(Box::new(mem::replace(
            &mut p.tree.options.applied_search,
            applied,
        )));
        p.search.pending = !current;
        p.search.needs_request = !current;
        if !current {
            return;
        }
        if p.tree.search_insert.is_some_and(|insert| !insert.requested) {
            p.tree.search_insert = None;
        }
        if p.search.query.is_empty()
            && let Some(row) = p
                .tree
                .search_cursor
                .and_then(|id| {
                    let view = p.tree_view()?;
                    view.index
                        .row_positions
                        .get(&id)
                        .and_then(|position| view.rows.get(*position))
                })
                .filter(|row| row.selectable && !row.context_only)
        {
            p.cursor = Some(row.entry.id);
            p.parent_cursor = false;
        }
        if !p.search.query.is_empty()
            && (p.search.choose_first || (p.search.filter && !p.tree_search_cursor_matches()))
            && let Some(row) = p.tree_view().and_then(|view| {
                view.index
                    .search_rows
                    .first()
                    .and_then(|row| view.rows.get(*row))
            })
        {
            p.cursor = Some(row.entry.id);
            p.parent_cursor = false;
            p.preserve_viewport = false;
        }
        p.search.choose_first = false;
        p.search.applied = p.search.revision;
        p.search.failed = false;
    }

    /// Slint clears the query before delivering Insert's existing panel action.
    pub(super) fn defer_tree_search_insert(&mut self, pane: PaneId) -> bool {
        let p = &mut self.panes[pane.index()];
        if p.tree.search_insert.is_some_and(|insert| insert.requested) {
            return true;
        }
        if p.tree_view().is_none() || !p.search.query.is_empty() || !p.search.pending {
            return false;
        }
        if let Some(insert) = &mut p.tree.search_insert {
            insert.requested = true;
        }
        true
    }

    /// Retain the intent across queue pressure, but admit it only on the committed empty view.
    pub(super) fn flush_tree_search_insert(
        &mut self,
        pane: PaneId,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let p = &mut self.panes[pane.index()];
        let Some(insert) = p.tree.search_insert.filter(|insert| insert.requested) else {
            return false;
        };
        if !p.search.query.is_empty() || p.tree_view().is_none_or(|view| !insert.visible_in(view)) {
            p.tree.search_insert = None;
            return false;
        }
        if !p.tree_input_ready()
            || !p.tree_search_current()
            || p.tree.revision != p.tree.resolved
            || !p.tree.preview.is_empty()
        {
            return true;
        }
        let row = p
            .tree_view()
            .expect("validated Insert projection")
            .index
            .row_positions[&insert.row];
        let target = row + p.parent_offset();
        if self.send_tree_selection(pane, TreeSelectionAction::Toggle(insert.row), send) {
            self.panes[pane.index()].tree.search_insert = None;
            self.focus_display_row(pane, target.saturating_add(1));
        }
        true
    }

    pub(super) fn move_tree_search_cursor(&mut self, pane: PaneId, key: i32) {
        let p = &mut self.panes[pane.index()];
        if !p.tree_input_ready() || !p.tree_search_current() {
            return;
        }
        let Some(view) = p.tree_view() else {
            return;
        };
        let matches = &view.index.search_rows;
        if matches.is_empty() {
            return;
        }
        let row = p
            .cursor
            .and_then(|id| view.index.directory.by_id.get(&id))
            .copied();
        let insertion = row.map_or(0, |row| matches.partition_point(|position| *position < row));
        let exact = row.is_some_and(|row| matches.get(insertion) == Some(&row));
        let last = matches.len() - 1;
        let target = match key {
            2 => insertion.saturating_sub(1),
            3 => (insertion + usize::from(exact)).min(last),
            4 => insertion.saturating_sub(20),
            5 => (insertion + 20).min(last),
            6 => 0,
            7 => last,
            _ => return,
        };
        p.cursor = Some(view.rows[matches[target]].entry.id);
        p.parent_cursor = false;
        p.preserve_viewport = false;
    }
}
