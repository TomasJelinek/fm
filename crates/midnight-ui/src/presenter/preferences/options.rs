//! One reversible Options session; unrelated session saves exclude its previews.
use super::*;

#[cfg(test)]
#[path = "options_tests.rs"]
mod tests;

const PLUGINS_TOPIC: i32 = 5;

#[derive(Clone, Copy, PartialEq, Eq)]
enum PluginExtensionPanel {
    Viewer,
    Thumbnailer,
}

#[derive(Default)]
pub(crate) struct State {
    pub(crate) open: bool,
    pub(crate) accepting: bool,
    pub(crate) saving: bool,
    pub(crate) error_topic: Option<i32>,
    pub(crate) feedback_revision: u64,
    plugin_extension_panel: Option<PluginExtensionPanel>,
    pub(super) baseline: Option<Box<UserPreferences>>,
    commit_revision: Option<PreferencesRevision>,
}

/// Options-owned fields only. Geometry, navigation, viewer settings and terminal
/// choice belong to independent session actions and survive cancellation.
pub(super) fn restore_fields(to: &mut UserPreferences, from: &UserPreferences) {
    to.theme = from.theme;
    to.app_icon = from.app_icon;
    to.density = from.density;
    to.font_family.clone_from(&from.font_family);
    to.desktop_font_size = from.desktop_font_size;
    to.touch_font_size = from.touch_font_size;
    to.glyph_fitting = from.glyph_fitting;
    to.file_list.destination = from.file_list.destination;
    to.file_list.append = from.file_list.append;
    to.quick_search_filter = from.quick_search_filter;
    to.show_hidden_files = from.show_hidden_files;
    to.size_display = from.size_display;
    to.remember_command_history = from.remember_command_history;
    to.copy_checksum_verification = from.copy_checksum_verification;
    to.disk_io_concurrency = from.disk_io_concurrency;
    to.command_history.clone_from(&from.command_history);
    to.editor.clone_from(&from.editor);
    to.file_comparators.clone_from(&from.file_comparators);
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
    to.thumbnailer_associations
        .clone_from(&from.thumbnailer_associations);
    to.archive_association_overrides
        .clone_from(&from.archive_association_overrides);
    to.archive_custom_extensions
        .clone_from(&from.archive_custom_extensions);
    to.foreign_settings.clone_from(&from.foreign_settings);
}

/// The settings outside Options, which an imported settings file replaces only once
/// OK's save succeeds. The exhaustive pattern makes every new preference choose
/// between this and `restore_fields`.
pub(super) fn restore_session_fields(to: &mut UserPreferences, from: &UserPreferences) {
    let UserPreferences {
        // Options-owned: `restore_fields`.
        user_commands: _,
        archive_association_overrides: _,
        archive_custom_extensions: _,
        disabled_viewer_plugins: _,
        viewer_associations: _,
        editor: _,
        file_comparators: _,
        theme: _,
        app_icon: _,
        density: _,
        font_family: _,
        desktop_font_size: _,
        touch_font_size: _,
        glyph_fitting: _,
        quick_search_filter: _,
        size_display: _,
        disk_io_concurrency: _,
        remember_command_history: _,
        copy_checksum_verification: _,
        command_history: _,
        thumbnails_enabled: _,
        thumbnails_network: _,
        thumbnails_archives: _,
        thumbnailer_associations: _,
        thumbnail_size: _,
        shortcuts: _,
        filename_colors: _,
        show_hidden_files: _,
        foreign_settings: _,
        // Options owns the output choice; the session owns history and path.
        file_list,
        checksum_algorithms,
        window_geometry,
        panel_split_basis_points,
        viewer,
        windows_shell,
        bookmarks,
        foreign_hot_paths,
        tabs,
        panel_view_modes,
        panel_filters,
        panel_filter_history,
        // The Select/Deselect by mask dialog remembers its last pattern itself.
        mask_selection,
    } = from;
    to.file_list
        .template_history
        .clone_from(&file_list.template_history);
    to.file_list.path.clone_from(&file_list.path);
    to.checksum_algorithms = *checksum_algorithms;
    // Without a saved placement the window stays where it is.
    if window_geometry.is_some() {
        to.window_geometry.clone_from(window_geometry);
    }
    to.panel_split_basis_points = *panel_split_basis_points;
    to.viewer.clone_from(viewer);
    to.windows_shell = *windows_shell;
    to.bookmarks.clone_from(bookmarks);
    to.foreign_hot_paths.clone_from(foreign_hot_paths);
    // A panel without saved tabs keeps its current tabs.
    for (current, saved) in to.tabs.iter_mut().zip(tabs) {
        if !saved.paths.is_empty() {
            current.clone_from(saved);
        }
    }
    to.panel_view_modes = *panel_view_modes;
    to.panel_filters.clone_from(panel_filters);
    to.panel_filter_history.clone_from(panel_filter_history);
    to.mask_selection.clone_from(mask_selection);
}

impl PreferencesState {
    pub(super) fn persistable_values(&self) -> UserPreferences {
        let mut values = self.values.clone();
        if self.options.saving {
            // OK's save, and any session save behind it, includes a staged import.
            if let Some(staged) = &self.settings_file.staged {
                restore_session_fields(&mut values, staged);
            }
        } else if let Some(baseline) = &self.options.baseline {
            restore_fields(&mut values, baseline);
        }
        self.without_session_commands(&mut values);
        values
    }

    /// Commands run while history was not remembered stay session-only.
    pub(super) fn without_session_commands(&self, values: &mut UserPreferences) {
        if !self.session_commands.is_empty() {
            values
                .command_history
                .retain(|entry| !self.session_commands.contains(&entry.command));
        }
    }
}

impl Presenter {
    pub(crate) fn begin_options(&mut self) {
        if self.preferences.options.open {
            return;
        }
        self.preferences.options.open = true;
        self.preferences.options.error_topic = None;
        self.preferences
            .settings_file
            .next_session(&mut self.retired);
        self.capture_options_baseline();
        self.begin_comparator_options();
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

    pub(crate) fn preview_app_icon(&mut self, index: i32, send: &dyn Fn(Command) -> bool) {
        let options = &self.preferences.options;
        if options.baseline.is_none() || options.accepting || options.saving {
            return;
        }
        let Some(app_icon) = midnight_core::AppIcon::from_index(index) else {
            return;
        };
        self.update_preferences(
            UserPreferences {
                app_icon,
                ..self.preferences.values.clone()
            },
            send,
        );
    }

    pub(super) fn discard_options_forms(&mut self) {
        self.discard_editor_draft();
        self.discard_comparator_draft();
        self.discard_viewer_association_draft();
        self.discard_thumbnailer_association_draft();
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
            || self.preferences.settings_file.importing()
        {
            return;
        }
        // Reset keeps the current session state, including over a staged import.
        self.preferences
            .settings_file
            .discard_staged(&mut self.retired);
        let defaults = UserPreferences {
            command_history: self.preferences.values.command_history.clone(),
            user_commands: self.preferences.values.user_commands.clone(),
            // Kept settings of another system are user content, like custom commands.
            foreign_settings: self.preferences.values.foreign_settings.clone(),
            ..UserPreferences::default()
        };
        let mut values = self.preferences.values.clone();
        restore_fields(&mut values, &defaults);
        values.file_list.reset_options();
        self.discard_editor_draft();
        self.discard_comparator_draft();
        self.discard_viewer_association_draft();
        self.discard_thumbnailer_association_draft();
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
            let hidden_changed =
                restored.show_hidden_files != self.preferences.values.show_hidden_files;
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
            if hidden_changed {
                self.flush_hidden_entries(send);
            }
        }
        self.preferences.options.open = false;
        self.preferences.options.accepting = false;
        self.preferences.options.error_topic = None;
        self.preferences
            .settings_file
            .next_session(&mut self.retired);
        self.discard_options_forms();
        self.settings_picker_context(-1, send);
    }

    fn options_error_on(&mut self, topic: i32) {
        self.preferences.options.accepting = false;
        self.preferences.options.error_topic = Some(topic);
        self.preferences.options.feedback_revision =
            self.preferences.options.feedback_revision.saturating_add(1);
    }

    /// Runtime routing uses the accepted map while Options has a pending draft or save.
    pub fn accepted_thumbnailer_associations(&self) -> Option<Arc<ThumbnailerAssociations>> {
        if !self.editor_preferences_ready() {
            return None;
        }
        let preferences = self
            .preferences
            .options
            .baseline
            .as_deref()
            .unwrap_or(&self.preferences.values);
        Some(preferences.thumbnailer_associations.clone())
    }

    pub(crate) fn viewer_extension_feedback_revision(&self) -> Option<u64> {
        self.plugin_extension_feedback_revision(PluginExtensionPanel::Viewer)
    }

    pub(crate) fn thumbnailer_extension_feedback_revision(&self) -> Option<u64> {
        self.plugin_extension_feedback_revision(PluginExtensionPanel::Thumbnailer)
    }

    fn plugin_extension_feedback_revision(&self, panel: PluginExtensionPanel) -> Option<u64> {
        let options = &self.preferences.options;
        (options.error_topic == Some(PLUGINS_TOPIC)
            && options.plugin_extension_panel == Some(panel))
        .then_some(options.feedback_revision)
    }

    pub(crate) fn accept_options(&mut self, send: &dyn Fn(Command) -> bool) {
        if !self.preferences.options.open
            || !self.window_geometry_ready()
            || self.preferences.options.accepting
            || self.preferences.options.saving
            || self.settings_picker.busy()
            || self.preferences.settings_file.importing()
        {
            return;
        }
        self.preferences.options.error_topic = None;
        self.preferences.options.plugin_extension_panel = None;
        let extension_error = if !self.save_viewer_extension_draft(send) {
            Some(PluginExtensionPanel::Viewer)
        } else if !self.stage_thumbnailer_extension_draft() {
            Some(PluginExtensionPanel::Thumbnailer)
        } else {
            None
        };
        if let Some(panel) = extension_error {
            self.preferences.options.plugin_extension_panel = Some(panel);
            self.options_error_on(PLUGINS_TOPIC);
            return;
        }
        if self.editor.dirty {
            self.save_editor_draft(send);
            if self.editor.dirty {
                self.options_error_on(4);
                return;
            }
        }
        if !self.stage_comparator_draft(send) {
            self.options_error_on(10);
            return;
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
        self.comparator_submission(self.preferences.revision);
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
        comparator: Option<&FileComparatorDiagnostic>,
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
                if comparator.is_some_and(|diagnostic| {
                    self.comparator_diagnostic(Some(revision), diagnostic)
                }) {
                    self.options_error_on(10);
                }
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
        self.apply_staged_settings();
        self.preferences
            .settings_file
            .next_session(&mut self.retired);
        self.discard_comparator_draft();
    }
}
