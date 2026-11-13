//! Viewer availability and next-start preferences; opening a viewer uses a frozen startup list.
use super::*;

fn filesystem_key(plugin: &FilesystemPluginInfo) -> String {
    format!(
        "{:?}:{:?}:{}:{}",
        plugin.kind,
        plugin.location,
        plugin.id,
        if plugin.id.is_empty() {
            &plugin.status
        } else {
            ""
        }
    )
}

#[derive(Default)]
pub(crate) struct ViewerPluginsState {
    pub(crate) catalog: Vec<ViewerPluginInfo>,
    pub(crate) filesystems: Vec<FilesystemPluginInfo>,
    pub(crate) filesystem_error: String,
    pub(crate) filesystems_pending: bool,
    pub(crate) filesystem_selected_key: String,
    pub(crate) thumbnailers: Vec<ThumbnailerPluginInfo>,
    pub(crate) thumbnailer_error: String,
    pub(crate) thumbnailers_pending: bool,
    pub(crate) thumbnailer_selected_id: String,
    pub(crate) error: String,
    pub(crate) pending: bool,
    pub(crate) startup_disabled: Option<Arc<[String]>>,
    pub(crate) startup_associations: Arc<ViewerAssociations>,
    pub(crate) revision: u64,
    pub(crate) selected_key: String,
    pub(crate) extension_draft: Option<super::viewer_associations::ExtensionDraft>,
    pub(crate) association_error: String,
    pub(crate) extension_input_rejected: bool,
}

impl Presenter {
    /// Provider choices frozen from the first successfully loaded settings.
    ///
    /// The pair contains disabled provider IDs and associations for this startup,
    /// matching `ViewerLaunch`. Options previews and next-start saves do not
    /// replace these snapshots. Trusted catalog identity belongs to the viewer
    /// service; presentation revisions are not provider-policy generations.
    pub fn viewer_startup_policy(&self) -> Option<(Arc<[String]>, Arc<ViewerAssociations>)> {
        if !self.editor_preferences_ready() || self.operations.closing {
            return None;
        }
        Some((
            self.viewer_plugins.startup_disabled.clone()?,
            self.viewer_plugins.startup_associations.clone(),
        ))
    }

    pub(crate) fn viewer_plugin_settings_ready(&self) -> bool {
        self.editor_preferences_ready()
            && !self.preferences.options.accepting
            && !self.preferences.options.saving
            && !self.operations.closing
    }
    pub(crate) fn request_viewer_plugins(&mut self, send: &dyn Fn(Command) -> bool) {
        if !self.viewer_plugins.pending
            && !self.viewer_plugins.filesystems_pending
            && !self.viewer_plugins.thumbnailers_pending
        {
            if send(Command::ListViewerPlugins) {
                self.viewer_plugins.pending = true;
                self.viewer_plugins.filesystems_pending = true;
                self.viewer_plugins.thumbnailers_pending = true;
                self.viewer_plugins.error.clear();
                self.viewer_plugins.filesystem_error.clear();
                self.viewer_plugins.thumbnailer_error.clear();
            } else {
                self.viewer_plugins.error = "Plugin discovery is busy. Try Refresh again.".into();
            }
        }
    }

    pub(super) fn thumbnailer_plugins_listed(
        &mut self,
        result: Result<Vec<ThumbnailerPluginInfo>, String>,
    ) {
        self.viewer_plugins.thumbnailers_pending = false;
        let catalog = match result {
            Ok(catalog) => {
                self.viewer_plugins.thumbnailer_error.clear();
                catalog
            }
            Err(error) => {
                self.viewer_plugins.thumbnailer_error = error;
                Vec::new()
            }
        };
        self.retired.push(Box::new(mem::replace(
            &mut self.viewer_plugins.thumbnailers,
            catalog,
        )));
        self.viewer_plugins.revision = self.viewer_plugins.revision.wrapping_add(1);
    }

    pub(crate) fn thumbnailer_plugin_selected(&self) -> Option<usize> {
        self.viewer_plugins
            .thumbnailers
            .iter()
            .position(|plugin| plugin.id == self.viewer_plugins.thumbnailer_selected_id)
            .or_else(|| (!self.viewer_plugins.thumbnailers.is_empty()).then_some(0))
    }

    pub(crate) fn select_thumbnailer_plugin(&mut self, index: usize) {
        if let Some(plugin) = self.viewer_plugins.thumbnailers.get(index) {
            self.viewer_plugins.thumbnailer_selected_id = plugin.id.clone();
            self.viewer_plugins.revision = self.viewer_plugins.revision.wrapping_add(1);
        }
    }

    pub(super) fn filesystem_plugins_listed(
        &mut self,
        result: Result<Vec<FilesystemPluginInfo>, String>,
    ) {
        self.viewer_plugins.filesystems_pending = false;
        let catalog = match result {
            Ok(catalog) => {
                self.viewer_plugins.filesystem_error.clear();
                catalog
            }
            Err(error) => {
                self.viewer_plugins.filesystem_error = error;
                Vec::new()
            }
        };
        self.retired.push(Box::new(mem::replace(
            &mut self.viewer_plugins.filesystems,
            catalog,
        )));
        self.viewer_plugins.revision = self.viewer_plugins.revision.wrapping_add(1);
    }

    pub(crate) fn filesystem_plugin_selected(&self) -> Option<usize> {
        self.viewer_plugins
            .filesystems
            .iter()
            .position(|plugin| {
                filesystem_key(plugin) == self.viewer_plugins.filesystem_selected_key
            })
            .or_else(|| (!self.viewer_plugins.filesystems.is_empty()).then_some(0))
    }

    pub(crate) fn select_filesystem_plugin(&mut self, index: usize) {
        if let Some(plugin) = self.viewer_plugins.filesystems.get(index) {
            self.viewer_plugins.filesystem_selected_key = filesystem_key(plugin);
            self.viewer_plugins.revision = self.viewer_plugins.revision.wrapping_add(1);
        }
    }

    pub(super) fn viewer_plugins_listed(&mut self, result: Result<Vec<ViewerPluginInfo>, String>) {
        self.viewer_plugins.pending = false;
        match result {
            Ok(catalog) => {
                self.retired.push(Box::new(mem::replace(
                    &mut self.viewer_plugins.catalog,
                    catalog,
                )));
                self.viewer_plugins.error.clear();
            }
            Err(error) => {
                self.retired
                    .push(Box::new(mem::take(&mut self.viewer_plugins.catalog)));
                self.viewer_plugins.error = error;
            }
        }
        self.viewer_plugins.revision = self.viewer_plugins.revision.wrapping_add(1);
    }

    pub(crate) fn set_viewer_plugin_enabled(
        &mut self,
        id: &str,
        enabled: bool,
        send: &dyn Fn(Command) -> bool,
    ) {
        self.viewer_plugins.revision = self.viewer_plugins.revision.wrapping_add(1);
        if !self.viewer_plugin_settings_ready() {
            self.viewer_plugins.error =
                "Saved preferences must load successfully before changing plugins.".into();
            return;
        }
        if validate_disabled_viewers(&[id.to_owned()]).is_err()
            || !self
                .viewer_plugins_ordered()
                .iter()
                .any(|plugin| plugin.id == id)
        {
            self.viewer_plugins.error =
                "This plugin is not in the current catalog. Refresh the list.".into();
            return;
        }
        let mut preferences = self.preferences.values.clone();
        if enabled {
            preferences
                .disabled_viewer_plugins
                .retain(|disabled| disabled != id);
        } else if !preferences
            .disabled_viewer_plugins
            .iter()
            .any(|disabled| disabled == id)
        {
            preferences.disabled_viewer_plugins.push(id.to_owned());
            preferences.disabled_viewer_plugins.sort();
        }
        if let Err(error) = validate_disabled_viewers(&preferences.disabled_viewer_plugins) {
            self.viewer_plugins.error = error.into();
            return;
        }
        self.viewer_plugins.error.clear();
        self.preferences.edited.disabled_viewer_plugins = true;
        self.update_preferences(preferences, send);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn presenter() -> Presenter {
        let path = std::env::current_dir().unwrap();
        let mut presenter = Presenter::new(path.clone(), path.clone());
        let entries = vec![Entry::new(
            EntryId(1),
            path.join("file.txt"),
            EntryKind::File,
        )];
        presenter.panes[0].index = Arc::new(DirectoryIndex::from_entries(&entries));
        presenter.panes[0].entries = entries.into();
        presenter.panes[0].cursor = Some(EntryId(1));
        presenter.panes[0].parent_cursor = false;
        presenter.viewer_plugins.catalog = vec![ViewerPluginInfo {
            extensions: vec!["rs".into(), "py".into()],
            fallback: false,
            id: "midnight.viewer.source".into(),
            name: "Source".into(),
            description: String::new(),
            version: "1".into(),
            status: "Available".into(),
            location: path,
        }];
        presenter
    }

    #[test]
    fn viewer_waits_for_saved_startup_plugin_choices() {
        let presenter = presenter();
        assert!(presenter.viewer_launch(PaneId::Left, false).is_none());
    }

    #[test]
    fn plugin_toggle_saves_next_start_choices_without_changing_current_viewer_routing() {
        let mut presenter = presenter();
        presenter.preferences_loaded(Ok(UserPreferences {
            disabled_viewer_plugins: vec!["midnight.viewer.source".into()],
            ..Default::default()
        }));
        assert_eq!(
            &*presenter
                .viewer_launch(PaneId::Left, false)
                .unwrap()
                .disabled_plugins,
            ["midnight.viewer.source"]
        );
        let saved = RefCell::new(Vec::new());
        presenter.set_viewer_plugin_enabled("midnight.viewer.source", true, &|command| {
            if let Command::SavePreferences { preferences, .. } = command {
                saved.borrow_mut().push(preferences.disabled_viewer_plugins);
            }
            true
        });
        assert_eq!(*saved.borrow(), vec![Vec::<String>::new()]);
        assert_eq!(
            &*presenter
                .viewer_launch(PaneId::Left, false)
                .unwrap()
                .disabled_plugins,
            ["midnight.viewer.source"]
        );
    }

    #[test]
    fn malformed_or_unknown_toggle_does_not_change_or_save_preferences() {
        let mut presenter = presenter();
        presenter.preferences_loaded(Ok(UserPreferences::default()));
        for id in ["../bad", "unknown.viewer"] {
            presenter
                .set_viewer_plugin_enabled(id, false, &|_| panic!("invalid request must not save"));
            assert!(!presenter.viewer_plugins.error.is_empty());
        }
        assert!(
            presenter
                .preferences
                .values
                .disabled_viewer_plugins
                .is_empty()
        );
    }

    fn startup_policy_saved_preferences() -> UserPreferences {
        UserPreferences {
            disabled_viewer_plugins: vec!["midnight.viewer.source".into()],
            viewer_associations: Arc::new(ViewerAssociations {
                order: vec![
                    "midnight.viewer.image".into(),
                    "midnight.viewer.source".into(),
                ],
                overrides: [("midnight.viewer.source".into(), vec!["cpp".into()])]
                    .into_iter()
                    .collect(),
            }),
            ..Default::default()
        }
    }

    fn startup_policy_assert_frozen(
        presenter: &Presenter,
        expected: &(Arc<[String]>, Arc<ViewerAssociations>),
    ) {
        let (disabled, associations) = presenter
            .viewer_startup_policy()
            .expect("the successfully loaded startup policy remains available");
        assert!(Arc::ptr_eq(&disabled, &expected.0));
        assert!(Arc::ptr_eq(&associations, &expected.1));
    }

    fn startup_policy_preview_provider_changes(presenter: &mut Presenter) {
        presenter.begin_options();
        presenter.set_viewer_plugin_enabled("midnight.viewer.source", true, &|_| {
            panic!("Options preview must not persist")
        });
        let index = presenter
            .viewer_plugins_ordered()
            .iter()
            .position(|plugin| plugin.id == "midnight.viewer.source")
            .unwrap();
        presenter.select_viewer_plugin(index, &|_| panic!("selection must not persist"));
        presenter.edit_viewer_extensions("rs; py");
        assert!(
            presenter
                .save_viewer_extension_draft(&|_| { panic!("extension preview must not persist") })
        );
        assert!(
            presenter
                .preferences
                .values
                .disabled_viewer_plugins
                .is_empty()
        );
        assert_eq!(
            presenter
                .preferences
                .values
                .viewer_associations
                .overrides
                .get("midnight.viewer.source"),
            Some(&vec!["rs".into(), "py".into()])
        );
    }

    #[test]
    fn startup_policy_waits_for_successful_load_and_failed_read_cannot_be_replaced() {
        let mut presenter = presenter();
        assert!(presenter.viewer_startup_policy().is_none());
        presenter.preferences_loaded(Err(FsError::from_io(
            "load preferences",
            std::path::Path::new("preferences"),
            std::io::ErrorKind::PermissionDenied.into(),
        )));
        assert!(presenter.preferences.loaded);
        assert!(!presenter.window_geometry_ready());
        assert!(presenter.viewer_startup_policy().is_none());
        presenter.preferences_loaded(Ok(startup_policy_saved_preferences()));
        assert!(presenter.viewer_startup_policy().is_none());
        assert!(presenter.viewer_plugins.startup_disabled.is_none());
    }

    #[test]
    fn startup_policy_exposes_saved_nondefault_arcs_without_focus_or_catalog() {
        let root = std::env::temp_dir().join("viewer-policy-no-focus");
        let mut presenter = Presenter::new(root.clone(), root);
        let saved = startup_policy_saved_preferences();
        let saved_associations = saved.viewer_associations.clone();
        presenter.preferences_loaded(Ok(saved));
        assert!(presenter.viewer_plugins.catalog.is_empty());
        assert!(presenter.viewer_launch(PaneId::Left, false).is_none());
        let (disabled, associations) = presenter.viewer_startup_policy().unwrap();
        assert_eq!(&*disabled, ["midnight.viewer.source"]);
        assert_eq!(
            associations.order,
            ["midnight.viewer.image", "midnight.viewer.source"]
        );
        assert_eq!(
            associations.overrides.get("midnight.viewer.source"),
            Some(&vec!["cpp".into()])
        );
        assert!(Arc::ptr_eq(&associations, &saved_associations));
        assert!(Arc::ptr_eq(
            &disabled,
            presenter.viewer_plugins.startup_disabled.as_ref().unwrap()
        ));
    }

    #[test]
    fn startup_policy_rejects_load_reconciliation_overflow_after_startup_fields_are_captured() {
        let root = std::env::temp_dir().join("viewer-policy-bookmark-collision");
        let mut presenter = Presenter::new(root.join("local"), root.clone());
        assert!(
            presenter.add_bookmark(PaneId::Left, "Early local".into(), None, &|_| {
                panic!("an early bookmark must wait for the saved preferences")
            })
        );
        assert_eq!(presenter.bookmarks()[0].id, BookmarkId(1));
        let mut saved = startup_policy_saved_preferences();
        saved.bookmarks = [1, u64::MAX]
            .into_iter()
            .map(|id| Bookmark {
                id: BookmarkId(id),
                name: format!("Saved {id}"),
                path: root.join(format!("saved-{id}")),
                archive: None,
                network: None,
                slot: None,
            })
            .collect();
        saved.validate().unwrap();
        presenter.preferences_loaded(Ok(saved));
        assert!(presenter.preferences.loaded);
        assert!(presenter.viewer_plugins.startup_disabled.is_some());
        assert!(
            !presenter
                .viewer_plugins
                .startup_associations
                .overrides
                .is_empty()
        );
        assert!(!presenter.window_geometry_ready());
        assert!(presenter.viewer_startup_policy().is_none());
    }

    #[test]
    fn startup_policy_ignores_options_previews_cancel_and_persistence_races() {
        let mut presenter = presenter();
        presenter.preferences_loaded(Ok(startup_policy_saved_preferences()));
        let frozen = presenter.viewer_startup_policy().unwrap();
        startup_policy_preview_provider_changes(&mut presenter);
        startup_policy_assert_frozen(&presenter, &frozen);
        presenter.reset_options(&|_| panic!("Reset is still an Options preview"));
        assert!(
            presenter
                .preferences
                .values
                .viewer_associations
                .overrides
                .is_empty()
        );
        startup_policy_assert_frozen(&presenter, &frozen);
        presenter.cancel_options(&|_| panic!("Cancel must not save"));
        assert_eq!(
            presenter.preferences.values.disabled_viewer_plugins,
            ["midnight.viewer.source"]
        );
        assert_eq!(presenter.preferences.values.viewer_associations, frozen.1);
        startup_policy_assert_frozen(&presenter, &frozen);

        startup_policy_preview_provider_changes(&mut presenter);
        let writes = RefCell::new(Vec::new());
        presenter.accept_options(&|command| {
            let Command::SavePreferences {
                revision,
                preferences,
            } = command
            else {
                panic!("Options OK must persist through the preferences command")
            };
            writes.borrow_mut().push((revision, preferences));
            false
        });
        let rejected = writes.borrow()[0].0;
        assert!(presenter.preferences.options.open);
        assert!(!presenter.preferences.options.saving);
        startup_policy_assert_frozen(&presenter, &frozen);
        let accept = |command| {
            let Command::SavePreferences {
                revision,
                preferences,
            } = command
            else {
                panic!("Options retry must persist through the preferences command")
            };
            assert!(preferences.disabled_viewer_plugins.is_empty());
            assert_eq!(
                preferences
                    .viewer_associations
                    .overrides
                    .get("midnight.viewer.source"),
                Some(&vec!["rs".into(), "py".into()])
            );
            writes.borrow_mut().push((revision, preferences));
            true
        };
        presenter.accept_options(&accept);
        let failed = writes.borrow()[1].0;
        assert!(failed > rejected);
        assert!(presenter.preferences.options.saving);
        assert!(!presenter.viewer_plugin_settings_ready());
        startup_policy_assert_frozen(&presenter, &frozen);
        presenter.preferences_saved(
            failed,
            Err(FsError::from_io(
                "save preferences",
                std::path::Path::new("preferences"),
                std::io::ErrorKind::PermissionDenied.into(),
            )),
        );
        assert!(presenter.preferences.options.open);
        assert!(!presenter.preferences.options.saving);
        startup_policy_assert_frozen(&presenter, &frozen);
        presenter.accept_options(&accept);
        let retry = writes.borrow()[2].0;
        assert!(retry > failed);
        presenter.preferences_saved(failed, Ok(()));
        assert!(presenter.preferences.options.open);
        assert!(presenter.preferences.options.saving);
        startup_policy_assert_frozen(&presenter, &frozen);
        presenter.preferences_saved(retry, Ok(()));
        assert!(!presenter.preferences.options.open);
        startup_policy_assert_frozen(&presenter, &frozen);
        let launch = presenter.viewer_launch(PaneId::Left, false).unwrap();
        assert!(Arc::ptr_eq(&launch.disabled_plugins, &frozen.0));
        assert!(Arc::ptr_eq(&launch.associations, &frozen.1));
    }

    #[test]
    fn startup_policy_ignores_successful_duplicate_load_catalog_and_session_revisions() {
        let mut presenter = presenter();
        presenter.preferences_loaded(Ok(startup_policy_saved_preferences()));
        let frozen = presenter.viewer_startup_policy().unwrap();
        presenter.preferences_loaded(Ok(UserPreferences::default()));
        startup_policy_assert_frozen(&presenter, &frozen);
        assert_eq!(
            presenter.preferences.values.disabled_viewer_plugins,
            ["midnight.viewer.source"]
        );
        presenter.viewer_plugins_listed(Ok(Vec::new()));
        assert!(presenter.viewer_plugins.catalog.is_empty());
        startup_policy_assert_frozen(&presenter, &frozen);
        let next_wrap = !presenter.viewer_preferences().wrap;
        assert!(presenter.save_viewer_preferences(
            ViewerPreferenceChange::Wrap(next_wrap),
            &|command| {
                assert!(matches!(command, Command::SavePreferences { .. }));
                true
            }
        ));
        assert!(presenter.preferences.saving);
        startup_policy_assert_frozen(&presenter, &frozen);
    }

    #[test]
    fn startup_policy_is_unavailable_after_operation_shutdown_begins() {
        let mut presenter = presenter();
        presenter.preferences_loaded(Ok(startup_policy_saved_preferences()));
        assert!(presenter.viewer_startup_policy().is_some());
        presenter.request_operation_shutdown(&|_| true);
        assert!(presenter.window_geometry_ready());
        assert!(presenter.viewer_plugins.startup_disabled.is_some());
        assert!(presenter.viewer_startup_policy().is_none());
    }
}
