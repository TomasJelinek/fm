use super::*;
use std::cell::RefCell;

#[test]
fn all_theme_choices_map_to_the_saved_choice_and_back() {
    for theme in PreferenceTheme::ALL {
        let mut presenter = Presenter::new("left".into(), "right".into());
        presenter.set_host_dark(!theme.is_light());
        presenter.preferences_loaded(Ok(UserPreferences::default()));
        let saved = RefCell::new(Vec::new());
        presenter.change_preferences(theme.index(), 0, 0, &|command| {
            let Command::SavePreferences { preferences, .. } = command else {
                panic!("unexpected command");
            };
            saved.borrow_mut().push(*preferences);
            true
        });
        assert_eq!(presenter.preferences.theme_index(), theme.index());
        assert_eq!(saved.borrow().len(), 1);
        let saved = &saved.borrow()[0];
        if theme.is_light() {
            assert_eq!(saved.light_theme, theme);
            assert_eq!(saved.dark_theme, PreferenceTheme::DEFAULT_DARK);
        } else {
            assert_eq!(saved.dark_theme, theme);
            assert_eq!(saved.light_theme, PreferenceTheme::DEFAULT_LIGHT);
        }

        let mut restored = Presenter::new("left".into(), "right".into());
        restored.set_host_dark(!theme.is_light());
        restored.preferences_loaded(Ok(saved.clone()));
        assert_eq!(restored.preferences.theme_index(), theme.index());
    }
}

#[test]
fn every_early_theme_preview_survives_delayed_load_without_losing_other_settings() {
    for theme in PreferenceTheme::ALL {
        let mut presenter = Presenter::new("left".into(), "right".into());
        presenter.set_host_dark(!theme.is_light());
        presenter.change_preferences(theme.index(), 0, 0, &|_| {
            panic!("theme preview must wait for stored settings");
        });
        presenter.preferences_loaded(Ok(UserPreferences {
            dark_theme: PreferenceTheme::Dark,
            light_theme: PreferenceTheme::Light,
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
            saved.borrow_mut().push(*preferences);
            true
        });
        assert_eq!(saved.borrow().len(), 1);
        let saved = saved.borrow();
        if theme.is_light() {
            assert_eq!(saved[0].light_theme, theme);
            assert_eq!(saved[0].dark_theme, PreferenceTheme::Dark);
        } else {
            assert_eq!(saved[0].dark_theme, theme);
            assert_eq!(saved[0].light_theme, PreferenceTheme::Light);
        }
        assert_eq!(saved[0].density, UiDensity::Touch);
        assert_eq!(saved[0].font_family, "Saved 文档");
        assert_eq!(saved[0].desktop_font_size, 18);
    }
}

#[test]
fn invalid_theme_callback_indices_keep_both_slots() {
    for index in [-1, PreferenceTheme::ALL.len() as i32, i32::MAX] {
        let mut presenter = Presenter::new("left".into(), "right".into());
        presenter.preferences_loaded(Ok(UserPreferences {
            dark_theme: PreferenceTheme::Nord,
            ..Default::default()
        }));
        presenter.change_preferences(index, 0, 0, &|_| panic!("invalid theme must not save"));
        assert_eq!(
            presenter.preferences.values.dark_theme,
            PreferenceTheme::Nord
        );
        assert_eq!(
            presenter.preferences.values.light_theme,
            PreferenceTheme::DEFAULT_LIGHT
        );
    }
}

#[test]
fn changing_host_appearance_during_options_applies_the_matching_draft_and_cancel_restores_it() {
    let mut p = Presenter::new("left".into(), "right".into());
    let saved = UserPreferences {
        dark_theme: PreferenceTheme::Dark,
        light_theme: PreferenceTheme::Light,
        ..Default::default()
    };
    p.preferences_loaded(Ok(saved.clone()));
    p.begin_options();
    let no_save = |_| panic!("Options previews must not save");
    p.change_preferences(PreferenceTheme::Iris.index(), 0, 0, &no_save);
    assert_eq!(p.preferences.theme_index(), saved.dark_theme.index());
    assert!(p.set_host_dark(false));
    assert!(!p.set_host_dark(false));
    assert_eq!(p.preferences.theme_index(), PreferenceTheme::Iris.index());
    p.change_preferences(PreferenceTheme::PlumSmoke.index(), 0, 0, &no_save);
    assert_eq!(p.preferences.theme_index(), PreferenceTheme::Iris.index());
    p.cancel_options(&no_save);
    assert_eq!(p.preferences.theme_index(), saved.light_theme.index());
    assert!(p.set_host_dark(true));
    assert_eq!(p.preferences.theme_index(), saved.dark_theme.index());
}
