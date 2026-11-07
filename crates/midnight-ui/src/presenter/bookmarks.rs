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
        if !self.native_action_allowed(pane) {
            return false;
        }
        let path = self.panes[pane.index()].display_path().to_owned();
        if slot.is_some_and(|slot| !(1..=10).contains(&slot)) {
            return false;
        }
        if let Some(existing) = self
            .bookmarks()
            .iter()
            .find(|bookmark| bookmark.path == path)
        {
            return if slot.is_some() {
                self.set_bookmark_slot(existing.id, slot, send)
            } else {
                true
            };
        }
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
            path,
            slot: slot.or_else(|| {
                (1..=10).find(|slot| !self.bookmarks().iter().any(|b| b.slot == Some(*slot)))
            }),
        };
        if let Err(error) = bookmark.validate() {
            self.desktop.report("Cannot save bookmark", error.into());
            return false;
        }
        let mut preferences = self.preferences.values.clone();
        if let Some(slot) = bookmark.slot {
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

    /// Change a saved path's slot without replacing either native location.
    /// Two occupied slots swap; an unnumbered path displaces only the slot.
    pub fn set_bookmark_slot(
        &mut self,
        id: BookmarkId,
        slot: Option<u8>,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        if slot.is_some_and(|slot| !(1..=10).contains(&slot)) {
            return false;
        }
        let mut preferences = self.preferences.values.clone();
        let Some(index) = preferences
            .bookmarks
            .iter()
            .position(|bookmark| bookmark.id == id)
        else {
            return false;
        };
        let previous_slot = preferences.bookmarks[index].slot;
        if previous_slot == slot {
            return true;
        }
        if let Some(slot) = slot {
            for bookmark in &mut preferences.bookmarks {
                if bookmark.slot == Some(slot) {
                    bookmark.slot = previous_slot;
                }
            }
        }
        preferences.bookmarks[index].slot = slot;
        self.update_preferences(preferences, send);
        true
    }

    /// Assign the current location, preserving every existing saved path.
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
        self.add_bookmark(pane, name, Some(slot), send)
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
