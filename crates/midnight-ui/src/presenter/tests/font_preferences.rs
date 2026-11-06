use super::*;

fn with_catalog(families: &[&str]) -> Presenter {
    let mut p = with_unloaded_catalog(families);
    p.apply(Event::PreferencesLoaded(Ok(UserPreferences::default())));
    p
}

fn with_unloaded_catalog(families: &[&str]) -> Presenter {
    let mut p = Presenter::new(test_root(), test_root());
    p.apply(Event::FontFamiliesLoaded(Ok(families
        .iter()
        .map(|family| (*family).into())
        .collect())));
    p
}

#[test]
fn font_requests_deduplicate_pending_and_loaded_catalogs() {
    let mut p = Presenter::new(test_root(), test_root());
    assert!(!p.fonts_ready());
    let sent = RefCell::new(vec![]);
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    p.request_fonts(&send);
    p.request_fonts(&send);
    assert!(!p.fonts_ready());
    assert_eq!(sent.borrow().len(), 1);
    assert!(matches!(sent.borrow()[0], Command::ListFontFamilies));
    p.apply(Event::FontFamiliesLoaded(Ok(vec!["Noto Sans".into()])));
    p.request_fonts(&|_| panic!("loaded catalog must not be requested again"));
    assert!(p.fonts_ready());
}

#[test]
fn font_catalog_queue_failure_allows_retry() {
    let mut p = Presenter::new(test_root(), test_root());
    p.request_fonts(&|_| false);
    assert!(!p.fonts_ready());
    let sent = RefCell::new(false);
    p.request_fonts(&|command| {
        assert!(matches!(command, Command::ListFontFamilies));
        *sent.borrow_mut() = true;
        true
    });
    assert!(*sent.borrow());
}

#[test]
fn catalog_keeps_unicode_names_and_missing_configured_family() {
    let mut p = with_unloaded_catalog(&["Noto Sans", "源ノ角ゴシック", "文档 🦀"]);
    p.apply(Event::PreferencesLoaded(Ok(UserPreferences {
        font_family: "Unavailable Café".into(),
        ..Default::default()
    })));
    assert_eq!(
        p.font_choices(),
        [
            "Application default",
            "Noto Sans",
            "源ノ角ゴシック",
            "文档 🦀",
            "Unavailable Café"
        ]
    );
    assert_eq!(p.font_index(), 4);
    assert_eq!(p.preferences.values.font_family, "Unavailable Café");
    p.change_fonts(3, 12, 18, -1, &|command| {
        assert!(
            matches!(command, Command::SavePreferences { preferences, .. }
            if preferences.font_family == "文档 🦀")
        );
        true
    });
    assert_eq!(p.preferences.values.font_family, "文档 🦀");
}

#[test]
fn installed_configured_font_is_not_duplicated_in_choices() {
    let mut p = with_catalog(&["Noto Sans", "Café"]);
    p.preferences.values.font_family = "Café".into();
    assert_eq!(
        p.font_choices(),
        ["Application default", "Noto Sans", "Café"]
    );
    assert_eq!(p.font_index(), 2);
}

#[test]
fn catalog_failure_enables_default_and_current_custom_fallbacks() {
    let mut p = Presenter::new(test_root(), test_root());
    p.preferences.values.font_family = "Custom 文档".into();
    p.apply(Event::FontFamiliesLoaded(Err(FsError::from_io(
        "list fonts",
        Path::new(""),
        io::Error::other("catalog unavailable"),
    ))));
    assert!(p.fonts_ready());
    assert_eq!(p.font_choices(), ["Application default", "Custom 文档"]);
    assert_eq!(p.font_index(), 1);
    assert!(
        p.desktop
            .notice
            .as_ref()
            .unwrap()
            .1
            .contains("catalog unavailable")
    );
    p.change_fonts(0, 11, 15, -1, &|_| true);
    assert!(p.preferences.values.font_family.is_empty());
}

#[test]
fn font_change_preserves_theme_density_shell_and_raw_family() {
    let mut p = with_catalog(&["Café 文档"]);
    p.preferences.values = UserPreferences {
        theme: PreferenceTheme::Light,
        density: UiDensity::Touch,
        windows_shell: WindowsShell::Cmd,
        ..Default::default()
    };
    p.change_fonts(1, 8, 32, -1, &|command| {
        assert!(
            matches!(command, Command::SavePreferences { preferences, .. }
            if preferences == UserPreferences {
                theme: PreferenceTheme::Light,
                density: UiDensity::Touch,
                windows_shell: WindowsShell::Cmd,
                font_family: "Café 文档".into(),
                desktop_font_size: 8,
                touch_font_size: 32,
                ..Default::default()
            })
        );
        true
    });
    assert_eq!(p.preferences.values.desktop_font_size, 8);
    assert_eq!(p.preferences.values.touch_font_size, 32);
}

#[test]
fn invalid_font_sizes_or_indices_do_not_preview_or_save() {
    for (index, desktop, touch) in [
        (-1, 11, 15),
        (2, 11, 15),
        (0, 7, 15),
        (0, 33, 15),
        (0, 11, 7),
        (0, 11, 33),
        (0, -1, 15),
        (0, 65_536, 15),
    ] {
        let mut p = with_catalog(&["Noto Sans"]);
        p.change_fonts(index, desktop, touch, -1, &|_| {
            panic!("invalid choice must not save")
        });
        assert_eq!(p.preferences.values, UserPreferences::default());
        assert_eq!(p.preferences.revision, PreferencesRevision(0));
    }
}

#[test]
fn font_size_preview_wins_over_late_initial_preferences_load() {
    let mut p = Presenter::new(test_root(), test_root());
    p.change_fonts(0, 13, 19, -1, &|_| true);
    p.apply(Event::PreferencesLoaded(Ok(UserPreferences {
        font_family: "Late loaded".into(),
        desktop_font_size: 21,
        ..Default::default()
    })));
    assert_eq!(p.preferences.values.desktop_font_size, 13);
    assert_eq!(p.preferences.values.touch_font_size, 19);
    assert_eq!(p.preferences.values.font_family, "Late loaded");
    assert_eq!(p.preferences.revision, PreferencesRevision(1));
}

#[test]
fn family_changes_are_rejected_until_catalog_settles() {
    let mut p = Presenter::new(test_root(), test_root());
    p.preferences.values.font_family = "Configured".into();
    p.change_fonts(0, 11, 15, -1, &|_| {
        panic!("pending catalog must not change families")
    });
    assert_eq!(p.preferences.values.font_family, "Configured");
    assert_eq!(p.preferences.revision, PreferencesRevision(0));
}

#[test]
fn theme_and_terminal_changes_preserve_font_choices_and_sizes() {
    let mut p = with_catalog(&["Café 文档"]);
    p.change_fonts(1, 12, 18, -1, &|_| true);
    p.change_preferences(2, 1, -1, &|_| true);
    p.apply(Event::DesktopContext {
        shell: ShellFlavor::PowerShell,
        locations: vec![],
    });
    p.change_terminal(1, false, &|_| true);
    assert_eq!(p.preferences.values.font_family, "Café 文档");
    assert_eq!(p.preferences.values.desktop_font_size, 12);
    assert_eq!(p.preferences.values.touch_font_size, 18);
    assert_eq!(p.preferences.values.theme, PreferenceTheme::Light);
    assert_eq!(p.preferences.values.density, UiDensity::Touch);
    assert_eq!(p.preferences.values.windows_shell, WindowsShell::Cmd);
}

#[test]
fn failed_font_save_keeps_preview_and_can_retry() {
    let mut p = with_catalog(&["Noto Sans"]);
    p.change_fonts(1, 12, 18, -1, &|_| false);
    assert_eq!(p.preferences.values.font_family, "Noto Sans");
    assert!(p.preferences.unsaved);
    p.change_fonts(1, 12, 18, -1, &|command| {
        assert!(matches!(command, Command::SavePreferences {
            revision: PreferencesRevision(2), preferences
        } if preferences.desktop_font_size == 12));
        true
    });
}
