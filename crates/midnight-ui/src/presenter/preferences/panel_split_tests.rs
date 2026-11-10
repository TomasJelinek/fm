use super::*;
use std::cell::RefCell;

#[test]
fn unchanged_or_invalid_ratios_do_not_write_preferences() {
    let mut p = Presenter::new("left".into(), "right".into());
    p.preferences_loaded(Ok(UserPreferences::default()));
    for ratio in [0.5, 0.500_001, 0.0, 1.0, -1.0, f32::NAN, f32::INFINITY] {
        p.change_panel_split(ratio, &|_| {
            panic!("invalid or unchanged split must not save")
        });
    }
    assert_eq!(p.preferences.values.panel_split_basis_points, 5000);
    assert!(p.pending_preferences_save().is_none());
}

#[test]
fn early_split_change_merges_with_saved_settings_and_waits_for_load() {
    let mut p = Presenter::new("left".into(), "right".into());
    p.change_panel_split(0.618, &|_| panic!("cannot overwrite unread settings"));
    assert!(p.pending_preferences_save().is_none());
    p.preferences_loaded(Ok(UserPreferences {
        panel_split_basis_points: 4000,
        font_family: "Saved font".into(),
        ..Default::default()
    }));
    let Command::SavePreferences { preferences, .. } = p.pending_preferences_save().unwrap() else {
        panic!()
    };
    assert_eq!(preferences.panel_split_basis_points, 6180);
    assert_eq!(preferences.font_family, "Saved font");
}

#[test]
fn failed_initial_read_never_overwrites_unknown_preferences_with_split() {
    let mut p = Presenter::new("left".into(), "right".into());
    p.preferences_loaded(Err(FsError::from_io(
        "load",
        std::path::Path::new("settings"),
        std::io::ErrorKind::PermissionDenied.into(),
    )));
    p.change_panel_split(0.6, &|_| panic!("failed settings load must block saves"));
    assert!(p.pending_preferences_save().is_none());
}

#[test]
fn split_save_survives_backpressure_and_options_cancel_without_saving_theme_preview() {
    let mut p = Presenter::new("left".into(), "right".into());
    p.preferences_loaded(Ok(UserPreferences::default()));
    p.begin_options();
    p.change_preferences(1, 1, 0, &|_| panic!("theme preview must not persist"));
    let sent = RefCell::new(Vec::new());
    p.change_panel_split(0.63, &|command| {
        sent.borrow_mut().push(command);
        false
    });
    assert_eq!(sent.borrow().len(), 1);
    p.cancel_options(&|_| true);
    let Command::SavePreferences { preferences, .. } = p.pending_preferences_save().unwrap() else {
        panic!()
    };
    assert_eq!(preferences.panel_split_basis_points, 6300);
    assert_eq!(preferences.theme, PreferenceTheme::default());
    assert_eq!(preferences.density, UiDensity::Desktop);
    assert_eq!(p.preferences.values.panel_split_basis_points, 6300);
}
