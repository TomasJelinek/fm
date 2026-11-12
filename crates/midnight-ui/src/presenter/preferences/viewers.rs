//! Viewer choices merge into the latest app preferences, then use the backend writer.
use super::*;

impl Presenter {
    pub fn viewer_preferences(&self) -> &ViewerPreferences {
        &self.preferences.values.viewer
    }

    pub fn save_viewer_preferences(
        &mut self,
        change: ViewerPreferenceChange,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let mut preferences = self.preferences.values.clone();
        if preferences.viewer.apply(&change).is_err() {
            return false;
        }
        if !self.preferences.loaded {
            let pending = &mut self.preferences.viewer_before_load;
            let resets_font =
                change == ViewerPreferenceChange::MarkdownPreset(MarkdownPreset::GitHub);
            remember(pending, change);
            if resets_font {
                // Retain the reset even if a later spacing preset replaces this
                // preset patch before loading completes. Explicit later font
                // edits still win, and the pending list remains field-bounded.
                let font = &preferences.viewer.markdown_font;
                remember(
                    pending,
                    ViewerPreferenceChange::FontFamily(
                        ViewerFontKind::Markdown,
                        font.family.clone(),
                    ),
                );
                remember(
                    pending,
                    ViewerPreferenceChange::FontSize(ViewerFontKind::Markdown, font.size),
                );
            }
        }
        self.update_preferences(preferences, send);
        true
    }
}

fn remember(pending: &mut Vec<ViewerPreferenceChange>, change: ViewerPreferenceChange) {
    if let Some(index) = pending.iter().position(|old| same_field(old, &change)) {
        pending.remove(index);
    }
    pending.push(change);
}

fn same_field(left: &ViewerPreferenceChange, right: &ViewerPreferenceChange) -> bool {
    use ViewerPreferenceChange::*;
    match (left, right) {
        (FontFamily(left, _), FontFamily(right, _)) | (FontSize(left, _), FontSize(right, _)) => {
            left == right
        }
        _ => std::mem::discriminant(left) == std::mem::discriminant(right),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn viewer_windows_merge_fields_and_preserve_other_application_settings() {
        let root = std::env::temp_dir();
        let mut p = Presenter::new(root.clone(), root);
        p.apply(Event::PreferencesLoaded(Ok(Box::new(UserPreferences {
            theme: PreferenceTheme::Dark,
            ..Default::default()
        }))));
        let saves = RefCell::new(Vec::new());
        let send = |command| {
            if let Command::SavePreferences { preferences, .. } = command {
                saves.borrow_mut().push(*preferences);
            }
            true
        };
        assert!(p.save_viewer_preferences(ViewerPreferenceChange::Wrap(true), &send));
        assert!(p.save_viewer_preferences(
            ViewerPreferenceChange::ImageZoom(ViewerZoomPreference::ActualPixels),
            &send
        ));
        let saved = saves.borrow();
        let latest = saved.last().unwrap();
        assert!(latest.viewer.wrap);
        assert_eq!(latest.viewer.image_zoom, ViewerZoomPreference::ActualPixels);
        assert_eq!(latest.theme, PreferenceTheme::Dark);
        drop(saved);
        let before = p.viewer_preferences().clone();
        assert!(!p.save_viewer_preferences(
            ViewerPreferenceChange::FontSize(ViewerFontKind::Text, 0),
            &send
        ));
        assert_eq!(p.viewer_preferences(), &before);
    }

    #[test]
    fn early_viewer_changes_merge_fieldwise_with_loaded_settings_and_coalesce() {
        let root = std::env::temp_dir();
        let mut p = Presenter::new(root.clone(), root);
        for index in 0..500 {
            assert!(p.save_viewer_preferences(
                ViewerPreferenceChange::Wrap(index % 2 == 0),
                &|_| panic!("must await initial load")
            ));
        }
        assert_eq!(p.preferences.viewer_before_load.len(), 1);
        let mut loaded = UserPreferences::default();
        loaded.viewer.wrap = true;
        loaded.viewer.source_font.size = 21;
        loaded.viewer.markdown_preset = MarkdownPreset::Compact;
        p.apply(Event::PreferencesLoaded(Ok(Box::new(loaded))));
        assert!(!p.viewer_preferences().wrap);
        assert_eq!(p.viewer_preferences().source_font.size, 21);
        assert_eq!(
            p.viewer_preferences().markdown_preset,
            MarkdownPreset::Compact
        );
        assert!(p.preferences.viewer_before_load.is_empty());
    }

    #[test]
    fn early_markdown_preset_and_font_edits_replay_their_final_effects_in_order() {
        use ViewerPreferenceChange::{FontFamily, FontSize, MarkdownPreset as Preset};
        for changes in [
            vec![
                Preset(MarkdownPreset::GitHub),
                FontFamily(ViewerFontKind::Markdown, "serif".into()),
                Preset(MarkdownPreset::GitHub),
            ],
            vec![
                Preset(MarkdownPreset::GitHub),
                Preset(MarkdownPreset::Compact),
            ],
            vec![
                Preset(MarkdownPreset::GitHub),
                FontSize(ViewerFontKind::Markdown, 23),
                Preset(MarkdownPreset::Compact),
            ],
            vec![
                Preset(MarkdownPreset::GitHub),
                FontFamily(ViewerFontKind::Markdown, "serif".into()),
                FontSize(ViewerFontKind::Markdown, 23),
            ],
        ] {
            let root = std::env::temp_dir();
            let mut p = Presenter::new(root.clone(), root);
            for change in &changes {
                assert!(
                    p.save_viewer_preferences(change.clone(), &|_| panic!("await initial load"))
                );
            }
            let expected = p.viewer_preferences().clone();
            let mut loaded = UserPreferences::default();
            loaded.viewer.markdown_font.family = "monospace".into();
            loaded.viewer.markdown_font.size = 35;
            loaded.viewer.source_font.size = 21;
            p.apply(Event::PreferencesLoaded(Ok(Box::new(loaded))));
            assert_eq!(
                p.viewer_preferences().markdown_preset,
                expected.markdown_preset,
                "{changes:?}"
            );
            assert_eq!(
                p.viewer_preferences().markdown_font,
                expected.markdown_font,
                "{changes:?}"
            );
            assert_eq!(p.viewer_preferences().source_font.size, 21);
        }
    }

    #[test]
    fn backpressure_keeps_viewer_preferences_for_the_existing_save_retry() {
        let root = std::env::temp_dir();
        let mut p = Presenter::new(root.clone(), root);
        p.apply(Event::PreferencesLoaded(Ok(Box::default())));
        assert!(p.save_viewer_preferences(ViewerPreferenceChange::LineNumbers(false), &|_| false));
        assert!(p.preferences.unsaved);
        let Command::SavePreferences { preferences, .. } = p.pending_preferences_save().unwrap()
        else {
            panic!("latest preferences must be retryable")
        };
        assert!(!preferences.viewer.line_numbers);
    }
}
