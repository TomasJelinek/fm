//! Show hidden files: one choice for both panels and their inline trees. The
//! backend leaves hidden entries out of listings and tree projections; a change
//! re-reads the panels in place, so the cursor moves to the nearest shown row and
//! marks on entries that are no longer listed are dropped, as for any re-read.
use super::*;

#[derive(Default)]
pub(crate) struct HiddenEntriesState {
    /// The backend was told to leave hidden entries out. It starts listing them.
    told_hide: bool,
    /// Panes that still owe a re-read with the choice the backend was told.
    reread: [bool; 2],
}

impl Pane {
    /// Hidden entries left out of what this pane shows: its listing's, or those of
    /// an inline tree (which counts the root listing's until it lists the root again).
    pub(crate) fn hidden_count(&self) -> usize {
        if self.showing_stream() || self.network.is_some() {
            return 0;
        }
        self.tree_view()
            .map_or(self.index.hidden, |view| view.hidden)
    }
}

impl Presenter {
    /// The Show hidden files command (Ctrl+H, F10 and each panel's View menu).
    pub(crate) fn toggle_hidden_files(&mut self, send: &dyn Fn(Command) -> bool) {
        // Both panels would be re-read only once a long comparison ends.
        if self.comparison_refuses("Show hidden files") {
            return;
        }
        let show = !self.preferences.values.show_hidden_files;
        self.change_show_hidden_files(show, send);
        self.desktop.status = if show {
            "Hidden files are shown"
        } else {
            "Hidden files are not shown"
        }
        .into();
    }

    /// The Options checkbox previews the choice; the command saves it at once.
    pub(crate) fn change_show_hidden_files(&mut self, show: bool, send: &dyn Fn(Command) -> bool) {
        self.preferences.edited.show_hidden_files = true;
        let preferences = UserPreferences {
            show_hidden_files: show,
            ..self.preferences.values.clone()
        };
        self.update_preferences(preferences, send);
        self.flush_hidden_entries(send);
    }

    /// Tell the backend the current choice, then re-read every pane that listed
    /// with the previous one. Refused sends and busy panes retry on later pumps.
    pub(super) fn flush_hidden_entries(&mut self, send: &dyn Fn(Command) -> bool) {
        let hide = !self.preferences.values.show_hidden_files;
        if self.hidden_entries.told_hide != hide {
            if !send(Command::ShowHiddenEntries(!hide)) {
                return;
            }
            self.hidden_entries.told_hide = hide;
            self.hidden_entries.reread = [true; 2];
        }
        for pane in [PaneId::Left, PaneId::Right] {
            // Cleared first: a tree's maintenance refresh flushes background
            // refreshes, which come back here before the re-read returns.
            if mem::take(&mut self.hidden_entries.reread[pane.index()])
                && !self.reread_for_hidden_entries(pane, send)
            {
                self.hidden_entries.reread[pane.index()] = true;
            }
        }
    }

    /// Returns false while the pane must be asked again later.
    fn reread_for_hidden_entries(&mut self, pane: PaneId, send: &dyn Fn(Command) -> bool) -> bool {
        let p = &mut self.panes[pane.index()];
        // Network locations list servers and shares, never hidden entries.
        if p.network.is_some() || p.network_request || p.network_pending.is_some() {
            return true;
        }
        if p.archive.is_some() || p.archive_request {
            return !p.loading && self.refresh_location(pane, send);
        }
        if p.successful_generation.is_none() {
            // The first listing (at startup, before settings were read) was asked
            // for with the previous choice: ask again before any row is shown.
            let Some(path) = p.pending.as_ref().map(|request| request.path.clone()) else {
                return true;
            };
            return self.navigate(pane, path, None, send);
        }
        // A re-read waits for a pending request to commit; a blocked one is retried.
        p.refresh_hint = true;
        if let Some(navigation) = p.successful_generation.filter(|_| !p.loading) {
            self.refresh_if_changed(pane, navigation, send);
        }
        true
    }
}
