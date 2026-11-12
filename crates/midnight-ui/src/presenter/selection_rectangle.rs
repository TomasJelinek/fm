//! A gesture predicts visible marks without editing selection snapshots.
use super::*;

#[derive(Clone)]
pub(crate) struct Capture {
    generation: u64,
    search: u64,
    // A weak owner prevents address reuse without retaining a large old index.
    view: std::sync::Weak<DirectoryIndex>,
    len: usize,
    revision: SelectionRevision,
    selection: usize,
    cancel: Cancellation,
}
impl Capture {
    fn revision(pane: &Pane) -> SelectionRevision {
        if pane.tree_view().is_some() {
            return pane.tree.revision;
        }
        pane.archive
            .as_ref()
            .map_or(pane.selection_revision, |a| a.selection_revision)
    }
    fn selection(pane: &Pane) -> usize {
        if let Some(publication) = &pane.tree.publication {
            return Arc::as_ptr(&publication.selection) as usize;
        }
        pane.archive.as_ref().map_or_else(
            || Arc::as_ptr(&pane.selected) as usize,
            |a| Arc::as_ptr(&a.selected) as usize,
        )
    }
    fn available(pane: &Pane) -> bool {
        !pane.loading
            && pane.network.is_none()
            && !pane.network_request
            && !pane.showing_stream()
            && !pane.search_view_waiting()
            && if pane.tree_view().is_some() {
                pane.tree.preview.is_empty() && pane.tree_input_ready()
            } else {
                pane.archive.as_ref().map_or_else(
                    || {
                        pane.selection_preview.is_empty()
                            && pane.selection_applied == pane.selection_revision
                            && !pane.selection_rebase_pending
                    },
                    |a| !a.selection_pending,
                )
            }
    }
    fn matches(&self, pane: &Pane) -> bool {
        !self.cancel.is_cancelled()
            && Self::available(pane)
            && pane.generation == self.generation
            && pane.search.view_revision == self.search
            && self.view.as_ptr() == Arc::as_ptr(pane.display_index())
            && Self::revision(pane) == self.revision
            && Self::selection(pane) == self.selection
    }
    pub(crate) fn cancel(&self) {
        self.cancel.cancel();
    }
}

pub(super) struct Preview {
    capture: Capture,
    ranges: SelectionRanges,
    mode: SelectionMode,
    token: Arc<()>,
    cancel: Cancellation,
    submitted: bool,
    totals: Option<ArchiveSelectionTotals>,
}
impl Drop for Preview {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
impl Preview {
    pub(super) fn visible(&self, pane: &Pane, entry: &Entry, selected: bool) -> bool {
        if !self.capture.matches(pane) {
            return selected;
        }
        let hit = pane
            .display_index()
            .by_id
            .get(&entry.id)
            .is_some_and(|row| self.ranges.contains(*row));
        self.mode.selected(hit, selected)
    }
}

impl Presenter {
    pub(crate) fn capture_rectangle(&self, pane: PaneId) -> Option<Capture> {
        let p = &self.panes[pane.index()];
        Capture::available(p).then(|| Capture {
            generation: p.generation,
            search: p.search.view_revision,
            view: Arc::downgrade(p.display_index()),
            len: p.display_index().by_id.len(),
            revision: Capture::revision(p),
            selection: Capture::selection(p),
            cancel: Cancellation::default(),
        })
    }
    pub(crate) fn rectangle_current(&self, pane: PaneId, capture: &Capture) -> bool {
        capture.matches(&self.panes[pane.index()])
    }
    pub(crate) fn preview_rectangle(
        &mut self,
        pane: PaneId,
        capture: &Capture,
        ranges: SelectionRanges,
        mode: SelectionMode,
    ) -> bool {
        let p = &mut self.panes[pane.index()];
        if !capture.matches(p) || ranges.len() != capture.len {
            p.rectangle_preview = None;
            return false;
        }
        if p.rectangle_preview.as_ref().is_some_and(|old| {
            old.capture.matches(p) && old.mode == mode && old.ranges.spans() == ranges.spans()
        }) {
            return true;
        }
        p.rectangle_preview = Some(Preview {
            capture: capture.clone(),
            ranges,
            mode,
            token: Arc::new(()),
            cancel: Cancellation::default(),
            submitted: false,
            totals: None,
        });
        true
    }
    pub(crate) fn flush_rectangle_summary(&mut self, pane: PaneId, send: &dyn Fn(Command) -> bool) {
        let p = &mut self.panes[pane.index()];
        let Some(preview) = p.rectangle_preview.as_ref() else {
            return;
        };
        if preview.submitted || !preview.capture.matches(p) {
            return;
        }
        let seed = if let Some(publication) = &p.tree.publication {
            RectangleSelectionSeed::Tree(publication.selection.clone())
        } else if let Some(archive) = &p.archive {
            RectangleSelectionSeed::Archive {
                selected: archive.selected.clone(),
                full: p.index.clone(),
            }
        } else {
            RectangleSelectionSeed::Native(p.selected.clone())
        };
        let request = RectangleSummaryRequest {
            pane,
            token: preview.token.clone(),
            view: p.display_index().clone(),
            seed,
            ranges: preview.ranges.clone(),
            mode: preview.mode,
            cancel: preview.cancel.clone(),
        };
        if send(Command::SummarizeRectangle(Arc::new(request))) {
            p.rectangle_preview
                .as_mut()
                .expect("retained gesture")
                .submitted = true;
        }
    }
    pub(super) fn apply_rectangle_summary(
        &mut self,
        pane: PaneId,
        token: Arc<()>,
        totals: ArchiveSelectionTotals,
    ) {
        let p = &mut self.panes[pane.index()];
        if p.rectangle_preview.as_ref().is_some_and(|preview| {
            Arc::ptr_eq(&preview.token, &token) && preview.capture.matches(p)
        }) {
            p.rectangle_preview
                .as_mut()
                .expect("validated preview")
                .totals = Some(totals);
        }
    }
    pub(crate) fn cancel_rectangle(&mut self, pane: PaneId) {
        self.panes[pane.index()].rectangle_preview = None;
    }
    pub(crate) fn commit_rectangle(
        &mut self,
        pane: PaneId,
        capture: &Capture,
        ranges: SelectionRanges,
        mode: SelectionMode,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        self.cancel_rectangle(pane);
        if !capture.matches(&self.panes[pane.index()]) || ranges.len() != capture.len {
            return false;
        }
        if self.panes[pane.index()].tree_view().is_some() {
            self.send_tree_selection(pane, TreeSelectionAction::Ranges { ranges, mode }, send)
        } else if self.panes[pane.index()].archive.is_some() {
            self.archive_bulk_selection(pane, ArchiveSelectionAction::Ranges { ranges, mode }, send)
        } else {
            self.send_selection(pane, SelectionAction::Ranges { ranges, mode }, send)
        }
    }
}

impl Pane {
    pub(super) fn rectangle_summary(&self) -> Option<Option<ArchiveSelectionTotals>> {
        self.rectangle_preview
            .as_ref()
            .filter(|preview| preview.capture.matches(self))
            .map(|preview| preview.totals)
    }
}
