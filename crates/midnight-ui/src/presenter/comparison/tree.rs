//! Read-only comparison selections retain exact tree occurrence identity.
use super::*;

pub(super) fn selection_revision(pane: &Pane) -> SelectionRevision {
    if pane.tree_view().is_some() {
        pane.tree.revision
    } else {
        pane.selection_revision
    }
}

pub(super) fn selection_ready(pane: &Pane) -> bool {
    if pane.tree_view().is_some() {
        pane.tree_input_ready()
            && pane.tree.preview.is_empty()
            && pane.tree.resolved == pane.tree.revision
    } else {
        !pane.selection_rebase_pending
            && pane.selection_applied == pane.selection_revision
            && !pane.selection_deferred.any()
    }
}

pub(super) fn result_matches(result: &CompareResult) -> bool {
    result.request.sides.iter().enumerate().all(|(slot, side)| {
        Arc::ptr_eq(result.selections[slot].basis(), &side.view)
            && match (&side.tree, &result.tree_replacements[slot]) {
                (None, None) => true,
                (Some(view), Some(selection)) => {
                    selection.matches_origin(view)
                        && selection.bits() == result.selections[slot].bits()
                }
                _ => false,
            }
    })
}

impl Presenter {
    pub(in crate::presenter) fn observe_tree_comparison(&mut self) {
        for slot in 0..2 {
            let Some((stamp, revision)) = self.comparison.tree_pending[slot] else {
                continue;
            };
            let pane = &self.panes[slot];
            let current = pane.tree_view().is_some_and(|view| {
                view.stamp.session == stamp.session && view.stamp.navigation == stamp.navigation
            });
            if current && pane.tree.applied >= revision {
                self.comparison.tree_pending[slot] = None;
            } else if !current
                || (pane.tree.resolved >= revision
                    && !pane
                        .tree
                        .preview
                        .iter()
                        .any(|(pending, _)| *pending == revision))
            {
                self.comparison.tree_pending[slot] = None;
                self.comparison.error =
                    "A panel changed before comparison selection could be applied. Compare again."
                        .into();
            }
        }
        if self.comparison.pending.is_none() && !self.comparison.waiting {
            self.comparison.busy = self.comparison.tree_pending.iter().any(Option::is_some);
        }
    }
}
