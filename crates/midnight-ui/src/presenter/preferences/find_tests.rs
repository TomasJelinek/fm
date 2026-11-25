use super::*;

fn geometry() -> WindowGeometry {
    WindowGeometry {
        normal: WindowRect {
            x: -800,
            y: 40,
            width: 900,
            height: 640,
        },
        monitor: None,
        maximized: false,
        fullscreen: false,
    }
}

#[test]
fn accepted_search_before_load_merges_saved_history_and_placement() {
    let mut p = Presenter::new("left".into(), "right".into());
    p.remember_find_mask("*.rs", &|_| panic!("must wait for the initial read"));
    p.remember_find_mask("*.txt", &|_| panic!("must wait for the initial read"));
    let mut loaded = UserPreferences::default();
    loaded.find.window_geometry = Some(geometry());
    loaded.find.mask_history = vec!["*.old".into(), "*.rs".into()];
    p.preferences_loaded(Ok(loaded));
    assert_eq!(p.preferences.values.find.window_geometry, Some(geometry()));
    assert_eq!(
        p.preferences.values.find.mask_history,
        ["*.txt", "*.rs", "*.old"]
    );
    let Command::SavePreferences { preferences, .. } = p.pending_preferences_save().unwrap() else {
        panic!()
    };
    assert_eq!(preferences.find, p.preferences.values.find);
}

#[test]
fn find_saves_survive_options_reset_and_cancel_without_saving_previewed_theme() {
    let mut p = Presenter::new("left".into(), "right".into());
    p.preferences.loaded = true;
    let original_theme = p.preferences.values.light_theme;
    p.begin_options();
    p.preferences.values.light_theme = PreferenceTheme::Light;
    let saved = std::cell::RefCell::new(Vec::new());
    let send = |command| {
        if let Command::SavePreferences { preferences, .. } = command {
            saved.borrow_mut().push(preferences);
        }
        true
    };
    p.remember_find_mask("*.文档", &send);
    p.save_find_window_geometry(geometry(), &send);
    assert_eq!(saved.borrow().len(), 2);
    assert!(
        saved
            .borrow()
            .iter()
            .all(|p| p.light_theme == original_theme)
    );
    p.reset_options(&|_| true);
    p.cancel_options(&|_| true);
    assert_eq!(p.preferences.values.find.mask_history, ["*.文档"]);
    assert_eq!(p.preferences.values.find.window_geometry, Some(geometry()));
}

#[test]
fn early_find_placement_preserves_saved_history_and_rejected_writes_are_retryable() {
    let mut p = Presenter::new("left".into(), "right".into());
    p.save_find_window_geometry(geometry(), &|_| panic!("must wait for the initial read"));
    let mut loaded = UserPreferences::default();
    loaded.find.mask_history = vec!["*.saved".into()];
    p.preferences_loaded(Ok(loaded));
    assert_eq!(p.preferences.values.find.mask_history, ["*.saved"]);
    assert_eq!(p.preferences.values.find.window_geometry, Some(geometry()));
    p.remember_find_mask("*.new", &|_| false);
    assert!(p.preferences.unsaved);
    let Command::SavePreferences { preferences, .. } = p.pending_preferences_save().unwrap() else {
        panic!()
    };
    assert_eq!(preferences.find.mask_history, ["*.new", "*.saved"]);
}

#[test]
fn imported_find_choices_apply_only_after_options_save_succeeds() {
    let mut p = Presenter::new("left".into(), "right".into());
    p.preferences.loaded = true;
    p.begin_options();
    let mut imported = UserPreferences::default();
    imported.find.window_geometry = Some(geometry());
    imported.find.mask_history = vec!["*.imported".into()];
    p.preferences.settings_file.staged = Some(Box::new(imported.clone()));
    let saved = std::cell::RefCell::new(None);
    p.accept_options(&|command| {
        if let Command::SavePreferences {
            revision,
            preferences,
        } = command
        {
            *saved.borrow_mut() = Some((revision, preferences));
        }
        true
    });
    assert_eq!(p.preferences.values.find, FindPreferences::default());
    let (revision, written) = saved.into_inner().unwrap();
    assert_eq!(written.find, imported.find);
    p.preferences_saved(revision, Ok(()));
    assert_eq!(p.preferences.values.find, imported.find);
}

#[test]
fn final_placement_flush_retries_rejected_save_without_committing_options_preview() {
    let mut p = Presenter::new("left".into(), "right".into());
    p.preferences.loaded = true;
    let original_theme = p.preferences.values.light_theme;
    p.begin_options();
    p.preferences.values.light_theme = PreferenceTheme::Light;
    *p.finds.pending_window_geometry.borrow_mut() = Some(geometry());
    p.flush_find_window_preferences(&|_| false);
    assert!(p.finds.pending_window_geometry.borrow().is_none());
    assert_eq!(p.preferences.values.find.window_geometry, Some(geometry()));
    assert_eq!(p.preferences.values.light_theme, PreferenceTheme::Light);
    let Command::SavePreferences { preferences, .. } = p.pending_preferences_save().unwrap() else {
        panic!()
    };
    assert_eq!(preferences.find.window_geometry, Some(geometry()));
    assert_eq!(preferences.light_theme, original_theme);
}
