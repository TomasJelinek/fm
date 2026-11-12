//! Discovery Hot Paths keep opaque identity and remain typed when unavailable.
use super::*;

fn address(id: u8, display: &str) -> NetworkAddress {
    NetworkAddress {
        provider: "network.bookmark.test".into(),
        opaque: Arc::from([0, 255, id, 128]),
        display: display.into(),
    }
}

fn displayed(address: NetworkAddress) -> Presenter {
    let mut p = Presenter::new(test_root().join("origin"), test_root().join("other"));
    p.apply(Event::PreferencesLoaded(Ok(Box::default())));
    p.panes[0].network = Some(
        crate::presenter::network_neighborhood::NetworkPane::restored(
            address,
            p.panes[0].path.clone(),
        ),
    );
    p
}

#[test]
fn network_hot_path_uses_provider_identity_and_refreshes_only_presentation() {
    let original = address(1, "display is not a native path @ 資料");
    let mut p = displayed(original.clone());
    assert!(p.add_bookmark(PaneId::Left, "Server".into(), Some(10), &|_| true));
    let saved = p.bookmarks()[0].clone();
    assert_eq!(saved.network.as_ref(), Some(&original));
    assert!(saved.archive.is_none());
    assert_eq!(saved.path, test_root().join("origin"));
    assert_eq!(saved.display_text(), original.display);
    let current = &mut p.panes[0].network.as_mut().unwrap().address;
    current.display = "Changed caption".into();
    assert!(p.add_bookmark(PaneId::Left, "Do not rename".into(), None, &|_| true));
    assert_eq!(p.bookmarks().len(), 1);
    assert_eq!(p.bookmarks()[0].id, saved.id);
    assert_eq!(p.bookmarks()[0].name, saved.name);
    assert_eq!(p.bookmarks()[0].slot, Some(10));
    assert_eq!(p.bookmarks()[0].display_text(), "Changed caption");
    assert_eq!(
        p.bookmarks()[0].network.as_ref().unwrap().opaque,
        original.opaque
    );

    // Equal captions are not equal provider locations; assigning an occupied
    // slot preserves the previous address without duplicating the shortcut.
    p.panes[0].network.as_mut().unwrap().address = address(2, "Changed caption");
    assert!(p.add_bookmark(PaneId::Left, "Second".into(), Some(10), &|_| true));
    assert_eq!(p.bookmarks().len(), 2);
    assert_eq!(p.bookmarks()[0].slot, None);
    assert_eq!(p.bookmarks()[1].slot, Some(10));
    p.preferences.values.validate().unwrap();
}

#[test]
fn pending_network_intents_are_not_saved_as_captions_or_native_origins() {
    let mut p = Presenter::new(test_root(), test_root());
    p.apply(Event::PreferencesLoaded(Ok(Box::default())));
    assert!(p.network_navigate(PaneId::Left, NetworkAction::OpenRoot, &|_| true));
    assert!(!p.add_bookmark(PaneId::Left, "Pending".into(), None, &|_| panic!("no save")));
    assert!(p.bookmarks().is_empty());
    let mut p = displayed(address(1, "Old location"));
    assert!(p.network_navigate(
        PaneId::Left,
        NetworkAction::Browse(address(2, "New location")),
        &|_| true
    ));
    assert!(!p.assign_bookmark_slot(PaneId::Left, 1, &|_| panic!("no save")));
    assert!(p.bookmarks().is_empty());
}

#[test]
fn network_hot_path_restart_keeps_unavailable_and_stale_locations_typed() {
    let original = address(1, "\\\\not-a-native-authority\\caption");
    let mut first = displayed(original.clone());
    assert!(first.add_bookmark(PaneId::Left, "Network".into(), Some(1), &|_| true));
    let saved = first.bookmarks()[0].clone();
    let mut p = Presenter::new(test_root().join("new session"), test_root().join("right"));
    p.apply(Event::PreferencesLoaded(Ok(Box::new(
        first.preferences.values.clone(),
    ))));
    p.apply(Event::NetworkAvailability(false));
    let commands = RefCell::new(Vec::new());
    let send = |command| {
        commands.borrow_mut().push(command);
        true
    };
    assert!(p.jump_bookmark(PaneId::Right, 1, &send));
    let navigation = NavigationId(p.panes[1].generation);
    assert!(commands.borrow().iter().any(|command| matches!(command,
        Command::NetworkNavigate { pane: PaneId::Right, action: NetworkAction::Browse(target), .. }
        if target == &original)));
    assert!(!commands.borrow().iter().any(|command| matches!(
        command,
        Command::Navigate { .. }
            | Command::NetworkNavigate {
                action: NetworkAction::Activate(_) | NetworkAction::Parse(_),
                ..
            }
    )));
    assert_eq!(p.panes[1].network.as_ref().unwrap().origin, saved.path);
    assert_eq!(p.panes[1].display_location_text(), original.display);
    assert!(!p.panes[1].native_capabilities());
    assert!(p.panes[1].entries.is_empty());
    p.apply(Event::NetworkUpdated {
        pane: PaneId::Right,
        navigation,
        update: Box::new(NetworkUpdate::Failed(NetworkFailure::new(
            NetworkFailureKind::Unavailable,
            "plugin absent",
        ))),
    });
    assert_eq!(p.panes[1].error, "Network service unavailable");
    assert_eq!(p.panes[1].network.as_ref().unwrap().address, original);
    assert!(p.visible_directory(PaneId::Right).is_none());
    assert_eq!(p.bookmarks(), std::slice::from_ref(&saved));

    assert!(p.jump_bookmark(PaneId::Right, 1, &send));
    let retry = NavigationId(p.panes[1].generation);
    assert_ne!(retry, navigation);
    p.apply(Event::NetworkUpdated {
        pane: PaneId::Right,
        navigation,
        update: Box::new(NetworkUpdate::Listing {
            listing: NetworkListing {
                address: address(99, "late reply"),
                parent: None,
                entries: Arc::default(),
                index: Arc::default(),
            },
            done: true,
            error: None,
        }),
    });
    assert_eq!(p.panes[1].network.as_ref().unwrap().address, original);
    p.apply(Event::NetworkUpdated {
        pane: PaneId::Right,
        navigation: retry,
        update: Box::new(NetworkUpdate::Failed(NetworkFailure::new(
            NetworkFailureKind::StaleReference,
            "expired",
        ))),
    });
    assert_eq!(p.panes[1].error, "Network location expired");
    assert_eq!(p.panes[1].network.as_ref().unwrap().address, original);
    assert!(!p.panes[1].native_capabilities());
    assert_eq!(p.bookmarks(), [saved]);
}

#[test]
fn rejected_network_hot_path_queue_preserves_the_previous_pane() {
    let mut p = displayed(address(1, "Saved network"));
    assert!(p.add_bookmark(PaneId::Left, "Network".into(), Some(1), &|_| true));
    let old = p.panes[1].path.clone();
    let generation = p.panes[1].generation;
    assert!(!p.jump_bookmark(PaneId::Right, 1, &|_| false));
    assert_eq!(p.panes[1].path, old);
    assert_eq!(p.panes[1].generation, generation);
    assert!(p.panes[1].network.is_none());
    assert!(!p.panes[1].network_request);
}
