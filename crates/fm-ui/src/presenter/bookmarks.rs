//! Durable native hot paths. All persistence uses the existing settings worker.
use super::*;

impl Presenter {
    pub fn bookmarks(&self) -> &[Bookmark] {
        &self.preferences.values.bookmarks
    }

    /// Save the pane's displayed raw native path, never the displayed path text.
    pub fn add_bookmark(
        &mut self,
        pane: PaneId,
        name: String,
        slot: Option<u8>,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        if self.bookmarks().len() >= MAX_BOOKMARKS {
            self.desktop.report(
                "Bookmark limit reached",
                "Up to 64 bookmarks can be saved.".into(),
            );
            return false;
        }
        let Some(id) = self
            .bookmarks()
            .iter()
            .map(|bookmark| bookmark.id.0)
            .max()
            .unwrap_or(0)
            .max(self.preferences.bookmark_generation)
            .checked_add(1)
            .map(BookmarkId)
        else {
            return false;
        };
        let bookmark = Bookmark {
            id,
            name,
            path: self.panes[pane.index()].display_path().to_owned(),
            slot,
        };
        if let Err(error) = bookmark.validate() {
            self.desktop.report("Cannot save bookmark", error.into());
            return false;
        }
        let mut preferences = self.preferences.values.clone();
        if let Some(slot) = slot {
            for previous in &mut preferences.bookmarks {
                if previous.slot == Some(slot) {
                    previous.slot = None;
                }
            }
        }
        preferences.bookmarks.push(bookmark);
        self.preferences.bookmark_generation = id.0;
        self.update_preferences(preferences, send);
        true
    }

    pub fn open_bookmark(
        &mut self,
        pane: PaneId,
        id: BookmarkId,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let Some(bookmark) = self.bookmarks().iter().find(|bookmark| bookmark.id == id) else {
            return false;
        };
        self.navigate(pane, bookmark.path.clone(), None, send)
    }

    pub fn remove_bookmark(&mut self, id: BookmarkId, send: &dyn Fn(Command) -> bool) -> bool {
        let mut preferences = self.preferences.values.clone();
        let Some(index) = preferences
            .bookmarks
            .iter()
            .position(|bookmark| bookmark.id == id)
        else {
            return false;
        };
        preferences.bookmarks.remove(index);
        self.update_preferences(preferences, send);
        true
    }

    pub fn rename_bookmark(
        &mut self,
        id: BookmarkId,
        name: String,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let mut preferences = self.preferences.values.clone();
        let Some(bookmark) = preferences
            .bookmarks
            .iter_mut()
            .find(|bookmark| bookmark.id == id)
        else {
            return false;
        };
        bookmark.name = name;
        if let Err(error) = bookmark.validate() {
            self.desktop.report("Cannot rename bookmark", error.into());
            return false;
        }
        self.update_preferences(preferences, send);
        true
    }

    /// Ctrl+Shift+1..9/0 assigns the current location to an existing numbered
    /// bookmark, preserving its identity, or creates the slot on first use.
    pub fn assign_bookmark_slot(
        &mut self,
        pane: PaneId,
        slot: u8,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        if !(1..=10).contains(&slot) {
            return false;
        }
        let path = self.panes[pane.index()].display_path().to_owned();
        let mut name = path
            .file_name()
            .unwrap_or(path.as_os_str())
            .to_string_lossy()
            .into_owned();
        if name.len() > MAX_BOOKMARK_NAME_BYTES
            || name.trim().is_empty()
            || name.chars().any(char::is_control)
        {
            // A valid native basename can contain controls or expand beyond the
            // Unicode label bound when malformed bytes need replacement glyphs.
            name = format!("Hot path {}", if slot == 10 { 0 } else { slot });
        }
        if let Some(index) = self
            .bookmarks()
            .iter()
            .position(|bookmark| bookmark.slot == Some(slot))
        {
            let mut preferences = self.preferences.values.clone();
            let bookmark = &mut preferences.bookmarks[index];
            bookmark.path = path;
            bookmark.name = name;
            if let Err(error) = bookmark.validate() {
                self.desktop.report("Cannot assign hot path", error.into());
                return false;
            }
            self.update_preferences(preferences, send);
            true
        } else {
            self.add_bookmark(pane, name, Some(slot), send)
        }
    }

    pub fn jump_bookmark(
        &mut self,
        pane: PaneId,
        slot: u8,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let Some(bookmark) = self
            .bookmarks()
            .iter()
            .find(|bookmark| bookmark.slot == Some(slot))
        else {
            return false;
        };
        self.open_bookmark(pane, bookmark.id, send)
    }
}
