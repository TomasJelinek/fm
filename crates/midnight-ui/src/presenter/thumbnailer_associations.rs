//! Thumbnailer extension edits staged inside the Options transaction.
use super::*;

const EXTENSION_INPUT_BYTES: usize = 32 * 1024;

pub(super) struct ThumbnailerExtensionDraft {
    id: String,
    text: Option<String>,
    error: String,
}

impl Presenter {
    pub(crate) fn thumbnailer_settings_ready(&self) -> bool {
        self.preferences.options.open && self.viewer_plugin_settings_ready()
    }

    fn selected_thumbnailer(&self) -> Option<&ThumbnailerPluginInfo> {
        self.thumbnailer_plugin_selected()
            .map(|index| &self.viewer_plugins.thumbnailers[index])
            .filter(|plugin| !plugin.id.is_empty())
    }

    fn selected_thumbnailer_draft(&self) -> Option<&ThumbnailerExtensionDraft> {
        let selected = self.selected_thumbnailer()?;
        self.viewer_plugins
            .thumbnailer_extension_draft
            .as_ref()
            .filter(|draft| draft.id == selected.id)
    }

    pub(crate) fn thumbnailer_extension_text(&self) -> String {
        let Some(plugin) = self.selected_thumbnailer() else {
            return String::new();
        };
        if let Some(text) = self
            .selected_thumbnailer_draft()
            .and_then(|draft| draft.text.as_ref())
        {
            return text.clone();
        }
        self.preferences
            .values
            .thumbnailer_associations
            .effective_extensions(&plugin.id, &plugin.extensions)
            .join("; ")
    }

    pub(crate) fn thumbnailer_extension_overridden(&self) -> bool {
        self.selected_thumbnailer_draft().is_some()
            || self.selected_thumbnailer().is_some_and(|plugin| {
                self.preferences
                    .values
                    .thumbnailer_associations
                    .overrides
                    .contains_key(&plugin.id)
            })
    }

    pub(crate) fn thumbnailer_extension_error(&self) -> &str {
        self.selected_thumbnailer_draft()
            .map_or("", |draft| draft.error.as_str())
    }

    pub(crate) fn thumbnailer_extension_input_rejected(&self) -> bool {
        self.selected_thumbnailer_draft()
            .is_some_and(|draft| draft.text.is_none())
    }

    pub(crate) fn edit_thumbnailer_extensions(&mut self, text: &str) {
        if !self.thumbnailer_settings_ready() {
            return;
        }
        let Some(plugin) = self.selected_thumbnailer() else {
            return;
        };
        let id = plugin.id.clone();
        self.viewer_plugins.thumbnailer_selected_id.clone_from(&id);
        let text = (text.len() <= EXTENSION_INPUT_BYTES).then(|| text.to_owned());
        let error = if text.is_none() {
            "The extension list exceeds 32 KiB. Shorten it or use Reset.".into()
        } else {
            String::new()
        };
        self.viewer_plugins.thumbnailer_extension_draft =
            Some(ThumbnailerExtensionDraft { id, text, error });
    }

    pub(crate) fn stage_thumbnailer_extension_draft(&mut self) -> bool {
        let settings_ready = self.thumbnailer_settings_ready();
        let Some(draft) = &mut self.viewer_plugins.thumbnailer_extension_draft else {
            return true;
        };
        let Some(text) = draft.text.as_deref().filter(|_| settings_ready) else {
            return false;
        };
        let extensions = match ThumbnailerAssociations::parse_extensions(text) {
            Ok(extensions) => extensions,
            Err(error) => {
                draft.error = error.to_string();
                return false;
            }
        };
        let mut associations = (*self.preferences.values.thumbnailer_associations).clone();
        associations.overrides.insert(draft.id.clone(), extensions);
        if let Err(error) = associations.validate() {
            draft.error = error.to_string();
            return false;
        }
        self.replace_thumbnailer_associations(associations);
        self.discard_thumbnailer_association_draft();
        true
    }

    pub(crate) fn reset_thumbnailer_extensions(&mut self) {
        if !self.thumbnailer_settings_ready() {
            return;
        }
        let Some(plugin) = self.selected_thumbnailer() else {
            return;
        };
        let mut associations = (*self.preferences.values.thumbnailer_associations).clone();
        if associations.reset(&plugin.id) {
            self.replace_thumbnailer_associations(associations);
        }
        self.discard_thumbnailer_association_draft();
    }

    fn replace_thumbnailer_associations(&mut self, associations: ThumbnailerAssociations) {
        if *self.preferences.values.thumbnailer_associations != associations {
            let old = mem::replace(
                &mut self.preferences.values.thumbnailer_associations,
                Arc::new(associations),
            );
            self.retired.push(Box::new(old));
        }
    }

    pub(super) fn discard_thumbnailer_association_draft(&mut self) {
        if let Some(draft) = self.viewer_plugins.thumbnailer_extension_draft.take() {
            self.retired.push(Box::new(draft));
        }
    }

    pub(super) fn thumbnailer_catalog_replaced(&mut self) {
        if self.viewer_plugins.thumbnailer_extension_draft.is_some()
            && self.selected_thumbnailer_draft().is_none()
        {
            self.discard_thumbnailer_association_draft();
        }
    }
}
