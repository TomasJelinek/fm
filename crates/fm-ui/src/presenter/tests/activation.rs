use super::*;

fn populated(kind: EntryKind) -> (Presenter, PathBuf) {
    let base = test_root().join("文档 ' & $() 👩🏽‍💻");
    let target = base.join("Café.txt");
    let mut presenter = Presenter::new(base.clone(), test_root());
    let navigation = begin(&mut presenter, PaneId::Left, base.to_str().unwrap());
    finish(
        &mut presenter,
        PaneId::Left,
        navigation,
        vec![Entry::new(EntryId(7), target.clone(), kind)],
    );
    (presenter, target)
}

#[test]
fn file_activation_freezes_raw_path_and_origin_without_navigating() {
    let (mut presenter, target) = populated(EntryKind::File);
    let sent = RefCell::new(vec![]);
    assert!(presenter.open(PaneId::Left, 1, &|command| {
        sent.borrow_mut().push(command);
        true
    }));
    assert!(matches!(&sent.borrow()[0], Command::OpenEntry {
            pane: PaneId::Left, navigation: NavigationId(1), path, ..
        } if path == &target));
    assert!(!presenter.panes[0].loading);
    assert_eq!(presenter.panes[0].display_path(), target.parent().unwrap());
}

#[test]
fn symlink_activation_is_classified_by_backend_not_assumed_directory() {
    let (mut presenter, target) = populated(EntryKind::Symlink);
    assert!(presenter.open(PaneId::Left, 1, &|command| {
        assert!(matches!(command, Command::OpenEntry { path, .. } if path == target));
        true
    }));
    assert!(!presenter.panes[0].loading);
}

#[test]
fn failed_file_dispatch_keeps_cursor_and_reports_busy_queue() {
    let (mut presenter, _) = populated(EntryKind::File);
    select_row(&mut presenter, PaneId::Left, 1, false);
    let cursor = presenter.panes[0].cursor;
    assert!(!presenter.open(PaneId::Left, 1, &|_| false));
    assert_eq!(presenter.panes[0].cursor, cursor);
    assert!(!presenter.panes[0].loading);
    assert!(presenter.desktop.notice.is_some());
}

#[test]
fn special_files_are_not_sent_to_system_associations() {
    let (mut presenter, _) = populated(EntryKind::Other);
    assert!(!presenter.open(PaneId::Left, 1, &|_| panic!("special file must not launch")));
    assert!(presenter.desktop.notice.is_some());
}

fn activate(presenter: &mut Presenter) -> (DesktopRequestId, NavigationId, PathBuf) {
    let sent = RefCell::new(None);
    assert!(presenter.open(PaneId::Left, 1, &|command| {
        *sent.borrow_mut() = Some(command);
        true
    }));
    match sent.into_inner().unwrap() {
        Command::OpenEntry {
            request,
            navigation,
            path,
            ..
        } => (request, navigation, path),
        _ => panic!("expected a backend open request"),
    }
}

#[test]
fn file_success_and_failure_are_reported_once_without_navigation() {
    let (mut presenter, _) = populated(EntryKind::File);
    let (request, _, _) = activate(&mut presenter);
    presenter.apply_with_sender(
        Event::DesktopActionFinished {
            request,
            result: Ok(DesktopActionResult::FileOpened),
        },
        &|_| panic!("opening a file must not navigate"),
    );
    assert_eq!(presenter.desktop.status, "File opened");
    presenter.apply_with_sender(
        Event::DesktopActionFinished {
            request,
            result: Err(FsError::from_io(
                "open",
                &test_root(),
                io::Error::other("duplicate"),
            )),
        },
        &|_| panic!(),
    );
    assert!(
        presenter.desktop.notice.is_none(),
        "duplicate result must be ignored"
    );
    let (request, _, _) = activate(&mut presenter);
    presenter.apply(Event::DesktopActionFinished {
        request,
        result: Err(FsError::from_io(
            "open",
            &test_root(),
            io::Error::other("association failed"),
        )),
    });
    assert!(
        presenter
            .desktop
            .notice
            .as_ref()
            .unwrap()
            .1
            .contains("association failed")
    );
}

#[test]
fn linked_directory_result_navigates_origin_without_stealing_active_pane() {
    let (mut presenter, _) = populated(EntryKind::Symlink);
    let (request, navigation, path) = activate(&mut presenter);
    presenter.active = PaneId::Right;
    let sent = RefCell::new(vec![]);
    presenter.apply_with_sender(
        Event::DesktopActionFinished {
            request,
            result: Ok(DesktopActionResult::OpenDirectory {
                pane: PaneId::Left,
                navigation,
                path: path.clone(),
            }),
        },
        &|command| {
            sent.borrow_mut().push(command);
            true
        },
    );
    assert_eq!(presenter.active, PaneId::Right);
    assert_eq!(sent.borrow().len(), 1);
    assert!(
        matches!(&sent.borrow()[0], Command::Navigate { pane: PaneId::Left, path: actual, .. }
        if actual == &path)
    );
}

#[test]
fn delayed_linked_directory_cannot_override_newer_navigation_or_refresh() {
    for same_location in [false, true] {
        let (mut presenter, _) = populated(EntryKind::Symlink);
        let (request, navigation, path) = activate(&mut presenter);
        let next = if same_location {
            path.parent().unwrap().to_owned()
        } else {
            test_root()
        };
        begin(&mut presenter, PaneId::Left, next.to_str().unwrap());
        presenter.apply_with_sender(
            Event::DesktopActionFinished {
                request,
                result: Ok(DesktopActionResult::OpenDirectory {
                    pane: PaneId::Left,
                    navigation,
                    path,
                }),
            },
            &|_| panic!("stale linked directory result must not send navigation"),
        );
    }
}

#[test]
fn linked_directory_requires_matching_request_pane_generation_and_raw_path() {
    for mismatch in 0..3 {
        let (mut presenter, _) = populated(EntryKind::Symlink);
        let (request, navigation, path) = activate(&mut presenter);
        presenter.apply_with_sender(
            Event::DesktopActionFinished {
                request,
                result: Ok(DesktopActionResult::OpenDirectory {
                    pane: if mismatch == 0 {
                        PaneId::Right
                    } else {
                        PaneId::Left
                    },
                    navigation: if mismatch == 1 {
                        NavigationId(999)
                    } else {
                        navigation
                    },
                    path: if mismatch == 2 { test_root() } else { path },
                }),
            },
            &|_| panic!("mismatched result must not navigate"),
        );
    }
}

#[test]
fn directory_followup_queue_failure_preserves_current_location() {
    let (mut presenter, _) = populated(EntryKind::Symlink);
    let original = presenter.panes[0].display_path().to_owned();
    let (request, navigation, path) = activate(&mut presenter);
    presenter.apply_with_sender(
        Event::DesktopActionFinished {
            request,
            result: Ok(DesktopActionResult::OpenDirectory {
                pane: PaneId::Left,
                navigation,
                path,
            }),
        },
        &|_| false,
    );
    assert_eq!(presenter.panes[0].display_path(), original);
    assert!(!presenter.panes[0].loading);
    assert!(presenter.panes[0].error.contains("queue"));
}

#[test]
fn file_activation_has_no_ui_path_length_limit() {
    let (mut presenter, _) = populated(EntryKind::File);
    let path = test_root().join("文档-".repeat(9000));
    let navigation = begin(&mut presenter, PaneId::Left, test_root().to_str().unwrap());
    finish(
        &mut presenter,
        PaneId::Left,
        navigation,
        vec![Entry::new(EntryId(7), path.clone(), EntryKind::File)],
    );
    assert!(presenter.open(PaneId::Left, 0, &|command| {
        assert!(matches!(command, Command::OpenEntry { path: actual, .. } if actual == path));
        true
    }));
}
