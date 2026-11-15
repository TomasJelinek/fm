//! One reversible Options session; unrelated session saves exclude its previews.
use super::*;

#[cfg(test)]
#[path = "options_tests.rs"]
mod tests;

#[derive(Default)]
pub(crate) struct State {
    pub(crate) open: bool,
    pub(crate) accepting: bool,
    pub(crate) saving: bool,
    pub(crate) error_topic: Option<i32>,
    pub(crate) feedback_revision: u64,
    pub(super) baseline: Option<Box<UserPreferences>>,
    commit_revision: Option<PreferencesRevision>,
}

/// Options-owned fields only. Geometry, navigation, viewer settings and terminal
/// choice belong to independent session actions and survive cancellation.
pub(super) fn restore_fields(to: &mut UserPreferences, from: &UserPreferences) {
    to.theme = from.theme;
    to.density = from.density;
    to.font_family.clone_from(&from.font_family);
    to.desktop_font_size = from.desktop_font_size;
    to.touch_font_size = from.touch_font_size;
    to.file_list.destination = from.file_list.destination;
    to.file_list.append = from.file_list.append;
    to.quick_search_filter = from.quick_search_filter;
    to.size_display = from.size_display;
    to.remember_command_history = from.remember_command_history;
    to.copy_checksum_verification = from.copy_checksum_verification;
    to.disk_io_concurrency = from.disk_io_concurrency;
    to.command_history.clone_from(&from.command_history);
    to.editor.clone_from(&from.editor);
    to.shortcuts.clone_from(&from.shortcuts);
    to.user_commands.clone_from(&from.user_commands);
    to.filename_colors.clone_from(&from.filename_colors);
    to.thumbnails_enabled = from.thumbnails_enabled;
    to.thumbnails_network = from.thumbnails_network;
    to.thumbnails_archives = from.thumbnails_archives;
    to.thumbnail_size = from.thumbnail_size;
    to.disabled_viewer_plugins
        .clone_from(&from.disabled_viewer_plugins);
    to.viewer_associations.clone_from(&from.viewer_associations);
    to.archive_association_overrides
        .clone_from(&from.archive_association_overrides);
    to.archive_custom_extensions
        .clone_from(&from.archive_custom_extensions);
}

impl PreferencesState {
    pub(super) fn persistable_values(&self) -> UserPreferences {
        let mut values = self.values.clone();
        if !self.options.saving
            && let Some(baseline) = &self.options.baseline
        {
            restore_fields(&mut values, baseline);
        }
        values
    }
}

impl Presenter {
    pub(crate) fn begin_options(&mut self) {
        if self.preferences.options.open {
            return;
        }
        self.preferences.options.open = true;
        self.preferences.options.error_topic = None;
        self.capture_options_baseline();
        self.discard_options_forms();
    }

    pub(super) fn capture_options_baseline(&mut self) {
        if self.preferences.options.open
            && self.preferences.options.baseline.is_none()
            && self.window_geometry_ready()
        {
            self.preferences.options.baseline = Some(Box::new(self.preferences.values.clone()));
        }
    }

    fn discard_options_forms(&mut self) {
        self.discard_editor_draft();
        self.discard_viewer_association_draft();
        self.cancel_color_draft();
        self.cancel_user_command_setting();
        self.keyboard.editor.candidate = None;
        self.keyboard.error.clear();
        self.keyboard.reset_editor();
    }

    /// Stage application defaults without discarding navigation or user content.
    pub(crate) fn reset_options(&mut self, send: &dyn Fn(Command) -> bool) {
        if !self.preferences.options.open
            || !self.window_geometry_ready()
            || self.preferences.options.baseline.is_none()
            || self.preferences.options.accepting
            || self.preferences.options.saving
            || self.settings_picker.busy()
        {
            return;
        }
        let defaults = UserPreferences {
            command_history: self.preferences.values.command_history.clone(),
            user_commands: self.preferences.values.user_commands.clone(),
            ..UserPreferences::default()
        };
        let mut values = self.preferences.values.clone();
        restore_fields(&mut values, &defaults);
        values.file_list.reset_options();
        self.discard_editor_draft();
        self.discard_viewer_association_draft();
        self.cancel_color_draft();
        self.keyboard.editor.candidate = None;
        self.keyboard.error.clear();
        self.keyboard.reset_editor();
        self.preferences.options.error_topic = None;
        // Preserve the custom-command draft too. It is user content, even before
        // OK, and resetting application preferences must not silently delete it.
        self.update_preferences(values, send);
    }

    pub(crate) fn cancel_options(&mut self, send: &dyn Fn(Command) -> bool) {
        if !self.preferences.options.open || self.preferences.options.saving {
            return;
        }
        if let Some(baseline) = self.preferences.options.baseline.take() {
            let mut restored = self.preferences.values.clone();
            restore_fields(&mut restored, &baseline);
            let filter_changed =
                restored.quick_search_filter != self.preferences.values.quick_search_filter;
            // The baseline was previously accepted. Rebuild shortcut hints along
            // with the values; don't route rollback through an autosave callback.
            if let Err(error) = self
                .keyboard
                .sync(&restored.shortcuts, &restored.user_commands)
            {
                self.preferences.save_error = error;
                self.preferences.options.baseline = Some(baseline);
                return;
            }
            self.retired.push(Box::new(mem::replace(
                &mut self.preferences.values,
                restored,
            )));
            self.retired.push(baseline);
            if filter_changed {
                self.search_filter_changed();
            }
        }
        self.preferences.options.open = false;
        self.preferences.options.accepting = false;
        self.preferences.options.error_topic = None;
        self.discard_options_forms();
        self.settings_picker_context(-1, send);
    }

    fn options_error_on(&mut self, topic: i32) {
        self.preferences.options.accepting = false;
        self.preferences.options.error_topic = Some(topic);
        self.preferences.options.feedback_revision =
            self.preferences.options.feedback_revision.saturating_add(1);
    }

    pub(crate) fn accept_options(&mut self, send: &dyn Fn(Command) -> bool) {
        if !self.preferences.options.open
            || !self.window_geometry_ready()
            || self.preferences.options.accepting
            || self.preferences.options.saving
            || self.settings_picker.busy()
        {
            return;
        }
        self.preferences.options.error_topic = None;
        if !self.save_viewer_extension_draft(send) {
            self.options_error_on(5);
            return;
        }
        if self.editor.dirty {
            self.save_editor_draft(send);
            if self.editor.dirty {
                self.options_error_on(4);
                return;
            }
        }
        if self.user_command_settings.dirty {
            let token = self.user_command_settings.token();
            let Some(form) = self.user_command_settings_form() else {
                self.user_command_settings.error =
                    "Select or discard the unfinished command.".into();
                self.options_error_on(9);
                return;
            };
            if !self.apply_user_command_setting(&token, &form, send) {
                self.options_error_on(9);
                return;
            }
        }
        self.preferences.options.accepting = true;
        if self.colors.editor.dirty && self.colors.validation.is_none() {
            crate::color_options::save_draft(self, send);
        }
        self.flush_options_acceptance(send);
    }

    pub(crate) fn flush_options_acceptance(&mut self, send: &dyn Fn(Command) -> bool) {
        if !self.preferences.options.accepting || self.colors.validation.is_some() {
            return;
        }
        if self.colors.editor.dirty || !self.colors.error.is_empty() {
            self.options_error_on(8);
            return;
        }
        self.preferences.options.accepting = false;
        self.preferences.options.saving = true;
        self.preferences.revision.0 = self.preferences.revision.0.saturating_add(1);
        self.preferences.options.commit_revision = Some(self.preferences.revision);
        self.preferences.unsaved = true;
        if !self.send_preferences_save(send) {
            self.preferences.options.saving = false;
            self.preferences.options.commit_revision = None;
        }
    }

    pub(super) fn options_save_result(
        &mut self,
        revision: PreferencesRevision,
        result: &Result<(), FsError>,
    ) {
        if !self.preferences.options.saving
            || self
                .preferences
                .options
                .commit_revision
                .is_none_or(|commit| revision < commit)
        {
            return;
        }
        if let Err(error) = result {
            // Local revisions can advance even when admission fails. Wait only
            // for a genuinely queued successor that can still commit Options.
            if self.preferences.pending_save_revision.is_none() {
                self.preferences.options.saving = false;
                self.preferences.options.commit_revision = None;
                self.preferences.save_error = crate::error_presentation::describe(error);
            }
            return;
        }
        // A later session save may already be queued. Any successful write at
        // or after OK's revision contains the frozen options and completes OK.
        self.preferences.options.open = false;
        self.preferences.options.saving = false;
        self.preferences.options.commit_revision = None;
        self.preferences.options.error_topic = None;
        self.retired
            .push(Box::new(self.preferences.options.baseline.take()));
    }
}
