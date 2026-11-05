//! Input choices and revision ordering; settings storage stays on the backend.
use super::*;

#[derive(Default)]
pub(crate) struct PreferencesState {
    pub(crate) values: UserPreferences,
    pub(crate) revision: PreferencesRevision,
    pub(crate) unsaved: bool,
    /// Never reuse a removed bookmark identity while stale UI callbacks can exist.
    pub(super) bookmark_generation: u64,
    pub(super) loaded: bool,
    flush_after_load: bool,
    save_blocked: Option<String>,
    pub(super) edited: EditedFields,
}

#[derive(Default)]
pub(super) struct EditedFields {
    pub(super) theme: bool,
    pub(super) density: bool,
    pub(super) windows_shell: bool,
    pub(super) font_family: bool,
    pub(super) desktop_font_size: bool,
    pub(super) touch_font_size: bool,
    pub(super) quick_search_filter: bool,
    bookmarks: bool,
}

impl PreferencesState {
    pub(crate) fn theme_index(&self) -> i32 {
        match self.values.theme {
            PreferenceTheme::Blue => 0,
            PreferenceTheme::Dark => 1,
            PreferenceTheme::Light => 2,
        }
    }
    pub(crate) fn density_index(&self) -> i32 {
        match self.values.density {
            UiDensity::Desktop => 0,
            UiDensity::Touch => 1,
        }
    }
}

impl Presenter {
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
                let local = &mut self.preferences.values;
                let edited = &self.preferences.edited;
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
                self.preferences.bookmark_generation = generation;
                self.retired
                    .push(Box::new(mem::replace(&mut self.preferences.values, loaded)));
            }
            Err(error) => {
                let message = format!(
                    "{error}\nSaving is disabled to preserve existing settings. \
                    Restart after resolving the read error."
                );
                self.desktop
                    .report("Could not load options", message.clone());
                self.preferences.save_blocked = Some(message);
            }
        }
        self.preferences.flush_after_load = self.preferences.unsaved;
        if previous_filter != self.preferences.values.quick_search_filter {
            self.search_filter_changed();
        }
    }

    pub(super) fn flush_loaded_preferences(&mut self, send: &dyn Fn(Command) -> bool) {
        if mem::take(&mut self.preferences.flush_after_load) {
            self.send_preferences_save(send);
        }
    }

    pub(super) fn preferences_saved(
        &mut self,
        revision: PreferencesRevision,
        result: Result<(), FsError>,
    ) {
        if revision != self.preferences.revision {
            return;
        }
        match result {
            Ok(()) => {
                self.preferences.unsaved = false;
                self.desktop.status = "Options saved".into();
            }
            Err(error) => {
                self.desktop.status = "Options apply to this session; saving failed".into();
                self.desktop
                    .report("Could not save options", error.to_string());
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
        match field {
            0 => self.preferences.edited.theme = true,
            1 => self.preferences.edited.density = true,
            _ => {} // Save again changes no field merely by being clicked.
        }
        let preferences = UserPreferences {
            theme: match theme {
                1 => PreferenceTheme::Dark,
                2 => PreferenceTheme::Light,
                _ => PreferenceTheme::Blue,
            },
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

    pub(super) fn update_preferences(
        &mut self,
        preferences: UserPreferences,
        send: &dyn Fn(Command) -> bool,
    ) {
        if preferences == self.preferences.values
            && !self.preferences.unsaved
            && self.preferences.revision.0 != 0
        {
            return;
        }
        let previous = &self.preferences.values;
        let edited = &mut self.preferences.edited;
        edited.theme |= preferences.theme != previous.theme;
        edited.density |= preferences.density != previous.density;
        edited.windows_shell |= preferences.windows_shell != previous.windows_shell;
        edited.font_family |= preferences.font_family != previous.font_family;
        edited.desktop_font_size |= preferences.desktop_font_size != previous.desktop_font_size;
        edited.touch_font_size |= preferences.touch_font_size != previous.touch_font_size;
        let filter_changed = preferences.quick_search_filter != previous.quick_search_filter;
        edited.quick_search_filter |= filter_changed;
        edited.bookmarks |= preferences.bookmarks != previous.bookmarks;
        self.retired.push(Box::new(mem::replace(
            &mut self.preferences.values,
            preferences,
        )));
        self.preferences.unsaved = true;
        self.preferences.revision.0 = self.preferences.revision.0.saturating_add(1);
        if filter_changed {
            self.search_filter_changed();
        }
        if !self.preferences.loaded {
            self.desktop.status = "Options previewed; waiting for saved options to load".into();
            return;
        }
        self.preferences.flush_after_load = false;
        self.send_preferences_save(send);
    }

    fn send_preferences_save(&mut self, send: &dyn Fn(Command) -> bool) {
        if let Some(message) = &self.preferences.save_blocked {
            self.desktop.report("Cannot save options", message.clone());
            return;
        }
        if let Err(error) = self.preferences.values.validate() {
            self.desktop.status = "Options apply to this session; saving needs attention".into();
            self.desktop.report(
                "Cannot save options",
                format!(
                    "{error}\nRemove bookmarks until at most 64 remain, then save again. \
                 Existing saved bookmarks have not been replaced."
                ),
            );
            return;
        }
        if send(Command::SavePreferences {
            revision: self.preferences.revision,
            preferences: self.preferences.values.clone(),
        }) {
            self.desktop.status = "Saving options…".into();
        } else {
            self.desktop.status = "Options apply to this session; saving is busy".into();
            self.desktop.report(
                "Options queue busy",
                "The preview is active. Use Save again in Options to retry saving.".into(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, io};

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
        p.apply(Event::PreferencesLoaded(Ok(UserPreferences {
            theme: PreferenceTheme::Light,
            density: UiDensity::Touch,
            ..Default::default()
        })));
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
        p.apply_with_sender(
            Event::PreferencesLoaded(Ok(UserPreferences::default())),
            &send,
        );
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
            sent.borrow()[0],
            Command::SavePreferences {
                revision: PreferencesRevision(1),
                preferences: UserPreferences {
                    theme: PreferenceTheme::Light,
                    density: UiDensity::Touch,
                    ..
                }
            }
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
        p.apply(Event::PreferencesLoaded(Ok(UserPreferences::default())));
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
