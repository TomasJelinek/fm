//! Input choices and revision ordering; settings storage stays on the backend.
use super::*;
mod archive_associations;
mod file_list;
mod options;
mod viewers;

#[cfg(test)]
#[path = "preferences/archive_associations_tests.rs"]
mod archive_associations_tests;
#[cfg(test)]
mod panel_split_tests;
#[cfg(test)]
#[path = "preferences/save_tests.rs"]
mod save_tests;
#[cfg(test)]
#[path = "preferences/theme_tests.rs"]
mod theme_tests;
#[cfg(test)]
mod window_geometry_tests;

#[derive(Default)]
pub(crate) struct PreferencesState {
    pub(crate) options: options::State,
    pub(crate) values: UserPreferences,
    pub(crate) revision: PreferencesRevision,
    pub(crate) unsaved: bool,
    pub(crate) saving: bool,
    /// Latest admitted write. The serial backend may coalesce older revisions;
    /// a newer local edit rejected by the command queue is not an in-flight save.
    pending_save_revision: Option<PreferencesRevision>,
    pub(crate) save_error: String,
    /// Never reuse a removed bookmark identity while stale UI callbacks can exist.
    pub(super) bookmark_generation: u64,
    pub(super) loaded: bool,
    flush_after_load: bool,
    save_blocked: Option<String>,
    pub(super) edited: EditedFields,
    /// At most one pending change per viewer setting while the initial read runs.
    viewer_before_load: Vec<ViewerPreferenceChange>,
}

#[derive(Default)]
pub(super) struct EditedFields {
    window_geometry: bool,
    panel_split: bool,
    archive_associations: HashSet<String>,
    archive_custom_extensions: bool,
    pub(super) disabled_viewer_plugins: bool,
    pub(super) viewer_associations: bool,
    pub(super) editor: bool,
    pub(super) theme: bool,
    pub(super) density: bool,
    pub(super) windows_shell: bool,
    pub(super) font_family: bool,
    pub(super) desktop_font_size: bool,
    pub(super) touch_font_size: bool,
    pub(super) quick_search_filter: bool,
    pub(super) size_display: bool,
    pub(super) remember_command_history: bool,
    pub(super) copy_checksum_verification: bool,
    pub(super) disk_io_concurrency: bool,
    pub(super) command_history: bool,
    pub(super) command_history_cleared: bool,
    pub(super) panel_view_modes: [bool; 2],
    pub(super) thumbnails_enabled: bool,
    pub(super) thumbnails_network: bool,
    pub(super) thumbnail_size: bool,
    pub(super) panel_filters: [bool; 2],
    pub(super) panel_filter_history: bool,
    pub(super) shortcuts: bool,
    pub(super) filename_colors: bool,
    pub(super) checksum_algorithms: bool,
    file_list: bool,
    pub(super) user_commands: bool,
    bookmarks: bool,
}

impl PreferencesState {
    pub(crate) fn theme_index(&self) -> i32 {
        self.values.theme.index()
    }
    pub(crate) fn density_index(&self) -> i32 {
        match self.values.density {
            UiDensity::Desktop => 0,
            UiDensity::Touch => 1,
        }
    }
}

impl Presenter {
    pub(crate) fn preferences_save_status(&self) -> String {
        if !self.preferences.loaded {
            "Loading settings…".into()
        } else if let Some(error) = &self.preferences.save_blocked {
            error.clone()
        } else if !self.preferences.save_error.is_empty() {
            self.preferences.save_error.clone()
        } else if self.preferences.saving {
            "Saving…".into()
        } else if self.preferences.options.open {
            "Preview only. OK saves; Cancel discards changes.".into()
        } else if self.preferences.unsaved {
            "Changes have not been saved.".into()
        } else {
            "Changes saved".into()
        }
    }

    /// A saved geometry is meaningful only after the initial settings read succeeds.
    pub fn window_geometry_ready(&self) -> bool {
        self.preferences.loaded && self.preferences.save_blocked.is_none()
    }

    pub fn window_geometry(&self) -> Option<&midnight_core::WindowGeometry> {
        self.preferences.values.window_geometry.as_ref()
    }

    /// The UI captures native bounds; the existing bounded settings worker stores them.
    pub fn save_window_geometry(
        &mut self,
        geometry: midnight_core::WindowGeometry,
        send: &dyn Fn(Command) -> bool,
    ) {
        if geometry.validate().is_err() {
            return;
        }
        let mut values = self.preferences.values.clone();
        values.window_geometry = Some(geometry);
        self.update_preferences(values, send);
    }

    /// Commit a finished splitter gesture through the asynchronous settings worker.
    pub(crate) fn change_panel_split(&mut self, ratio: f32, send: &dyn Fn(Command) -> bool) {
        if !ratio.is_finite() || ratio <= 0.0 || ratio >= 1.0 {
            return;
        }
        let basis_points = (ratio * 10_000.0).round().clamp(1.0, 9999.0) as u16;
        if basis_points == self.preferences.values.panel_split_basis_points {
            return;
        }
        let mut values = self.preferences.values.clone();
        values.panel_split_basis_points = basis_points;
        self.update_preferences(values, send);
    }

    /// Retry an unsent final save during the same bounded shutdown grace.
    pub fn pending_preferences_save(&self) -> Option<Command> {
        (self.window_geometry_ready() && self.preferences.unsaved).then(|| {
            Command::SavePreferences {
                revision: self.preferences.revision,
                preferences: Box::new(self.preferences.persistable_values()),
            }
        })
    }

    pub(crate) fn change_thumbnail_options(
        &mut self,
        enabled: bool,
        network: bool,
        field: i32,
        send: &dyn Fn(Command) -> bool,
    ) {
        let mut values = self.preferences.values.clone();
        match field {
            0 => {
                self.preferences.edited.thumbnails_enabled = true;
                values.thumbnails_enabled = enabled;
            }
            1 => {
                self.preferences.edited.thumbnails_network = true;
                values.thumbnails_network = network;
            }
            _ => return,
        }
        self.update_preferences(values, send);
    }
    pub(super) fn editor_preferences_ready(&self) -> bool {
        self.preferences.loaded && self.preferences.save_blocked.is_none()
    }
    pub(crate) fn change_thumbnail_size(&mut self, size: i32, send: &dyn Fn(Command) -> bool) {
        let Ok(size) = u16::try_from(size) else {
            return;
        };
        if !midnight_core::THUMBNAIL_SIZES.contains(&size) {
            return;
        }
        self.preferences.edited.thumbnail_size = true;
        let mut values = self.preferences.values.clone();
        values.thumbnail_size = size;
        self.update_preferences(values, send);
    }
    pub(crate) fn change_panel_view_mode(
        &mut self,
        pane: PaneId,
        index: i32,
        send: &dyn Fn(Command) -> bool,
    ) {
        let mode = match index {
            0 => PanelViewMode::Detailed,
            1 => PanelViewMode::Brief,
            2 => PanelViewMode::Thumbnails,
            _ => return,
        };
        self.preferences.edited.panel_view_modes[pane.index()] = true;
        let mut values = self.preferences.values.clone();
        values.panel_view_modes[pane.index()] = mode;
        self.update_preferences(values, send);
    }

    /// Closing may briefly drain the initial read to publish local previews
    /// without overwriting settings that have not been read yet.
    pub fn needs_initial_preferences(&self) -> bool {
        !self.preferences.loaded && self.preferences.unsaved
    }

    pub(super) fn preferences_loaded(&mut self, result: Result<UserPreferences, FsError>) {
        if self.preferences.loaded {
            self.retired.push(Box::new(result));
            return;
        }
        self.preferences.loaded = true;
        let previous_filter = self.preferences.values.quick_search_filter;
        match result {
            Ok(mut loaded) => {
                for change in self.preferences.viewer_before_load.drain(..) {
                    // Each patch was validated before it entered this bounded set.
                    let _ = loaded.viewer.apply(&change);
                }
                self.viewer_plugins.startup_disabled =
                    Some(loaded.disabled_viewer_plugins.clone().into());
                self.viewer_plugins.startup_associations = loaded.viewer_associations.clone();
                self.prepare_loaded_tab_sessions(&mut loaded);
                let local = &mut self.preferences.values;
                let edited = &self.preferences.edited;
                if edited.window_geometry {
                    loaded.window_geometry = local.window_geometry.clone();
                }
                if edited.panel_split {
                    loaded.panel_split_basis_points = local.panel_split_basis_points;
                }
                for extension in &edited.archive_associations {
                    if let Some(enabled) = local.archive_association_overrides.get(extension) {
                        loaded
                            .archive_association_overrides
                            .insert(extension.clone(), *enabled);
                    } else {
                        loaded.archive_association_overrides.remove(extension);
                    }
                }
                if edited.archive_custom_extensions {
                    loaded.archive_custom_extensions = local.archive_custom_extensions.clone();
                }
                if edited.disabled_viewer_plugins {
                    loaded.disabled_viewer_plugins = local.disabled_viewer_plugins.clone();
                }
                if edited.viewer_associations {
                    loaded.viewer_associations = local.viewer_associations.clone();
                }
                if edited.editor {
                    loaded.editor = local.editor.clone();
                }
                if edited.theme {
                    loaded.theme = local.theme;
                }
                if edited.density {
                    loaded.density = local.density;
                }
                if edited.windows_shell {
                    loaded.windows_shell = local.windows_shell;
                }
                if edited.font_family {
                    loaded.font_family = local.font_family.clone();
                }
                if edited.desktop_font_size {
                    loaded.desktop_font_size = local.desktop_font_size;
                }
                if edited.touch_font_size {
                    loaded.touch_font_size = local.touch_font_size;
                }
                if edited.quick_search_filter {
                    loaded.quick_search_filter = local.quick_search_filter;
                }
                if edited.size_display {
                    loaded.size_display = local.size_display;
                }
                if edited.remember_command_history {
                    loaded.remember_command_history = local.remember_command_history;
                }
                if edited.copy_checksum_verification {
                    loaded.copy_checksum_verification = local.copy_checksum_verification;
                }
                if edited.disk_io_concurrency {
                    loaded.disk_io_concurrency = local.disk_io_concurrency;
                }
                if edited.shortcuts {
                    loaded.shortcuts = local.shortcuts.clone();
                }
                if edited.filename_colors {
                    loaded.filename_colors = local.filename_colors.clone();
                }
                if edited.checksum_algorithms {
                    loaded.checksum_algorithms = local.checksum_algorithms;
                }
                if edited.file_list {
                    loaded.file_list = local.file_list.clone();
                }
                if edited.user_commands
                    && let Err(error) = loaded.replace_user_commands(local.user_commands.clone())
                {
                    self.desktop.report(
                        "Could not merge Custom Commands",
                        crate::error_presentation::describe(&error),
                    );
                }
                if edited.thumbnails_enabled {
                    loaded.thumbnails_enabled = local.thumbnails_enabled;
                }
                if edited.thumbnails_network {
                    loaded.thumbnails_network = local.thumbnails_network;
                }
                if edited.thumbnail_size {
                    loaded.thumbnail_size = local.thumbnail_size;
                }
                for slot in 0..2 {
                    if edited.panel_filters[slot] {
                        loaded.panel_filters[slot] = local.panel_filters[slot].clone();
                    }
                    if edited.panel_view_modes[slot] {
                        loaded.panel_view_modes[slot] = local.panel_view_modes[slot];
                    }
                }
                if edited.panel_filter_history {
                    for filter in local.panel_filter_history.iter().rev() {
                        super::filter::remember_history(&mut loaded.panel_filter_history, filter);
                    }
                }
                if edited.command_history_cleared {
                    loaded.command_history = local.command_history.clone();
                } else if edited.command_history {
                    // Executed session commands are newer than the initial disk read.
                    for entry in local.command_history.iter().rev() {
                        remember_command(&mut loaded.command_history, entry.clone());
                    }
                }
                let local_generation = self.preferences.bookmark_generation;
                let mut generation = local_generation.max(
                    loaded
                        .bookmarks
                        .iter()
                        .map(|bookmark| bookmark.id.0)
                        .max()
                        .unwrap_or(0),
                );
                if edited.bookmarks {
                    // No saved bookmark has been shown before this first load.
                    // Preserve local callback IDs, including IDs removed locally.
                    // Explicit local slot assignments take precedence.
                    loaded
                        .bookmarks
                        .retain(|saved| !local.bookmarks.iter().any(|b| b.same_location(saved)));
                    let slots: HashSet<_> = local.bookmarks.iter().filter_map(|b| b.slot).collect();
                    let remapped = loaded
                        .bookmarks
                        .iter()
                        .filter(|bookmark| bookmark.id.0 <= local_generation)
                        .count() as u64;
                    if generation.checked_add(remapped).is_none() {
                        let message = "Bookmark identities are exhausted. Saved bookmarks \
                            remain on disk; restart before saving new bookmarks."
                            .to_owned();
                        self.desktop
                            .report("Cannot reconcile bookmarks", message.clone());
                        self.preferences.save_blocked = Some(message);
                        self.preferences.unsaved = true;
                        self.retired.push(Box::new(loaded));
                        return;
                    }
                    for bookmark in &mut loaded.bookmarks {
                        if bookmark.id.0 <= local_generation {
                            generation += 1; // The bounded remap count was checked above.
                            bookmark.id = BookmarkId(generation);
                        }
                        if bookmark.slot.is_some_and(|slot| slots.contains(&slot)) {
                            bookmark.slot = None;
                        }
                    }
                    loaded.bookmarks.append(&mut local.bookmarks);
                }
                if loaded.deduplicate_bookmarks() {
                    self.preferences.unsaved = true;
                }
                self.preferences.bookmark_generation = generation;
                if let Err(error) = self.keyboard.sync(&loaded.shortcuts, &loaded.user_commands) {
                    loaded.shortcuts = self.preferences.values.shortcuts.clone();
                    self.desktop.report("Cannot apply keyboard options", error);
                }
                self.retired
                    .push(Box::new(mem::replace(&mut self.preferences.values, loaded)));
            }
            Err(error) => {
                let message = format!(
                    "{}\n\nSaving is disabled to preserve existing settings. \
                    Restart after resolving the read error.",
                    crate::error_presentation::describe(&error)
                );
                self.desktop
                    .report("Could not load options", message.clone());
                self.preferences.save_blocked = Some(message);
            }
        }
        self.preferences.flush_after_load = self.preferences.unsaved;
        self.capture_options_baseline();
        self.sync_user_command_settings();
        let _ = previous_filter;
        self.search_filter_changed();
    }

    pub(super) fn flush_loaded_preferences(&mut self, send: &dyn Fn(Command) -> bool) {
        if !self.preferences.loaded {
            return;
        }
        self.flush_tab_sessions(send);
        if mem::take(&mut self.preferences.flush_after_load) {
            self.send_preferences_save(send);
        }
    }

    pub(super) fn preferences_saved(
        &mut self,
        revision: PreferencesRevision,
        result: Result<(), FsError>,
    ) {
        if self
            .preferences
            .pending_save_revision
            .is_some_and(|pending| revision >= pending)
        {
            self.preferences.pending_save_revision = None;
        }
        self.preferences.saving = self.preferences.pending_save_revision.is_some();
        self.options_save_result(revision, &result);
        if revision != self.preferences.revision {
            return;
        }
        self.preferences.saving = false;
        match result {
            Ok(()) => {
                self.preferences.unsaved = false;
                self.preferences.save_error.clear();
                self.desktop.status = self.preferences_save_status();
            }
            Err(error) => {
                self.preferences.options.saving = false;
                self.preferences.save_error = crate::error_presentation::describe(&error);
                self.desktop.status = "Options apply to this session; saving failed".into();
                if !self.preferences.options.open {
                    self.desktop.report(
                        "Could not save options",
                        crate::error_presentation::describe(&error),
                    );
                }
            }
        }
    }

    pub(crate) fn change_preferences(
        &mut self,
        theme: i32,
        density: i32,
        field: i32,
        send: &dyn Fn(Command) -> bool,
    ) {
        if field == 2 {
            if self.preferences.options.open {
                self.accept_options(send);
                return;
            }
            if self.preferences.loaded && !self.preferences.saving {
                // The persistence worker coalesces accepted revisions, including
                // completed failures. A deliberate retry needs a fresh identity.
                self.preferences.revision.0 = self.preferences.revision.0.saturating_add(1);
                self.preferences.unsaved = true;
                self.send_preferences_save(send);
            }
            return;
        }
        match field {
            0 => self.preferences.edited.theme = true,
            1 => self.preferences.edited.density = true,
            _ => {} // Save again changes no field merely by being clicked.
        }
        let preferences = UserPreferences {
            // Keep the existing Blue fallback for malformed picker callbacks.
            theme: PreferenceTheme::from_index(theme).unwrap_or_default(),
            density: if density == 1 {
                UiDensity::Touch
            } else {
                UiDensity::Desktop
            },
            ..self.preferences.values.clone()
        };
        self.update_preferences(preferences, send);
    }

    pub(crate) fn change_search_filter(&mut self, enabled: bool, send: &dyn Fn(Command) -> bool) {
        self.preferences.edited.quick_search_filter = true;
        let preferences = UserPreferences {
            quick_search_filter: enabled,
            ..self.preferences.values.clone()
        };
        self.update_preferences(preferences, send);
    }

    pub(crate) fn change_size_display(&mut self, bytes: bool, send: &dyn Fn(Command) -> bool) {
        self.preferences.edited.size_display = true;
        self.update_preferences(
            UserPreferences {
                size_display: if bytes {
                    SizeDisplay::Bytes
                } else {
                    SizeDisplay::Readable
                },
                ..self.preferences.values.clone()
            },
            send,
        );
    }

    pub(crate) fn change_remember_command_history(
        &mut self,
        enabled: bool,
        send: &dyn Fn(Command) -> bool,
    ) {
        self.preferences.edited.remember_command_history = true;
        self.update_preferences(
            UserPreferences {
                remember_command_history: enabled,
                ..self.preferences.values.clone()
            },
            send,
        );
    }

    pub(crate) fn change_copy_verification(
        &mut self,
        enabled: bool,
        send: &dyn Fn(Command) -> bool,
    ) {
        self.preferences.edited.copy_checksum_verification = true;
        self.update_preferences(
            UserPreferences {
                copy_checksum_verification: enabled,
                ..self.preferences.values.clone()
            },
            send,
        );
    }

    pub(crate) fn change_disk_io_concurrency(
        &mut self,
        automatic: bool,
        maximum: i32,
        send: &dyn Fn(Command) -> bool,
    ) {
        let disk_io_concurrency = if automatic {
            DiskIoConcurrency::Automatic
        } else {
            let Ok(maximum) = u8::try_from(maximum) else {
                return;
            };
            if !(1..=32).contains(&maximum) {
                return;
            }
            DiskIoConcurrency::Manual(maximum)
        };
        self.preferences.edited.disk_io_concurrency = true;
        self.update_preferences(
            UserPreferences {
                disk_io_concurrency,
                ..self.preferences.values.clone()
            },
            send,
        );
    }

    pub(crate) fn transfer_verification(&self) -> TransferVerification {
        // Preview changes belong to Options until OK; queued requests retain this value.
        let preferences = self
            .preferences
            .options
            .baseline
            .as_deref()
            .filter(|_| !self.preferences.options.saving)
            .unwrap_or(&self.preferences.values);
        TransferVerification::from_checksum_enabled(preferences.copy_checksum_verification)
    }

    pub(crate) fn clear_command_history(&mut self, send: &dyn Fn(Command) -> bool) {
        self.preferences.edited.command_history_cleared = true;
        self.preferences.edited.command_history = true;
        self.update_preferences(
            UserPreferences {
                command_history: Vec::new(),
                ..self.preferences.values.clone()
            },
            send,
        );
    }

    /// The backend reports the exact frozen command only after a successful launch.
    pub(super) fn record_command_history(&mut self, entry: CommandHistoryEntry) {
        if let Some(baseline) = &mut self.preferences.options.baseline {
            remember_command(&mut baseline.command_history, entry.clone());
        }
        if remember_command(&mut self.preferences.values.command_history, entry) {
            self.preferences.edited.command_history = true;
            self.preferences.unsaved = true;
            self.preferences.revision.0 = self.preferences.revision.0.saturating_add(1);
            self.preferences.flush_after_load = true;
        }
    }

    pub(super) fn update_preferences(
        &mut self,
        mut preferences: UserPreferences,
        send: &dyn Fn(Command) -> bool,
    ) {
        if self.preferences.options.open && !self.preferences.loaded {
            return;
        }
        if self.preferences.options.saving {
            let file_list = preferences.file_list.clone();
            options::restore_fields(&mut preferences, &self.preferences.values);
            // A completed file-list output remains an independent session action.
            preferences.file_list = file_list;
        }
        let before = self
            .preferences
            .options
            .open
            .then(|| self.preferences.persistable_values());
        if preferences == self.preferences.values
            && !self.preferences.unsaved
            && self.preferences.revision.0 != 0
        {
            return;
        }
        if let Err(error) = self
            .keyboard
            .sync(&preferences.shortcuts, &preferences.user_commands)
        {
            self.keyboard.error = error;
            return;
        }
        let previous = &self.preferences.values;
        let edited = &mut self.preferences.edited;
        edited.window_geometry |= preferences.window_geometry != previous.window_geometry;
        edited.panel_split |=
            preferences.panel_split_basis_points != previous.panel_split_basis_points;
        edited.archive_custom_extensions |=
            preferences.archive_custom_extensions != previous.archive_custom_extensions;
        for association in ARCHIVE_ASSOCIATIONS {
            if preferences
                .archive_association_overrides
                .get(association.extension)
                != previous
                    .archive_association_overrides
                    .get(association.extension)
            {
                edited
                    .archive_associations
                    .insert(association.extension.to_owned());
            }
        }
        edited.disabled_viewer_plugins |=
            preferences.disabled_viewer_plugins != previous.disabled_viewer_plugins;
        edited.viewer_associations |=
            preferences.viewer_associations != previous.viewer_associations;
        edited.editor |= preferences.editor != previous.editor;
        edited.theme |= preferences.theme != previous.theme;
        edited.density |= preferences.density != previous.density;
        edited.windows_shell |= preferences.windows_shell != previous.windows_shell;
        edited.font_family |= preferences.font_family != previous.font_family;
        edited.desktop_font_size |= preferences.desktop_font_size != previous.desktop_font_size;
        edited.touch_font_size |= preferences.touch_font_size != previous.touch_font_size;
        let filter_changed = preferences.quick_search_filter != previous.quick_search_filter
            || preferences.panel_filters != previous.panel_filters;
        edited.quick_search_filter |=
            preferences.quick_search_filter != previous.quick_search_filter;
        edited.panel_filter_history |=
            preferences.panel_filter_history != previous.panel_filter_history;
        edited.size_display |= preferences.size_display != previous.size_display;
        edited.copy_checksum_verification |=
            preferences.copy_checksum_verification != previous.copy_checksum_verification;
        edited.disk_io_concurrency |=
            preferences.disk_io_concurrency != previous.disk_io_concurrency;
        edited.remember_command_history |=
            preferences.remember_command_history != previous.remember_command_history;
        edited.command_history |= preferences.command_history != previous.command_history;
        edited.bookmarks |= preferences.bookmarks != previous.bookmarks;
        edited.shortcuts |= preferences.shortcuts != previous.shortcuts;
        edited.filename_colors |= preferences.filename_colors != previous.filename_colors;
        edited.file_list |= preferences.file_list != previous.file_list;
        edited.user_commands |= preferences.user_commands != previous.user_commands;
        edited.thumbnails_enabled |= preferences.thumbnails_enabled != previous.thumbnails_enabled;
        edited.thumbnails_network |= preferences.thumbnails_network != previous.thumbnails_network;
        edited.thumbnail_size |= preferences.thumbnail_size != previous.thumbnail_size;
        for slot in 0..2 {
            edited.panel_filters[slot] |=
                preferences.panel_filters[slot] != previous.panel_filters[slot];
            edited.panel_view_modes[slot] |=
                preferences.panel_view_modes[slot] != previous.panel_view_modes[slot];
        }
        self.retired.push(Box::new(mem::replace(
            &mut self.preferences.values,
            preferences,
        )));
        self.sync_user_command_settings();
        if filter_changed {
            self.search_filter_changed();
        }
        if before.is_some_and(|before| before == self.preferences.persistable_values()) {
            return;
        }
        self.preferences.unsaved = true;
        self.preferences.revision.0 = self.preferences.revision.0.saturating_add(1);
        if !self.preferences.loaded {
            self.desktop.status = "Options previewed; waiting for saved options to load".into();
            return;
        }
        self.preferences.flush_after_load = false;
        self.send_preferences_save(send);
    }

    fn send_preferences_save(&mut self, send: &dyn Fn(Command) -> bool) -> bool {
        self.preferences.saving = self.preferences.pending_save_revision.is_some();
        self.preferences.save_error.clear();
        if let Some(message) = &self.preferences.save_blocked {
            self.preferences.save_error = message.clone();
            self.desktop.report("Cannot save options", message.clone());
            return false;
        }
        let values = self.preferences.persistable_values();
        if let Err(error) = values.validate() {
            self.preferences.save_error = crate::error_presentation::describe(&error);
            self.desktop.status = "Options apply to this session; saving needs attention".into();
            self.desktop.report(
                "Cannot save options",
                format!(
                    "{error}\nRemove bookmarks until at most 64 remain, then save again. \
                 Existing saved bookmarks have not been replaced."
                ),
            );
            return false;
        }
        if send(Command::SavePreferences {
            revision: self.preferences.revision,
            preferences: Box::new(values),
        }) {
            self.preferences.pending_save_revision = Some(self.preferences.revision);
            self.preferences.saving = true;
            self.desktop.status = "Saving options…".into();
            true
        } else {
            self.preferences.save_error = "The save queue is busy. Try OK again.".into();
            self.desktop.status = "Options apply to this session; saving is busy".into();
            if !self.preferences.options.open {
                self.desktop.report(
                    "Options queue busy",
                    "The preview is active. Use OK in Options to retry saving.".into(),
                );
            }
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, io};

    #[test]
    fn thumbnail_size_survives_a_late_load_and_rejects_unsupported_input() {
        let mut p = Presenter::new("left".into(), "right".into());
        p.change_thumbnail_size(192, &|_| panic!("must await preferences load"));
        p.preferences_loaded(Ok(UserPreferences {
            thumbnail_size: 48,
            thumbnails_network: true,
            theme: PreferenceTheme::Nord,
            ..Default::default()
        }));
        assert_eq!(p.preferences.values.thumbnail_size, 192);
        assert!(p.preferences.values.thumbnails_network);
        assert_eq!(p.preferences.values.theme, PreferenceTheme::Nord);
        for invalid in [-1, 0, 73, 257, i32::MAX] {
            p.change_thumbnail_size(invalid, &|_| panic!("invalid input must not save"));
            assert_eq!(p.preferences.values.thumbnail_size, 192);
        }
    }

    #[test]
    fn early_preview_enable_edit_preserves_saved_network_choice_and_other_preferences() {
        let mut presenter = Presenter::new("left".into(), "right".into());
        presenter
            .change_thumbnail_options(false, false, 0, &|_| panic!("Must await preferences load"));
        presenter.preferences_loaded(Ok(UserPreferences {
            thumbnails_enabled: true,
            thumbnails_network: true,
            theme: PreferenceTheme::Nord,
            ..Default::default()
        }));
        assert!(!presenter.preferences.values.thumbnails_enabled);
        assert!(presenter.preferences.values.thumbnails_network);
        presenter.flush_loaded_preferences(&|command| {
            let Command::SavePreferences { preferences, .. } = command else {
                panic!("Expected merged save");
            };
            assert!(!preferences.thumbnails_enabled);
            assert!(preferences.thumbnails_network);
            assert_eq!(preferences.theme, PreferenceTheme::Nord);
            true
        });
    }

    #[test]
    fn explicit_default_network_choice_wins_over_late_opt_in_without_changing_enable() {
        let mut presenter = Presenter::new("left".into(), "right".into());
        presenter
            .change_thumbnail_options(true, false, 1, &|_| panic!("Must await preferences load"));
        presenter.preferences_loaded(Ok(UserPreferences {
            thumbnails_enabled: false,
            thumbnails_network: true,
            ..Default::default()
        }));
        assert!(!presenter.preferences.values.thumbnails_enabled);
        assert!(!presenter.preferences.values.thumbnails_network);
    }

    #[test]
    fn quick_search_filter_preview_survives_late_load_and_saves_other_loaded_choices() {
        let mut p = Presenter::new("left".into(), "right".into());
        p.change_search_filter(true, &|_| panic!("must await saved preferences"));
        p.preferences_loaded(Ok(UserPreferences {
            theme: PreferenceTheme::Dark,
            font_family: "Stored 文档 e\u{301}".into(),
            quick_search_filter: false,
            ..Default::default()
        }));
        assert!(p.preferences.values.quick_search_filter);
        assert_eq!(p.preferences.values.font_family, "Stored 文档 e\u{301}");
        p.flush_loaded_preferences(&|command| {
            let Command::SavePreferences { preferences, .. } = command else {
                panic!()
            };
            assert!(preferences.quick_search_filter);
            assert_eq!(preferences.theme, PreferenceTheme::Dark);
            assert_eq!(preferences.font_family, "Stored 文档 e\u{301}");
            true
        });
    }

    #[test]
    fn quick_search_filter_explicit_default_choice_wins_over_late_enabled_setting() {
        let mut p = Presenter::new("left".into(), "right".into());
        p.change_search_filter(false, &|_| panic!("must await saved preferences"));
        p.preferences_loaded(Ok(UserPreferences {
            quick_search_filter: true,
            density: UiDensity::Touch,
            ..Default::default()
        }));
        assert!(!p.preferences.values.quick_search_filter);
        assert_eq!(p.preferences.values.density, UiDensity::Touch);
    }

    #[test]
    fn unchanged_provisional_defaults_do_not_mask_saved_values() {
        let mut p = Presenter::new("left".into(), "right".into());
        p.change_preferences(0, 0, -1, &|_| true);
        p.apply(Event::PreferencesLoaded(Ok(Box::new(UserPreferences {
            theme: PreferenceTheme::Light,
            density: UiDensity::Touch,
            ..Default::default()
        }))));
        assert_eq!(p.preferences.values.theme, PreferenceTheme::Light);
        assert_eq!(p.preferences.values.density, UiDensity::Touch);
        assert_eq!(p.preferences.revision, PreferencesRevision(1));
    }

    #[test]
    fn local_preview_wins_over_late_load_and_stale_save_results() {
        let mut p = Presenter::new("left".into(), "right".into());
        let sent = RefCell::new(vec![]);
        let send = |command| {
            sent.borrow_mut().push(command);
            true
        };
        p.change_preferences(2, 1, -1, &send);
        p.apply_with_sender(Event::PreferencesLoaded(Ok(Box::default())), &send);
        assert_eq!(
            p.preferences.values,
            UserPreferences {
                theme: PreferenceTheme::Light,
                density: UiDensity::Touch,
                ..Default::default()
            }
        );
        assert_eq!(p.preferences.theme_index(), 2);
        assert_eq!(p.preferences.density_index(), 1);
        p.change_preferences(1, 0, -1, &send);
        let error = || {
            FsError::from_io(
                "save",
                Path::new("options"),
                io::Error::from(io::ErrorKind::PermissionDenied),
            )
        };
        p.apply(Event::PreferencesSaved {
            revision: PreferencesRevision(1),
            result: Err(error()),
        });
        assert!(p.desktop.notice.is_none());
        p.apply(Event::PreferencesSaved {
            revision: PreferencesRevision(2),
            result: Err(error()),
        });
        assert!(p.desktop.notice.is_some());
        assert_eq!(
            p.preferences.values,
            UserPreferences {
                theme: PreferenceTheme::Dark,
                density: UiDensity::Desktop,
                ..Default::default()
            }
        );
        assert!(matches!(
            &sent.borrow()[0],
            Command::SavePreferences {
                revision: PreferencesRevision(1),
                preferences,
            } if preferences.theme == PreferenceTheme::Light && preferences.density == UiDensity::Touch
        ));
        assert!(matches!(
            sent.borrow()[1],
            Command::SavePreferences {
                revision: PreferencesRevision(2),
                ..
            }
        ));
    }

    #[test]
    fn failed_enqueue_keeps_preview_and_allows_retrying_the_same_choices() {
        let mut p = Presenter::new("left".into(), "right".into());
        p.apply(Event::PreferencesLoaded(Ok(Box::default())));
        p.change_preferences(2, 1, -1, &|_| false);
        assert_eq!(p.preferences.theme_index(), 2);
        assert_eq!(p.preferences.density_index(), 1);
        let sent = RefCell::new(vec![]);
        p.change_preferences(2, 1, -1, &|command| {
            sent.borrow_mut().push(command);
            true
        });
        assert_eq!(sent.borrow().len(), 1);
        assert!(matches!(
            sent.borrow()[0],
            Command::SavePreferences {
                revision: PreferencesRevision(2),
                ..
            }
        ));
        p.apply(Event::PreferencesSaved {
            revision: PreferencesRevision(2),
            result: Ok(()),
        });
        p.change_preferences(2, 1, -1, &|_| {
            panic!("confirmed unchanged choices must not save again")
        });
    }
}
