use super::*;

#[test]
fn panel_modes_save_independently_and_preserve_the_other_panel() {
    let mut p = Presenter::new(test_root(), test_root());
    p.apply_with_sender(
        Event::PreferencesLoaded(Ok(UserPreferences::default())),
        &|_| true,
    );
    let last = RefCell::new(None);
    let send = |command| {
        if let Command::SavePreferences { preferences, .. } = command {
            *last.borrow_mut() = Some(preferences.panel_view_modes);
        }
        true
    };
    p.change_panel_view_mode(PaneId::Left, 1, &send);
    assert_eq!(
        *last.borrow(),
        Some([PanelViewMode::Brief, PanelViewMode::Detailed])
    );
    p.change_panel_view_mode(PaneId::Right, 2, &send);
    assert_eq!(
        *last.borrow(),
        Some([PanelViewMode::Brief, PanelViewMode::Thumbnails])
    );
}

#[test]
fn explicit_default_panel_mode_wins_over_late_loaded_choice() {
    let mut p = Presenter::new(test_root(), test_root());
    p.change_panel_view_mode(PaneId::Right, 0, &|_| {
        panic!("initial read must precede save")
    });
    p.apply_with_sender(
        Event::PreferencesLoaded(Ok(UserPreferences {
            panel_view_modes: [PanelViewMode::Brief, PanelViewMode::Thumbnails],
            ..Default::default()
        })),
        &|_| true,
    );
    assert_eq!(
        p.preferences.values.panel_view_modes,
        [PanelViewMode::Brief, PanelViewMode::Detailed]
    );
}

fn stored_bookmark(id: u64, slot: Option<u8>) -> Bookmark {
    Bookmark {
        id: BookmarkId(id),
        name: format!("Saved {id}"),
        path: test_root().join(format!("saved-{id}")),
        slot,
    }
}

#[test]
fn early_theme_preview_defers_saving_and_preserves_every_untouched_loaded_field() {
    let mut p = Presenter::new(test_root(), test_root());
    p.change_preferences(1, 0, 0, &|_| {
        panic!("the initial read must precede every save")
    });
    assert_eq!(p.preferences.values.theme, PreferenceTheme::Dark);
    assert!(p.new_tab(PaneId::Left, &|_| true));
    let tabs = p.tabs(PaneId::Left);
    let generation = p.panes[0].generation;
    let saved = UserPreferences {
        archive_association_overrides: Default::default(),
        disabled_viewer_plugins: Vec::new(),
        editor: EditorPreferences::default(),
        theme: PreferenceTheme::Light,
        density: UiDensity::Touch,
        windows_shell: WindowsShell::Cmd,
        font_family: "Stored 文档 e\u{301} 🧪".into(),
        desktop_font_size: 18,
        touch_font_size: 23,
        quick_search_filter: true,
        size_display: SizeDisplay::Bytes,
        remember_command_history: true,
        command_history: vec![CommandHistoryEntry {
            command: "echo Saved 文档".into(),
            shell: ShellFlavor::Cmd,
        }],
        bookmarks: vec![stored_bookmark(9, Some(1))],
        tabs: Default::default(),
        panel_view_modes: Default::default(),
        thumbnails_enabled: true,
        thumbnails_network: false,
        shortcuts: vec![],
        panel_filters: Default::default(),
        filename_colors: Default::default(),
    };
    let mut expected = saved.clone();
    expected.theme = PreferenceTheme::Dark;
    expected.tabs = [
        PaneTabPreferences {
            archives: Default::default(),
            paths: vec![test_root(), test_root()],
            active: 1,
        },
        PaneTabPreferences {
            archives: Default::default(),
            paths: vec![test_root()],
            active: 0,
        },
    ];
    let sent = RefCell::new(0);
    p.apply_with_sender(Event::PreferencesLoaded(Ok(saved)), &|command| {
        let Command::SavePreferences {
            revision,
            preferences,
        } = command
        else {
            panic!()
        };
        assert_eq!(revision, PreferencesRevision(2));
        assert_eq!(preferences, expected);
        *sent.borrow_mut() += 1;
        true
    });
    assert_eq!(*sent.borrow(), 1);
    assert_eq!(p.preferences.values, expected);
    assert_eq!(p.tabs(PaneId::Left), tabs);
    assert_eq!(p.panes[0].generation, generation);
}

#[test]
fn early_desktop_size_edit_preserves_loaded_family_touch_size_and_bookmarks() {
    let mut p = Presenter::new(test_root(), test_root());
    p.change_fonts(0, 13, 15, 1, &|_| {
        panic!("size preview cannot save before loading")
    });
    assert_eq!(p.preferences.values.desktop_font_size, 13);
    let saved = UserPreferences {
        font_family: "Installed 字🧪".into(),
        desktop_font_size: 21,
        touch_font_size: 25,
        bookmarks: vec![stored_bookmark(7, None)],
        ..Default::default()
    };
    let mut expected = saved.clone();
    expected.desktop_font_size = 13;
    p.apply_with_sender(Event::PreferencesLoaded(Ok(saved)), &|command| {
        let Command::SavePreferences { preferences, .. } = command else {
            panic!()
        };
        assert_eq!(preferences, expected);
        true
    });
    assert_eq!(p.preferences.values, expected);
}

#[test]
fn early_bookmark_rename_removal_and_assignment_survive_reconciliation() {
    let mut p = Presenter::new(test_root().join("local"), test_root());
    assert!(p.add_bookmark(PaneId::Left, "Local".into(), Some(1), &|_| panic!()));
    let local = p.bookmarks()[0].id;
    assert!(p.rename_bookmark(local, "Local renamed".into(), &|_| panic!()));
    p.panes[0].path = test_root().join("removed-locally");
    assert!(p.add_bookmark(
        PaneId::Left,
        "Removed locally".into(),
        Some(2),
        &|_| panic!()
    ));
    let removed = p.bookmarks()[1].id;
    assert!(p.remove_bookmark(removed, &|_| panic!()));
    let saved = UserPreferences {
        bookmarks: vec![stored_bookmark(1, Some(1)), stored_bookmark(2, Some(2))],
        ..Default::default()
    };
    let sent = RefCell::new(0);
    p.apply_with_sender(Event::PreferencesLoaded(Ok(saved)), &|command| {
        let Command::SavePreferences { preferences, .. } = command else {
            panic!()
        };
        preferences.validate().unwrap();
        assert_eq!(preferences.bookmarks.len(), 3);
        *sent.borrow_mut() += 1;
        true
    });
    assert_eq!(*sent.borrow(), 1);
    let local_bookmark = p.bookmarks().iter().find(|b| b.id == local).unwrap();
    assert_eq!(local_bookmark.name, "Local renamed");
    assert_eq!(local_bookmark.slot, Some(1));
    assert_eq!(p.bookmarks()[0].name, "Saved 1");
    assert_eq!(p.bookmarks()[0].slot, None);
    assert_eq!(p.bookmarks()[1].name, "Saved 2");
    assert_eq!(p.bookmarks()[1].slot, Some(2));
    assert!(!p.open_bookmark(PaneId::Left, removed, &|_| panic!(
        "removed callback must stay stale"
    )));
}

#[test]
fn startup_reconciliation_over_capacity_preserves_all_bookmarks_and_never_saves_invalid_data() {
    let mut p = Presenter::new(test_root(), test_root());
    for index in 0..MAX_BOOKMARKS {
        p.panes[0].path = test_root().join(format!("local-{index}"));
        assert!(p.add_bookmark(PaneId::Left, format!("Local {index}"), None, &|_| panic!()));
    }
    let saved = UserPreferences {
        bookmarks: (1..=MAX_BOOKMARKS as u64)
            .map(|id| stored_bookmark(id, None))
            .collect(),
        ..Default::default()
    };
    p.apply_with_sender(Event::PreferencesLoaded(Ok(saved)), &|_| {
        panic!("128 bookmarks must never escape as a save command")
    });
    assert_eq!(p.bookmarks().len(), 128);
    assert!(p.preferences.unsaved);
    assert!(p.desktop.notice.as_ref().unwrap().1.contains("at most 64"));
    p.change_preferences(1, 1, -1, &|_| {
        panic!("global edits cannot bypass the aggregate validation")
    });
    let remove: Vec<_> = p.bookmarks().iter().take(64).map(|b| b.id).collect();
    let sent = RefCell::new(0);
    for (index, id) in remove.into_iter().enumerate() {
        assert!(p.remove_bookmark(id, &|command| {
            assert_eq!(
                index, 63,
                "saving remains blocked until 64 bookmarks remain"
            );
            let Command::SavePreferences { preferences, .. } = command else {
                panic!()
            };
            preferences.validate().unwrap();
            assert_eq!(preferences.bookmarks.len(), 64);
            *sent.borrow_mut() += 1;
            true
        }));
    }
    assert_eq!(*sent.borrow(), 1);
    assert_eq!(p.bookmarks().len(), 64);
}

#[test]
fn failed_initial_read_blocks_saves_that_could_replace_unknown_saved_bookmarks() {
    let mut p = Presenter::new(test_root(), test_root());
    p.change_preferences(1, 0, -1, &|_| panic!("read has not completed"));
    p.apply_with_sender(
        Event::PreferencesLoaded(Err(FsError::from_io(
            "load preferences",
            &test_root(),
            io::Error::from(io::ErrorKind::PermissionDenied),
        ))),
        &|_| panic!("unread settings must not be overwritten"),
    );
    p.change_preferences(2, 1, -1, &|_| {
        panic!("read error continues to protect saved settings")
    });
    assert_eq!(p.preferences.values.theme, PreferenceTheme::Light);
    assert!(
        p.desktop
            .notice
            .as_ref()
            .unwrap()
            .1
            .contains("preserve existing settings")
    );
}

#[test]
fn explicit_same_index_default_font_preserves_loaded_sizes_and_other_preferences() {
    let mut p = Presenter::new(test_root(), test_root());
    p.apply(Event::FontFamiliesLoaded(Ok(vec!["Installed".into()])));
    p.change_fonts(0, 11, 15, 0, &|_| {
        panic!("default-font preview must wait for the read")
    });
    p.apply_with_sender(
        Event::PreferencesLoaded(Ok(UserPreferences {
            font_family: "Stored custom".into(),
            desktop_font_size: 19,
            touch_font_size: 25,
            bookmarks: vec![stored_bookmark(1, None)],
            ..Default::default()
        })),
        &|command| {
            let Command::SavePreferences { preferences, .. } = command else {
                panic!()
            };
            assert_eq!(preferences.font_family, "");
            assert_eq!(preferences.desktop_font_size, 19);
            assert_eq!(preferences.touch_font_size, 25);
            assert_eq!(preferences.bookmarks.len(), 1);
            true
        },
    );
    assert_eq!(p.preferences.values.font_family, "");
}

#[test]
fn explicit_same_index_default_theme_preserves_loaded_density() {
    let mut p = Presenter::new(test_root(), test_root());
    p.change_preferences(0, 0, 0, &|_| panic!("theme preview must wait for the read"));
    p.apply_with_sender(
        Event::PreferencesLoaded(Ok(UserPreferences {
            theme: PreferenceTheme::Dark,
            density: UiDensity::Touch,
            ..Default::default()
        })),
        &|command| {
            let Command::SavePreferences { preferences, .. } = command else {
                panic!()
            };
            assert_eq!(preferences.theme, PreferenceTheme::Blue);
            assert_eq!(preferences.density, UiDensity::Touch);
            true
        },
    );
}
