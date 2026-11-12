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

fn assert_denied_row_warning(p: &Presenter) {
    let pane = &p.panes[0];
    let entry = &pane.entries[0];
    assert_eq!(
        pane.root_presence_disclosure(entry),
        6,
        "the denied folder keeps its red row warning independently of the modal"
    );
    assert!(!pane.root_presence_error(entry.id).is_empty());
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
        let index = p.panes[0].index.clone();
        let original = p.panes[0].path.clone();
        let committed = p.panes[0].successful_generation;
        let cursor = p.panes[0].cursor;
        let parent_cursor = p.panes[0].parent_cursor;
        let offset = p.panes[0].parent_offset();
        let viewport = (offset, offset + 1);
        p.panes[0].set_tree_viewport(viewport);
        p.panes[0].preserve_viewport = true;
        assert_denied_row_warning(&p);
        let original_row_diagnostic = p.panes[0]
            .root_presence_diagnostic(EntryId(1))
            .cloned()
            .unwrap();
        offer(&mut p, navigation, &path);
        assert_denied_row_warning(&p);
        let commands = RefCell::new(Vec::new());
        p.retry_notice_authorized(p.desktop.notice_generation, &|command| {
            commands.borrow_mut().push(command);
            true
        });
        let retry = NavigationId(p.panes[0].generation);
        assert!(retry > navigation);
        assert!(p.panes[0].loading);
        assert_eq!(p.panes[0].successful_generation, committed);
        assert_denied_row_warning(&p);
        assert!(!p.panes[0].root_disclosure_available());
        p.flush_folder_presence(&|_| panic!("a loading retry cannot probe the old source"));
        assert_denied_row_warning(&p);
        assert!(!p.notice_can_authorize());
        assert_eq!(
            commands
                .borrow()
                .iter()
                .filter(|command| matches!(command,
                    Command::RetryDirectoryWithElevation {
                        denied_navigation, navigation: retry_navigation, ..
                    } if *denied_navigation == navigation && *retry_navigation == retry))
                .count(),
            1
        );
        p.retry_notice_authorized(p.desktop.notice_generation, &|_| {
            panic!("receipt already used")
        });
        p.apply(Event::ListingFailed {
            pane: PaneId::Left,
            navigation: retry,
            error: FsError::from_io("authorized listing", &path, io::Error::new(kind, reason)),
        });
        p.flush_folder_presence(&|_| panic!("a failed retry cannot revive stale probe authority"));
        assert_eq!(p.panes[0].path, original);
        assert!(Arc::ptr_eq(&p.panes[0].entries, &entries));
        assert!(Arc::ptr_eq(&p.panes[0].index, &index));
        assert_eq!(p.panes[0].generation, retry.0);
        assert_eq!(p.panes[0].successful_generation, committed);
        assert_denied_row_warning(&p);
        if kind == io::ErrorKind::Interrupted {
            assert_eq!(
                p.panes[0].root_presence_diagnostic(EntryId(1)),
                Some(&original_row_diagnostic),
                "authorization cancellation must retain the row's denial and diagnostic details"
            );
        }
        assert_eq!(p.panes[0].cursor, cursor);
        assert_eq!(p.panes[0].parent_cursor, parent_cursor);
        assert_eq!(p.panes[0].tree.viewport, viewport);
        assert!(p.panes[0].preserve_viewport);
        assert!(!p.panes[0].root_disclosure_available());
        assert!(p.take_warning_viewer().is_none());
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
fn matching_authorized_listing_success_retires_the_denied_row_warning() {
    let (mut p, denied, path) = denied_notice();
    let source_entries = p.panes[0].entries.clone();
    let source_index = p.panes[0].index.clone();
    let committed = p.panes[0].successful_generation;
    assert_denied_row_warning(&p);
    offer(&mut p, denied, &path);
    p.retry_notice_authorized(p.desktop.notice_generation, &|_| true);
    let retry = NavigationId(p.panes[0].generation);
    assert!(retry > denied);
    assert_denied_row_warning(&p);

    finish(
        &mut p,
        PaneId::Left,
        denied,
        vec![Entry::new(
            EntryId(1),
            path.join("stale-child"),
            EntryKind::Directory,
        )],
    );
    assert!(p.panes[0].loading, "an old success cannot settle the retry");
    assert_eq!(p.panes[0].generation, retry.0);
    assert_eq!(p.panes[0].successful_generation, committed);
    assert!(Arc::ptr_eq(&p.panes[0].entries, &source_entries));
    assert!(Arc::ptr_eq(&p.panes[0].index, &source_index));
    assert_denied_row_warning(&p);

    let streamed_path = path.join("streamed-child");
    p.apply(Event::EntriesBatch {
        pane: PaneId::Left,
        navigation: retry,
        entries: vec![Entry::new(
            EntryId(1),
            streamed_path.clone(),
            EntryKind::Directory,
        )],
    });
    let streamed = &p.panes[0].rows()[0];
    assert_eq!(streamed.path, streamed_path);
    assert_ne!(p.panes[0].root_presence_disclosure(streamed), 6);
    assert!(p.panes[0].root_presence_error(EntryId(1)).is_empty());
    assert_eq!(p.panes[0].root_presence_disclosure(&source_entries[0]), 6);

    let current_path = path.join("current-child");
    finish(
        &mut p,
        PaneId::Left,
        retry,
        vec![Entry::new(
            EntryId(1),
            current_path.clone(),
            EntryKind::Directory,
        )],
    );
    assert!(!p.panes[0].loading);
    assert_eq!(p.panes[0].path, path);
    assert_eq!(p.panes[0].generation, retry.0);
    assert_eq!(p.panes[0].successful_generation, Some(retry));
    assert_eq!(p.panes[0].entries[0].path, current_path);
    assert_eq!(
        p.panes[0].root_presence_disclosure(&p.panes[0].entries[0]),
        1,
        "a reused entry ID in the new listing must not inherit the old warning"
    );
    assert!(p.panes[0].root_presence_error(EntryId(1)).is_empty());
    assert!(p.panes[0].error.is_empty());
    p.apply(Event::ListingFailed {
        pane: PaneId::Left,
        navigation: denied,
        error: FsError::from_io("list", &path, io::ErrorKind::PermissionDenied.into()),
    });
    offer(&mut p, denied, &path);
    assert!(p.panes[0].root_presence_error(EntryId(1)).is_empty());
    assert_eq!(
        p.panes[0].root_presence_disclosure(&p.panes[0].entries[0]),
        1
    );
    assert!(!p.notice_can_authorize());
    assert!(p.take_warning_viewer().is_none());
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
