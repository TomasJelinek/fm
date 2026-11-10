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
        if publication.selection.is_empty() && pane.tree.preview.is_empty() && fallback.is_none() {
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
