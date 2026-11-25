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
fn options_previews_the_last_picked_theme_whatever_the_host_and_cancel_restores_it() {
    let mut p = Presenter::new("left".into(), "right".into());
    p.set_host_dark(true);
    let saved = UserPreferences {
        dark_theme: PreferenceTheme::Dark,
        light_theme: PreferenceTheme::Light,
        ..Default::default()
    };
    p.preferences_loaded(Ok(saved.clone()));
    assert_eq!(p.preferences.theme_index(), saved.dark_theme.index());
    p.begin_options();
    let no_save = |_| panic!("Options previews must not save");
    // A light pick is previewed while the host is dark.
    p.change_preferences(PreferenceTheme::Iris.index(), 0, 0, &no_save);
    assert_eq!(p.preferences.theme_index(), PreferenceTheme::Iris.index());
    assert!(p.set_host_dark(false));
    assert!(!p.set_host_dark(false));
    assert_eq!(p.preferences.theme_index(), PreferenceTheme::Iris.index());
    // A dark pick is previewed while the host is light.
    p.change_preferences(PreferenceTheme::PlumSmoke.index(), 0, 0, &no_save);
    assert_eq!(
        p.preferences.theme_index(),
        PreferenceTheme::PlumSmoke.index()
    );
    // Density keeps the previewed theme.
    p.change_preferences(PreferenceTheme::Iris.index(), 1, 1, &no_save);
    assert_eq!(
        p.preferences.theme_index(),
        PreferenceTheme::PlumSmoke.index()
    );
    p.cancel_options(&no_save);
    assert_eq!(p.preferences.theme_index(), saved.light_theme.index());
    assert!(p.set_host_dark(true));
    assert_eq!(p.preferences.theme_index(), saved.dark_theme.index());
}

#[test]
fn ok_shows_the_theme_that_matches_the_host_and_reset_ends_the_preview() {
    let mut p = Presenter::new("left".into(), "right".into());
    p.set_host_dark(true);
    p.preferences_loaded(Ok(UserPreferences::default()));
    assert_eq!(
        p.preferences.theme_index(),
        PreferenceTheme::DEFAULT_DARK.index()
    );
    let no_save = |_| panic!("Options previews must not save");
    p.begin_options();
    p.change_preferences(PreferenceTheme::Iris.index(), 0, 0, &no_save);
    p.reset_options(&no_save);
    assert_eq!(
        p.preferences.theme_index(),
        PreferenceTheme::DEFAULT_DARK.index()
    );
    p.change_preferences(PreferenceTheme::Iris.index(), 0, 0, &no_save);
    assert_eq!(p.preferences.theme_index(), PreferenceTheme::Iris.index());
    let saves = RefCell::new(Vec::new());
    p.accept_options(&|command| {
        if let Command::SavePreferences {
            revision,
            preferences,
        } = command
        {
            saves.borrow_mut().push((revision, *preferences));
        }
        true
    });
    let (revision, saved) = saves.borrow()[0].clone();
    assert_eq!(saved.light_theme, PreferenceTheme::Iris);
    assert_eq!(saved.dark_theme, PreferenceTheme::DEFAULT_DARK);
    // Still the preview until the save completes Options.
    assert_eq!(p.preferences.theme_index(), PreferenceTheme::Iris.index());
    p.preferences_saved(revision, Ok(()));
    assert!(!p.preferences.options.open);
    // The host is dark, so the saved dark choice is shown, not the last preview.
    assert_eq!(
        p.preferences.theme_index(),
        PreferenceTheme::DEFAULT_DARK.index()
    );
    assert!(p.set_host_dark(false));
    assert_eq!(p.preferences.theme_index(), PreferenceTheme::Iris.index());
    // A new session starts without a preview.
    p.begin_options();
    assert!(p.set_host_dark(true));
    assert_eq!(
        p.preferences.theme_index(),
        PreferenceTheme::DEFAULT_DARK.index()
    );
}
