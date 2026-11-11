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
