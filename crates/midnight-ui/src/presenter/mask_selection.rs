//! Select and deselect by mask (keypad + and −) and the keypad's invert (*).
//! The dialog checks the mask's grammar, then a backend worker compiles it
//! (`Command::ValidateSelectionMask`), as the panel filter dialog has its filter
//! checked; only an accepted mask is applied and remembered with the folder choice.
//! Selection workers match the names of the rows the panel shows, so hidden or
//! filtered entries never change.
use super::*;

#[cfg(test)]
#[path = "mask_selection_tests.rs"]
mod tests;

#[derive(Default)]
pub(crate) struct MaskDialogState {
    pub(crate) open: bool,
    /// Each opening has a new identity; stale dialog callbacks are ignored.
    pub(crate) generation: u64,
    pane: Option<PaneId>,
    pub(crate) select: bool,
    pub(crate) pattern: String,
    pub(crate) folders: bool,
    pub(crate) error: String,
    /// The identity of the last backend check; it only grows, across openings too.
    checks: u64,
    /// The check of the mask that OK submitted, until its answer.
    pending: Option<u64>,
    /// A mask the backend accepted, applied once a sender is at hand.
    accepted: Option<MaskValidationRequest>,
}

impl Presenter {
    /// Ctrl+= or keypad + (`select`), Ctrl+− or keypad − opens the dialog for `pane`
    /// with the last choices.
    pub(crate) fn open_mask_selection(&mut self, pane: PaneId, select: bool) {
        let p = &mut self.panes[pane.index()];
        if p.network.is_some() || p.network_request {
            p.set_error(
                "Network locations cannot be selected by mask.",
                &mut self.retired,
            );
            return;
        }
        let Some(generation) = self.mask_dialog.generation.checked_add(1) else {
            return;
        };
        let saved = &self.preferences.values.mask_selection;
        let pattern = saved.pattern.clone();
        let folders = saved.folders;
        let checks = self.mask_dialog.checks;
        self.retired.push(Box::new(mem::replace(
            &mut self.mask_dialog,
            MaskDialogState {
                open: true,
                generation,
                pane: Some(pane),
                select,
                pattern,
                folders,
                error: String::new(),
                checks,
                pending: None,
                accepted: None,
            },
        )));
    }

    pub(crate) fn cancel_mask_selection(&mut self) {
        self.mask_dialog.open = false;
        self.mask_dialog.error.clear();
        self.mask_dialog.pending = None;
        self.mask_dialog.accepted = None;
    }

    /// OK in the dialog: a mask outside the panel filter's Masks grammar keeps the
    /// dialog open with the reason; any other is sent to the backend to compile. The
    /// dialog stays open until the answer (`apply_mask_event`); nothing is applied or
    /// remembered before it.
    pub(crate) fn submit_mask_selection(
        &mut self,
        pattern: &str,
        folders: bool,
        send: &dyn Fn(Command) -> bool,
    ) {
        let Some(pane) = self.mask_dialog.pane.filter(|_| self.mask_dialog.open) else {
            return;
        };
        if self.mask_feed_refuses(pane) {
            return;
        }
        pattern.clone_into(&mut self.mask_dialog.pattern);
        self.mask_dialog.folders = folders;
        self.mask_dialog.error.clear();
        // A new OK replaces a check that is still under way.
        self.mask_dialog.pending = None;
        self.mask_dialog.accepted = None;
        if pattern.trim().is_empty() {
            self.mask_dialog.error = "Enter a mask, for example *.txt or *.jpg;*.png.".into();
            return;
        }
        let choices = MaskSelectionPreferences {
            pattern: pattern.to_owned(),
            folders,
        };
        if let Err(error) = choices.validate_mask() {
            self.mask_dialog.error = error.into();
            return;
        }
        let Some(id) = self.mask_dialog.checks.checked_add(1) else {
            self.mask_dialog.error = "Mask check identities are exhausted.".into();
            return;
        };
        if send(Command::ValidateSelectionMask(MaskValidationRequest {
            id,
            pattern: choices.pattern,
        })) {
            self.mask_dialog.checks = id;
            self.mask_dialog.pending = Some(id);
        } else {
            self.mask_dialog.error = "The command queue is busy. Try OK again.".into();
        }
    }

    /// Search results fed to `pane` since the dialog opened hide its folder: say so
    /// and close the dialog.
    fn mask_feed_refuses(&mut self, pane: PaneId) -> bool {
        let action = if self.mask_dialog.select {
            "Select by mask"
        } else {
            "Deselect by mask"
        };
        if !self.feed_refuses(pane, action) {
            return false;
        }
        self.cancel_mask_selection();
        true
    }

    /// The backend's answer to the dialog's last check. A refused mask keeps the
    /// dialog open with the reason; an accepted one waits for `flush_mask_selection`.
    /// Answers to an earlier OK or a closed dialog are dropped.
    pub(super) fn apply_mask_event(&mut self, event: Event) -> Option<Event> {
        let Event::SelectionMaskValidated { request, result } = event else {
            return Some(event);
        };
        if self.mask_dialog.open && self.mask_dialog.pending == Some(request.id) {
            self.mask_dialog.pending = None;
            match result {
                Ok(()) => self.mask_dialog.accepted = Some(request),
                Err(reason) => self.mask_dialog.error = reason,
            }
        }
        None
    }

    /// Mark or unmark the rows that match an accepted mask, close the dialog and
    /// remember the mask and folder choice.
    pub(super) fn flush_mask_selection(&mut self, send: &dyn Fn(Command) -> bool) {
        let Some(request) = self.mask_dialog.accepted.take() else {
            return;
        };
        let Some(pane) = self.mask_dialog.pane.filter(|_| self.mask_dialog.open) else {
            return;
        };
        // Search results fed to the panel during the check hide its folder.
        if self.mask_feed_refuses(pane) {
            return;
        }
        let folders = self.mask_dialog.folders;
        let mask = Arc::new(MaskSelection {
            pattern: request.pattern.clone(),
            select: self.mask_dialog.select,
            folders,
        });
        if !self.send_mask_selection(pane, mask, send) {
            self.mask_dialog.error =
                "The panel is still loading or busy. Try OK again in a moment.".into();
            return;
        }
        self.mask_dialog.open = false;
        let choices = MaskSelectionPreferences {
            pattern: request.pattern,
            folders,
        };
        if self.preferences.values.mask_selection != choices {
            self.preferences.edited.mask_selection = true;
            let preferences = UserPreferences {
                mask_selection: choices,
                ..self.preferences.values.clone()
            };
            self.update_preferences(preferences, send);
        }
    }

    /// Flat native folders, archives and inline trees each resolve the mask on
    /// their own selection worker against the rows they currently show.
    fn send_mask_selection(
        &mut self,
        pane: PaneId,
        mask: Arc<MaskSelection>,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let p = &self.panes[pane.index()];
        if p.tree_view().is_some() {
            return self.send_tree_selection(pane, TreeSelectionAction::Mask(mask), send);
        }
        if p.network.is_some() || p.network_request {
            return false;
        }
        if p.archive.is_some() {
            return self.archive_bulk_selection(pane, ArchiveSelectionAction::Mask(mask), send);
        }
        // Names come from a complete view; a streaming listing has none yet.
        if p.showing_stream() || p.search_view_waiting() {
            return false;
        }
        self.send_selection(pane, SelectionAction::Mask(mask), send)
    }

    /// Keypad *: invert files, and folders too when Select by mask includes them.
    pub(crate) fn mask_invert_operation(&self) -> SelectionOperation {
        if self.preferences.values.mask_selection.folders {
            SelectionOperation::InvertAll
        } else {
            SelectionOperation::Invert
        }
    }
}
