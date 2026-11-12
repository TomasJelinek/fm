use super::*;

#[test]
fn authorization_requires_backend_offer_and_explicit_action_then_sends_only_receipt_ids() {
    let mut p = Presenter::new(test_root(), "right".into());
    let destination = test_root().join("denied");
    assert!(p.navigate(PaneId::Left, destination.clone(), None, &|_| true));
    let navigation = NavigationId(p.panes[0].generation);
    p.apply(Event::ListingFailed {
        pane: PaneId::Left,
        navigation,
        error: FsError::from_io("list", &destination, io::ErrorKind::PermissionDenied.into()),
    });
    assert!(!p.can_retry_directory_elevated(PaneId::Left));
    p.apply(Event::DirectoryElevationAvailable {
        pane: PaneId::Left,
        navigation,
        path: destination,
    });
    assert!(p.can_retry_directory_elevated(PaneId::Left));
    let sent = RefCell::new(Vec::new());
    p.retry_directory_elevated(PaneId::Left, &|command| {
        sent.borrow_mut().push(command);
        true
    });
    assert!(!p.can_retry_directory_elevated(PaneId::Left));
    assert!(sent.borrow().iter().any(|command| matches!(command,
        Command::RetryDirectoryWithElevation { pane: PaneId::Left, denied_navigation, navigation: next }
            if *denied_navigation == navigation && *next > navigation)));
    p.retry_directory_elevated(PaneId::Left, &|_| panic!("receipt already used"));
}

#[test]
fn superseded_authorization_offer_cannot_retarget_current_navigation() {
    let mut p = Presenter::new(test_root(), "right".into());
    let old = test_root().join("old");
    assert!(p.navigate(PaneId::Left, old.clone(), None, &|_| true));
    let navigation = NavigationId(p.panes[0].generation);
    assert!(p.navigate(PaneId::Left, test_root().join("new"), None, &|_| true));
    p.apply(Event::DirectoryElevationAvailable {
        pane: PaneId::Left,
        navigation,
        path: old,
    });
    assert!(!p.can_retry_directory_elevated(PaneId::Left));
    p.retry_directory_elevated(PaneId::Left, &|_| panic!("stale offer cannot send"));
}

#[test]
fn denied_folder_navigation_opens_a_brief_modal_without_prompting() {
    let mut p = Presenter::new(test_root(), "right".into());
    let destination = test_root().join("denied");
    assert!(p.navigate(PaneId::Left, destination.clone(), None, &|_| true));
    let navigation = NavigationId(p.panes[0].generation);
    p.apply(Event::ListingFailed {
        pane: PaneId::Left,
        navigation,
        error: FsError::from_io("list", &destination, io::ErrorKind::PermissionDenied.into()),
    });
    let (title, body) = p
        .desktop
        .notice
        .as_ref()
        .expect("explicit folder failure needs a message box");
    assert_eq!(title, "Cannot open folder");
    assert_eq!(body, "Access to this folder was denied.");
    assert!(!p.can_retry_directory_elevated(PaneId::Left));
}

fn denied_notice() -> (Presenter, NavigationId, PathBuf) {
    let root = test_root();
    let destination = root.join("denied");
    let mut p = Presenter::new(root.clone(), root.clone());
    let first = begin(&mut p, PaneId::Left, root.to_str().unwrap());
    finish(
        &mut p,
        PaneId::Left,
        first,
        vec![Entry::new(
            EntryId(1),
            destination.clone(),
            EntryKind::Directory,
        )],
    );
    assert!(p.open(PaneId::Left, p.panes[0].parent_offset(), &|_| true));
    let navigation = NavigationId(p.panes[0].generation);
    p.apply(Event::ListingFailed {
        pane: PaneId::Left,
        navigation,
        error: FsError::from_io("list", &destination, io::ErrorKind::PermissionDenied.into()),
    });
    (p, navigation, destination)
}

fn offer(p: &mut Presenter, navigation: NavigationId, path: &Path) {
    p.apply(Event::DirectoryElevationAvailable {
        pane: PaneId::Left,
        navigation,
        path: path.to_owned(),
    });
}

#[test]
fn modal_authorization_is_explicit_bound_to_the_denied_destination_and_single_use() {
    let (mut p, navigation, path) = denied_notice();
    let generation = p.desktop.notice_generation;
    assert!(
        p.panel_error_text(PaneId::Left).is_empty(),
        "do not duplicate the modal in a banner"
    );
    assert!(!p.notice_can_authorize());
    p.retry_notice_authorized(generation, &|_| {
        panic!("a missing backend offer cannot prompt")
    });
    offer(&mut p, navigation, &path);
    assert_eq!(p.desktop.notice_generation, generation);
    assert!(p.notice_can_authorize());
    let commands = RefCell::new(Vec::new());
    p.retry_notice_authorized(generation, &|command| {
        commands.borrow_mut().push(command);
        true
    });
    assert_eq!(
        commands
            .borrow()
            .iter()
            .filter(|command| matches!(command,
        Command::RetryDirectoryWithElevation { pane: PaneId::Left, denied_navigation, .. }
            if *denied_navigation == navigation))
            .count(),
        1
    );
    assert!(!p.notice_can_authorize());
    p.retry_notice_authorized(generation, &|_| panic!("used receipt"));
}

#[test]
fn closing_or_replacing_the_error_fences_late_modal_authorization() {
    for replacement in 0..4 {
        let (mut p, navigation, path) = denied_notice();
        let generation = p.desktop.notice_generation;
        match replacement {
            0 => p.dismiss_diagnostic_notice(generation),
            1 => p
                .desktop
                .report("Another message", "Different context".into()),
            2 => {
                p.panes[0].set_error("New error", &mut p.retired);
            }
            _ => {
                p.navigate(PaneId::Left, test_root().join("other"), None, &|_| true);
            }
        }
        offer(&mut p, navigation, &path);
        assert!(!p.notice_can_authorize());
        assert!(!p.can_retry_directory_elevated(PaneId::Left));
        p.retry_directory_elevated(PaneId::Left, &|_| panic!("stale panel must not prompt"));
        p.retry_notice_authorized(generation, &|_| panic!("stale dialog must not prompt"));
    }
    let (mut p, navigation, path) = denied_notice();
    offer(&mut p, navigation, &path.with_file_name("different"));
    assert!(
        !p.notice_can_authorize(),
        "a different destination cannot reuse this dialog"
    );
    assert!(!p.can_retry_directory_elevated(PaneId::Left));
}

#[test]
fn cancelled_or_denied_authorization_preserves_listing_and_reports_without_retrying() {
    for (kind, reason) in [
        (io::ErrorKind::Interrupted, "Authorization was canceled."),
        (io::ErrorKind::PermissionDenied, "Authorization was denied."),
    ] {
        let (mut p, navigation, path) = denied_notice();
        let entries = p.panes[0].entries.clone();
        let original = p.panes[0].path.clone();
        offer(&mut p, navigation, &path);
        p.retry_notice_authorized(p.desktop.notice_generation, &|_| true);
        let retry = NavigationId(p.panes[0].generation);
        p.apply(Event::ListingFailed {
            pane: PaneId::Left,
            navigation: retry,
            error: FsError::from_io("authorized listing", &path, io::Error::new(kind, reason)),
        });
        assert_eq!(p.panes[0].path, original);
        assert!(Arc::ptr_eq(&p.panes[0].entries, &entries));
        assert!(p.desktop.notice.is_some());
        assert!(p.desktop.notice_copy.as_ref().unwrap().contains(reason));
        assert!(
            !p.desktop
                .notice_details
                .contains(path.to_string_lossy().as_ref())
        );
        assert!(!p.notice_can_authorize());
        p.retry_notice_authorized(p.desktop.notice_generation, &|_| {
            panic!("no implicit repeated prompt")
        });
    }
}

#[test]
fn refresh_errors_keep_the_inline_notice_and_do_not_open_a_message_box() {
    let root = test_root();
    let mut p = Presenter::new(root.clone(), root.clone());
    let first = begin(&mut p, PaneId::Left, root.to_str().unwrap());
    finish(&mut p, PaneId::Left, first, vec![]);
    let refresh = begin(&mut p, PaneId::Left, root.to_str().unwrap());
    p.apply(Event::ListingFailed {
        pane: PaneId::Left,
        navigation: refresh,
        error: FsError::from_io("refresh", &root, io::ErrorKind::PermissionDenied.into()),
    });
    assert!(p.desktop.notice.is_none());
    assert!(!p.panel_error_text(PaneId::Left).is_empty());
}

#[test]
fn a_busy_authorization_queue_reports_failure_without_consuming_a_receipt() {
    let (mut p, navigation, path) = denied_notice();
    offer(&mut p, navigation, &path);
    p.retry_notice_authorized(p.desktop.notice_generation, &|_| false);
    assert!(!p.panes[0].loading);
    assert_eq!(p.panes[0].generation, navigation.0);
    assert!(
        p.desktop
            .notice
            .as_ref()
            .unwrap()
            .1
            .contains("queue is busy")
    );
    assert!(!p.notice_can_authorize());
}
