use super::*;
use midnight_core::ElevationAvailability;

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
        availability: ElevationAvailability::Available,
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
        availability: ElevationAvailability::Available,
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

#[test]
fn a_refused_authorized_retry_states_the_installation_requirement() {
    let (mut p, navigation, path) = denied_notice();
    offer(&mut p, navigation, &path);
    p.retry_notice_authorized(p.desktop.notice_generation, &|_| true);
    let retry = NavigationId(p.panes[0].generation);
    assert!(retry > navigation);
    // The fixed refusal of a helper in an unprotected folder (midnight-elevation).
    let requirement = "Retry with authorization needs Midnight Explorer installed where only \
        administrators can change it, such as a folder under C:\\Program Files. \
        C:\\Users\\me\\資料\\helper.exe is writable by its user.";
    p.apply(Event::ListingFailed {
        pane: PaneId::Left,
        navigation: retry,
        error: FsError::from_io(
            "authorized directory listing",
            &path,
            io::Error::new(io::ErrorKind::PermissionDenied, requirement),
        ),
    });
    let (title, body) = p.desktop.notice.clone().expect("the refusal is explained");
    assert_eq!(title, "Cannot open folder");
    assert_eq!(body, requirement);
    // An ordinary denial of the same folder keeps the concise text.
    assert!(p.navigate(PaneId::Left, path.clone(), None, &|_| true));
    p.apply(Event::ListingFailed {
        pane: PaneId::Left,
        navigation: NavigationId(p.panes[0].generation),
        error: FsError::from_io("list", &path, io::ErrorKind::PermissionDenied.into()),
    });
    let (_, body) = p.desktop.notice.clone().unwrap();
    assert_eq!(body, "Access to this folder was denied.");
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
        availability: ElevationAvailability::Available,
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
fn a_busy_authorization_queue_reports_failure_and_closes_both_authorization_entry_points() {
    let (mut p, navigation, path) = denied_notice();
    offer(&mut p, navigation, &path);
    p.retry_notice_authorized(p.desktop.notice_generation, &|_| false);
    // The refused send starts no navigation.
    assert!(!p.panes[0].loading);
    assert_eq!(p.panes[0].generation, navigation.0);
    let (title, body) = p.desktop.notice.clone().unwrap();
    assert_eq!(title, "Cannot retry folder");
    assert!(body.contains("queue is busy"), "{body}");
    assert!(p.desktop.notice_path.contains(path.to_str().unwrap()));
    // The busy notice replaces the offer: neither the notice nor the pane can
    // start an authorized retry until the folder is opened again, so the pane
    // drops its now unreachable receipt.
    assert!(!p.notice_can_authorize());
    assert!(!p.can_retry_directory_elevated(PaneId::Left));
    assert!(p.panes[0].elevation_retry.is_none());
}

/// The reason and refusal midnight-elevation reports for an installation whose retry
/// cannot ask for permission.
const UNAVAILABLE_REASON: &str = "Opening it needs administrator permission, which this \
    installation cannot ask for: other accounts on this computer can change its installation \
    folder.";
const UNAVAILABLE_DETAILS: &str = "Retry with authorization needs the authorization helper \
    shipped with this version of Midnight Explorer. D:\\Tools\\Midnight lets a \
    non-administrative account change, rename or replace it.";

fn unavailable(p: &mut Presenter, navigation: NavigationId, path: &Path) {
    p.apply(Event::DirectoryElevationAvailable {
        pane: PaneId::Left,
        navigation,
        path: path.to_owned(),
        availability: ElevationAvailability::Unavailable {
            reason: UNAVAILABLE_REASON.into(),
            details: UNAVAILABLE_DETAILS.into(),
        },
    });
}

#[test]
fn available_authorization_keeps_the_offer_and_the_concise_denial() {
    let (mut p, navigation, path) = denied_notice();
    let generation = p.desktop.notice_generation;
    offer(&mut p, navigation, &path);
    let (title, body) = p.desktop.notice.clone().unwrap();
    assert_eq!(title, "Cannot open folder");
    assert_eq!(body, "Access to this folder was denied.");
    assert_eq!(p.desktop.notice_generation, generation);
    assert!(p.notice_can_authorize());
}

#[test]
fn unavailable_authorization_states_why_in_the_notice_without_an_offer() {
    let (mut p, navigation, path) = denied_notice();
    let generation = p.desktop.notice_generation;
    unavailable(&mut p, navigation, &path);
    let (title, body) = p.desktop.notice.clone().unwrap();
    assert_eq!(title, "Cannot open folder");
    assert_eq!(
        body,
        format!("Access to this folder was denied. {UNAVAILABLE_REASON}")
    );
    assert_eq!(
        p.desktop.notice_generation, generation,
        "the shown notice is completed in place, not reopened"
    );
    assert!(p.desktop.notice_details.ends_with(UNAVAILABLE_DETAILS));
    let copy = p.desktop.notice_copy.clone().unwrap();
    assert!(copy.contains(UNAVAILABLE_REASON), "{copy}");
    assert!(copy.contains(UNAVAILABLE_DETAILS), "{copy}");
    // No retry is offered from the notice or the pane.
    assert!(!p.notice_can_authorize());
    assert!(!p.can_retry_directory_elevated(PaneId::Left));
    assert!(p.panes[0].elevation_retry.is_none());
    p.retry_notice_authorized(generation, &|_| panic!("no prompt can be shown"));
}

#[test]
fn unavailable_authorization_leaves_other_notices_unchanged() {
    // A closed or replaced notice, or another destination, is not amended.
    for case in 0..3 {
        let (mut p, navigation, path) = denied_notice();
        let target = match case {
            0 => {
                p.dismiss_diagnostic_notice(p.desktop.notice_generation);
                path.clone()
            }
            1 => {
                p.desktop
                    .report("Another message", "Different context".into());
                path.clone()
            }
            _ => path.with_file_name("different"),
        };
        let before = p.desktop.notice.clone();
        let details = p.desktop.notice_details.clone();
        unavailable(&mut p, navigation, &target);
        assert_eq!(p.desktop.notice, before, "case {case}");
        assert_eq!(p.desktop.notice_details, details, "case {case}");
        assert!(!p.notice_can_authorize());
    }
}

/// A pane that shows the rows an authorized retry of the denied folder listed.
fn authorized_listing() -> (Presenter, NavigationId, PathBuf) {
    let (mut p, denied, path) = denied_notice();
    offer(&mut p, denied, &path);
    p.retry_notice_authorized(p.desktop.notice_generation, &|_| true);
    let retry = NavigationId(p.panes[0].generation);
    finish(
        &mut p,
        PaneId::Left,
        retry,
        vec![Entry::new(
            EntryId(7),
            path.join("private.txt"),
            EntryKind::File,
        )],
    );
    assert_eq!(p.panes[0].path, path);
    assert_eq!(p.panes[0].successful_generation, Some(retry));
    (p, retry, path)
}

/// The native directory requests that `sent` holds.
fn listings(sent: &[Command]) -> Vec<(NavigationId, PathBuf, &'static str)> {
    sent.iter()
        .filter_map(|command| match command {
            Command::Navigate {
                navigation, path, ..
            } => Some((*navigation, path.clone(), "Navigate")),
            Command::Refresh {
                navigation, path, ..
            } => Some((*navigation, path.clone(), "Refresh")),
            Command::Resort {
                navigation, path, ..
            } => Some((*navigation, path.clone(), "Resort")),
            _ => None,
        })
        .collect()
}

/// The reported bug: the panel showed the authorized listing and then went back to
/// the previous folder. The pane's watch checked the folder with ordinary rights,
/// hinted the denied check as a change, the hint re-read the folder, and the
/// backend's recovery of that denial named the parent folder. Automatic re-reads
/// (a watch hint, Show hidden files, a removed drive, a finished operation) leave
/// the rows as listed, and a denied re-read never moves the pane.
#[test]
fn an_authorized_listing_stays_shown_without_automatic_re_reads() {
    let (mut p, listed, path) = authorized_listing();
    let rows = p.panes[0].entries.clone();
    assert!(!p.refresh_if_changed(PaneId::Left, listed, &|_| {
        panic!("a watch hint re-read the authorized listing with ordinary rights")
    }));
    p.operations
        .refresh
        .push((PaneId::Left, path.clone(), None));
    let sent = RefCell::new(Vec::new());
    p.refresh_operation_paths(&|command| {
        sent.borrow_mut().push(command);
        true
    });
    assert!(
        listings(&sent.borrow()).is_empty(),
        "a finished operation re-read the authorized listing"
    );
    assert!(p.operations.refresh.is_empty(), "and owes no re-read");
    assert!(!p.panes[0].loading);
    assert!(Arc::ptr_eq(&p.panes[0].entries, &rows));

    // A re-read started any other way: its denial is reported in place.
    assert!(p.navigate_impl(
        PaneId::Left,
        path.clone(),
        None,
        Reload::Background,
        &|_| true
    ));
    let reread = NavigationId(p.panes[0].generation);
    let sent = RefCell::new(Vec::new());
    p.apply_with_sender(
        Event::ListingFallback {
            pane: PaneId::Left,
            navigation: reread,
            path: path.parent().unwrap().to_owned(),
            error: FsError::from_io("list", &path, io::ErrorKind::PermissionDenied.into()),
        },
        &|command| {
            sent.borrow_mut().push(command);
            true
        },
    );
    let requests = listings(&sent.borrow());
    assert!(
        requests.is_empty(),
        "the pane went back to the parent folder: {requests:?}"
    );
    assert!(!p.panes[0].loading);
    assert_eq!(p.panes[0].path, path);
    assert!(Arc::ptr_eq(&p.panes[0].entries, &rows));
    assert_eq!(p.panes[0].successful_generation, Some(listed));
}

/// Refresh lists the folder again with ordinary rights, as a navigation: while access
/// is still denied, the rows stay and the denial offers authorization again, which
/// only the explicit retry uses.
#[test]
fn refresh_of_an_authorized_listing_offers_authorization_again() {
    let (mut p, listed, path) = authorized_listing();
    let rows = p.panes[0].entries.clone();
    let sent = RefCell::new(Vec::new());
    assert!(p.refresh_location(PaneId::Left, &|command| {
        sent.borrow_mut().push(command);
        true
    }));
    let requests = listings(&sent.borrow());
    let [(refresh, ref refreshed, kind)] = requests[..] else {
        panic!("Refresh lists the folder once: {requests:?}");
    };
    assert_eq!(
        (refreshed.as_path(), kind),
        (path.as_path(), "Navigate"),
        "a denied Refresh offers no authorization"
    );
    assert!(Arc::ptr_eq(&p.panes[0].entries, &rows));
    p.apply(Event::ListingFailed {
        pane: PaneId::Left,
        navigation: refresh,
        error: FsError::from_io("list", &path, io::ErrorKind::PermissionDenied.into()),
    });
    assert_eq!(p.panes[0].path, path);
    assert!(Arc::ptr_eq(&p.panes[0].entries, &rows));
    assert_eq!(p.panes[0].successful_generation, Some(listed));
    let (title, body) = p
        .desktop
        .notice
        .clone()
        .expect("the denial is presented as when the folder was opened");
    assert_eq!(title, "Cannot open folder");
    assert_eq!(body, "Access to this folder was denied.");
    assert!(!p.notice_can_authorize(), "only the backend's offer");
    offer(&mut p, refresh, &path);
    assert!(p.notice_can_authorize());
    let sent = RefCell::new(Vec::new());
    p.retry_notice_authorized(p.desktop.notice_generation, &|command| {
        sent.borrow_mut().push(command);
        true
    });
    let retry = sent
        .borrow()
        .iter()
        .find_map(|command| match command {
            Command::RetryDirectoryWithElevation {
                denied_navigation,
                navigation,
                ..
            } if *denied_navigation == refresh => Some(*navigation),
            _ => None,
        })
        .expect("the explicit retry");
    finish(
        &mut p,
        PaneId::Left,
        retry,
        vec![Entry::new(
            EntryId(8),
            path.join("new.txt"),
            EntryKind::File,
        )],
    );
    assert_eq!(p.panes[0].entries[0].path, path.join("new.txt"));
    assert!(!p.refresh_if_changed(PaneId::Left, retry, &|_| {
        panic!("the new authorized listing is re-read automatically")
    }));
}

/// The status row says how the rows were listed; a re-sort keeps them, a subfolder's
/// denial offers authorization for that subfolder, and leaving the folder ends it.
#[test]
fn an_authorized_listing_says_so_until_the_pane_leaves_it() {
    let (mut p, _, path) = authorized_listing();
    let notice = p
        .watch_notice(PaneId::Left)
        .expect("the status row says that the rows are not updated");
    assert!(
        notice.text.starts_with("Listed with authorization"),
        "{}",
        notice.text
    );
    let sent = RefCell::new(Vec::new());
    assert!(p.sort(PaneId::Left, SortColumn::Size, &|command| {
        sent.borrow_mut().push(command);
        true
    }));
    let requests = listings(&sent.borrow());
    let [(resort, _, "Resort")] = requests[..] else {
        panic!("a re-sort uses the shown rows: {requests:?}");
    };
    let rows = p.panes[0].entries.to_vec();
    finish(&mut p, PaneId::Left, resort, rows);
    assert!(p.watch_notice(PaneId::Left).is_some());
    assert!(!p.refresh_if_changed(PaneId::Left, resort, &|_| {
        panic!("re-sorted authorized rows are re-read automatically")
    }));

    let child = path.join("private");
    assert!(p.navigate(PaneId::Left, child.clone(), None, &|_| true));
    let opened = NavigationId(p.panes[0].generation);
    p.apply(Event::ListingFailed {
        pane: PaneId::Left,
        navigation: opened,
        error: FsError::from_io("list", &child, io::ErrorKind::PermissionDenied.into()),
    });
    offer(&mut p, opened, &child);
    assert!(p.notice_can_authorize());
    assert_eq!(p.panes[0].path, path);
    assert!(p.watch_notice(PaneId::Left).is_some());

    let parent = begin(
        &mut p,
        PaneId::Left,
        path.parent().unwrap().to_str().unwrap(),
    );
    finish(&mut p, PaneId::Left, parent, vec![]);
    assert!(p.watch_notice(PaneId::Left).is_none());
}

/// The rows of a subfolder that the backend listed through the pane's authorization
/// session (`ListingAuthorized` before its rows), as the pane shows after entering it.
fn session_listing(p: &mut Presenter, path: &Path, name: &str) -> NavigationId {
    let sent = RefCell::new(Vec::new());
    assert!(p.navigate(PaneId::Left, path.to_owned(), None, &|command| {
        sent.borrow_mut().push(command);
        true
    }));
    let requests = listings(&sent.borrow());
    let [(navigation, ref listed, "Navigate")] = requests[..] else {
        panic!("entering a folder sends one navigation: {requests:?}");
    };
    assert_eq!(listed, path);
    p.apply(Event::ListingAuthorized {
        pane: PaneId::Left,
        navigation,
    });
    finish(
        p,
        PaneId::Left,
        navigation,
        vec![Entry::new(EntryId(9), path.join(name), EntryKind::File)],
    );
    navigation
}

/// One consent covers the subfolders: entering one inside the authorized folder shows its
/// rows as an authorized listing, without any dialog or prompt.
#[test]
fn entering_a_subfolder_of_an_authorized_folder_shows_its_rows_without_a_dialog() {
    let (mut p, _, path) = authorized_listing();
    let notices = p.desktop.notice_generation;
    let child = path.join("private");
    let entered = session_listing(&mut p, &child, "inner.txt");
    assert_eq!(p.panes[0].path, child);
    assert_eq!(p.panes[0].successful_generation, Some(entered));
    assert_eq!(p.panes[0].entries[0].path, child.join("inner.txt"));
    assert_eq!(
        p.desktop.notice_generation, notices,
        "entering the subfolder opened a dialog"
    );
    assert!(!p.can_retry_directory_elevated(PaneId::Left));
    let notice = p
        .watch_notice(PaneId::Left)
        .expect("the status row says so");
    assert!(
        notice.text.starts_with("Listed with authorization"),
        "{}",
        notice.text
    );
    assert!(notice.text.contains("Subfolders"), "{}", notice.text);
    assert!(
        notice.details.contains("cannot be opened"),
        "{}",
        notice.details
    );
    assert!(!p.refresh_if_changed(PaneId::Left, entered, &|_| {
        panic!("a subfolder listed with authorization is re-read automatically")
    }));
    // A deeper level too.
    let grandchild = child.join("deeper");
    session_listing(&mut p, &grandchild, "deepest.txt");
    assert_eq!(p.panes[0].path, grandchild);
    assert!(p.watch_notice(PaneId::Left).is_some());
    assert_eq!(p.desktop.notice_generation, notices);
}

/// Ctrl+R of a folder listed through the session lists it again as a navigation, which
/// the backend serves through the session: the rows are replaced without a prompt.
#[test]
fn refresh_of_an_authorized_subfolder_re_lists_it_through_the_session() {
    let (mut p, _, path) = authorized_listing();
    let child = path.join("private");
    session_listing(&mut p, &child, "inner.txt");
    let notices = p.desktop.notice_generation;
    let sent = RefCell::new(Vec::new());
    assert!(p.refresh_location(PaneId::Left, &|command| {
        sent.borrow_mut().push(command);
        true
    }));
    let requests = listings(&sent.borrow());
    let [(refresh, ref refreshed, "Navigate")] = requests[..] else {
        panic!("Refresh lists the folder once, as a navigation: {requests:?}");
    };
    assert_eq!(refreshed, &child);
    p.apply(Event::ListingAuthorized {
        pane: PaneId::Left,
        navigation: refresh,
    });
    finish(
        &mut p,
        PaneId::Left,
        refresh,
        vec![Entry::new(
            EntryId(10),
            child.join("changed.txt"),
            EntryKind::File,
        )],
    );
    assert_eq!(p.panes[0].successful_generation, Some(refresh));
    assert_eq!(p.panes[0].entries[0].path, child.join("changed.txt"));
    assert_eq!(p.desktop.notice_generation, notices);
    assert!(p.watch_notice(PaneId::Left).is_some());
    assert!(!p.refresh_if_changed(PaneId::Left, refresh, &|_| {
        panic!("re-listed authorized rows are re-read automatically")
    }));
    // A stale mark for another navigation changes nothing.
    p.apply(Event::ListingAuthorized {
        pane: PaneId::Left,
        navigation: NavigationId(refresh.0 - 1),
    });
    assert!(!p.panes[0].loading);
}

/// Backspace above the authorized folder lists its parent with ordinary rights: the pane
/// shows those rows as an ordinary listing, and a later denial of the folder offers
/// authorization again instead of reusing the ended authorization.
#[test]
fn backspace_above_the_authorized_folder_ends_the_authorized_state() {
    let (mut p, _, path) = authorized_listing();
    let child = path.join("private");
    session_listing(&mut p, &child, "inner.txt");
    // Up to the authorized folder: still inside, still authorized.
    let sent = RefCell::new(Vec::new());
    assert!(p.up(PaneId::Left, &|command| {
        sent.borrow_mut().push(command);
        true
    }));
    let requests = listings(&sent.borrow());
    let [(to_root, ref listed, "Navigate")] = requests[..] else {
        panic!("{requests:?}");
    };
    assert_eq!(listed, &path);
    p.apply(Event::ListingAuthorized {
        pane: PaneId::Left,
        navigation: to_root,
    });
    finish(&mut p, PaneId::Left, to_root, vec![]);
    assert!(p.watch_notice(PaneId::Left).is_some());
    // Above it: an ordinary listing.
    let sent = RefCell::new(Vec::new());
    assert!(p.up(PaneId::Left, &|command| {
        sent.borrow_mut().push(command);
        true
    }));
    let requests = listings(&sent.borrow());
    let [(above, ref listed, "Navigate")] = requests[..] else {
        panic!("{requests:?}");
    };
    assert_eq!(listed.as_path(), path.parent().unwrap());
    finish(
        &mut p,
        PaneId::Left,
        above,
        vec![Entry::new(EntryId(1), path.clone(), EntryKind::Directory)],
    );
    assert_eq!(p.panes[0].path, path.parent().unwrap());
    assert!(p.watch_notice(PaneId::Left).is_none());
    assert!(!p.panes[0].shows_authorized_rows());
    // Entering the folder again is denied with ordinary rights; the backend offers a new
    // authorization, and nothing is listed through the ended one.
    assert!(p.navigate(PaneId::Left, path.clone(), None, &|_| true));
    let again = NavigationId(p.panes[0].generation);
    p.apply(Event::ListingFailed {
        pane: PaneId::Left,
        navigation: again,
        error: FsError::from_io("list", &path, io::ErrorKind::PermissionDenied.into()),
    });
    let (title, body) = p.desktop.notice.clone().unwrap();
    assert_eq!(title, "Cannot open folder");
    assert_eq!(body, "Access to this folder was denied.");
    offer(&mut p, again, &path);
    assert!(p.notice_can_authorize());
}

/// A link inside the authorized folder that leads elsewhere is refused by the helper; the
/// notice says why, and the backend's offer authorizes that folder on its own.
#[test]
fn a_link_that_leads_outside_the_authorized_folder_says_so_and_offers_its_own_retry() {
    let (mut p, _, path) = authorized_listing();
    let link = path.join("link");
    assert!(p.navigate(PaneId::Left, link.clone(), None, &|_| true));
    let opened = NavigationId(p.panes[0].generation);
    p.apply(Event::ListingFailed {
        pane: PaneId::Left,
        navigation: opened,
        error: FsError::outside_authorization(&link, &path, "The helper refused it."),
    });
    let (title, body) = p.desktop.notice.clone().unwrap();
    assert_eq!(title, "Cannot open folder");
    assert!(
        body.starts_with(
            "Access to this folder was denied. This folder is outside the authorized folder"
        ),
        "{body}"
    );
    assert!(body.contains(path.to_str().unwrap()), "{body}");
    assert!(!body.contains("The helper refused it."), "{body}");
    assert!(p.desktop.notice_details.contains("The helper refused it."));
    offer(&mut p, opened, &link);
    assert!(p.notice_can_authorize());
    assert_eq!(p.panes[0].path, path, "the authorized rows stay shown");
}
