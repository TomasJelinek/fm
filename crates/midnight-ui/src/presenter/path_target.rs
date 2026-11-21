//! A path typed or pasted in the path bar that names a file: the panel shows that
//! file's folder with the cursor on the file. The backend decides what the path names
//! (`Event::ListingNotAFolder`); a file that the panel's own listing shows gets the
//! cursor at once. While the panel does not show the file (it is hidden, the panel
//! filter leaves it out, or the folder does not list that name), a note leads its
//! status line and says why.
use super::*;

/// A file that the path bar named: the listing that shows its folder, and whether the
/// backend found it hidden.
pub(crate) struct FileTarget {
    navigation: NavigationId,
    path: PathBuf,
    hidden: bool,
}

/// What edited path text names: the text without surrounding whitespace and without
/// one pair of surrounding double quotes, which Windows "Copy as path" adds. Path Paste
/// ends what it pastes with a separator (`PATH_PASTE.md`), after a closing quote too;
/// such a separator belongs to no path. Other separators stay: a navigation drops a
/// final one, so the path of a file still names the file with one.
pub(super) fn typed_path(text: &str) -> &str {
    let text = text.trim();
    text.strip_prefix('"')
        .and_then(|quoted| {
            quoted.strip_suffix('"').or_else(|| {
                quoted
                    .strip_suffix(std::path::is_separator)?
                    .strip_suffix('"')
            })
        })
        .unwrap_or(text)
}

impl Presenter {
    /// Open the native `path` typed in the path bar. A file that the panel's listing
    /// shows gets the cursor at once; any other path is navigated to, and when the
    /// backend finds a file there, its folder opens with the cursor on it.
    pub(super) fn navigate_typed_path(
        &mut self,
        pane: PaneId,
        path: PathBuf,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        if self.focus_listed_file(pane, &path) {
            return true;
        }
        if !self.navigate(pane, path, None, send) {
            return false;
        }
        if let Some(request) = &mut self.panes[pane.index()].pending {
            request.accepts_file = true;
        }
        true
    }

    /// The panel's own listing of its folder (the backend's snapshot) has a file at
    /// `path`: the cursor moves onto it without a navigation, unless the panel filter
    /// leaves it out, which the status line then says. False for anything else, and in
    /// a view that a navigation leaves: an inline tree, a quick search or search results.
    fn focus_listed_file(&mut self, pane: PaneId, path: &Path) -> bool {
        let p = &self.panes[pane.index()];
        let Some(navigation) = p.successful_generation else {
            return false;
        };
        // An automatic re-read keeps the listed rows on screen, as `open` acts on them.
        if (p.loading && !p.keeps_rows_in_background())
            || !p.native_capabilities()
            || p.tree_view().is_some()
            || !p.search.query.is_empty()
            || path.parent() != Some(p.path.as_path())
            || self.feed(pane).is_some()
        {
            return false;
        }
        let Some(&id) = p.index.by_path.get(path) else {
            return false;
        };
        if p.index.is_directory(id) {
            return false;
        }
        let shown = p.search_view_waiting() || p.display_index().by_id.contains_key(&id);
        let p = &mut self.panes[pane.index()];
        if shown {
            if let Some(request) = &mut p.pending {
                // The re-read keeps this cursor rather than the one it started with.
                request.cursor_touched = true;
            }
            p.cursor = Some(id);
            p.parent_cursor = false;
            p.preserve_viewport = false;
        }
        let target = FileTarget {
            navigation,
            path: path.to_owned(),
            hidden: false,
        };
        self.retired.push(Box::new(p.file_target.replace(target)));
        true
    }

    /// The backend found that the pending path names `entry`, a file (or another entry
    /// that is not a folder) in `parent`. A path typed in the path bar lists `parent`
    /// instead, with the cursor on `entry`. Any other request waits for its failure,
    /// which follows; so does this one when the navigation queue refuses the folder.
    pub(super) fn apply_not_a_folder(
        &mut self,
        (pane, navigation): (PaneId, NavigationId),
        parent: PathBuf,
        entry: PathBuf,
        hidden: bool,
        send: &dyn Fn(Command) -> bool,
    ) {
        let p = &self.panes[pane.index()];
        let typed = p.generation == navigation.0
            && !p.archive_request
            && !p.network_request
            && p.pending.as_ref().is_some_and(|request| {
                request.navigation == navigation && request.accepts_file && request.path == entry
            })
            && entry.parent() == Some(parent.as_path());
        if !typed {
            self.retired.push(Box::new((parent, entry)));
            return;
        }
        // As Go to file from search results: a list of them leaves for the folder.
        self.feed_close(pane);
        if !self.navigate(pane, parent, None, send) {
            self.retired.push(Box::new(entry));
            return;
        }
        let p = &mut self.panes[pane.index()];
        if let Some(request) = &mut p.pending {
            request.wanted_cursor = Some(entry.clone());
        }
        let target = FileTarget {
            navigation: NavigationId(p.generation),
            path: entry,
            hidden,
        };
        self.retired.push(Box::new(p.file_target.replace(target)));
    }

    /// Why the panel does not show the file that the path bar named, as the note that
    /// leads its status line while it shows the listing that should show the file.
    pub(crate) fn file_target_note(&self, pane: PaneId) -> Option<panel_details::PanelDetail> {
        let p = &self.panes[pane.index()];
        let target = p.file_target.as_ref()?;
        if p.successful_generation != Some(target.navigation)
            || p.showing_stream()
            || !p.native_capabilities()
            || p.tree_view().is_some()
            || !p.search.query.is_empty()
            || p.search_view_waiting()
            || self.feed(pane).is_some()
        {
            return None;
        }
        let name = target.path.file_name()?.to_string_lossy();
        let hint = |action, known: &str, keys_do: &str| {
            let keys = self.shortcut_hint(action);
            if keys.is_empty() {
                format!("{known}.")
            } else {
                format!("{known}; {keys} {keys_do}.")
            }
        };
        let (reason, value, explanation) = match p.index.by_path.get(&target.path) {
            Some(id) if p.display_index().by_id.contains_key(id) => return None,
            Some(_) if p.search.panel_filter.enabled => (
                "filtered",
                format!("{name} is filtered out"),
                hint(
                    "panel.filter",
                    "The panel filter leaves this file out",
                    "edits the filter",
                ),
            ),
            Some(_) => return None,
            None if target.hidden && !self.preferences.values.show_hidden_files => (
                "hidden",
                format!("{name} is hidden"),
                hint(
                    "panel.hidden-files",
                    "Hidden files are not shown",
                    "shows them",
                ),
            ),
            // Removed meanwhile, or (on a filesystem that ignores letter case) typed
            // with other letters than the folder lists: the name is matched exactly.
            None => (
                "unlisted",
                format!("{name} is not listed here"),
                "The folder lists no file of exactly this name: it was removed, or its \
                 letters differ in case."
                    .into(),
            ),
        };
        Some(panel_details::PanelDetail {
            identity: format!("detail:{}:file-target:{reason}", p.generation),
            label: "Not shown",
            value,
            explanation,
        })
    }
}
