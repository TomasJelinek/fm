use super::*;

fn geometry() -> midnight_core::WindowGeometry {
    midnight_core::WindowGeometry {
        normal: midnight_core::WindowRect {
            x: -1400,
            y: 80,
            width: 901,
            height: 633,
        },
        monitor: None,
        maximized: false,
        fullscreen: false,
    }
}

#[test]
fn early_geometry_change_waits_for_read_and_merges_unrelated_saved_fields() {
    let mut presenter = Presenter::new("left".into(), "right".into());
    presenter.save_window_geometry(geometry(), &|_| panic!("cannot save before read"));
    assert!(!presenter.window_geometry_ready());
    assert!(presenter.needs_initial_preferences());
    let loaded = UserPreferences {
        font_family: "Saved font".into(),
        ..Default::default()
    };
    presenter.preferences_loaded(Ok(loaded));
    assert!(presenter.window_geometry_ready());
    let Command::SavePreferences { preferences, .. } =
        presenter.pending_preferences_save().unwrap()
    else {
        panic!()
    };
    assert_eq!(preferences.window_geometry, Some(geometry()));
    assert_eq!(preferences.font_family, "Saved font");
}

#[test]
fn rejected_initial_read_never_publishes_geometry_over_unknown_settings() {
    let mut presenter = Presenter::new("left".into(), "right".into());
    presenter.save_window_geometry(geometry(), &|_| false);
    presenter.preferences_loaded(Err(FsError::from_io(
        "load",
        std::path::Path::new("settings"),
        std::io::ErrorKind::PermissionDenied.into(),
    )));
    assert!(!presenter.window_geometry_ready());
    assert!(presenter.pending_preferences_save().is_none());
}
