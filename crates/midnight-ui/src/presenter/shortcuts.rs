//! File-panel shortcut actions operate only on cached state and bounded messages.
use super::*;

impl Presenter {
    /// Why `viewer_launch` declined, for every case in which the user asked to view a
    /// file; a viewer request must never end silently. Focusing a folder or the parent
    /// entry is not such a request.
    pub fn viewer_unavailable_reason(&self, pane: PaneId) -> Option<String> {
        if self.viewer_plugins.startup_disabled.is_none() {
            return Some(if self.viewer_plugins.startup_read_failed {
                self.viewer_plugins.policy_unavailable().into()
            } else {
                "Viewer settings are still being read. Try again in a moment.".into()
            });
        }
        let panel = &self.panes[pane.index()];
        if panel.tree_view().is_some() && self.viewer_launch(pane, false).is_none() {
            return Some(
                "Focus a readable regular file in the adopted tree before viewing.".into(),
            );
        }
        if panel.parent_cursor {
            return None;
        }
        let entry = panel.cursor.and_then(|id| panel.entry_by_id(id))?;
        if let Some(node) = &entry.virtual_node {
            if !node.readable {
                return Some(node.unreadable_reason());
            }
            if panel
                .archive
                .as_ref()
                .is_none_or(|archive| archive.location.mount.is_none())
            {
                return Some(
                    "This archive is no longer open. Open it again to view its files.".into(),
                );
            }
        }
        match entry.kind {
            EntryKind::Directory | EntryKind::DirectoryLink => return None,
            EntryKind::Symlink => {
                return Some(
                    "Symbolic links are not opened by the viewer. View the file the link points to."
                        .into(),
                );
            }
            EntryKind::Other => {
                return Some(
                    "Devices, pipes, sockets and other special files cannot be viewed.".into(),
                );
            }
            EntryKind::File => {}
        }
        if panel.loading {
            return Some(
                "This folder is still being listed. View the file when the listing is complete."
                    .into(),
            );
        }
        if !panel.search_allows_activation() {
            return Some(
                "The quick search does not match the focused file yet. Finish or clear it, then view the file."
                    .into(),
            );
        }
        None
    }
    /// Capture the exact displayed ordering without walking or opening any files.
    pub fn viewer_launch(&self, pane: PaneId, basic: bool) -> Option<ViewerLaunch> {
        let disabled_plugins = self.viewer_plugins.startup_disabled.clone()?;
        let panel = &self.panes[pane.index()];
        if panel.loading || panel.parent_cursor || !panel.search_allows_activation() {
            return None;
        }
        let entry = panel.cursor.and_then(|id| panel.entry_by_id(id))?;
        if entry.kind != EntryKind::File {
            return None;
        }
        let tree = if let Some(view) = panel.tree_view() {
            if !panel.tree_input_ready() {
                return None;
            }
            let row = panel.tree_cursor_row()?;
            if row.depth != 0
                || row.parent.is_some()
                || !row.selectable
                || row.context_only
                || row.original_root_entry.is_none()
                || row.error.is_some()
            {
                return None;
            }
            Some(TreeViewer {
                view: view.clone(),
                focused: row.id,
            })
        } else {
            None
        };
        let (entries, view) = tree.as_ref().map_or_else(
            || (panel.entries.clone(), panel.display_index().clone()),
            |tree| {
                (
                    tree.view.root_actions.entries.clone(),
                    tree.view.root_actions.index.clone(),
                )
            },
        );
        Some(ViewerLaunch {
            find: None,
            associations: self.viewer_plugins.startup_associations.clone(),
            tree,
            generated: None,
            preferences: Arc::new(self.preferences.values.viewer.clone()),
            archive: match (&panel.archive, &entry.virtual_node) {
                (Some(archive), Some(node)) if node.readable => Some(ArchiveViewer {
                    mount: archive.location.mount.clone()?,
                    source_version: archive.location.address.source_version.clone(),
                    focused: node.id,
                }),
                (_, Some(_)) => return None,
                _ => None,
            },
            entries,
            view,
            focused: entry.path.clone(),
            basic,
            disabled_plugins,
        })
    }

    pub fn report_viewer_error(&mut self, message: String) {
        self.desktop.report("Cannot open viewer", message);
    }

    pub fn copy_viewer_text(&mut self, text: String, send: &dyn Fn(Command) -> bool) {
        self.queue_clipboard_text(text, send);
    }

    pub fn viewer_key(&self, chord: &ShortcutChord, search: bool) -> Option<String> {
        let context = if search {
            ShortcutContext::ViewerSearch
        } else {
            ShortcutContext::Viewer
        };
        self.tool_key(chord, context)
    }

    pub fn comparison_key(&self, chord: &ShortcutChord, search: bool) -> Option<String> {
        let context = if search {
            ShortcutContext::ComparisonSearch
        } else {
            ShortcutContext::Comparison
        };
        self.tool_key(chord, context)
    }

    fn tool_key(&self, chord: &ShortcutChord, context: ShortcutContext) -> Option<String> {
        self.keyboard
            .bindings
            .borrow()
            .map
            .resolve(chord, context)
            .map(|action| action.id.clone())
    }

    /// Navigate to the displayed path's native absolute anchor without resolving it.
    pub(crate) fn go_root(&mut self, pane: PaneId, send: &dyn Fn(Command) -> bool) -> bool {
        if self.panes[pane.index()].network.is_some()
            || self.panes[pane.index()].network_pending.is_some()
        {
            return self.network_navigate(pane, NetworkAction::OpenRoot, send);
        }
        if let Some(a) = &self.panes[pane.index()].archive {
            let mut location = a.location.clone();
            location.directory = 1;
            location.trail.truncate(1);
            location.address.components.clear();
            return self.archive_navigate(pane, ArchiveAction::List(location), send);
        }
        let p = &mut self.panes[pane.index()];
        let path = p.display_path();
        if !path.is_absolute() {
            p.set_error(
                "Cannot determine a root for a relative pane path.",
                &mut self.retired,
            );
            return false;
        }
        let root = path
            .components()
            .take_while(|component| matches!(component, Component::Prefix(_) | Component::RootDir))
            .collect();
        self.navigate(pane, root, None, send)
    }

    /// Explicitly browse a known folder/link or parent; never launch a host file handler.
    pub(crate) fn open_folder(&mut self, pane: PaneId, send: &dyn Fn(Command) -> bool) -> bool {
        if self.panes[pane.index()].network_hides_rows() {
            return false;
        }
        if self.panes[pane.index()].tree_view().is_some() {
            let panel = &self.panes[pane.index()];
            if !panel.parent_cursor
                && !panel
                    .cursor
                    .and_then(|id| panel.entry_by_id(id))
                    .is_some_and(|entry| {
                        matches!(entry.kind, EntryKind::Directory | EntryKind::DirectoryLink)
                    })
            {
                return false;
            }
            let row = panel.cursor_index().unwrap_or(0);
            return self.open(pane, row, send);
        }
        if self.panes[pane.index()].network.is_some() {
            return self.network_open(
                pane,
                self.panes[pane.index()].cursor_index().unwrap_or(0),
                send,
            );
        }
        if self.panes[pane.index()].archive.is_some() {
            let row = self.panes[pane.index()].cursor_index().unwrap_or(0);
            return self.archive_open(pane, row, send);
        }
        let p = &self.panes[pane.index()];
        if p.parent_cursor && p.has_parent() {
            return self.up(pane, send);
        }
        let path = p
            .cursor
            .and_then(|id| p.entry_by_id(id))
            .filter(|entry| matches!(entry.kind, EntryKind::Directory | EntryKind::DirectoryLink))
            .map(|entry| (entry.kind, entry.path.clone()));
        path.is_some_and(|(kind, path)| {
            if kind == EntryKind::DirectoryLink {
                self.open_native_directory(pane, path, send)
            } else {
                self.navigate(pane, path, None, send)
            }
        })
    }

    /// Return the current raw native item path for literal command-draft insertion.
    /// Search results fed to the panel give their focused result, never the
    /// folder hidden under them.
    pub(crate) fn focused_full_path(&self, pane: PaneId) -> Option<PathBuf> {
        if let Some(feed) = self.feed(pane) {
            return feed
                .focused()
                .map(|entry| entry.path.clone())
                .filter(|path| path.is_absolute());
        }
        focused_native_path(&self.panes[pane.index()])
            .filter(|path| path.is_absolute())
            .map(Path::to_path_buf)
    }

    /// Return the focused raw native basename for literal command-line insertion.
    /// None under search results fed to the panel: a result's name alone would
    /// name a file of the folder under the list, where the command runs.
    pub(crate) fn focused_name(&self, pane: PaneId) -> Option<PathBuf> {
        if self.feed(pane).is_some() {
            return None;
        }
        let p = &self.panes[pane.index()];
        if p.loading || !p.native_capabilities() {
            return None;
        }
        if p.parent_cursor && p.has_parent() {
            return (p.tree_view().is_none() || p.search_allows_activation())
                .then(|| PathBuf::from(".."));
        }
        focused_native_path(p)
            .and_then(Path::file_name)
            .map(PathBuf::from)
    }
}

/// Borrow only a ready visible native occurrence; insertion performs no provider work.
fn focused_native_path(pane: &Pane) -> Option<&Path> {
    if pane.loading
        || !pane.native_capabilities()
        || pane.parent_cursor
        || !pane.search_allows_activation()
    {
        return None;
    }
    let entry = if pane.tree_view().is_some() {
        &pane.tree_cursor_row()?.entry
    } else {
        pane.entry_by_id(pane.cursor?)?
    };
    entry.native_path()
}
