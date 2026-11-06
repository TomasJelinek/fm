//! Viewer availability and next-start preferences; opening a viewer uses a frozen startup list.
use super::*;

#[derive(Default)]
pub(crate) struct ViewerPluginsState {
    pub(crate) catalog: Vec<ViewerPluginInfo>,
    pub(crate) error: String,
    pub(crate) pending: bool,
    pub(crate) startup_disabled: Option<Arc<[String]>>,
    pub(crate) revision: u64,
}

impl Presenter {
    pub(crate) fn viewer_plugin_settings_ready(&self) -> bool {
        self.editor_preferences_ready()
    }
    pub(crate) fn request_viewer_plugins(&mut self, send: &dyn Fn(Command) -> bool) {
        if !self.viewer_plugins.pending {
            if send(Command::ListViewerPlugins) {
                self.viewer_plugins.pending = true;
                self.viewer_plugins.error.clear();
            } else {
                self.viewer_plugins.error = "Plugin discovery is busy. Try Refresh again.".into();
            }
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
        if !self.editor_preferences_ready() {
            self.viewer_plugins.error =
                "Saved preferences must load successfully before changing plugins.".into();
            return;
        }
        if validate_disabled_viewers(&[id.to_owned()]).is_err()
            || (!self
                .viewer_plugins
                .catalog
                .iter()
                .any(|plugin| plugin.id == id)
                && !self
                    .preferences
                    .values
                    .disabled_viewer_plugins
                    .iter()
                    .any(|disabled| disabled == id))
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
            id: "midnight.viewer.source".into(),
            name: "Source".into(),
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
