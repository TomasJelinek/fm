//! Immediate association choices; storage and provider discovery stay on backend workers.
use super::*;

impl Presenter {
    pub(crate) fn archive_options_ready(&self) -> bool {
        self.editor_preferences_ready()
    }
    pub(crate) fn add_archive_extension(
        &mut self,
        input: &str,
        format: ArchiveFormat,
        send: &dyn Fn(Command) -> bool,
    ) -> Result<(), String> {
        if !self.editor_preferences_ready() {
            return Err("Wait for settings to finish loading before adding an extension.".into());
        }
        let extension = midnight_core::normalize_archive_extension(input)?;
        if let Some(known) = ARCHIVE_ASSOCIATIONS
            .iter()
            .find(|known| known.extension == extension)
        {
            return Err(format!(
                ".{} is built into {}. Use its existing checkbox.",
                extension,
                known.format.label()
            ));
        }
        if let Some(existing) = self
            .preferences
            .values
            .archive_custom_extensions
            .get(&extension)
        {
            return Err(format!(
                ".{} is already assigned to {}. Remove that assignment first.",
                extension,
                existing.label()
            ));
        }
        let mut values = self.preferences.values.clone();
        values.archive_custom_extensions.insert(extension, format);
        midnight_core::validate_archive_custom_extensions(&values.archive_custom_extensions)?;
        self.update_preferences(values, send);
        Ok(())
    }

    pub(crate) fn remove_archive_extension(
        &mut self,
        extension: &str,
        format: ArchiveFormat,
        send: &dyn Fn(Command) -> bool,
    ) {
        if !self.editor_preferences_ready()
            || self
                .preferences
                .values
                .archive_custom_extensions
                .get(extension)
                != Some(&format)
        {
            return;
        }
        let mut values = self.preferences.values.clone();
        values.archive_custom_extensions.remove(extension);
        self.update_preferences(values, send);
    }

    pub(crate) fn set_archive_association_enabled(
        &mut self,
        extension: &str,
        enabled: bool,
        send: &dyn Fn(Command) -> bool,
    ) {
        let Some(association) = ARCHIVE_ASSOCIATIONS
            .iter()
            .find(|association| association.extension == extension)
        else {
            return;
        };
        self.preferences
            .edited
            .archive_associations
            .insert(extension.to_owned());
        let mut values = self.preferences.values.clone();
        if enabled == association.enabled_by_default {
            values.archive_association_overrides.remove(extension);
        } else {
            values
                .archive_association_overrides
                .insert(extension.to_owned(), enabled);
        }
        self.update_preferences(values, send);
    }
}
