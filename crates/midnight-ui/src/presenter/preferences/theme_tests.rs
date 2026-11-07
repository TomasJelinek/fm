use super::*;
use std::cell::RefCell;

#[test]
fn all_theme_choices_map_to_the_saved_choice_and_back() {
    for theme in PreferenceTheme::ALL {
        let mut presenter = Presenter::new("left".into(), "right".into());
        presenter.preferences_loaded(Ok(UserPreferences::default()));
        let saved = RefCell::new(Vec::new());
        presenter.change_preferences(theme.index(), 0, 0, &|command| {
            let Command::SavePreferences { preferences, .. } = command else {
                panic!("unexpected command");
            };
            saved.borrow_mut().push(preferences);
            true
        });
        assert_eq!(presenter.preferences.values.theme, theme);
        assert_eq!(presenter.preferences.theme_index(), theme.index());
        assert_eq!(saved.borrow().len(), 1);
        assert_eq!(saved.borrow()[0].theme, theme);

        let mut restored = Presenter::new("left".into(), "right".into());
        restored.preferences_loaded(Ok(saved.borrow()[0].clone()));
        assert_eq!(restored.preferences.theme_index(), theme.index());
    }
}

#[test]
fn every_early_theme_preview_survives_delayed_load_without_losing_other_settings() {
    for theme in PreferenceTheme::ALL {
        let mut presenter = Presenter::new("left".into(), "right".into());
        presenter.change_preferences(theme.index(), 0, 0, &|_| {
            panic!("theme preview must wait for stored settings");
        });
        presenter.preferences_loaded(Ok(UserPreferences {
            theme: PreferenceTheme::Dark,
            density: UiDensity::Touch,
            font_family: "Saved 文档".into(),
            desktop_font_size: 18,
            ..Default::default()
        }));
        assert_eq!(presenter.preferences.theme_index(), theme.index());
        let saved = RefCell::new(Vec::new());
        presenter.flush_loaded_preferences(&|command| {
            let Command::SavePreferences { preferences, .. } = command else {
                panic!("unexpected command");
            };
            saved.borrow_mut().push(preferences);
            true
        });
        assert_eq!(saved.borrow().len(), 1);
        let saved = saved.borrow();
        assert_eq!(saved[0].theme, theme);
        assert_eq!(saved[0].density, UiDensity::Touch);
        assert_eq!(saved[0].font_family, "Saved 文档");
        assert_eq!(saved[0].desktop_font_size, 18);
    }
}

#[test]
fn invalid_theme_callback_indices_retain_the_existing_blue_fallback() {
    for index in [-1, 9, i32::MAX] {
        let mut presenter = Presenter::new("left".into(), "right".into());
        presenter.preferences_loaded(Ok(UserPreferences {
            theme: PreferenceTheme::Nord,
            ..Default::default()
        }));
        presenter.change_preferences(index, 0, 0, &|_| true);
        assert_eq!(presenter.preferences.values.theme, PreferenceTheme::Blue);
        assert_eq!(presenter.preferences.theme_index(), 0);
    }
}
