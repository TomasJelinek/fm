use super::*;

#[test]
fn archive_hot_path_add_uses_the_typed_member_location() {
    let root = test_root();
    let mut presenter = Presenter::new(root.clone(), root.clone());
    presenter.panes[0].archive = Some(crate::presenter::archives::ArchivePane::unavailable(
        ArchiveAddress {
            format: None,
            provider: "midnight.fs.archive".into(),
            container: root.join("資料@source.zip"),
            source_version: vec![1, 2, 3],
            components: vec!["日本語".as_bytes().to_vec(), b"src".to_vec()],
        },
        root,
    ));
    assert!(presenter.add_bookmark(PaneId::Left, "src".into(), None, &|_| true));
    assert_eq!(presenter.bookmarks().len(), 1);
    assert_eq!(presenter.bookmarks()[0].slot, Some(1));
    let saved = presenter.bookmarks()[0].clone();
    let address = saved.archive.as_ref().unwrap();
    assert_eq!(
        address.components,
        ["日本語".as_bytes().to_vec(), b"src".to_vec()]
    );
    assert_eq!(saved.path, test_root());
    assert!(saved.display_text().contains("資料@source.zip@日本語"));

    // A changed archive revision updates the same saved location and shortcut.
    presenter.panes[0]
        .archive
        .as_mut()
        .unwrap()
        .location
        .address
        .source_version = vec![9];
    assert!(presenter.add_bookmark(
        PaneId::Left,
        "ignored duplicate label".into(),
        None,
        &|_| true
    ));
    assert_eq!(presenter.bookmarks().len(), 1);
    assert_eq!(presenter.bookmarks()[0].id, saved.id);
    assert_eq!(
        presenter.bookmarks()[0]
            .archive
            .as_ref()
            .unwrap()
            .source_version,
        [9]
    );

    let old = presenter.panes[1].generation;
    assert!(!presenter.open_bookmark(PaneId::Right, saved.id, &|_| false));
    assert!(presenter.panes[1].archive.is_none());
    assert_eq!(presenter.panes[1].generation, old);

    let commands = RefCell::new(Vec::new());
    assert!(
        presenter.open_bookmark(PaneId::Right, saved.id, &|command| {
            commands.borrow_mut().push(command);
            true
        })
    );
    assert!(commands.borrow().iter().any(|command| matches!(command,
        Command::ArchiveNavigate { pane: PaneId::Right, action: ArchiveAction::List(location), .. }
            if location.address == *presenter.bookmarks()[0].archive.as_ref().unwrap()
                && location.mount.is_none())));
    assert!(presenter.panes[1].archive_request);
    assert!(!presenter.panes[1].native_capabilities());
    assert!(presenter.panes[1].entries.is_empty());
    assert_eq!(
        presenter.panes[0].generation, 0,
        "opening the other pane preserves this pane"
    );
}

#[test]
fn hot_path_menu_labels_include_slots_one_through_zero_and_actual_keys() {
    let left = test_root().join("Projects 文档");
    let right = test_root().join("Archive");
    let mut p = ready(left, right);
    assert!(p.assign_bookmark_slot(PaneId::Left, 1, &|_| true));
    assert!(p.assign_bookmark_slot(PaneId::Right, 10, &|_| true));
    let rows = crate::panel_models::bookmark_choices(&p);
    assert!(rows[0].label.starts_with("1  "));
    assert!(rows[1].label.starts_with("0  "));
    assert!(rows[0].label.contains("Projects 文档"));
    assert!(rows.iter().all(|row| row.tooltip.contains("Hot Path")));
}

fn ready(left: PathBuf, right: PathBuf) -> Presenter {
    let mut p = Presenter::new(left, right);
    p.apply(Event::PreferencesLoaded(Ok(Box::default())));
    p
}

#[test]
fn hot_paths_use_the_origin_pane_and_latest_options_save_keeps_bookmarks() {
    let left = test_root().join("文档 e\u{301} 🧪");
    let right = test_root().join("right");
    let mut p = ready(left.clone(), right.clone());
    let saved = RefCell::new(Vec::new());
    let send = |command| {
        saved.borrow_mut().push(command);
        true
    };
    assert!(p.assign_bookmark_slot(PaneId::Left, 10, &send));
    assert!(p.assign_bookmark_slot(PaneId::Right, 1, &send));
    p.change_preferences(2, 1, -1, &send);
    let sent = saved.borrow();
    let Command::SavePreferences {
        preferences,
        revision,
    } = sent.last().unwrap()
    else {
        panic!("expected settings save")
    };
    assert_eq!(*revision, PreferencesRevision(3));
    assert_eq!(preferences.theme, PreferenceTheme::Light);
    assert_eq!(preferences.bookmarks.len(), 2);
    assert_eq!(preferences.bookmarks[0].path, left);
    assert_eq!(preferences.bookmarks[1].path, right);
    drop(sent);
    assert!(p.jump_bookmark(PaneId::Right, 10, &|command| {
        match command {
            Command::Navigate { pane, path, .. } => {
                assert_eq!(pane, PaneId::Right);
                assert_eq!(path, left);
            }
            Command::SavePreferences { preferences, .. } => {
                assert_eq!(
                    preferences.tabs[1].paths.as_slice(),
                    std::slice::from_ref(&left)
                );
                assert_eq!(preferences.bookmarks.len(), 2);
            }
            _ => panic!("expected navigation or its persisted tab location"),
        }
        true
    }));
    assert_eq!(p.panes[0].display_path(), left);
    assert_eq!(p.panes[1].display_path(), left);
}

#[test]
fn reassignment_preserves_both_paths_and_removal_never_reuses_callback_id() {
    let base = test_root();
    let mut p = ready(base.clone(), base.join("other"));
    assert!(p.assign_bookmark_slot(PaneId::Left, 1, &|_| true));
    let id = p.bookmarks()[0].id;
    assert!(p.assign_bookmark_slot(PaneId::Right, 1, &|_| true));
    assert_eq!(p.bookmarks().len(), 2);
    assert_eq!(p.bookmarks()[0].id, id);
    assert_eq!(p.bookmarks()[0].path, base);
    assert_eq!(p.bookmarks()[0].slot, None);
    assert_eq!(p.bookmarks()[1].path, base.join("other"));
    assert_eq!(p.bookmarks()[1].slot, Some(1));
    assert!(p.remove_bookmark(id, &|_| true));
    assert!(p.add_bookmark(PaneId::Left, "new".into(), None, &|_| true));
    assert_ne!(p.bookmarks()[0].id, id);
    assert!(!p.open_bookmark(PaneId::Left, id, &|_| panic!("stale identity")));
}

#[test]
fn adding_the_same_path_twice_keeps_identity_name_and_first_free_slot() {
    let mut p = ready(test_root(), test_root().join("other"));
    assert!(p.add_bookmark(PaneId::Left, "Original".into(), None, &|_| true));
    let first = p.bookmarks()[0].clone();
    assert!(
        p.add_bookmark(PaneId::Left, "Duplicate".into(), None, &|_| panic!(
            "no redundant save"
        ))
    );
    assert_eq!(p.bookmarks(), [first]);
    assert_eq!(p.bookmarks()[0].slot, Some(1));
    assert!(p.add_bookmark(PaneId::Right, "Other".into(), None, &|_| true));
    assert_eq!(p.bookmarks()[1].slot, Some(2));
}

#[test]
fn moving_an_existing_hot_path_to_an_occupied_slot_keeps_both_paths() {
    let mut p = ready(test_root(), test_root().join("other"));
    assert!(p.assign_bookmark_slot(PaneId::Left, 1, &|_| true));
    assert!(p.assign_bookmark_slot(PaneId::Right, 2, &|_| true));
    let first = p.bookmarks()[0].clone();
    let second = p.bookmarks()[1].clone();
    assert!(p.assign_bookmark_slot(PaneId::Left, 2, &|_| true));
    assert_eq!(p.bookmarks().len(), 2);
    assert_eq!(p.bookmarks()[0].id, first.id);
    assert_eq!(p.bookmarks()[0].path, first.path);
    assert_eq!(p.bookmarks()[0].slot, Some(2));
    assert_eq!(p.bookmarks()[1].id, second.id);
    assert_eq!(p.bookmarks()[1].path, second.path);
    assert_eq!(p.bookmarks()[1].slot, Some(1));
}

#[test]
fn new_hot_paths_fill_free_slots_and_keep_overflow_accessible() {
    let mut p = ready(test_root(), test_root());
    for index in 1..=11 {
        p.panes[0].path = test_root().join(format!("folder-{index}"));
        assert!(p.add_bookmark(PaneId::Left, format!("Folder {index}"), None, &|_| true));
    }
    assert_eq!(p.bookmarks().len(), 11);
    assert_eq!(p.bookmarks()[9].slot, Some(10));
    assert_eq!(p.bookmarks()[10].slot, None);
    assert!(p.remove_bookmark(p.bookmarks()[3].id, &|_| true));
    p.panes[0].path = test_root().join("replacement");
    assert!(p.add_bookmark(PaneId::Left, "Replacement".into(), None, &|_| true));
    assert_eq!(p.bookmarks().last().unwrap().slot, Some(4));
}

#[test]
fn hot_path_identity_does_not_fold_case_or_unicode() {
    let first = test_root().join("Folder e\u{301}");
    for second in [
        test_root().join("folder e\u{301}"),
        test_root().join("Folder é"),
    ] {
        let mut p = ready(first.clone(), second.clone());
        assert!(p.add_bookmark(PaneId::Left, "First".into(), None, &|_| true));
        assert!(p.add_bookmark(PaneId::Right, "Second".into(), None, &|_| true));
        assert_eq!(p.bookmarks()[0].path, first);
        assert_eq!(p.bookmarks()[1].path, second);
    }
}

#[test]
fn component_equivalent_hot_paths_keep_the_first_raw_spelling() {
    let original = test_root().join("folder");
    let equivalent = original.join(".").join("");
    let mut p = ready(original.clone(), equivalent);
    assert!(p.add_bookmark(PaneId::Left, "Original".into(), None, &|_| true));
    assert!(
        p.add_bookmark(PaneId::Right, "Again".into(), None, &|_| panic!(
            "same path"
        ))
    );
    assert_eq!(p.bookmarks().len(), 1);
    assert_eq!(p.bookmarks()[0].path.as_os_str(), original.as_os_str());
}

#[test]
fn manager_slot_assignment_uses_selected_identity_and_rejects_stale_ids() {
    let left = test_root().join("left");
    let right = test_root().join("right");
    let mut p = ready(left.clone(), right.clone());
    assert!(p.add_bookmark(PaneId::Left, "Left".into(), None, &|_| true));
    assert!(p.add_bookmark(PaneId::Right, "Right".into(), None, &|_| true));
    let selected = p.bookmarks()[1].id;
    p.active = PaneId::Left;
    assert!(p.set_bookmark_slot(selected, Some(10), &|command| {
        let Command::SavePreferences { preferences, .. } = command else {
            panic!("no navigation")
        };
        assert_eq!(preferences.bookmarks[0].path, left);
        assert_eq!(preferences.bookmarks[0].slot, Some(1));
        assert_eq!(preferences.bookmarks[1].path, right);
        assert_eq!(preferences.bookmarks[1].slot, Some(10));
        true
    }));
    assert!(p.set_bookmark_slot(selected, None, &|_| true));
    assert_eq!(p.bookmarks()[1].slot, None);
    assert!(p.remove_bookmark(selected, &|_| true));
    assert!(!p.set_bookmark_slot(selected, Some(3), &|_| panic!("stale identity")));
}

#[test]
fn startup_duplicate_merge_preserves_local_callback_and_other_preferences() {
    let path = test_root().join("same");
    let mut p = Presenter::new(path.clone(), test_root());
    assert!(
        p.add_bookmark(PaneId::Left, "Local".into(), Some(2), &|_| panic!(
            "not loaded"
        ))
    );
    let local = p.bookmarks()[0].clone();
    p.apply_with_sender(
        Event::PreferencesLoaded(Ok(Box::new(UserPreferences {
            theme: PreferenceTheme::Light,
            bookmarks: vec![Bookmark {
                archive: None,
                id: BookmarkId(40),
                name: "Stored".into(),
                path,
                slot: Some(5),
            }],
            ..Default::default()
        }))),
        &|command| {
            let Command::SavePreferences { preferences, .. } = command else {
                panic!()
            };
            assert_eq!(preferences.theme, PreferenceTheme::Light);
            assert_eq!(preferences.bookmarks, std::slice::from_ref(&local));
            true
        },
    );
    assert_eq!(p.bookmarks(), [local]);
}

#[test]
fn rejected_save_keeps_bookmark_and_retry_preserves_latest_global_choices() {
    let mut p = ready(test_root(), test_root());
    assert!(p.add_bookmark(PaneId::Left, "字🧪".into(), Some(9), &|_| false));
    let bookmark = p.bookmarks()[0].clone();
    assert!(p.preferences.unsaved);
    p.change_preferences(1, 1, -1, &|command| {
        let Command::SavePreferences { preferences, .. } = command else {
            panic!()
        };
        assert_eq!(preferences.theme, PreferenceTheme::Dark);
        assert_eq!(preferences.density, UiDensity::Touch);
        assert_eq!(
            preferences.bookmarks.as_slice(),
            std::slice::from_ref(&bookmark)
        );
        true
    });
    p.apply(Event::PreferencesLoaded(Ok(Box::default())));
    assert_eq!(p.bookmarks(), [bookmark]);
    assert_eq!(p.preferences.values.theme, PreferenceTheme::Dark);
}

#[test]
fn invalid_bookmarks_and_unknown_slots_do_not_dispatch() {
    let mut p = ready(test_root(), test_root());
    for name in ["", " \t ", "bad\nname"] {
        assert!(!p.add_bookmark(PaneId::Left, name.into(), None, &|_| panic!()));
    }
    assert!(!p.add_bookmark(PaneId::Left, "valid".into(), Some(0), &|_| panic!()));
    assert!(!p.assign_bookmark_slot(PaneId::Left, 11, &|_| panic!()));
    assert!(!p.jump_bookmark(PaneId::Right, 1, &|_| panic!()));
    assert!(p.bookmarks().is_empty());
}

#[test]
fn removed_loaded_identity_is_never_reused_by_a_later_bookmark() {
    let mut p = Presenter::new(test_root(), test_root());
    p.apply(Event::PreferencesLoaded(Ok(Box::new(UserPreferences {
        bookmarks: vec![Bookmark {
            archive: None,
            id: BookmarkId(500),
            name: "Loaded".into(),
            path: test_root(),
            slot: None,
        }],
        ..Default::default()
    }))));
    assert!(p.remove_bookmark(BookmarkId(500), &|_| true));
    assert!(p.add_bookmark(PaneId::Left, "New".into(), None, &|_| true));
    assert_eq!(p.bookmarks()[0].id, BookmarkId(501));
}

#[test]
fn automatic_hot_path_label_never_caps_or_rewrites_the_operation_path() {
    let path = test_root().join("字".repeat(40_000));
    let mut p = ready(path.clone(), test_root());
    assert!(p.assign_bookmark_slot(PaneId::Left, 10, &|_| true));
    assert_eq!(p.bookmarks()[0].name, "Hot path 0");
    assert_eq!(p.bookmarks()[0].path, path);
}
