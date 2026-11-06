use super::*;

#[test]
fn stale_results_cannot_replace_a_newer_location_or_other_pane() {
    let mut p = Presenter::new("left".into(), "right".into());
    let old = begin(&mut p, PaneId::Left, "old");
    let new = begin(&mut p, PaneId::Left, "new");
    finish(&mut p, PaneId::Left, new, vec![entry(2, "new")]);
    finish(&mut p, PaneId::Left, old, vec![entry(1, "old")]);
    assert_eq!(p.panes[0].path, PathBuf::from("new"));
    assert_eq!(p.panes[0].entries[0].id, EntryId(2));
    assert_eq!(p.panes[1].path, PathBuf::from("right"));
    assert!(p.panes[1].entries.is_empty());
}
#[test]
fn failed_navigation_retains_last_listing_and_successful_history() {
    let mut p = Presenter::new("a".into(), "right".into());
    let first = begin(&mut p, PaneId::Left, "a");
    finish(&mut p, PaneId::Left, first, vec![entry(1, "usable")]);
    let next = begin(&mut p, PaneId::Left, "missing");
    p.apply(Event::ListingStarted {
        pane: PaneId::Left,
        navigation: next,
        path: "missing".into(),
    });
    p.apply(Event::EntriesBatch {
        pane: PaneId::Left,
        navigation: next,
        entries: vec![entry(2, "partial")],
    });
    p.apply(Event::ListingFailed {
        pane: PaneId::Left,
        navigation: next,
        error: FsError::from_io(
            "list",
            &PathBuf::from("missing"),
            io::Error::from(io::ErrorKind::PermissionDenied),
        ),
    });
    assert_eq!(p.panes[0].path, PathBuf::from("a"));
    assert_eq!(p.panes[0].entries[0].id, EntryId(1));
    assert_eq!(p.panes[0].history, [PathBuf::from("a")]);
    assert!(p.panes[0].error.contains("missing"));
}
#[test]
fn back_then_new_location_discards_forward_branch_only_after_success() {
    let mut p = Presenter::new("a".into(), "right".into());
    for path in ["a", "b", "c"] {
        let id = begin(&mut p, PaneId::Left, path);
        finish(&mut p, PaneId::Left, id, vec![]);
    }
    let sent = RefCell::new(None);
    assert!(p.history(PaneId::Left, -1, &|c| {
        *sent.borrow_mut() = Some(c);
        true
    }));
    let id = match sent.into_inner().unwrap() {
        Command::Navigate {
            navigation, path, ..
        } => {
            assert_eq!(path, PathBuf::from("b"));
            navigation
        }
        _ => panic!(),
    };
    assert_eq!(p.panes[0].history_index, Some(2));
    finish(&mut p, PaneId::Left, id, vec![]);
    let id = begin(&mut p, PaneId::Left, "d");
    finish(&mut p, PaneId::Left, id, vec![]);
    assert_eq!(
        p.panes[0].history,
        [PathBuf::from("a"), PathBuf::from("b"), PathBuf::from("d")]
    );
    assert!(!p.history(PaneId::Left, 1, &|_| true));
}
#[test]
fn rejected_command_keeps_previous_request_and_root_up_is_noop() {
    let mut p = Presenter::new(test_root(), "right".into());
    let first = begin(&mut p, PaneId::Left, "a");
    assert!(!p.navigate(PaneId::Left, "rejected".into(), None, &|_| false));
    finish(&mut p, PaneId::Left, first, vec![entry(1, "a")]);
    assert_eq!(p.panes[0].path, PathBuf::from("a"));
    p.panes[0].path = test_root();
    assert!(!p.up(PaneId::Left, &|_| panic!("root must not navigate")));
}
#[test]
fn rejected_sort_does_not_change_the_accepted_sort_order() {
    let mut p = Presenter::new("a".into(), "right".into());
    assert!(!p.sort(PaneId::Left, SortColumn::Size, &|_| false));
    assert_eq!(p.panes[0].sort, SortSpec::default());
    assert!(p.sort(PaneId::Left, SortColumn::Size, &|c| {
        assert!(matches!(
            c,
            Command::Resort {
                sort: SortSpec {
                    column: SortColumn::Size,
                    descending: false
                },
                ..
            }
        ));
        true
    }));
    assert!(p.sort(PaneId::Left, SortColumn::Size, &|c| {
        assert!(matches!(
            c,
            Command::Resort {
                sort: SortSpec {
                    column: SortColumn::Size,
                    descending: true
                },
                ..
            }
        ));
        true
    }));
}
#[test]
fn unicode_path_editing_preserves_scripts_emoji_and_combining_sequences() {
    let path = test_root().join("Žluťoučký-東京-العربية-עברית-👩🏽‍💻-Cafe\u{301}");
    let child = "資料-קובץ-ملف-📁-e\u{301}";
    let expected_child = path.join(child);
    let mut p = Presenter::new(path.clone(), "right".into());
    assert!(
        p.navigate_text(PaneId::Left, path.to_str().unwrap(), &|command| {
            assert!(matches!(command, Command::Navigate { path: actual, .. } if actual == path));
            true
        })
    );
    assert!(!p.navigate_text(PaneId::Left, child, &|command| {
        assert!(
            matches!(command, Command::Navigate { path: actual, .. } if actual == expected_child)
        );
        false
    }));
    finish(&mut p, PaneId::Left, NavigationId(1), vec![]);
    assert_eq!(p.panes[0].display_path(), path);
}
#[test]
fn space_received_before_completion_survives_location_change() {
    let mut p = Presenter::new("a".into(), "right".into());
    let first = begin(&mut p, PaneId::Left, "new");
    p.apply(Event::SpaceUpdated {
        pane: PaneId::Left,
        navigation: first,
        space: SpaceInfo {
            available: 42,
            total: 100,
        },
    });
    finish(&mut p, PaneId::Left, first, vec![]);
    assert_eq!(p.panes[0].space.unwrap().available, 42);
}
#[test]
fn each_pane_remembers_its_own_last_successful_path_on_a_volume() {
    let c = test_root().join("volume-c");
    let d = test_root().join("volume-d");
    let left = c.join("left");
    let right = c.join("right");
    let work = d.join("work");
    let mut p = Presenter::new(c.clone(), c.clone());
    p.apply(Event::VolumesUpdated(vec![
        Volume {
            id: "c".into(),
            label: "C:".into(),
            root: c,
        },
        Volume {
            id: "d".into(),
            label: "D:".into(),
            root: d,
        },
    ]));
    for (pane, path) in [
        (PaneId::Left, &left),
        (PaneId::Right, &right),
        (PaneId::Left, &work),
    ] {
        let id = begin(&mut p, pane, path.to_str().unwrap());
        finish(&mut p, pane, id, vec![]);
    }
    assert!(p.choose_volume(PaneId::Left, 0, &|c| {
        assert!(matches!(c, Command::Navigate { path, .. } if path == left));
        true
    }));
    assert!(p.choose_volume(PaneId::Right, 0, &|c| {
        assert!(matches!(c, Command::Refresh { path, .. } if path == right));
        true
    }));
}
#[test]
fn links_are_sent_to_engine_for_directory_resolution() {
    let mut p = Presenter::new("a".into(), "right".into());
    let first = begin(&mut p, PaneId::Left, "a");
    finish(
        &mut p,
        PaneId::Left,
        first,
        vec![Entry::new(
            EntryId(1),
            "junction".into(),
            EntryKind::Symlink,
        )],
    );
    assert!(p.open(PaneId::Left, 0, &|command| {
        assert!(
            matches!(command, Command::OpenEntry { path, .. } if path == Path::new("junction"))
        );
        true
    }));
}
#[test]
fn volumes_arriving_after_first_listing_remember_the_current_location() {
    let root = test_root().join("volume-c");
    let work = root.join("work");
    let mut p = Presenter::new(work.clone(), root.clone());
    let id = begin(&mut p, PaneId::Left, work.to_str().unwrap());
    finish(&mut p, PaneId::Left, id, vec![]);
    p.apply(Event::VolumesUpdated(vec![Volume {
        id: "c".into(),
        label: "C:".into(),
        root,
    }]));
    assert!(p.choose_volume(PaneId::Left, 0, &|c| {
        assert!(matches!(c, Command::Refresh { path, .. } if path == work));
        true
    }));
}
#[test]
fn late_space_from_failed_navigation_cannot_replace_previous_locations_space() {
    let mut p = Presenter::new("a".into(), "right".into());
    let id = begin(&mut p, PaneId::Left, "a");
    finish(&mut p, PaneId::Left, id, vec![]);
    p.apply(Event::SpaceUpdated {
        pane: PaneId::Left,
        navigation: id,
        space: SpaceInfo {
            available: 42,
            total: 100,
        },
    });
    let id = begin(&mut p, PaneId::Left, "failed");
    p.apply(Event::ListingFailed {
        pane: PaneId::Left,
        navigation: id,
        error: FsError::from_io(
            "list",
            &PathBuf::from("failed"),
            io::Error::from(io::ErrorKind::PermissionDenied),
        ),
    });
    p.apply(Event::SpaceUpdated {
        pane: PaneId::Left,
        navigation: id,
        space: SpaceInfo {
            available: 1,
            total: 100,
        },
    });
    assert_eq!(p.panes[0].space.unwrap().available, 42);
}
#[test]
fn typed_dot_components_and_parent_follow_a_consistent_logical_path() {
    let root = test_root();
    let users = root.join("Users");
    let alice = users.join("Alice");
    let logical_link = root.join("link/child/../../target");
    let target = root.join("target");
    let mut p = Presenter::new(alice.clone(), "right".into());
    for (text, expected) in [
        (".", &alice),
        ("..", &users),
        ("./work/../..", &root),
        (logical_link.to_str().unwrap(), &target),
    ] {
        assert!(p.navigate_text(PaneId::Left, text, &|c| {
            assert!(matches!(c, Command::Navigate { path, .. } if path == *expected));
            true
        }));
    }
    let mut p = Presenter::new(alice, "right".into());
    assert!(p.navigate_text(PaneId::Left, "..", &|_| true));
    finish(&mut p, PaneId::Left, NavigationId(1), vec![]);
    assert_eq!(p.panes[0].path, users);
    assert!(!p.up(PaneId::Left, &|c| {
        assert!(matches!(c, Command::Navigate { path, .. } if path == root));
        false
    }));
}
#[test]
fn relative_entry_sort_and_parent_use_the_requested_visible_location() {
    let root = test_root();
    let a = root.join("a");
    let b = root.join("b");
    let child = b.join("child");
    let next = child.join("next");
    let mut p = Presenter::new(a, "right".into());
    begin(&mut p, PaneId::Left, child.to_str().unwrap());
    assert!(!p.navigate_text(PaneId::Left, "next", &|c| {
        assert!(matches!(c, Command::Navigate { path, .. } if path == next));
        false
    }));
    assert!(!p.up(PaneId::Left, &|c| {
        assert!(matches!(c, Command::Navigate { path, .. } if path == b));
        false
    }));
    assert!(!p.sort(PaneId::Left, SortColumn::Size, &|c| {
        assert!(matches!(c, Command::Resort { path, .. } if path == child));
        false
    }));
}
