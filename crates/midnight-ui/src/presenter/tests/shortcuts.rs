use super::*;

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

fn focused(kind: EntryKind, name: &str) -> (Presenter, PathBuf) {
    let base = test_root().join("folder");
    let path = base.join(name);
    let mut p = Presenter::new(base.clone(), test_root());
    let navigation = begin(&mut p, PaneId::Left, base.to_str().unwrap());
    finish(
        &mut p,
        PaneId::Left,
        navigation,
        vec![Entry::new(EntryId(8), path.clone(), kind)],
    );
    select_row(&mut p, PaneId::Left, 1, false);
    (p, path)
}

#[test]
fn root_shortcut_uses_the_native_anchor_of_the_pending_location() {
    let mut p = Presenter::new(test_root().join("committed"), test_root());
    let provisional = test_root().join("pending").join("nested");
    begin(&mut p, PaneId::Left, provisional.to_str().unwrap());
    assert!(p.go_root(PaneId::Left, &|command| {
        assert!(matches!(command, Command::Navigate {
            pane: PaneId::Left, path, ..
        } if path == test_root()));
        true
    }));
    assert_eq!(p.panes[0].display_path(), test_root());
}

#[test]
fn root_shortcut_rejects_relative_paths_without_guessing_a_drive() {
    let mut p = Presenter::new("relative/nested".into(), test_root());
    assert!(!p.go_root(PaneId::Left, &|_| panic!(
        "relative roots must not navigate"
    )));
    assert_eq!(p.panes[0].display_path(), Path::new("relative/nested"));
    assert!(!p.panes[0].error.is_empty());
}

#[test]
fn root_queue_failure_preserves_the_visible_location() {
    let initial = test_root().join("unchanged");
    let mut p = Presenter::new(initial.clone(), test_root());
    assert!(!p.go_root(PaneId::Left, &|_| false));
    assert_eq!(p.panes[0].display_path(), initial);
    assert!(!p.panes[0].loading);
    assert!(p.panes[0].error.contains("queue"));
}

#[test]
fn folder_shortcut_navigates_the_focused_known_directory() {
    let (mut p, path) = focused(EntryKind::Directory, "文档-🦀");
    assert!(p.open_folder(PaneId::Left, &|command| {
        assert!(matches!(command, Command::Navigate { path: actual, .. } if actual == path));
        true
    }));
    assert_eq!(p.panes[0].display_path(), path);
}

#[test]
fn folder_shortcut_never_launches_regular_files_links_or_special_files() {
    for kind in [EntryKind::File, EntryKind::Symlink, EntryKind::Other] {
        let (mut p, _) = focused(kind, "target");
        let before = p.panes[0].display_path().to_owned();
        assert!(!p.open_folder(PaneId::Left, &|_| panic!(
            "folder-only action must not send"
        )));
        assert_eq!(p.panes[0].display_path(), before);
        assert!(!p.panes[0].loading);
    }
}

#[test]
fn folder_shortcut_opens_the_synthetic_parent() {
    let (mut p, _) = focused(EntryKind::Directory, "child");
    select_row(&mut p, PaneId::Left, 0, false);
    assert!(p.open_folder(PaneId::Left, &|command| {
        assert!(matches!(command, Command::Navigate { path, .. } if path == test_root()));
        true
    }));
}

#[test]
fn folder_queue_failure_preserves_cursor_path_and_selection() {
    let (mut p, _) = focused(EntryKind::Directory, "child");
    select_row(&mut p, PaneId::Left, 1, true);
    let before = p.panes[0].display_path().to_owned();
    assert!(!p.open_folder(PaneId::Left, &|_| false));
    assert_eq!(p.panes[0].display_path(), before);
    assert_eq!(p.panes[0].cursor, Some(EntryId(8)));
    assert!(selected_ids(&p, PaneId::Left).contains(&EntryId(8)));
    assert!(p.panes[0].error.contains("queue"));
}

#[test]
fn focused_name_uses_the_raw_basename_without_interpreting_shell_text() {
    let name = "文档 ' & $() 👩🏽‍💻.txt";
    let (p, _) = focused(EntryKind::File, name);
    assert_eq!(p.focused_name(PaneId::Left), Some(PathBuf::from(name)));
}

#[test]
fn focused_name_has_no_artificial_length_limit() {
    let name = "文档".repeat(9000);
    let (p, _) = focused(EntryKind::File, &name);
    assert_eq!(p.focused_name(PaneId::Left), Some(PathBuf::from(name)));
}

#[test]
fn focused_name_represents_parent_but_not_an_empty_root() {
    let (mut p, _) = focused(EntryKind::Directory, "child");
    select_row(&mut p, PaneId::Left, 0, false);
    assert_eq!(p.focused_name(PaneId::Left), Some(PathBuf::from("..")));
    assert_eq!(p.focused_name(PaneId::Right), None);
}

#[test]
fn drive_information_requests_fresh_backend_details_even_with_cached_capacity() {
    let base = test_root().join("nested");
    let mut p = Presenter::new(base.clone(), test_root());
    p.drive_info(PaneId::Left, &|command| {
        matches!(command, Command::QueryVolumeInformation(_))
    });
    let (_, body) = p.desktop.notice.as_ref().unwrap();
    assert!(body.contains(base.to_string_lossy().as_ref()));
    assert!(body.contains("Reading native"));
    assert!(!body.contains("0 bytes"));
    p.panes[0].space = Some(SpaceInfo {
        available: 0,
        total: 1_234,
    });
    p.drive_info(PaneId::Left, &|command| {
        matches!(command, Command::QueryVolumeInformation(_))
    });
    let (_, body) = p.desktop.notice.as_ref().unwrap();
    assert!(!body.contains("0 bytes"));
    assert!(!body.contains("1234 bytes"));
    assert!(body.contains("Reading native"));
}

#[test]
fn drive_information_uses_pending_path_and_space_without_stale_capacity() {
    let mut p = Presenter::new(test_root().join("old"), test_root());
    p.panes[0].space = Some(SpaceInfo {
        available: 99,
        total: 123,
    });
    let pending = test_root().join("loading");
    let navigation = begin(&mut p, PaneId::Left, pending.to_str().unwrap());
    p.drive_info(PaneId::Left, &|command| matches!(command, Command::QueryVolumeInformation(request) if request.path == pending && request.navigation == navigation));
    let (_, body) = p.desktop.notice.as_ref().unwrap();
    assert!(body.contains(pending.to_string_lossy().as_ref()));
    assert!(body.contains("Reading native"));
    assert!(!body.contains("99 bytes"));
    p.apply(Event::SpaceUpdated {
        pane: PaneId::Left,
        navigation,
        space: SpaceInfo {
            available: 17,
            total: 42,
        },
    });
    p.drive_info(PaneId::Left, &|command| matches!(command, Command::QueryVolumeInformation(request) if request.path == pending && request.navigation == navigation));
    let (_, body) = p.desktop.notice.as_ref().unwrap();
    assert!(!body.contains("17 bytes"));
    assert!(!body.contains("42 bytes"));
    assert_eq!(p.panes[0].display_path(), pending);
}
