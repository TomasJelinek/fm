//! Immediate association choices; storage and provider discovery stay on backend workers.
use super::*;

impl Presenter {
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
