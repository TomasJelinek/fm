//! Small input state and immutable backend projections; matching stays off the UI thread.
use super::*;

#[derive(Default)]
pub(crate) struct SearchState {
    pub(crate) query: String,
    pub(crate) revision: SearchRevision,
    pub(crate) input_context: SearchRevision,
    pub(crate) pending: bool,
    pub(crate) filter: bool,
    pub(crate) panel_filter: PanelFilter,
    pub(crate) view_revision: u64,
    needs_request: bool,
    choose_first: bool,
    applied: SearchRevision,
    failed: bool,
    pub(super) result: Option<Arc<SearchResult>>,
}

impl Pane {
    /// The underlying native listing is retained independently of the visible projection.
    pub(crate) fn display_len(&self) -> usize {
        if self.search.filter && !self.search.query.is_empty() {
            return self
                .search_projection()
                .map_or(0, |result| result.matches.len());
        }
        if self.search.panel_filter.enabled {
            return self
                .search_projection()
                .map_or(0, |result| result.filtered.len());
        }
        self.rows().len()
    }

    pub(crate) fn display_entry(&self, row: usize) -> Option<&Entry> {
        if self.search.filter && !self.search.query.is_empty() {
            let result = self.search_projection()?;
            return result.source.get(result.matches.get(row)?.source_row);
        }
        if self.search.panel_filter.enabled {
            let result = self.search_projection()?;
            return result.source.get(result.filtered.get(row)?.source_row);
        }
        self.rows().get(row)
    }

    pub(crate) fn display_index(&self) -> &Arc<DirectoryIndex> {
        if let Some(result) = self.search_projection() {
            if self.search.filter && !self.search.query.is_empty() {
                return &result.index;
            }
            if self.search.panel_filter.enabled {
                return &result.filtered_index;
            }
        }
        &self.index
    }

    pub(crate) fn search_view_waiting(&self) -> bool {
        ((self.search.filter && !self.search.query.is_empty()) || self.search.panel_filter.enabled)
            && (self.search.pending || self.search_projection().is_none())
    }

    fn search_projection(&self) -> Option<&SearchResult> {
        self.search
            .result
            .as_deref()
            .filter(|result| !self.showing_stream() && Arc::ptr_eq(&result.source, &self.entries))
    }

    fn current_search_result(&self) -> Option<&SearchResult> {
        (!self.search.pending && self.search.applied == self.search.revision)
            .then(|| self.search_projection())
            .flatten()
    }

    pub(crate) fn search_match_count(&self) -> usize {
        self.current_search_result()
            .map_or(0, |result| result.matches.len())
    }

    pub(crate) fn search_prefix_end(&self, row: usize) -> usize {
        let Some(result) = self.current_search_result() else {
            return 0;
        };
        if self.search.filter && !self.search.query.is_empty() {
            return result
                .matches
                .get(row)
                .map_or(0, |item| item.display_prefix_end);
        }
        let row = if self.search.panel_filter.enabled {
            let Some(item) = result.filtered.get(row) else {
                return 0;
            };
            item.source_row
        } else {
            row
        };
        result
            .matches
            .binary_search_by_key(&row, |item| item.source_row)
            .ok()
            .map_or(0, |index| result.matches[index].display_prefix_end)
    }

    pub(crate) fn search_allows_activation(&self) -> bool {
        if self.search.panel_filter.enabled
            && !self.parent_cursor
            && !self
                .cursor
                .is_some_and(|id| self.display_index().by_id.contains_key(&id))
        {
            return false;
        }
        !self.search_view_waiting()
            && (self.search.query.is_empty()
                || self.current_search_result().is_some_and(|result| {
                    !self.parent_cursor
                        && self
                            .cursor
                            .is_some_and(|id| result.index.by_id.contains_key(&id))
                }))
    }
}

impl Presenter {
    fn next_search_revision(&mut self, pane: PaneId) -> Option<SearchRevision> {
        let slot = pane.index();
        let Some(next) = self.search_revisions[slot]
            .0
            .checked_add(1)
            .map(SearchRevision)
        else {
            self.panes[slot].error = "Search request identities are exhausted.".into();
            return None;
        };
        self.search_revisions[slot] = next;
        Some(next)
    }

    pub(crate) fn change_search_query(
        &mut self,
        pane: PaneId,
        query: &str,
        send: &dyn Fn(Command) -> bool,
    ) {
        if self.panes[pane.index()].search.query == query {
            return;
        }
        let Some(revision) = self.next_search_revision(pane) else {
            return;
        };
        self.cancel_selection_actions(pane);
        let p = &mut self.panes[pane.index()];
        if p.search.failed {
            p.error.clear();
            p.search.failed = false;
        }
        self.retired.push(Box::new(mem::replace(
            &mut p.search.query,
            query.to_owned(),
        )));
        p.search.revision = revision;
        p.search.pending = !query.is_empty() || p.search.panel_filter.enabled;
        p.search.needs_request = p.search.pending;
        p.search.choose_first = !query.is_empty();
        p.search.filter = self.preferences.values.quick_search_filter;
        if query.is_empty() && !p.search.panel_filter.enabled {
            self.retired.push(Box::new(p.search.result.take()));
            if p.search.filter {
                p.search.view_revision = p.search.view_revision.saturating_add(1);
                p.selection_rebase_pending = true;
            }
            self.search_cancellations[pane.index()] = Some(revision);
        } else if p.loading {
            self.search_cancellations[pane.index()] = Some(revision);
        }
        self.flush_search(send);
        self.flush_selection(send);
    }

    pub(super) fn search_navigation_accepted(
        &mut self,
        pane: PaneId,
        new_location: bool,
        send: &dyn Fn(Command) -> bool,
    ) {
        let Some(revision) = self.next_search_revision(pane) else {
            return;
        };
        let p = &mut self.panes[pane.index()];
        let had_query = !p.search.query.is_empty();
        p.search.revision = revision;
        p.search.failed = false;
        p.search.filter = self.preferences.values.quick_search_filter;
        p.search.panel_filter = self.preferences.values.panel_filters[pane.index()].clone();
        if new_location {
            self.retired.push(Box::new(mem::take(&mut p.search.query)));
            self.retired.push(Box::new(p.search.result.take()));
            p.search.input_context = revision;
            p.search.choose_first = false;
        }
        p.search.pending = !p.search.query.is_empty() || p.search.panel_filter.enabled;
        p.search.needs_request = p.search.pending;
        if had_query || self.search_cancellations[pane.index()].is_some() {
            self.search_cancellations[pane.index()] = Some(revision);
            self.flush_search_cancellation(pane, send);
        }
    }

    fn flush_search_cancellation(&mut self, pane: PaneId, send: &dyn Fn(Command) -> bool) {
        let slot = pane.index();
        if let Some(revision) = self.search_cancellations[slot]
            && send(Command::CancelSearch { pane, revision })
        {
            self.search_cancellations[slot] = None;
        }
    }

    /// Retry bounded, nonblocking request delivery from the normal UI/event pump.
    pub fn flush_search(&mut self, send: &dyn Fn(Command) -> bool) {
        for pane in [PaneId::Left, PaneId::Right] {
            self.flush_search_cancellation(pane, send);
            let slot = pane.index();
            let p = &self.panes[slot];
            if !p.search.needs_request || p.loading || p.successful_generation.is_none() {
                continue;
            }
            let Some(revision) = self.search_revisions[slot]
                .0
                .checked_add(1)
                .map(SearchRevision)
            else {
                self.panes[slot].error = "Search request identities are exhausted.".into();
                continue;
            };
            let request = SearchRequest {
                pane,
                navigation: p.displayed_generation(),
                revision,
                location: p.path.clone(),
                source: p.entries.clone(),
                query: p.search.query.clone(),
                filter: p.search.panel_filter.clone(),
            };
            if send(Command::Search(request)) {
                // The accepted newer request supersedes a cancellation even if
                // its earlier send hit backpressure during this same callback.
                self.search_cancellations[slot] = None;
                self.search_revisions[slot] = revision;
                let search = &mut self.panes[slot].search;
                search.revision = revision;
                search.needs_request = false;
            }
        }
    }

    pub(super) fn apply_search_event(&mut self, event: Event) -> Option<Event> {
        match event {
            Event::SearchReady {
                pane,
                navigation,
                revision,
                result,
            } => {
                let p = &mut self.panes[pane.index()];
                if navigation != p.displayed_generation()
                    || revision != p.search.revision
                    || !p.search.pending
                    || (p.search.query.is_empty() && !p.search.panel_filter.enabled)
                    || p.loading
                    || !Arc::ptr_eq(&result.source, &p.entries)
                {
                    self.retired.push(Box::new(result));
                    return None;
                }
                let displayed_index = if p.search.filter && !p.search.query.is_empty() {
                    &result.index
                } else if p.search.panel_filter.enabled {
                    &result.filtered_index
                } else {
                    &p.index
                };
                let preserve_cursor = !p.search.choose_first
                    && !p.parent_cursor
                    && p.cursor
                        .is_some_and(|id| displayed_index.by_id.contains_key(&id));
                let empty_display = displayed_index.by_id.is_empty();
                if !preserve_cursor
                    && let Some(first) = result
                        .matches
                        .first()
                        .and_then(|item| result.source.get(item.source_row))
                {
                    p.cursor = Some(first.id);
                    p.parent_cursor = false;
                } else if !preserve_cursor && empty_display {
                    p.cursor = None;
                    p.parent_cursor = false;
                }
                self.retired.push(Box::new(p.search.result.replace(result)));
                p.search.applied = revision;
                p.search.pending = false;
                p.search.needs_request = false;
                p.search.choose_first = false;
                if p.search.failed {
                    p.error.clear();
                }
                p.search.failed = false;
                if !preserve_cursor {
                    p.preserve_viewport = false;
                }
                if p.search.filter || p.search.panel_filter.enabled {
                    p.search.view_revision = p.search.view_revision.saturating_add(1);
                    p.selection_rebase_pending = p.archive.is_none();
                }
                None
            }
            Event::SearchFailed {
                pane,
                navigation,
                revision,
                error,
            } => {
                let p = &mut self.panes[pane.index()];
                if navigation == p.displayed_generation()
                    && revision == p.search.revision
                    && p.search.pending
                    && (!p.search.query.is_empty() || p.search.panel_filter.enabled)
                {
                    p.search.pending = false;
                    p.search.needs_request = false;
                    p.search.failed = true;
                    p.error = crate::error_presentation::describe(&error);
                }
                None
            }
            event => Some(event),
        }
    }

    /// Preference preview/loading changes the display basis without filesystem work.
    pub(super) fn search_filter_changed(&mut self) {
        for pane in [PaneId::Left, PaneId::Right] {
            let enabled = self.preferences.values.quick_search_filter;
            let persistent = self.preferences.values.panel_filters[pane.index()].clone();
            if self.panes[pane.index()].search.panel_filter != persistent {
                let Some(revision) = self.next_search_revision(pane) else {
                    continue;
                };
                self.cancel_selection_actions(pane);
                let p = &mut self.panes[pane.index()];
                p.search.panel_filter = persistent;
                p.search.revision = revision;
                p.search.pending = p.search.panel_filter.enabled || !p.search.query.is_empty();
                p.search.needs_request = p.search.pending;
                p.search.choose_first = false;
                p.search.view_revision = p.search.view_revision.saturating_add(1);
                self.search_cancellations[pane.index()] = Some(revision);
                if !p.search.pending {
                    self.retired.push(Box::new(p.search.result.take()));
                    p.selection_rebase_pending = true;
                }
            }
            if self.panes[pane.index()].search.filter != enabled {
                if !self.panes[pane.index()].search.query.is_empty() {
                    self.cancel_selection_actions(pane);
                }
                let p = &mut self.panes[pane.index()];
                p.search.filter = enabled;
                if !p.search.query.is_empty() {
                    if enabled
                        && let Some(result) = p.current_search_result()
                        && (p.parent_cursor
                            || !p
                                .cursor
                                .is_some_and(|id| result.index.by_id.contains_key(&id)))
                        && let Some(first) = result
                            .matches
                            .first()
                            .and_then(|item| result.source.get(item.source_row))
                    {
                        p.cursor = Some(first.id);
                        p.parent_cursor = false;
                    }
                    p.search.view_revision = p.search.view_revision.saturating_add(1);
                    p.selection_rebase_pending = true;
                }
            }
        }
    }

    pub(crate) fn move_search_cursor(&mut self, pane: PaneId, key: i32) {
        let p = &mut self.panes[pane.index()];
        let Some(result) = p.current_search_result() else {
            return;
        };
        if result.matches.is_empty() {
            return;
        }
        let source_row = p.cursor.and_then(|id| p.index.by_id.get(&id)).copied();
        let insertion = source_row.map_or(0, |row| {
            result.matches.partition_point(|item| item.source_row < row)
        });
        let exact = source_row.is_some_and(|row| {
            result
                .matches
                .get(insertion)
                .is_some_and(|item| item.source_row == row)
        });
        let last = result.matches.len() - 1;
        let target = match key {
            2 => insertion.saturating_sub(1),
            3 => (insertion + usize::from(exact)).min(last),
            4 => insertion.saturating_sub(20),
            5 => (insertion + 20).min(last),
            6 => 0,
            7 => last,
            _ => return,
        };
        p.cursor = Some(result.source[result.matches[target].source_row].id);
        p.parent_cursor = false;
        p.preserve_viewport = false;
    }
}
