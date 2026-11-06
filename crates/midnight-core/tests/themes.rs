use midnight_core::PreferenceTheme;

#[test]
fn theme_catalog_keeps_original_ids_and_appends_five_stable_names() {
    let expected = [
        (PreferenceTheme::Blue, "blue"),
        (PreferenceTheme::Dark, "dark"),
        (PreferenceTheme::Light, "light"),
        (PreferenceTheme::Dracula, "dracula"),
        (PreferenceTheme::Nord, "nord"),
        (PreferenceTheme::GruvboxDark, "gruvbox-dark"),
        (PreferenceTheme::CatppuccinMocha, "catppuccin-mocha"),
        (PreferenceTheme::SolarizedLight, "solarized-light"),
    ];
    assert_eq!(PreferenceTheme::default(), PreferenceTheme::Blue);
    assert_eq!(PreferenceTheme::ALL.len(), expected.len());
    for (index, (theme, name)) in expected.into_iter().enumerate() {
        assert_eq!(PreferenceTheme::ALL[index], theme);
        assert_eq!(theme.index(), index as i32);
        assert_eq!(PreferenceTheme::from_index(index as i32), Some(theme));
        assert_eq!(theme.storage_name(), name);
        assert_eq!(PreferenceTheme::from_storage_name(name), Some(theme));
    }
}

#[test]
fn unknown_theme_ids_and_names_are_never_silently_coerced() {
    for index in [i32::MIN, -1, 8, 99, i32::MAX] {
        assert_eq!(PreferenceTheme::from_index(index), None);
    }
    for name in ["", "future-theme", "Nord", "gruvbox_dark", "dracula "] {
        assert_eq!(PreferenceTheme::from_storage_name(name), None);
    }
}
