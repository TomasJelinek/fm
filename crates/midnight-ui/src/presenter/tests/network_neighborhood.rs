use super::*;

fn address(id: u8) -> NetworkAddress {
    NetworkAddress {
        provider: "network.test".into(),
        opaque: Arc::from([id]),
        display: format!("Network {id}"),
    }
}
fn listing(id: u8) -> NetworkUpdate {
    let entries: Arc<[Entry]> = vec![Entry::from_network(
        EntryId(1),
        NetworkNode {
            address: address(id + 1),
            display: "server".into(),
        },
    )]
    .into();
    NetworkUpdate::Listing {
        listing: NetworkListing {
            address: address(id),
            parent: None,
            index: Arc::new(DirectoryIndex::from_entries(&entries)),
            entries,
        },
        done: true,
        error: None,
    }
}
fn network_pane() -> Presenter {
    let mut p = Presenter::new(test_root(), test_root());
    assert!(p.network_navigate(PaneId::Left, NetworkAction::OpenRoot, &|_| true));
    p.apply(Event::NetworkUpdated {
        pane: PaneId::Left,
        navigation: NavigationId(p.panes[0].generation),
        update: Box::new(listing(1)),
    });
    p
}

#[test]
fn accepted_network_root_immediately_replaces_native_presentation_without_native_authority() {
    let root = test_root().join("native");
    let mut p = Presenter::new(root.clone(), test_root());
    let navigation = begin(&mut p, PaneId::Left, root.to_str().unwrap());
    finish(
        &mut p,
        PaneId::Left,
        navigation,
        vec![entry(10, "old native folder")],
    );
    assert!(p.network_navigate(PaneId::Left, NetworkAction::OpenRoot, &|_| true));
    assert_eq!(
        p.panes[0].display_location_text(),
        NETWORK_NEIGHBORHOOD_LABEL
    );
    assert_eq!(p.tabs(PaneId::Left)[0].title, NETWORK_NEIGHBORHOOD_LABEL);
    assert!(p.panes[0].loading);
    assert_eq!(p.panes[0].display_len(), 0);
    assert!(!p.panes[0].has_parent());
    assert!(p.panes[0].display_entry(0).is_none());
    assert!(p.panes[0].entry_by_id(EntryId(10)).is_none());
    assert!(!p.panes[0].native_capabilities());
    assert!(p.visible_directory(PaneId::Left).is_none());
    assert!(!p.open(PaneId::Left, 0, &|_| panic!("pending rows cannot activate")));
    assert!(p.panes[0].visible_space().is_none());
    assert_eq!(p.panes[0].visible_storage_kind(), StorageKind::Network);
}

#[test]
fn pending_network_refresh_retries_the_requested_provider_action() {
    let mut p = Presenter::new(test_root(), test_root());
    assert!(p.network_navigate(PaneId::Left, NetworkAction::OpenRoot, &|_| true));
    let old = NavigationId(p.panes[0].generation);
    assert!(p.refresh_location(PaneId::Left, &|command| {
        assert!(matches!(
            command,
            Command::NetworkNavigate {
                action: NetworkAction::OpenRoot,
                ..
            }
        ));
        true
    }));
    p.apply(Event::NetworkUpdated {
        pane: PaneId::Left,
        navigation: old,
        update: Box::new(listing(1)),
    });
    assert_eq!(
        p.panes[0].display_location_text(),
        NETWORK_NEIGHBORHOOD_LABEL
    );
    assert_eq!(p.panes[0].display_len(), 0);
    assert!(p.panes[0].loading);
}

#[test]
fn unchanged_pending_network_path_preserves_root_or_opaque_target_without_parsing_the_caption() {
    for target in [None, Some(address(8))] {
        let mut p = network_pane();
        let action = target
            .clone()
            .map_or(NetworkAction::OpenRoot, NetworkAction::Browse);
        assert!(p.network_navigate(PaneId::Left, action, &|_| true));
        let caption = p.panes[0].display_location_text();
        assert!(p.navigate_text(PaneId::Left, &caption, &|command| {
            match (&target, command) {
                (
                    None,
                    Command::NetworkNavigate {
                        action: NetworkAction::OpenRoot,
                        ..
                    },
                ) => {}
                (
                    Some(expected),
                    Command::NetworkNavigate {
                        action: NetworkAction::Browse(actual),
                        ..
                    },
                ) => assert_eq!(*expected, actual),
                _ => panic!("unchanged captions must preserve the captured navigation identity"),
            }
            true
        }));
    }
}

#[test]
fn pending_share_activation_is_not_repeated_by_refresh_sort_or_matching_the_other_panel() {
    for action in [
        NetworkAction::Activate(address(2)),
        NetworkAction::Parse("network://server/share".into()),
    ] {
        let mut p = network_pane();
        assert!(p.network_navigate(PaneId::Left, action, &|_| true));
        let sort = p.panes[0].sort;
        let reject = |_: Command| panic!("presentation actions must not activate shares");
        assert!(!p.refresh_location(PaneId::Left, &reject));
        assert!(!p.sort(PaneId::Left, SortColumn::Size, &reject));
        assert_eq!(p.panes[0].sort, sort);
        assert!(!p.navigate_as_other_panel(PaneId::Right, &reject));
        assert!(p.panes[0].loading);
    }
}

#[test]
fn pending_network_from_native_or_archive_keeps_root_and_actions_in_the_requested_filesystem() {
    for mut p in [
        Presenter::new(test_root(), test_root()),
        archives::archive_pane(),
    ] {
        assert!(p.network_navigate(PaneId::Left, NetworkAction::OpenRoot, &|_| true));
        let reject = |_: Command| panic!("pending Network cannot access the previous filesystem");
        assert!(!p.open_folder(PaneId::Left, &reject));
        assert!(p.focused_name(PaneId::Left).is_none());
        p.copy_files(PaneId::Left, &reject);
        p.paste_files(PaneId::Left, &reject);
        p.start_operation(PaneId::Left, MutationKind::CreateDirectory, &reject);
        assert!(!p.open_terminal(&reject));
        assert!(p.go_root(PaneId::Left, &|command| {
            assert!(matches!(
                command,
                Command::NetworkNavigate {
                    action: NetworkAction::OpenRoot,
                    ..
                }
            ));
            true
        }));
        assert_eq!(
            p.panes[0].display_location_text(),
            NETWORK_NEIGHBORHOOD_LABEL
        );
        p.retire_all();
        assert!(p.panes[0].network_pending.is_none());
        assert!(!p.panes[0].network_request);
        assert!(!p.is_loading());
    }
}

#[test]
fn pending_network_browse_shows_requested_address_and_failure_restores_committed_view() {
    let mut p = network_pane();
    assert!(p.network_navigate(PaneId::Left, NetworkAction::Browse(address(8)), &|_| true));
    assert_eq!(p.panes[0].display_location_text(), address(8).display);
    assert_eq!(p.panes[0].display_len(), 0);
    p.apply(Event::NetworkUpdated {
        pane: PaneId::Left,
        navigation: NavigationId(p.panes[0].generation),
        update: Box::new(NetworkUpdate::Failed(NetworkFailure::new(
            NetworkFailureKind::Unavailable,
            "Unavailable target",
        ))),
    });
    assert_eq!(p.panes[0].display_location_text(), address(1).display);
    assert_eq!(p.panes[0].display_len(), 1);
    assert!(!p.panes[0].loading);
    assert!(!p.panes[0].native_capabilities());
    assert_eq!(p.panes[0].error, "Network service unavailable");
}

#[test]
fn refused_network_request_preserves_the_committed_native_presentation() {
    let mut p = Presenter::new(test_root(), test_root());
    let navigation = begin(&mut p, PaneId::Left, test_root().to_str().unwrap());
    finish(&mut p, PaneId::Left, navigation, vec![entry(10, "native")]);
    assert!(!p.network_navigate(PaneId::Left, NetworkAction::OpenRoot, &|_| false));
    assert_eq!(
        p.panes[0].display_location_text(),
        test_root().to_string_lossy()
    );
    assert_eq!(p.panes[0].display_len(), 1);
    assert!(!p.panes[0].loading);
    assert!(p.panes[0].native_capabilities());
}

#[test]
fn failed_network_root_restores_only_the_committed_native_location_with_an_error() {
    let root = test_root().join("original");
    let mut p = Presenter::new(root.clone(), test_root());
    let navigation = begin(&mut p, PaneId::Left, root.to_str().unwrap());
    finish(
        &mut p,
        PaneId::Left,
        navigation,
        vec![entry(10, "original")],
    );
    assert!(p.network_navigate(PaneId::Left, NetworkAction::OpenRoot, &|_| true));
    assert_eq!(p.panes[0].display_len(), 0);
    p.apply(Event::NetworkUpdated {
        pane: PaneId::Left,
        navigation: NavigationId(p.panes[0].generation),
        update: Box::new(NetworkUpdate::Failed(NetworkFailure::new(
            NetworkFailureKind::Unavailable,
            "No discovery provider",
        ))),
    });
    assert_eq!(p.panes[0].display_location_text(), root.to_string_lossy());
    assert_eq!(p.panes[0].display_len(), 1);
    assert!(p.panes[0].native_capabilities());
    assert!(!p.panes[0].loading);
    assert_eq!(p.panes[0].error, "Network service unavailable");
}

#[test]
fn partial_network_listing_replaces_preview_but_keeps_loading_until_completion() {
    let mut p = Presenter::new(test_root(), test_root());
    assert!(p.network_navigate(PaneId::Left, NetworkAction::OpenRoot, &|_| true));
    let navigation = NavigationId(p.panes[0].generation);
    let NetworkUpdate::Listing { listing, .. } = listing(1) else {
        unreachable!()
    };
    for done in [false, true] {
        p.apply(Event::NetworkUpdated {
            pane: PaneId::Left,
            navigation,
            update: Box::new(NetworkUpdate::Listing {
                listing: listing.clone(),
                done,
                error: None,
            }),
        });
        assert_eq!(p.panes[0].display_location_text(), address(1).display);
        assert_eq!(p.panes[0].display_len(), 1);
        assert_eq!(p.panes[0].loading, !done);
        assert!(!p.panes[0].native_capabilities());
    }
}

#[test]
fn native_navigation_supersedes_pending_network_presentation_and_ignores_late_failure() {
    let mut p = Presenter::new(test_root(), test_root());
    assert!(p.network_navigate(PaneId::Left, NetworkAction::OpenRoot, &|_| true));
    let old = NavigationId(p.panes[0].generation);
    let target = test_root().join("new native");
    let current = begin(&mut p, PaneId::Left, target.to_str().unwrap());
    p.apply(Event::NetworkUpdated {
        pane: PaneId::Left,
        navigation: old,
        update: Box::new(NetworkUpdate::Failed(NetworkFailure::new(
            NetworkFailureKind::Cancelled,
            "Canceled",
        ))),
    });
    assert_eq!(p.panes[0].display_location_text(), target.to_string_lossy());
    assert!(p.panes[0].loading);
    assert!(p.panes[0].error.is_empty());
    finish(&mut p, PaneId::Left, current, vec![]);
    assert!(!p.panes[0].loading);
    assert!(p.panes[0].native_capabilities());
}
#[test]
fn network_rows_never_supply_native_targets_and_actions_only_navigate() {
    let mut p = network_pane();
    assert!(!p.panes[0].native_capabilities());
    assert!(p.visible_directory(PaneId::Left).is_none());
    assert!(p.panes[0].entries[0].native_path().is_none());
    let sent = RefCell::new(vec![]);
    p.select_row(PaneId::Left, 1, false, &|command| {
        sent.borrow_mut().push(command);
        true
    });
    assert!(sent.borrow().is_empty());
    assert_eq!(p.panes[0].cursor, Some(EntryId(1)));
    assert!(p.open(PaneId::Left, 1, &|command| {
        sent.borrow_mut().push(command);
        true
    }));
    assert!(
        matches!(&sent.borrow()[0], Command::NetworkNavigate { action: NetworkAction::Activate(target), .. } if target.opaque.as_ref() == [2])
    );
    assert!(!p.native_action_allowed(PaneId::Left));
}
#[test]
fn stale_network_update_does_not_replace_current_native_navigation() {
    let mut p = network_pane();
    let old = NavigationId(p.panes[0].generation);
    p.navigate(PaneId::Left, test_root(), None, &|_| true);
    p.apply(Event::NetworkUpdated {
        pane: PaneId::Left,
        navigation: old,
        update: Box::new(listing(7)),
    });
    assert_eq!(
        p.panes[0].network.as_ref().unwrap().address.opaque.as_ref(),
        &[1]
    );
}
#[test]
fn native_handoff_keeps_network_context_until_native_listing_commits() {
    let mut p = network_pane();
    p.network_navigate(PaneId::Left, NetworkAction::Activate(address(2)), &|_| true);
    let navigation = NavigationId(p.panes[0].generation);
    let calls = RefCell::new(vec![]);
    p.apply_with_sender(
        Event::NetworkUpdated {
            pane: PaneId::Left,
            navigation,
            update: Box::new(NetworkUpdate::Native(test_root().join("share"))),
        },
        &|command| {
            calls.borrow_mut().push(command);
            true
        },
    );
    assert!(p.panes[0].network.is_some());
    assert!(!p.panes[0].native_capabilities());
    assert!(calls.borrow().iter().any(|command| matches!(command, Command::Navigate { path, .. } if *path == test_root().join("share"))));
}

fn handoff(p: &mut Presenter) -> NavigationId {
    p.network_navigate(PaneId::Left, NetworkAction::Activate(address(2)), &|_| true);
    p.apply_with_sender(
        Event::NetworkUpdated {
            pane: PaneId::Left,
            navigation: NavigationId(p.panes[0].generation),
            update: Box::new(NetworkUpdate::Native(test_root().join("share"))),
        },
        &|_| true,
    );
    NavigationId(p.panes[0].generation)
}

#[test]
fn pending_native_handoff_persists_the_committed_network_tab_until_listing_success() {
    for park in [false, true] {
        let mut p = network_pane();
        p.tab_session_changed(PaneId::Left, &|_| true);
        let navigation = handoff(&mut p);
        p.flush_tab_sessions(&|_| true);
        assert_eq!(
            p.preferences.values.tabs[0].networks.get(&0),
            Some(&address(1))
        );
        assert_eq!(p.preferences.values.tabs[0].paths[0], test_root());
        if park {
            // Parking a still-loading handoff must not persist its provisional
            // native destination as the Network tab's return origin either.
            assert!(p.new_tab(PaneId::Left, &|_| true));
            p.flush_tab_sessions(&|_| true);
            assert_eq!(
                p.preferences.values.tabs[0].networks.get(&0),
                Some(&address(1))
            );
            assert_eq!(p.preferences.values.tabs[0].paths[0], test_root());
        } else {
            finish(&mut p, PaneId::Left, navigation, vec![]);
            p.flush_tab_sessions(&|_| true);
            assert!(!p.preferences.values.tabs[0].networks.contains_key(&0));
            assert_eq!(
                p.preferences.values.tabs[0].paths[0],
                test_root().join("share")
            );
        }
    }
}

#[test]
fn committed_share_returns_to_network_with_up_or_back() {
    for back in [false, true] {
        let mut p = network_pane();
        let navigation = handoff(&mut p);
        finish(&mut p, PaneId::Left, navigation, vec![]);
        assert!(p.panes[0].network.is_none());
        assert!(p.panes[0].native_capabilities());
        let calls = RefCell::new(vec![]);
        let send = |command| {
            calls.borrow_mut().push(command);
            true
        };
        assert!(if back {
            p.history(PaneId::Left, -1, &send)
        } else {
            p.up(PaneId::Left, &send)
        });
        assert!(matches!(
            &calls.borrow()[0],
            Command::NetworkNavigate { action: NetworkAction::Browse(target), .. }
                if target.opaque.as_ref() == [1]
        ));
        p.apply(Event::NetworkUpdated {
            pane: PaneId::Left,
            navigation: NavigationId(p.panes[0].generation),
            update: Box::new(listing(1)),
        });
        assert_eq!(p.panes[0].network.as_ref().unwrap().origin, test_root());
        assert!(p.panes[0].network_return.is_none());
    }
}

#[test]
fn returning_from_share_preserves_network_history_and_the_activated_share_cursor() {
    for back in [false, true] {
        let mut p = network_pane();
        assert!(p.open(PaneId::Left, 1, &|_| true));
        p.apply(Event::NetworkUpdated {
            pane: PaneId::Left,
            navigation: NavigationId(p.panes[0].generation),
            update: Box::new(listing(2)),
        });
        // Activation's exact row must be retained even when no cursor was set first.
        assert!(p.open(PaneId::Left, 1, &|_| true));
        p.apply_with_sender(
            Event::NetworkUpdated {
                pane: PaneId::Left,
                navigation: NavigationId(p.panes[0].generation),
                update: Box::new(NetworkUpdate::Native(test_root().join("share"))),
            },
            &|_| true,
        );
        let navigation = NavigationId(p.panes[0].generation);
        finish(&mut p, PaneId::Left, navigation, vec![]);
        assert!(if back {
            p.history(PaneId::Left, -1, &|_| true)
        } else {
            p.up(PaneId::Left, &|_| true)
        });
        p.apply(Event::NetworkUpdated {
            pane: PaneId::Left,
            navigation: NavigationId(p.panes[0].generation),
            update: Box::new(listing(2)),
        });
        let network = p.panes[0].network.as_ref().unwrap();
        assert_eq!(network.history, vec![address(1), address(2)]);
        assert_eq!(network.history_index, 1);
        assert_eq!(p.panes[0].cursor, Some(EntryId(1)));
        assert!(!p.panes[0].parent_cursor);
        assert!(p.history(PaneId::Left, -1, &|command| {
            assert!(matches!(command, Command::NetworkNavigate {
                action: NetworkAction::Browse(target), ..
            } if target == address(1)));
            true
        }));
    }
}

#[test]
fn failed_share_handoff_keeps_network_and_never_recovers_to_an_unrelated_path() {
    let mut p = network_pane();
    let navigation = handoff(&mut p);
    let calls = RefCell::new(vec![]);
    p.apply_with_sender(
        Event::ListingFallback {
            pane: PaneId::Left,
            navigation,
            path: test_root(),
            error: FsError::from_io(
                "list",
                &test_root().join("share"),
                io::ErrorKind::NotFound.into(),
            ),
        },
        &|command| {
            calls.borrow_mut().push(command);
            true
        },
    );
    assert!(p.panes[0].network.is_some());
    assert!(!p.panes[0].native_capabilities());
    assert!(!p.panes[0].loading);
    assert!(!p.panes[0].error.is_empty());
    assert!(
        !calls
            .borrow()
            .iter()
            .any(|command| matches!(command, Command::Navigate { .. } | Command::Refresh { .. }))
    );
}

#[test]
fn refused_native_handoff_settles_loading_and_keeps_the_network_view() {
    let mut p = network_pane();
    p.network_navigate(PaneId::Left, NetworkAction::Activate(address(2)), &|_| true);
    p.apply_with_sender(
        Event::NetworkUpdated {
            pane: PaneId::Left,
            navigation: NavigationId(p.panes[0].generation),
            update: Box::new(NetworkUpdate::Native(test_root().join("share"))),
        },
        &|_| false,
    );
    assert!(p.panes[0].network.is_some());
    assert!(!p.panes[0].native_capabilities());
    assert!(!p.panes[0].network_request);
    assert!(!p.panes[0].loading);
    assert!(!p.panes[0].error.is_empty());
}
#[test]
fn parked_network_tab_restores_opaque_location_instead_of_native_origin() {
    let mut p = network_pane();
    let original = p.tabs(PaneId::Left)[0].id;
    p.new_tab(PaneId::Left, &|_| true);
    let calls = RefCell::new(vec![]);
    assert!(p.select_tab(PaneId::Left, original, &|command| {
        calls.borrow_mut().push(command);
        true
    }));
    assert!(calls.borrow().iter().any(|command| matches!(
        command,
        Command::NetworkNavigate {
            action: NetworkAction::Browse(_),
            ..
        }
    )));
    assert!(p.panes[0].network_request);
}

#[test]
fn unavailable_restored_network_tab_keeps_its_address_without_native_pending_authority() {
    let mut p = network_pane();
    let original = p.tabs(PaneId::Left)[0].id;
    p.new_tab(PaneId::Left, &|_| true);
    p.select_tab(PaneId::Left, original, &|_| true);
    p.apply(Event::NetworkUpdated {
        pane: PaneId::Left,
        navigation: NavigationId(p.panes[0].generation),
        update: Box::new(NetworkUpdate::Failed(NetworkFailure::new(
            NetworkFailureKind::Unavailable,
            "Network plugin is unavailable",
        ))),
    });
    assert!(!p.panes[0].loading);
    assert!(!p.panes[0].network_request);
    assert!(!p.panes[0].native_capabilities());
    assert!(p.panes[0].pending.is_none());
    assert_eq!(
        p.panes[0].network.as_ref().unwrap().address.opaque.as_ref(),
        &[1]
    );
    p.flush_tab_sessions(&|_| true);
    assert_eq!(p.preferences.values.tabs[0].networks[&0], address(1));
}

#[test]
fn completed_network_navigation_saves_typed_tab_and_does_not_watch_native_origin() {
    let mut p = network_pane();
    let commands = RefCell::new(vec![]);
    p.network_navigate(PaneId::Left, NetworkAction::Browse(address(2)), &|_| true);
    let navigation = NavigationId(p.panes[0].generation);
    p.apply_with_sender(
        Event::NetworkUpdated {
            pane: PaneId::Left,
            navigation,
            update: Box::new(listing(2)),
        },
        &|command| {
            commands.borrow_mut().push(command);
            true
        },
    );
    assert_eq!(
        p.preferences.values.tabs[0].networks[&0].opaque.as_ref(),
        &[2]
    );
    assert!(!p.refresh_if_changed(PaneId::Left, navigation, &|_| panic!(
        "virtual view cannot refresh native origin"
    )));
    assert!(!p.add_bookmark(PaneId::Left, "".into(), None, &|_| panic!(
        "must not bookmark origin"
    )));
}

#[test]
fn network_history_and_sort_use_provider_addresses_and_never_native_origin() {
    let mut p = network_pane();
    p.network_navigate(PaneId::Left, NetworkAction::Browse(address(2)), &|_| true);
    p.apply(Event::NetworkUpdated {
        pane: PaneId::Left,
        navigation: NavigationId(p.panes[0].generation),
        update: Box::new(listing(2)),
    });
    let choices = p.history_entries(PaneId::Left, false);
    assert_eq!(choices.len(), 1);
    let calls = RefCell::new(vec![]);
    assert!(
        p.choose_history_entry(PaneId::Left, false, &choices[0].identity, &|command| {
            calls.borrow_mut().push(command);
            true
        })
    );
    assert!(
        matches!(&calls.borrow()[0], Command::NetworkNavigate { action: NetworkAction::Browse(target), .. } if target.opaque.as_ref() == [1])
    );
    calls.borrow_mut().clear();
    assert!(p.sort(PaneId::Left, SortColumn::Name, &|command| {
        calls.borrow_mut().push(command);
        true
    }));
    assert!(matches!(
        &calls.borrow()[0],
        Command::NetworkNavigate {
            action: NetworkAction::Browse(_),
            ..
        }
    ));
}
