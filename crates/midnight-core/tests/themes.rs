use midnight_core::PreferenceTheme;

#[test]
fn carbon_green_is_an_appended_stable_theme() {
    let theme = PreferenceTheme::from_storage_name("carbon-green")
        .expect("Carbon Green must be offered as a saved theme");
    assert_eq!(theme.index(), 8);
}

#[test]
fn modern_and_retro_themes_append_without_reusing_saved_ids() {
    for (index, name) in [
        (9, "tokyo-night"),
        (10, "one-dark"),
        (11, "rose-pine"),
        (12, "norton-commander"),
        (13, "amber-terminal"),
        (14, "verdant-light"),
        (15, "catppuccin-latte"),
        (16, "rose-pine-dawn"),
    ] {
        let theme = PreferenceTheme::from_storage_name(name)
            .unwrap_or_else(|| panic!("missing saved theme {name}"));
        assert_eq!(theme.index(), index);
        assert_eq!(PreferenceTheme::from_index(index), Some(theme));
        assert_eq!(theme.storage_name(), name);
    }
    assert_eq!(PreferenceTheme::ALL.len(), 26);
}

#[test]
fn contrast_and_slate_themes_append_with_stable_storage_names() {
    for (index, name) in [
        (17, "paper-contrast"),
        (18, "sandstone-contrast"),
        (19, "deep-ocean"),
        (20, "graphite-contrast"),
        (21, "high-contrast-dark"),
        (22, "high-contrast-light"),
        (23, "slate-blue"),
        (24, "slate-teal"),
        (25, "slate-violet"),
    ] {
        let theme = PreferenceTheme::from_storage_name(name)
            .unwrap_or_else(|| panic!("missing saved theme {name}"));
        assert_eq!(theme.index(), index);
        assert_eq!(PreferenceTheme::from_index(index), Some(theme));
        assert_eq!(theme.storage_name(), name);
    }
}

#[test]
fn removed_dos_theme_migrates_to_its_replacement_without_adding_a_picker_entry() {
    let replacement = PreferenceTheme::from_storage_name("amber-terminal").unwrap();
    assert_eq!(
        PreferenceTheme::from_storage_name("dos-commander"),
        Some(replacement)
    );
    assert_eq!(replacement.index(), 13);
    assert!(
        PreferenceTheme::ALL
            .iter()
            .all(|theme| theme.storage_name() != "dos-commander")
    );
}

#[test]
fn theme_catalog_keeps_original_ids_and_appends_stable_names() {
    let expected = [
        (PreferenceTheme::Blue, "blue"),
        (PreferenceTheme::Dark, "dark"),
        (PreferenceTheme::Light, "light"),
        (PreferenceTheme::Dracula, "dracula"),
        (PreferenceTheme::Nord, "nord"),
        (PreferenceTheme::GruvboxDark, "gruvbox-dark"),
        (PreferenceTheme::CatppuccinMocha, "catppuccin-mocha"),
        (PreferenceTheme::SolarizedLight, "solarized-light"),
        (PreferenceTheme::CarbonGreen, "carbon-green"),
    ];
    assert_eq!(PreferenceTheme::default(), PreferenceTheme::Blue);
    assert!(PreferenceTheme::ALL.len() >= expected.len());
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
    for index in [
        i32::MIN,
        -1,
        PreferenceTheme::ALL.len() as i32,
        99,
        i32::MAX,
    ] {
        assert_eq!(PreferenceTheme::from_index(index), None);
    }
    for name in ["", "future-theme", "Nord", "gruvbox_dark", "dracula "] {
        assert_eq!(PreferenceTheme::from_storage_name(name), None);
    }
}
