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
    pub(crate) fn viewer_plugin_settings_ready(&self) -> bool {
        self.editor_preferences_ready()
            && !self.preferences.options.accepting
            && !self.preferences.options.saving
            && !self.operations.closing
    }
    pub(crate) fn request_viewer_plugins(&mut self, send: &dyn Fn(Command) -> bool) {
        if !self.viewer_plugins.pending && !self.viewer_plugins.filesystems_pending {
            if send(Command::ListViewerPlugins) {
                self.viewer_plugins.pending = true;
                self.viewer_plugins.filesystems_pending = true;
                self.viewer_plugins.error.clear();
                self.viewer_plugins.filesystem_error.clear();
            } else {
                self.viewer_plugins.error = "Plugin discovery is busy. Try Refresh again.".into();
            }
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
}
