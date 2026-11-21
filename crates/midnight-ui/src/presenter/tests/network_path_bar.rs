//! A Network location's path bar names the ancestors its provider published, the
//! outermost first, each a part that browses its location; no caption is parsed.
use super::*;
use std::collections::HashSet;

/// The root 1, a provider 2, a workgroup 3, a server 4 and its shares 5, 6, 7.
fn address(id: u8) -> NetworkAddress {
    let display = match id {
        1 => "Network".to_owned(),
        2 => "Windows Network".to_owned(),
        3 => "WORKGROUP 文档".to_owned(),
        4 => "SERVER".to_owned(),
        id => format!("Location {id}"),
    };
    NetworkAddress {
        provider: "network.test".into(),
        opaque: Arc::from([0, id, 255]),
        display,
    }
}

/// What the provider reports above location `id`: every lower id, the root first.
fn ancestors(id: u8) -> Arc<[NetworkAddress]> {
    (1..id).map(address).collect()
}

/// Publish location `id` for the pending left request, with the rows `rows`.
fn publish(p: &mut Presenter, id: u8, rows: &[u8]) {
    let entries: Arc<[Entry]> = rows
        .iter()
        .enumerate()
        .map(|(index, &row)| {
            Entry::from_network(
                EntryId(index as u64 + 1),
                NetworkNode {
                    address: address(row),
                    display: address(row).display,
                },
            )
        })
        .collect();
    p.apply(Event::NetworkUpdated {
        pane: PaneId::Left,
        navigation: NavigationId(p.panes[0].generation),
        update: Box::new(NetworkUpdate::Listing {
            listing: NetworkListing {
                address: address(id),
                ancestors: ancestors(id),
                index: Arc::new(DirectoryIndex::from_entries(&entries)),
                entries,
            },
            done: true,
            error: None,
        }),
    });
}

/// The server 4 shown with its shares.
fn server_pane() -> Presenter {
    let mut p = Presenter::new(test_root(), test_root());
    assert!(p.network_navigate(PaneId::Left, NetworkAction::Browse(address(4)), &|_| true));
    publish(&mut p, 4, &[5, 6]);
    p
}

fn labels(p: &Presenter) -> Vec<String> {
    p.path_parts(PaneId::Left)
        .into_iter()
        .map(|part| part.label)
        .collect()
}

const SERVER: [&str; 4] = [
    "Network",
    " › Windows Network",
    " › WORKGROUP 文档",
    " › SERVER",
];

/// The single left request that `act` sends, which must browse `expected`.
fn browses(expected: u8, act: impl FnOnce(&dyn Fn(Command) -> bool) -> bool) {
    let sent = RefCell::new(Vec::new());
    assert!(act(&|command| {
        sent.borrow_mut().push(command);
        true
    }));
    assert!(
        matches!(&sent.borrow()[..], [Command::NetworkNavigate {
            pane: PaneId::Left, action: NetworkAction::Browse(target), ..
        }] if *target == address(expected)),
        "expected to browse {expected}: {:?}",
        sent.borrow()
    );
}

#[test]
fn the_path_bar_names_the_published_ancestors_outermost_first() {
    let p = server_pane();
    let parts = p.path_parts(PaneId::Left);
    assert_eq!(labels(&p), SERVER);
    // Each prefix is the labels so far, so the bar measures and shortens it as it does
    // a native path, keeping the root and the shown location whole while they fit.
    let mut shown = String::new();
    for part in &parts {
        shown.push_str(&part.label);
        assert_eq!(part.prefix, shown);
        assert!(!part.keep_whole);
    }
    let identities: HashSet<_> = parts.iter().map(|part| &part.identity).collect();
    assert_eq!(identities.len(), parts.len());
    // The editor and the tab keep the location's own caption.
    assert_eq!(p.panes[0].display_location_text(), "SERVER");
    assert_eq!(p.tabs(PaneId::Left)[0].title, "SERVER");
    // A root has no ancestors; a location whose provider reports none keeps its caption.
    for id in [1, 9] {
        let mut p = Presenter::new(test_root(), test_root());
        assert!(p.network_navigate(PaneId::Left, NetworkAction::OpenRoot, &|_| true));
        let navigation = NavigationId(p.panes[0].generation);
        p.apply(Event::NetworkUpdated {
            pane: PaneId::Left,
            navigation,
            update: Box::new(NetworkUpdate::Listing {
                listing: NetworkListing {
                    address: address(id),
                    ancestors: Arc::default(),
                    entries: Arc::default(),
                    index: Arc::default(),
                },
                done: true,
                error: None,
            }),
        });
        assert_eq!(labels(&p), [address(id).display]);
    }
}

#[test]
fn an_ancestor_part_browses_its_location_with_the_cursor_on_the_way_back() {
    let mut p = server_pane();
    let parts = p.path_parts(PaneId::Left);
    browses(3, |send| {
        p.navigate_path_part(PaneId::Left, &parts[2].identity, send)
    });
    assert!(!p.navigate_path_part(PaneId::Left, &parts[2].identity, &|_| panic!("stale part")));
    // The workgroup lists its servers; the cursor is on the one left.
    publish(&mut p, 3, &[8, 4, 9]);
    assert_eq!(labels(&p), SERVER[..3]);
    assert_eq!(p.panes[0].cursor, Some(EntryId(2)));
    assert!(!p.panes[0].parent_cursor);
    let parts = p.path_parts(PaneId::Left);
    browses(1, |send| {
        p.navigate_path_part(PaneId::Left, &parts[0].identity, send)
    });
    publish(&mut p, 1, &[2]);
    assert_eq!(labels(&p), SERVER[..1]);
    assert_eq!(
        p.panes[0].cursor,
        Some(EntryId(1)),
        "on the way to the server"
    );
    // Up opens the provider's parent the same way.
    let mut p = server_pane();
    browses(3, |send| p.up(PaneId::Left, send));
    publish(&mut p, 3, &[4]);
    assert_eq!(p.panes[0].cursor, Some(EntryId(1)));
    // The shown location's own part reads it again and keeps the focused row.
    let mut p = server_pane();
    p.panes[0].cursor = Some(EntryId(2));
    let parts = p.path_parts(PaneId::Left);
    browses(4, |send| {
        p.navigate_path_part(PaneId::Left, &parts[3].identity, send)
    });
    assert_eq!(
        p.panes[0].display_len(),
        2,
        "the rows stay while it is read"
    );
    publish(&mut p, 4, &[5, 6]);
    assert_eq!(p.panes[0].cursor, Some(EntryId(2)));
}

#[test]
fn a_pending_reread_or_ancestor_visit_keeps_the_known_chain() {
    // A refresh or a sort of the shown location: the chain stays.
    let rereads: [fn(&mut Presenter) -> bool; 3] = [
        |p| p.refresh_location(PaneId::Left, &|_| true),
        |p| p.sort(PaneId::Left, SortColumn::Size, &|_| true),
        |p| p.network_navigate(PaneId::Left, NetworkAction::Browse(address(4)), &|_| true),
    ];
    for reread in rereads {
        let mut p = server_pane();
        assert!(reread(&mut p));
        assert!(p.panes[0].network_pending.is_some());
        assert_eq!(labels(&p), SERVER);
    }
    // A pending ancestor shows its part of the chain, and its parts browse on.
    let mut p = server_pane();
    assert!(p.network_navigate(PaneId::Left, NetworkAction::Browse(address(2)), &|_| true));
    assert_eq!(labels(&p), SERVER[..2]);
    let parts = p.path_parts(PaneId::Left);
    browses(1, |send| {
        p.navigate_path_part(PaneId::Left, &parts[0].identity, send)
    });
    publish(&mut p, 1, &[2]);
    assert_eq!(p.panes[0].cursor, Some(EntryId(1)));
    // Another location, a share activation, entered text or the root: the caption
    // alone until the provider publishes its chain.
    for (action, caption) in [
        (NetworkAction::Browse(address(9)), "Location 9"),
        (NetworkAction::Activate(address(5)), "Location 5"),
        (NetworkAction::Parse("\\\\other".into()), "\\\\other"),
        (NetworkAction::OpenRoot, NETWORK_NEIGHBORHOOD_LABEL),
    ] {
        let mut p = server_pane();
        assert!(p.network_navigate(PaneId::Left, action, &|_| true));
        assert_eq!(labels(&p), [caption]);
        // A failure restores the committed chain.
        p.apply(Event::NetworkUpdated {
            pane: PaneId::Left,
            navigation: NavigationId(p.panes[0].generation),
            update: Box::new(NetworkUpdate::Failed(NetworkFailure::new(
                NetworkFailureKind::NotFound,
                "missing",
            ))),
        });
        assert_eq!(labels(&p), SERVER);
    }
}

#[test]
fn a_share_return_shows_the_chain_of_the_location_it_was_opened_from() {
    for back in [false, true] {
        let mut p = server_pane();
        assert!(p.open(PaneId::Left, 1, &|_| true));
        p.apply_with_sender(
            Event::NetworkUpdated {
                pane: PaneId::Left,
                navigation: NavigationId(p.panes[0].generation),
                update: Box::new(NetworkUpdate::Native(test_root().join("share"))),
            },
            &|_| true,
        );
        // Until the share lists, the bar names the Network location.
        assert_eq!(labels(&p), SERVER);
        let navigation = NavigationId(p.panes[0].generation);
        finish(&mut p, PaneId::Left, navigation, vec![]);
        assert!(p.panes[0].network.is_none());
        assert_ne!(labels(&p), SERVER);
        browses(4, |send| {
            if back {
                p.history(PaneId::Left, -1, send)
            } else {
                p.up(PaneId::Left, send)
            }
        });
        assert_eq!(labels(&p), SERVER, "back: {back}");
        publish(&mut p, 4, &[5, 6]);
        assert_eq!(labels(&p), SERVER);
        assert_eq!(p.panes[0].cursor, Some(EntryId(1)), "on the share left");
    }
}

#[test]
fn a_part_copies_the_captions_up_to_it_without_navigating() {
    let mut p = server_pane();
    let generation = p.panes[0].generation;
    for (index, expected) in [
        (0, "Network"),
        (2, "Network › Windows Network › WORKGROUP 文档"),
        (3, "Network › Windows Network › WORKGROUP 文档 › SERVER"),
    ] {
        let parts = p.path_parts(PaneId::Left);
        let sent = RefCell::new(Vec::new());
        assert!(
            p.copy_path_part(PaneId::Left, &parts[index].identity, &|command| {
                sent.borrow_mut().push(command);
                true
            })
        );
        assert!(
            matches!(&sent.borrow()[..], [Command::CopyText { text, .. }] if text == expected),
            "{:?}",
            sent.borrow()
        );
    }
    assert_eq!(p.panes[0].generation, generation);
}
