//! Find's accepted searches and placement are independent of Options previews.
use super::*;

impl Presenter {
    /// Persist the latest native Find placement, including final shutdown capture.
    pub fn flush_find_window_preferences(&mut self, send: &dyn Fn(Command) -> bool) {
        let geometry = self.finds.pending_window_geometry.borrow_mut().take();
        if let Some(geometry) = geometry {
            self.save_find_window_geometry(geometry, send);
        }
    }

    pub(crate) fn remember_find_mask(&mut self, mask: &str, send: &dyn Fn(Command) -> bool) {
        let mut find = self.preferences.values.find.clone();
        if find.remember_mask(mask).is_ok() {
            self.save_find_preferences(find, send);
        }
    }

    pub(crate) fn save_find_window_geometry(
        &mut self,
        geometry: WindowGeometry,
        send: &dyn Fn(Command) -> bool,
    ) {
        if geometry.validate().is_err() {
            return;
        }
        let mut find = self.preferences.values.find.clone();
        find.window_geometry = Some(geometry);
        self.save_find_preferences(find, send);
    }

    fn save_find_preferences(&mut self, find: FindPreferences, send: &dyn Fn(Command) -> bool) {
        if find.validate().is_err() || find == self.preferences.values.find {
            return;
        }
        if self.preferences.options.open && !self.preferences.loaded {
            self.preferences.edited.find_window_geometry |=
                find.window_geometry != self.preferences.values.find.window_geometry;
            self.preferences.edited.find_mask_history |=
                find.mask_history != self.preferences.values.find.mask_history;
            self.preferences.values.find = find;
            self.preferences.unsaved = true;
            self.preferences.revision.0 = self.preferences.revision.0.saturating_add(1);
            return;
        }
        let mut values = self.preferences.values.clone();
        values.find = find;
        self.update_preferences(values, send);
    }
}

#[cfg(test)]
#[path = "find_tests.rs"]
mod tests;
