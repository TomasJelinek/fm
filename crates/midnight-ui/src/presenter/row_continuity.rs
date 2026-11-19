//! A background re-read of the shown folder (a watch hint or a finished
//! operation) keeps the rows on screen. It shows no loading state, the rows
//! keep their identities while it is pending, and when the backend described
//! the new listing as edits of the shown rows (`Event::ListingChanges`) only
//! the edited rows change: the row model applies the edits instead of
//! replacing every row.

use super::{Pane, Request};
use midnight_core::{Entry, NavigationId, RowEdit};
use std::sync::Arc;

/// The edits a committed background listing made to the shown rows.
#[derive(Clone)]
pub(crate) struct RowEdits {
    /// Addresses of the rows the edits apply to and of the rows they give,
    /// as the row model records them (`Pane::rows().as_ptr()`).
    pub(crate) before: usize,
    pub(crate) after: usize,
    pub(crate) edits: Arc<[RowEdit]>,
}

/// The generations that row identities and the row model name while
/// background re-reads keep the shown rows.
#[derive(Clone, Copy)]
pub(super) struct RowsEpoch {
    generation: u64,
    displayed: NavigationId,
    /// The re-read the epoch was held for. Any other navigation advances the
    /// pane's generation past it, which ends the epoch.
    held_for: u64,
}

/// The backend's row edits for a pending re-read (`Event::ListingChanges`).
pub(super) struct PendingChanges {
    base: Arc<[Entry]>,
    edits: Arc<[RowEdit]>,
}

impl Pane {
    /// Whether the pane shows that it is loading. A background re-read keeps
    /// the shown rows and stays silent; the pane still waits for it
    /// (`loading`) before it acts on rows.
    pub(crate) fn loading_shown(&self) -> bool {
        self.loading
            && !self
                .pending
                .as_ref()
                .is_some_and(|request| request.background)
    }

    fn rows_epoch(&self) -> Option<RowsEpoch> {
        self.rows_epoch
            .filter(|epoch| epoch.held_for == self.generation)
    }

    /// The generation that row identities and the row model name.
    pub(crate) fn row_generation(&self) -> u64 {
        self.rows_epoch()
            .map_or(self.generation, |epoch| epoch.generation)
    }

    /// The displayed generation that row identities and the row model name.
    pub(crate) fn row_displayed_generation(&self) -> NavigationId {
        self.rows_epoch()
            .map_or_else(|| self.displayed_generation(), |epoch| epoch.displayed)
    }

    /// The row edits of the last in-place commit, for the row model.
    pub(crate) fn row_edits(&self) -> Option<&RowEdits> {
        self.row_edits.as_ref()
    }

    /// Plain rows of a native folder: no tree, search projection, archive or
    /// network view, so row `i` shows `entries[i]`.
    fn plain_rows(&self) -> bool {
        self.tree_view().is_none()
            && self.archive.is_none()
            && self.network.is_none()
            && !self.network_hides_rows()
            && !(self.search.filter && !self.search.query.is_empty())
            && !self.search.panel_filter.enabled
    }

    /// Called before `generation` advances to `navigation`. A background
    /// re-read of the shown plain rows keeps their identities; any other
    /// navigation ends that.
    pub(super) fn rows_navigation_accepted(&mut self, navigation: NavigationId, background: bool) {
        self.row_edits = None;
        self.rows_epoch = (background && self.plain_rows()).then(|| {
            let epoch = self.rows_epoch().unwrap_or(RowsEpoch {
                generation: self.generation,
                displayed: self.displayed_generation(),
                held_for: 0,
            });
            RowsEpoch {
                held_for: navigation.0,
                ..epoch
            }
        });
    }

    /// Records the backend's row edits for the pending re-read. Returns what
    /// it replaces, for deferred reclamation.
    pub(super) fn listing_changes(
        &mut self,
        base: Arc<[Entry]>,
        edits: Arc<[RowEdit]>,
    ) -> Option<PendingChanges> {
        self.pending
            .as_mut()?
            .changes
            .replace(PendingChanges { base, edits })
    }

    /// Called when `request` commits `entries`, before they replace the shown
    /// rows. A background re-read whose rows are the shown ones, or that the
    /// backend described as edits of them, updates the rows in place and keeps
    /// their identities; any other listing replaces them.
    pub(super) fn rows_committing(&mut self, request: &Request, entries: &Arc<[Entry]>) {
        let changes = request.changes.as_ref();
        let in_place = request.background
            && self.rows_epoch().is_some()
            && self.plain_rows()
            && changes.as_ref().map_or_else(
                || Arc::ptr_eq(entries, &self.entries),
                |changes| Arc::ptr_eq(&changes.base, &self.entries),
            );
        if !in_place {
            self.rows_epoch = None;
            self.row_edits = None;
            return;
        }
        self.row_edits = changes.map(|changes| RowEdits {
            before: self.entries.as_ptr() as usize,
            after: entries.as_ptr() as usize,
            edits: changes.edits.clone(),
        });
    }
}
