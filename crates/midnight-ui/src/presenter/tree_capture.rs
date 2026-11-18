//! Shared admission state for actions that capture an exact tree selection.
use super::*;

pub(super) struct Capture {
    pub view: Arc<TreeProjection>,
    pub request: Option<TreeRequestId>,
    pub fallback: Option<TreeRowId>,
    pub cancel: Cancellation,
    revision: SelectionRevision,
}

pub(super) enum Status {
    Gone,
    SelectionChanged,
    Waiting,
    Ready(TreeRequestId, TreePublication),
}

impl Capture {
    pub fn new(pane: &Pane) -> Option<Self> {
        Self::with_empty(pane, false)
    }

    /// Location-only commands may capture a valid view with neither marks nor a file caret.
    pub fn optional_targets(pane: &Pane) -> Option<Self> {
        Self::with_empty(pane, true)
    }

    fn with_empty(pane: &Pane, allow_empty: bool) -> Option<Self> {
        if pane.loading || !pane.tree_input_ready() {
            return None;
        }
        let publication = pane.tree.publication.as_ref()?;
        let view = publication.view.clone();
        let fallback = (!pane.parent_cursor)
            .then_some(pane.cursor)
            .flatten()
            .and_then(|id| view.index.directory.by_id.get(&id))
            .and_then(|position| view.rows.get(*position))
            .filter(|row| row.selectable && !row.context_only)
            .map(|row| row.id);
        if !allow_empty
            && publication.selection.is_empty()
            && pane.tree.preview.is_empty()
            && fallback.is_none()
        {
            return None;
        }
        Some(Self {
            view,
            request: None,
            fallback,
            cancel: Cancellation::default(),
            revision: if pane.tree.preview.is_empty() {
                pane.tree.applied
            } else {
                pane.tree.revision
            },
        })
    }

    /// The pane's selection is still the one this capture saw.
    pub fn selection_current(&self, pane: &Pane) -> bool {
        pane.tree.preview.is_empty() && pane.tree.applied == self.revision
    }

    /// Menu receipts keep their original target even after the capture request finished.
    pub fn matches_current(&self, pane: &Pane) -> bool {
        !pane.loading
            && !self.cancel.is_cancelled()
            && pane.tree_input_ready()
            && pane.tree.preview.is_empty()
            && pane.tree.applied == self.revision
            && pane
                .tree_view()
                .is_some_and(|view| Arc::ptr_eq(view, &self.view))
    }

    /// A chosen menu action owns independent cancellation and a fresh request identity.
    pub fn fork_if_current(&self, pane: &Pane) -> Option<Self> {
        self.matches_current(pane).then(|| Self {
            view: self.view.clone(),
            request: None,
            fallback: self.fallback,
            cancel: Cancellation::default(),
            revision: self.revision,
        })
    }

    pub fn status(&self, pane: &Pane, closing: bool) -> Status {
        if closing
            || pane.loading
            || self.cancel.is_cancelled()
            || !pane.tree_view().is_some_and(|view| {
                view.stamp.navigation == self.view.stamp.navigation
                    && view.stamp.session == self.view.stamp.session
                    && (self.request.is_some() || Arc::ptr_eq(view, &self.view))
            })
        {
            return Status::Gone;
        }
        if self.request.is_some() || !pane.tree_input_ready() || !pane.tree.preview.is_empty() {
            return Status::Waiting;
        }
        if pane.tree.applied != self.revision {
            return Status::SelectionChanged;
        }
        let Some(request) = pane.tree.request.0.checked_add(1).map(TreeRequestId) else {
            return Status::Gone;
        };
        Status::Ready(
            request,
            pane.tree
                .publication
                .as_ref()
                .expect("checked tree view")
                .clone(),
        )
    }
}

impl Presenter {
    /// A pending tree action loses its exact capture when a newer publication of
    /// the same session (watch refresh, branch loading) or a selection change
    /// replaces the view. Navigation, shutdown and explicit cancellation stay
    /// silent; this case did not come from the user, so ask for a retry.
    pub(super) fn report_tree_capture_lost(
        &mut self,
        view: &TreeProjection,
        cancel: &Cancellation,
        action: &str,
    ) {
        let p = &mut self.panes[view.stamp.pane.index()];
        let same_session = p.tree_view().is_some_and(|current| {
            current.stamp.navigation == view.stamp.navigation
                && current.stamp.session == view.stamp.session
        });
        if self.operations.closing || cancel.is_cancelled() || p.loading || !same_session {
            return;
        }
        p.set_error(
            format!("The tree or its selection changed before {action} could start. Try again."),
            &mut self.retired,
        );
    }
}
