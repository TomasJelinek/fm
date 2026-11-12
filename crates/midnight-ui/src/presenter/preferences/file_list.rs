//! Successful modeless exports survive an independently previewed Options session.
use super::*;

impl Presenter {
    pub(in crate::presenter) fn save_file_list_preferences(
        &mut self,
        file_list: FileListPreferences,
        send: &dyn Fn(Command) -> bool,
    ) {
        if self.preferences.options.open && !self.preferences.loaded {
            self.preferences.values.file_list = file_list;
            self.preferences.edited.file_list = true;
            self.preferences.unsaved = true;
            self.preferences.revision.0 = self.preferences.revision.0.saturating_add(1);
            return;
        }
        let previous = self.preferences.persistable_values();
        let revision = self.preferences.revision;
        if let Some(baseline) = &mut self.preferences.options.baseline {
            baseline.file_list = file_list.clone();
        }
        let mut values = self.preferences.values.clone();
        values.file_list = file_list;
        self.update_preferences(values, send);
        // Changing only destination/append can be hidden by the newly committed
        // Options baseline. It still requires its own asynchronous settings save.
        if self.preferences.revision == revision
            && self.preferences.persistable_values() != previous
        {
            self.preferences.revision.0 = self.preferences.revision.0.saturating_add(1);
            self.preferences.unsaved = true;
            self.send_preferences_save(send);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepted_export_survives_options_cancel_but_reset_is_a_preview() {
        let root = std::env::temp_dir();
        let mut p = Presenter::new(root.clone(), root.clone());
        p.preferences.loaded = true;
        p.begin_options();
        let accepted = FileListPreferences {
            template_history: vec!["$(FileSize)$(LF)".into()],
            destination: 2,
            path: Some(root.join("list.txt")),
            append: true,
        };
        p.save_file_list_preferences(accepted.clone(), &|_| true);
        p.reset_options(&|_| true);
        assert_eq!(p.preferences.values.file_list.destination, 0);
        assert!(!p.preferences.values.file_list.append);
        assert_eq!(
            p.preferences.values.file_list.template_history,
            accepted.template_history
        );
        assert_eq!(p.preferences.values.file_list.path, accepted.path);
        assert_eq!(p.preferences.persistable_values().file_list, accepted);
        p.cancel_options(&|_| true);
        assert_eq!(p.preferences.values.file_list, accepted);
    }

    #[test]
    fn accepted_template_before_initial_load_is_not_lost() {
        let root = std::env::temp_dir();
        let mut p = Presenter::new(root.clone(), root);
        let accepted = FileListPreferences {
            template_history: vec!["$(FileName)$(LF)".into()],
            ..Default::default()
        };
        p.save_file_list_preferences(accepted.clone(), &|_| true);
        p.preferences_loaded(Ok(UserPreferences::default()));
        assert_eq!(p.preferences.values.file_list, accepted);
    }

    #[test]
    fn destination_only_success_is_saved_without_leaking_options_theme_preview() {
        let root = std::env::temp_dir();
        let mut p = Presenter::new(root.clone(), root);
        p.preferences.loaded = true;
        let original_theme = p.preferences.values.theme;
        p.begin_options();
        p.preferences.values.theme = PreferenceTheme::Light;
        let mut accepted = p.preferences.values.file_list.clone();
        accepted.destination = 1;
        let saved = std::cell::RefCell::new(Vec::new());
        p.save_file_list_preferences(accepted.clone(), &|command| {
            if let Command::SavePreferences { preferences, .. } = command {
                saved.borrow_mut().push(preferences);
            }
            true
        });
        assert_eq!(saved.borrow().len(), 1);
        assert_eq!(saved.borrow()[0].file_list, accepted);
        assert_eq!(saved.borrow()[0].theme, original_theme);
        p.cancel_options(&|_| true);
        assert_eq!(p.preferences.values.file_list, accepted);
        assert_eq!(p.preferences.values.theme, original_theme);
    }
}
