//! File-panel shortcut actions operate only on cached state and bounded messages.
use super::*;

impl Presenter {
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
        Some(ViewerLaunch {
            entries: panel.entries.clone(),
            view: panel.display_index().clone(),
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

    pub fn viewer_key(
        &self,
        event: &i_slint_core::input::KeyEvent,
        search: bool,
    ) -> Option<String> {
        let chord = crate::shortcut_input::chord(event)?;
        let context = if search {
            ShortcutContext::ViewerSearch
        } else {
            ShortcutContext::Viewer
        };
        self.keyboard
            .bindings
            .borrow()
            .map
            .resolve(&chord, context)
            .map(|action| action.id.clone())
    }

    /// Navigate to the displayed path's native absolute anchor without resolving it.
    pub(crate) fn go_root(&mut self, pane: PaneId, send: &dyn Fn(Command) -> bool) -> bool {
        let p = &mut self.panes[pane.index()];
        let path = p.display_path();
        if !path.is_absolute() {
            p.error = "Cannot determine a root for a relative pane path.".into();
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
        let p = &self.panes[pane.index()];
        if p.parent_cursor && p.has_parent() {
            return self.up(pane, send);
        }
        let path = p
            .cursor
            .and_then(|id| p.entry_by_id(id))
            .filter(|entry| matches!(entry.kind, EntryKind::Directory | EntryKind::DirectoryLink))
            .map(|entry| entry.path.clone());
        path.is_some_and(|path| self.navigate(pane, path, None, send))
    }

    /// Return the focused raw native basename for literal command-line insertion.
    pub(crate) fn focused_name(&self, pane: PaneId) -> Option<PathBuf> {
        let p = &self.panes[pane.index()];
        if p.parent_cursor && p.has_parent() {
            return Some(PathBuf::from(".."));
        }
        p.cursor_index()
            .and_then(|row| row.checked_sub(p.parent_offset()))
            .and_then(|row| p.display_entry(row))
            .and_then(|entry| entry.path.file_name())
            .map(PathBuf::from)
    }

    /// Show the displayed path and its cached backend capacity without filesystem work.
    pub(crate) fn drive_info(&mut self, pane: PaneId) {
        let p = &self.panes[pane.index()];
        let capacity = p.visible_space().map_or_else(
            || "Available: Unknown\nTotal: Unknown".into(),
            |space| {
                format!(
                    "Available: {} bytes\nTotal: {} bytes",
                    self.number_locale.format_u64(space.available),
                    self.number_locale.format_u64(space.total)
                )
            },
        );
        let body = format!("Path: {}\n{capacity}", p.display_path().display());
        self.desktop.report("Drive information", body);
    }
}
