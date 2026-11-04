use super::*;

fn ready(left: PathBuf, right: PathBuf) -> Presenter {
    let mut p = Presenter::new(left, right);
    p.apply(Event::PreferencesLoaded(Ok(UserPreferences::default())));
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
        assert!(matches!(command, Command::Navigate {
            pane: PaneId::Right, ref path, ..
        } if path == &left));
        true
    }));
    assert_eq!(p.panes[0].display_path(), left);
    assert_eq!(p.panes[1].display_path(), left);
}

#[test]
fn reassignment_preserves_slot_identity_and_removal_never_reuses_callback_id() {
    let base = test_root();
    let mut p = ready(base.clone(), base.join("other"));
    assert!(p.assign_bookmark_slot(PaneId::Left, 1, &|_| true));
    let id = p.bookmarks()[0].id;
    assert!(p.assign_bookmark_slot(PaneId::Right, 1, &|_| true));
    assert_eq!(p.bookmarks().len(), 1);
    assert_eq!(p.bookmarks()[0].id, id);
    assert_eq!(p.bookmarks()[0].path, base.join("other"));
    assert!(p.remove_bookmark(id, &|_| true));
    assert!(p.add_bookmark(PaneId::Left, "new".into(), None, &|_| true));
    assert_ne!(p.bookmarks()[0].id, id);
    assert!(!p.open_bookmark(PaneId::Left, id, &|_| panic!("stale identity")));
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
    p.apply(Event::PreferencesLoaded(Ok(UserPreferences::default())));
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
    p.apply(Event::PreferencesLoaded(Ok(UserPreferences {
        bookmarks: vec![Bookmark {
            id: BookmarkId(500),
            name: "Loaded".into(),
            path: test_root(),
            slot: None,
        }],
        ..Default::default()
    })));
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
