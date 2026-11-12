//! Atomic publication of two immutable, revision-checked selection snapshots.
use super::*;
#[cfg(test)]
#[path = "comparison_tests.rs"]
mod tests;
mod tree;

#[derive(Default)]
pub(crate) struct ComparisonState {
    pub(crate) open: bool,
    pub(crate) busy: bool,
    pub(crate) error: String,
    pub(crate) summary: String,
    pub(crate) options: CompareOptions,
    pub(super) seeds: [Option<Arc<AuthoritativeSelection>>; 2],
    next: u64,
    waiting: bool,
    pending: Option<Arc<CompareRequest>>,
    cancellation: Option<CompareId>,
    tree_pending: [Option<(TreeViewStamp, SelectionRevision)>; 2],
}

impl Presenter {
    pub(crate) fn open_comparison(&mut self) {
        if self.panes.iter().any(|p| !p.native_capabilities()) {
            self.desktop.report(
                "Archive comparison unavailable",
                "Directory comparison currently requires two native filesystem panels.".into(),
            );
            return;
        }
        self.comparison.open = true;
        self.comparison.error.clear();
    }

    pub(crate) fn submit_comparison(
        &mut self,
        options: CompareOptions,
        send: &dyn Fn(Command) -> bool,
    ) {
        if self.comparison.busy || self.panes.iter().any(|pane| !pane.native_capabilities()) {
            return;
        }
        if options.time_tolerance_seconds > 86_400 {
            self.comparison.error = "Time tolerance must be between 0 and 86400 seconds.".into();
            return;
        }
        if [PaneId::Left, PaneId::Right]
            .iter()
            .any(|pane| self.visible_directory(*pane).is_none())
        {
            self.comparison.error = "Wait until both directories have finished loading.".into();
            return;
        }
        self.comparison.options = options;
        self.comparison.error.clear();
        self.comparison.summary.clear();
        for pane in [PaneId::Left, PaneId::Right] {
            self.change_search_query(pane, "", send);
        }
        self.comparison.busy = true;
        self.comparison.waiting = true;
        self.flush_comparison(send);
    }

    pub(crate) fn cancel_comparison(&mut self, send: &dyn Fn(Command) -> bool) {
        if let Some(request) = self.comparison.pending.take() {
            self.comparison.cancellation = Some(request.id);
            self.retired.push(Box::new(request));
        }
        self.comparison.waiting = false;
        self.comparison.busy = false;
        self.comparison.open = false;
        self.flush_comparison(send);
    }

    fn comparison_current(&self, request: &CompareRequest) -> bool {
        request.sides.iter().enumerate().all(|(slot, side)| {
            let pane = &self.panes[slot];
            !pane.loading
                && !pane.search.pending
                && !pane.search_view_waiting()
                && tree::selection_ready(pane)
                && tree::selection_revision(pane) == side.selection_revision
                && match (&side.tree, pane.tree_view()) {
                    (Some(captured), Some(current)) => Arc::ptr_eq(captured, current),
                    (None, None) => true,
                    _ => false,
                }
                && pane.search.view_revision == side.view_revision
                && pane.successful_generation == Some(side.source.navigation)
                && pane.path == side.source.path
                && Arc::ptr_eq(&pane.entries, &side.source.entries)
                && Arc::ptr_eq(pane.display_index(), &side.view)
        })
    }

    pub fn flush_comparison(&mut self, send: &dyn Fn(Command) -> bool) {
        self.observe_tree_comparison();
        if self
            .comparison
            .pending
            .as_ref()
            .is_some_and(|request| !self.comparison_current(request))
        {
            let request = self.comparison.pending.take().unwrap();
            self.comparison.cancellation = Some(request.id);
            self.retired.push(Box::new(request));
            self.comparison.busy = false;
            self.comparison.error =
                "Comparison cancelled because a panel or selection changed.".into();
        }
        if let Some(id) = self.comparison.cancellation
            && send(Command::CancelComparison(id))
        {
            self.comparison.cancellation = None;
        }
        if !self.comparison.waiting {
            return;
        }
        let mut sides = Vec::with_capacity(2);
        for pane in [PaneId::Left, PaneId::Right] {
            let Some(source) = self.visible_directory(pane) else {
                self.comparison.waiting = false;
                self.comparison.busy = false;
                self.comparison.error = "Comparison cancelled because a directory changed.".into();
                return;
            };
            let p = &self.panes[pane.index()];
            if p.search.pending || p.search_view_waiting() || !tree::selection_ready(p) {
                return;
            }
            sides.push(CompareSide {
                source,
                tree: p.tree_view().cloned(),
                view: p.display_index().clone(),
                view_revision: p.search.view_revision,
                selection_revision: tree::selection_revision(p),
            });
        }
        let Some(next) = self.comparison.next.checked_add(1) else {
            self.comparison.error = "Comparison identities are exhausted.".into();
            self.comparison.waiting = false;
            self.comparison.busy = false;
            return;
        };
        let request = CompareRequest {
            id: CompareId(next),
            sides: sides.try_into().unwrap(),
            options: self.comparison.options.clone(),
        };
        if send(Command::CompareDirectories(request.clone())) {
            self.comparison.next = next;
            self.comparison.pending = Some(Arc::new(request));
            self.comparison.waiting = false;
        }
    }

    pub(super) fn apply_comparison_event(&mut self, event: Event) -> Option<Event> {
        let Event::DirectoriesCompared { id, result } = event else {
            return Some(event);
        };
        let Some(request) = self
            .comparison
            .pending
            .as_ref()
            .filter(|request| request.id == id)
        else {
            self.retired.push(Box::new(result));
            return None;
        };
        let valid = self.comparison_current(request);
        self.retired.push(Box::new(self.comparison.pending.take()));
        self.comparison.busy = false;
        if !valid {
            self.comparison.error =
                "Comparison discarded because a panel or selection changed.".into();
            self.retired.push(Box::new(result));
            return None;
        }
        match result {
            Err(error) => self.comparison.error = crate::error_presentation::describe(&error),
            Ok(result) => {
                if !self.comparison_current(&result.request)
                    || self
                        .panes
                        .iter()
                        .any(|pane| tree::selection_revision(pane).0 == u64::MAX)
                    || !tree::result_matches(&result)
                {
                    self.comparison.error =
                        "Comparison no longer matches the displayed panels.".into();
                    self.retired.push(Box::new(result));
                    return None;
                }
                for (slot, (pane, selected)) in
                    self.panes.iter_mut().zip(&result.selections).enumerate()
                {
                    if let Some(selection) = &result.tree_replacements[slot] {
                        let revision = SelectionRevision(pane.tree.revision.0 + 1);
                        let action = TreeSelectionAction::Replace(selection.clone());
                        pane.tree.revision = revision;
                        pane.tree.preview.push((revision, action.clone()));
                        pane.tree.replay.push_back((revision, action));
                        self.comparison.tree_pending[slot] = Some((selection.stamp(), revision));
                        continue;
                    }
                    let seed = Arc::new(AuthoritativeSelection {
                        id: result.request.id.0,
                        origin: SelectionOrigin {
                            navigation: pane.selection_context,
                            location: pane.path.clone(),
                        },
                        snapshot: selected.clone(),
                    });
                    self.retired
                        .push(Box::new(self.comparison.seeds[slot].replace(seed)));
                    pane.selection_revision.0 += 1;
                    pane.selection_applied = pane.selection_revision;
                    pane.selection_resolved = pane.selection_revision;
                    pane.selection_dirty = true;
                    pane.selection_preview.clear();
                    self.retired
                        .push(Box::new(mem::replace(&mut pane.selected, selected.clone())));
                }
                self.comparison.summary = if result
                    .selections
                    .iter()
                    .all(|selection| selection.is_empty())
                    && result.unknown == 0
                    && result.ambiguous == 0
                {
                    "Displayed metadata matches for the chosen criteria.".into()
                } else {
                    format!(
                        "Found {} differences in the first directory and {} in the second directory. Unknown metadata: {}. Ambiguous name groups: {}.",
                        result.selections[0].len(),
                        result.selections[1].len(),
                        result.unknown,
                        result.ambiguous
                    )
                };
                self.comparison.busy = self.comparison.tree_pending.iter().any(Option::is_some);
                self.retired.push(Box::new(result));
            }
        }
        None
    }
}
