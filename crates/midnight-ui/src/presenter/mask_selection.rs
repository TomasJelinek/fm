//! Select and deselect by mask (keypad + and −) and the keypad's invert (*).
//! The dialog checks the mask's grammar and keeps the last valid mask and folder
//! choice; selection workers match the names of the rows the panel shows, so
//! hidden or filtered entries never change.
use super::*;

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
            },
        )));
    }

    pub(crate) fn cancel_mask_selection(&mut self) {
        self.mask_dialog.open = false;
        self.mask_dialog.error.clear();
    }

    /// OK in the dialog: remember the choices and mark or unmark matching rows. A
    /// mask outside the panel filter's Masks grammar keeps the dialog open with the
    /// reason; it is neither applied nor remembered.
    pub(crate) fn submit_mask_selection(
        &mut self,
        pattern: &str,
        folders: bool,
        send: &dyn Fn(Command) -> bool,
    ) {
        let Some(pane) = self.mask_dialog.pane.filter(|_| self.mask_dialog.open) else {
            return;
        };
        // Search results fed to the panel since the dialog opened hide its folder.
        let action = if self.mask_dialog.select {
            "Select by mask"
        } else {
            "Deselect by mask"
        };
        if self.feed_refuses(pane, action) {
            self.cancel_mask_selection();
            return;
        }
        pattern.clone_into(&mut self.mask_dialog.pattern);
        self.mask_dialog.folders = folders;
        self.mask_dialog.error.clear();
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
        let mask = Arc::new(MaskSelection {
            pattern: pattern.to_owned(),
            select: self.mask_dialog.select,
            folders,
        });
        if !self.send_mask_selection(pane, mask, send) {
            self.mask_dialog.error =
                "The panel is still loading or busy. Try OK again in a moment.".into();
            return;
        }
        self.mask_dialog.open = false;
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
