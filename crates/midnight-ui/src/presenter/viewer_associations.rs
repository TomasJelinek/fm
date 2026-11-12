//! Reversible viewer routing drafts inside the Options transaction.
use super::*;

pub(crate) struct ExtensionDraft {
    pub(crate) id: String,
    pub(crate) text: String,
}

fn key(plugin: &ViewerPluginInfo, index: usize) -> String {
    if plugin.id.is_empty() {
        format!("unavailable:{index}")
    } else {
        plugin.id.clone()
    }
}

impl Presenter {
    pub(crate) fn viewer_plugins_ordered(&self) -> Vec<ViewerPluginInfo> {
        let preferences = &self.preferences.values;
        let associations = &preferences.viewer_associations;
        let mut catalog = self.viewer_plugins.catalog.clone();
        for id in preferences
            .disabled_viewer_plugins
            .iter()
            .chain(associations.order.iter())
            .chain(associations.overrides.keys())
            .chain(
                self.viewer_plugins
                    .extension_draft
                    .iter()
                    .map(|draft| &draft.id),
            )
        {
            if !catalog.iter().any(|plugin| &plugin.id == id) {
                catalog.push(ViewerPluginInfo {
                    id: id.clone(),
                    name: viewer_plugin_name(id, None),
                    description: String::new(),
                    version: String::new(),
                    status: "Not installed · settings retained".into(),
                    location: PathBuf::new(),
                    extensions: Vec::new(),
                    fallback: false,
                });
            }
        }
        catalog.sort_by_key(|plugin| std::cmp::Reverse(associations.priority(&plugin.id)));
        catalog
    }

    pub(crate) fn viewer_plugin_selected(&self, catalog: &[ViewerPluginInfo]) -> Option<usize> {
        catalog
            .iter()
            .enumerate()
            .position(|(index, plugin)| key(plugin, index) == self.viewer_plugins.selected_key)
            .or_else(|| (!catalog.is_empty()).then_some(0))
    }

    pub(crate) fn discard_viewer_association_draft(&mut self) {
        if let Some(draft) = self.viewer_plugins.extension_draft.take() {
            self.retired.push(Box::new(draft));
        }
        self.viewer_plugins.association_error.clear();
        self.viewer_plugins.extension_input_rejected = false;
        // Options previews and Cancel can change values without changing the
        // persisted preference revision. Invalidate rows for those transitions.
        self.viewer_plugins.revision = self.viewer_plugins.revision.wrapping_add(1);
    }

    pub(crate) fn edit_viewer_extensions(&mut self, text: &str) {
        if !self.viewer_plugin_settings_ready() {
            return;
        }
        if text.len() > 32_768 {
            self.viewer_plugins.extension_input_rejected = true;
            self.viewer_plugins.association_error =
                "The extension list exceeds 32 KiB. Shorten it or use Reset.".into();
            return;
        }
        let catalog = self.viewer_plugins_ordered();
        let Some(index) = self.viewer_plugin_selected(&catalog) else {
            return;
        };
        let plugin = &catalog[index];
        if plugin.id.is_empty() {
            return;
        }
        self.viewer_plugins.selected_key = plugin.id.clone();
        self.viewer_plugins.extension_draft = Some(ExtensionDraft {
            id: plugin.id.clone(),
            text: text.into(),
        });
        self.viewer_plugins.association_error.clear();
        self.viewer_plugins.extension_input_rejected = false;
    }

    pub(crate) fn save_viewer_extension_draft(&mut self, send: &dyn Fn(Command) -> bool) -> bool {
        if self.viewer_plugins.extension_input_rejected {
            return false;
        }
        let Some(draft) = &self.viewer_plugins.extension_draft else {
            return true;
        };
        if !self.viewer_plugin_settings_ready() {
            return false;
        }
        let extensions = match ViewerAssociations::parse_extensions(&draft.text) {
            Ok(value) => value,
            Err(error) => {
                self.viewer_plugins.association_error = error.into();
                return false;
            }
        };
        let mut associations = (*self.preferences.values.viewer_associations).clone();
        let defaults = self
            .viewer_plugins
            .catalog
            .iter()
            .find(|plugin| plugin.id == draft.id)
            .map(|plugin| &plugin.extensions);
        if associations.overrides.contains_key(&draft.id) || defaults != Some(&extensions) {
            associations.overrides.insert(draft.id.clone(), extensions);
        }
        if let Err(error) = associations.validate() {
            self.viewer_plugins.association_error = error.into();
            return false;
        }
        let mut values = self.preferences.values.clone();
        values.viewer_associations = Arc::new(associations);
        if values.viewer_associations != self.preferences.values.viewer_associations {
            self.update_preferences(values, send);
        }
        self.discard_viewer_association_draft();
        true
    }

    pub(crate) fn reset_viewer_extensions(&mut self, send: &dyn Fn(Command) -> bool) {
        if !self.viewer_plugin_settings_ready() {
            return;
        }
        let catalog = self.viewer_plugins_ordered();
        let Some(index) = self.viewer_plugin_selected(&catalog) else {
            return;
        };
        let id = &catalog[index].id;
        if id.is_empty() {
            return;
        }
        let mut associations = (*self.preferences.values.viewer_associations).clone();
        associations.overrides.remove(id);
        self.discard_viewer_association_draft();
        let mut values = self.preferences.values.clone();
        values.viewer_associations = Arc::new(associations);
        if values.viewer_associations != self.preferences.values.viewer_associations {
            self.update_preferences(values, send);
        }
    }

    pub(crate) fn move_viewer_plugin(&mut self, delta: i32, send: &dyn Fn(Command) -> bool) {
        if !self.viewer_plugin_settings_ready()
            || !matches!(delta, -1 | 1)
            || !self.save_viewer_extension_draft(send)
        {
            return;
        }
        let catalog = self.viewer_plugins_ordered();
        let Some(index) = self.viewer_plugin_selected(&catalog) else {
            return;
        };
        if catalog[index].id.is_empty() {
            return;
        }
        let mut ids: Vec<_> = catalog
            .iter()
            .filter(|plugin| !plugin.id.is_empty())
            .map(|plugin| plugin.id.clone())
            .collect();
        let Some(selected) = ids.iter().position(|id| id == &catalog[index].id) else {
            return;
        };
        let Some(other) = selected
            .checked_add_signed(delta as isize)
            .filter(|at| *at < ids.len())
        else {
            return;
        };
        ids.swap(selected, other);
        let mut associations = (*self.preferences.values.viewer_associations).clone();
        associations.order = ids;
        if let Err(error) = associations.validate() {
            self.viewer_plugins.association_error = error.into();
            return;
        }
        self.viewer_plugins.selected_key = catalog[index].id.clone();
        self.viewer_plugins.revision = self.viewer_plugins.revision.wrapping_add(1);
        let mut values = self.preferences.values.clone();
        values.viewer_associations = Arc::new(associations);
        self.update_preferences(values, send);
    }

    pub(crate) fn select_viewer_plugin(&mut self, index: usize, send: &dyn Fn(Command) -> bool) {
        let catalog = self.viewer_plugins_ordered();
        let Some(plugin) = catalog.get(index) else {
            return;
        };
        let key = key(plugin, index);
        if key == self.viewer_plugins.selected_key {
            return;
        }
        if !self.save_viewer_extension_draft(send) {
            return;
        }
        self.viewer_plugins.selected_key = key;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn loaded() -> Presenter {
        let mut presenter = Presenter::new("left".into(), "right".into());
        presenter.preferences_loaded(Ok(UserPreferences::default()));
        presenter.viewer_plugins.catalog = [
            ("midnight.viewer.markdown", "Markdown", vec!["md"], false),
            ("midnight.viewer.image", "Images", vec!["png", "jpg"], false),
            ("midnight.viewer.basic", "Basic", vec![], true),
        ]
        .into_iter()
        .map(|(id, name, extensions, fallback)| ViewerPluginInfo {
            id: id.into(),
            name: name.into(),
            description: String::new(),
            extensions: extensions.into_iter().map(String::from).collect(),
            fallback,
            status: "Available".into(),
            version: "1".into(),
            location: "viewers".into(),
        })
        .collect();
        presenter
    }

    #[test]
    fn extension_draft_is_normalized_on_ok_and_cancel_restores_preview() {
        let mut presenter = loaded();
        let baseline = presenter.preferences.values.clone();
        presenter.begin_options();
        presenter.select_viewer_plugin(0, &|_| panic!("selection cannot save"));
        presenter.edit_viewer_extensions(" .MD; *.md, mdown 日本語 ");
        assert!(presenter.save_viewer_extension_draft(&|_| panic!("draft cannot save")));
        assert_eq!(
            presenter
                .preferences
                .values
                .viewer_associations
                .overrides
                .get("midnight.viewer.markdown"),
            Some(&vec!["md".into(), "mdown".into(), "日本語".into()])
        );
        assert!(
            presenter
                .viewer_plugins
                .startup_associations
                .overrides
                .is_empty()
        );
        presenter.cancel_options(&|_| panic!("cancel cannot save"));
        assert_eq!(presenter.preferences.values, baseline);
    }

    #[test]
    fn unfinished_invalid_extension_blocks_ok_in_plugins_topic() {
        let mut presenter = loaded();
        presenter.begin_options();
        presenter.select_viewer_plugin(0, &|_| true);
        presenter.edit_viewer_extensions("md; ../secret");
        presenter.accept_options(&|_| panic!("invalid draft cannot be persisted"));
        assert!(presenter.preferences.options.open);
        assert!(!presenter.preferences.options.saving);
        assert_eq!(presenter.preferences.options.error_topic, Some(5));
        assert!(
            presenter
                .preferences
                .values
                .viewer_associations
                .overrides
                .is_empty()
        );
    }

    #[test]
    fn ok_saves_extension_draft_in_one_preferences_commit() {
        let mut presenter = loaded();
        presenter.begin_options();
        presenter.select_viewer_plugin(1, &|_| true);
        presenter.edit_viewer_extensions("png; webp");
        let commands = RefCell::new(Vec::new());
        presenter.accept_options(&|command| {
            commands.borrow_mut().push(command);
            true
        });
        let commands = commands.borrow();
        let saves: Vec<_> = commands
            .iter()
            .filter_map(|command| match command {
                Command::SavePreferences { preferences, .. } => Some(preferences),
                _ => None,
            })
            .collect();
        assert_eq!(saves.len(), 1);
        assert_eq!(
            saves[0]
                .viewer_associations
                .overrides
                .get("midnight.viewer.image"),
            Some(&vec!["png".into(), "webp".into()])
        );
    }

    #[test]
    fn priority_moves_and_manifest_reset_keep_uninstalled_preferences() {
        let mut presenter = loaded();
        let mut associations = ViewerAssociations::default();
        associations
            .overrides
            .insert("missing.viewer".into(), vec!["orphan".into()]);
        associations.order = vec!["midnight.viewer.markdown".into(), "missing.viewer".into()];
        presenter.preferences.values.viewer_associations = Arc::new(associations);
        presenter.begin_options();
        presenter.select_viewer_plugin(0, &|_| true);
        presenter.edit_viewer_extensions("");
        assert!(presenter.save_viewer_extension_draft(&|_| true));
        assert_eq!(
            presenter
                .preferences
                .values
                .viewer_associations
                .overrides
                .get("midnight.viewer.markdown"),
            Some(&vec![])
        );
        presenter.move_viewer_plugin(1, &|_| true);
        let associations = &presenter.preferences.values.viewer_associations;
        assert!(
            associations.priority("missing.viewer")
                > associations.priority("midnight.viewer.markdown")
        );
        assert_eq!(
            associations.overrides.get("missing.viewer"),
            Some(&vec!["orphan".into()])
        );
        presenter.reset_viewer_extensions(&|_| true);
        assert!(
            !presenter
                .preferences
                .values
                .viewer_associations
                .overrides
                .contains_key("midnight.viewer.markdown")
        );
        assert!(
            presenter
                .preferences
                .values
                .viewer_associations
                .overrides
                .contains_key("missing.viewer")
        );
    }

    #[test]
    fn resetting_options_discards_invalid_draft_and_cancel_restores_associations() {
        let mut presenter = loaded();
        let mut associations = ViewerAssociations::default();
        associations
            .overrides
            .insert("midnight.viewer.markdown".into(), vec!["custom".into()]);
        presenter.preferences.values.viewer_associations = Arc::new(associations.clone());
        presenter.begin_options();
        presenter.edit_viewer_extensions("*bad");
        presenter.reset_options(&|_| panic!("reset is a preview"));
        assert!(presenter.save_viewer_extension_draft(&|_| true));
        assert_eq!(
            *presenter.preferences.values.viewer_associations,
            ViewerAssociations::default()
        );
        presenter.cancel_options(&|_| panic!("cancel cannot save"));
        assert_eq!(
            *presenter.preferences.values.viewer_associations,
            associations
        );
    }

    #[test]
    fn oversized_extension_input_is_not_retained_and_blocks_ok_until_reset() {
        let mut presenter = loaded();
        presenter.begin_options();
        presenter.edit_viewer_extensions(&"r".repeat(32_769));
        assert!(presenter.viewer_plugins.extension_draft.is_none());
        assert!(!presenter.viewer_plugins.association_error.is_empty());
        presenter.accept_options(&|_| panic!("oversized draft cannot be persisted"));
        assert_eq!(presenter.preferences.options.error_topic, Some(5));
        presenter.reset_viewer_extensions(&|_| panic!("reset cannot save"));
        assert!(presenter.save_viewer_extension_draft(&|_| true));
    }

    #[test]
    fn invalid_text_cannot_be_lost_by_selecting_a_different_viewer() {
        let mut presenter = loaded();
        presenter.begin_options();
        presenter.select_viewer_plugin(0, &|_| true);
        presenter.edit_viewer_extensions("md; *");
        presenter.select_viewer_plugin(1, &|_| panic!("invalid draft cannot save"));
        assert_eq!(
            presenter.viewer_plugin_selected(&presenter.viewer_plugins_ordered()),
            Some(0)
        );
        assert_eq!(
            presenter
                .viewer_plugins
                .extension_draft
                .as_ref()
                .unwrap()
                .text,
            "md; *"
        );
        assert!(!presenter.viewer_plugins.association_error.is_empty());
    }

    #[test]
    fn pending_catalog_refresh_cannot_hide_a_newly_edited_plugin_draft() {
        let mut presenter = loaded();
        presenter.begin_options();
        presenter.request_viewer_plugins(&|_| true);
        // Typing into the initially selected row need not click the list first.
        presenter.edit_viewer_extensions("md; custom");
        let mut refreshed = presenter.viewer_plugins.catalog.clone();
        refreshed.remove(0);
        presenter.viewer_plugins_listed(Ok(refreshed));
        let catalog = presenter.viewer_plugins_ordered();
        let selected = presenter.viewer_plugin_selected(&catalog).unwrap();
        assert_eq!(catalog[selected].id, "midnight.viewer.markdown");
        assert!(catalog[selected].status.contains("Not installed"));
        assert_eq!(
            presenter
                .viewer_plugins
                .extension_draft
                .as_ref()
                .unwrap()
                .text,
            "md; custom"
        );
    }
}
