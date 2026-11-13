use super::*;

#[test]
fn other_panel_navigation_preserves_raw_pending_path_and_queue_refusal() {
    let left = test_root().join("left 文档");
    let right = test_root().join("right e\u{301}");
    let mut presenter = Presenter::new(left.clone(), right);
    let pending = test_root().join("next 👩🏽‍💻 & ' folder");
    assert!(presenter.navigate(PaneId::Right, pending.clone(), None, &|_| true));
    assert!(!presenter.navigate_as_other_panel(PaneId::Left, &|_| false));
    assert_eq!(presenter.panes[0].display_path(), left);
    let sent = RefCell::new(Vec::new());
    assert!(presenter.navigate_as_other_panel(PaneId::Left, &|command| {
        sent.borrow_mut().push(command);
        true
    }));
    assert!(sent.borrow().iter().any(|command| matches!(command,
        Command::Navigate { pane: PaneId::Left, path, .. } if path == &pending)));
    assert_eq!(presenter.panes[0].display_path(), pending);
    assert_eq!(presenter.panes[1].display_path(), pending);
}

#[test]
fn special_location_keeps_the_containing_volume_selected() {
    let root = test_root();
    let special = root.join("Documents 文档");
    let mut presenter = Presenter::new(special.clone(), root.clone());
    presenter.apply(Event::DesktopContext {
        system_locations: Vec::new(),
        network_position: 0,
        shell: ShellFlavor::Posix,
        locations: vec![
            NavigationLocation {
                id: "disk".into(),
                label: "Disk".into(),
                path: root,
                kind: LocationKind::Volume,
                drive_letter: None,
            },
            NavigationLocation {
                id: "documents".into(),
                label: "Documents".into(),
                path: special,
                kind: LocationKind::Documents,
                drive_letter: None,
            },
        ],
    });
    assert_eq!(presenter.navigation_choice_index(PaneId::Left), 0);
}

#[test]
fn space_and_insert_advance_after_accepted_toggle_and_clamp_at_last_entry() {
    for key in [8, 9] {
        let root = test_root();
        let mut p = Presenter::new(root.clone(), root.clone());
        let generation = begin(&mut p, PaneId::Left, root.to_str().unwrap());
        finish(
            &mut p,
            PaneId::Left,
            generation,
            vec![
                Entry::new(EntryId(1), root.join("one"), EntryKind::File),
                Entry::new(EntryId(2), root.join("two"), EntryKind::File),
            ],
        );
        let sent = RefCell::new(Vec::new());
        p.move_cursor(PaneId::Left, key, &|command| {
            sent.borrow_mut().push(command);
            true
        });
        assert_eq!(p.panes[0].cursor, Some(EntryId(2)));
        assert!(matches!(&sent.borrow()[0], Command::Selection(request)
            if matches!(&request.action, SelectionAction::Toggle(path) if path == &root.join("one"))));
        p.move_cursor(PaneId::Left, key, &|_| true);
        assert_eq!(p.panes[0].cursor, Some(EntryId(2)));
    }
}

#[test]
fn rejected_toggle_keeps_cursor_on_the_unselected_entry() {
    let root = test_root();
    let mut p = Presenter::new(root.clone(), root.clone());
    let generation = begin(&mut p, PaneId::Left, root.to_str().unwrap());
    finish(
        &mut p,
        PaneId::Left,
        generation,
        vec![entry(1, "one"), entry(2, "two")],
    );
    p.move_cursor(PaneId::Left, 9, &|_| false);
    assert_eq!(p.panes[0].cursor, Some(EntryId(1)));
}

#[test]
fn breadcrumbs_navigate_raw_ancestors_and_reject_stale_tokens() {
    let root = test_root();
    let parent = root.join("文档 e\u{301}");
    let current = parent.join("👩🏽‍💻");
    let mut p = Presenter::new(current.clone(), root);
    let generation = begin(&mut p, PaneId::Left, current.to_str().unwrap());
    finish(&mut p, PaneId::Left, generation, vec![]);
    let parts = p.path_parts(PaneId::Left);
    assert_eq!(parts.last().unwrap().prefix, current.to_string_lossy());
    assert_eq!(
        parts
            .iter()
            .map(|part| part.label.as_str())
            .collect::<String>(),
        current.to_string_lossy()
    );
    let token = parts[parts.len() - 2].identity.clone();
    let sent = RefCell::new(Vec::new());
    assert!(p.navigate_path_part(PaneId::Left, &token, &|command| {
        sent.borrow_mut().push(command);
        true
    }));
    assert!(
        sent.borrow()
            .iter()
            .any(|command| matches!(command, Command::Navigate {path,..} if path == &parent))
    );
    assert!(!p.navigate_path_part(PaneId::Left, &token, &|_| panic!("stale breadcrumb")));
}

#[test]
fn breadcrumb_copy_preserves_unicode_prefix_without_navigation_and_rejects_stale_token() {
    let root = test_root();
    let parent = root.join("文档 e\u{301}");
    let current = parent.join("👩🏽‍💻");
    let mut p = Presenter::new(current.clone(), root);
    let generation = begin(&mut p, PaneId::Left, current.to_str().unwrap());
    finish(&mut p, PaneId::Left, generation, vec![]);
    let parts = p.path_parts(PaneId::Left);
    let token = parts[parts.len() - 2].identity.clone();
    let sent = RefCell::new(Vec::new());
    assert!(p.copy_path_part(PaneId::Left, &token, &|command| {
        sent.borrow_mut().push(command);
        true
    }));
    assert!(matches!(&sent.borrow()[0], Command::CopyText { text, .. }
        if text == &parent.to_string_lossy()));
    assert_eq!(sent.borrow().len(), 1);
    assert_eq!(p.panes[0].display_path(), current);
    assert!(!p.copy_path_part(PaneId::Left, &token, &|_| false));
    begin(&mut p, PaneId::Left, parent.to_str().unwrap());
    assert!(!p.copy_path_part(PaneId::Left, &token, &|_| panic!("stale clipboard path")));
}

#[test]
fn history_dropdown_uses_exact_position_and_never_creates_a_new_history_entry() {
    let root = test_root();
    let locations = [root.clone(), root.join("one"), root.join("two")];
    let mut p = Presenter::new(root.clone(), root);
    for path in &locations {
        let generation = begin(&mut p, PaneId::Left, path.to_str().unwrap());
        finish(&mut p, PaneId::Left, generation, vec![]);
    }
    let nearest = p.history_entry(PaneId::Left, false, 0).unwrap();
    assert_eq!(nearest.path, locations[1]);
    let token = p.history_entry(PaneId::Left, false, 1).unwrap().identity;
    assert!(p.history_entry(PaneId::Left, false, 2).is_none());
    assert!(p.choose_history_entry(PaneId::Left, false, &token, &|_| true));
    let generation = NavigationId(p.panes[0].generation);
    finish(&mut p, PaneId::Left, generation, vec![]);
    assert_eq!(p.panes[0].history, locations);
    assert_eq!(p.panes[0].history_index, Some(0));
    assert!(p.history_entry(PaneId::Left, true, 1).is_some());
    assert!(p.history_entry(PaneId::Left, true, 2).is_none());
    assert!(!p.choose_history_entry(PaneId::Left, false, &token, &|_| panic!("stale history")));
}

#[test]
fn focused_details_copy_exact_bytes_and_reject_a_stale_cursor_or_value() {
    let root = test_root();
    let mut p = Presenter::new(root.clone(), root.clone());
    let generation = begin(&mut p, PaneId::Left, root.to_str().unwrap());
    let mut file = Entry::new(EntryId(1), root.join("文档👩🏽‍💻"), EntryKind::File);
    file.metadata.size = Some(u64::MAX);
    finish(
        &mut p,
        PaneId::Left,
        generation,
        vec![
            file,
            Entry::new(EntryId(2), root.join("next"), EntryKind::File),
        ],
    );
    let fields = p.panel_details(PaneId::Left);
    let name = fields.iter().find(|field| field.label == "Name").unwrap();
    assert_eq!(name.value, "文档👩🏽‍💻");
    let bytes = fields.iter().find(|field| field.label == "Bytes").unwrap();
    assert_eq!(bytes.value, "18446744073709551615");
    let sent = RefCell::new(None);
    assert!(
        p.copy_panel_detail(PaneId::Left, &bytes.identity, &|command| {
            *sent.borrow_mut() = Some(command);
            true
        })
    );
    assert!(
        matches!(sent.into_inner(), Some(Command::CopyText {text,..})
        if text == "18446744073709551615")
    );
    p.move_cursor(PaneId::Left, 3, &|_| true);
    assert!(!p.copy_panel_detail(PaneId::Left, &name.identity, &|_| panic!("stale field")));
}

#[test]
fn space_sizes_original_directory_and_result_survives_sort_and_recycled_ids() {
    let root = test_root();
    let directory = root.join("large folder");
    let mut p = Presenter::new(root.clone(), root.clone());
    let navigation = begin(&mut p, PaneId::Left, root.to_str().unwrap());
    finish(
        &mut p,
        PaneId::Left,
        navigation,
        vec![
            Entry::new(EntryId(1), directory.clone(), EntryKind::Directory),
            Entry::new(EntryId(2), root.join("file"), EntryKind::File),
        ],
    );
    let sent = RefCell::new(Vec::new());
    p.move_cursor(PaneId::Left, 8, &|command| {
        sent.borrow_mut().push(command);
        true
    });
    let scan = sent
        .borrow()
        .iter()
        .find_map(|command| match command {
            Command::ComputeEntrySize { scan, entry, .. } => {
                assert_eq!(*entry, EntryId(1));
                Some(*scan)
            }
            _ => None,
        })
        .expect("Space requests directory size");
    assert_eq!(p.panes[0].cursor, Some(EntryId(2)));
    assert!(p.sort(PaneId::Left, SortColumn::Name, &|_| true));
    let sorted = NavigationId(p.panes[0].generation);
    finish(
        &mut p,
        PaneId::Left,
        sorted,
        vec![
            Entry::new(EntryId(91), directory.clone(), EntryKind::Directory),
            Entry::new(EntryId(1), root.join("unrelated"), EntryKind::File),
        ],
    );
    p.apply(Event::FolderSizeUpdated {
        pane: PaneId::Left,
        navigation,
        scan,
        entry: EntryId(1),
        size: FolderSize {
            bytes: 123,
            finished: true,
            ..Default::default()
        },
    });
    assert_eq!(p.panes[0].folder_sizes[&directory].bytes, 123);
    assert!(
        !p.panes[0]
            .folder_sizes
            .contains_key(&root.join("unrelated"))
    );
    p.apply(Event::FolderSizesFinished {
        pane: PaneId::Left,
        navigation,
        scan,
        cancelled: false,
    });
    assert!(!p.panes[0].entry_size_running(&directory));
}

#[test]
fn space_scans_directory_links_without_restarting_an_active_all_folder_scan() {
    let root = test_root();
    let mut p = Presenter::new(root.clone(), root.clone());
    let navigation = begin(&mut p, PaneId::Left, root.to_str().unwrap());
    finish(
        &mut p,
        PaneId::Left,
        navigation,
        vec![
            Entry::new(EntryId(1), root.join("link"), EntryKind::DirectoryLink),
            Entry::new(EntryId(2), root.join("folder"), EntryKind::Directory),
        ],
    );
    let sent = RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    p.move_cursor(PaneId::Left, 8, &send);
    assert!(sent.borrow().iter().any(|command| matches!(
        command,
        Command::ComputeEntrySize {
            entry: EntryId(1),
            ..
        }
    )));
    let position = sent
        .borrow()
        .iter()
        .position(|command| matches!(command, Command::Selection(_)))
        .unwrap();
    let toggle = Some(sent.borrow_mut().remove(position));
    fixture_toggle(&mut p, PaneId::Left, toggle);
    assert!(p.bulk_selection(PaneId::Left, SelectionOperation::Clear, &send));
    let Command::Selection(clear) = sent.borrow_mut().pop().unwrap() else {
        panic!("expected selection clear");
    };
    p.apply(Event::SelectionChanged {
        pane: PaneId::Left,
        navigation: clear.navigation,
        revision: clear.revision,
        selection: Arc::new(SelectionSnapshot::default()),
        deferred: Default::default(),
    });
    p.folder_sizes(PaneId::Left, &send);
    let active = p.panes[0].size_scan;
    assert!(
        active.is_some(),
        "the empty selection starts the whole-panel scan"
    );
    sent.borrow_mut().clear();
    p.move_cursor(PaneId::Left, 8, &send);
    assert_eq!(p.panes[0].size_scan, active);
    assert!(sent.borrow().iter().all(|command| matches!(
        command,
        Command::Selection(_) | Command::SummarizeSelectedFolders(_)
    )));
}

#[test]
fn numbered_tabs_switch_existing_tabs_without_mutating_hot_paths() {
    let root = test_root();
    let mut p = Presenter::new(root.clone(), root.clone());
    let navigation = begin(&mut p, PaneId::Left, root.to_str().unwrap());
    finish(&mut p, PaneId::Left, navigation, vec![]);
    assert!(p.new_tab(PaneId::Left, &|_| true));
    let tabs = p.tabs(PaneId::Left);
    assert!(tabs[1].active);
    assert!(!p.select_tab_number(PaneId::Left, 9, &|_| panic!("absent tab")));
    assert!(p.select_tab_number(PaneId::Left, 1, &|_| true));
    assert!(p.tabs(PaneId::Left)[0].active);
    assert!(p.bookmarks().is_empty());
}

#[test]
fn network_classification_tracks_the_displayed_generation_and_restores_on_failure() {
    let root = test_root();
    let mut p = Presenter::new(root.clone(), root.clone());
    let network = begin(&mut p, PaneId::Left, root.to_str().unwrap());
    p.apply(Event::StorageKindUpdated {
        pane: PaneId::Left,
        navigation: network,
        kind: StorageKind::Network,
    });
    assert_eq!(p.panes[0].visible_storage_kind(), StorageKind::Network);
    finish(&mut p, PaneId::Left, network, vec![]);
    let other = begin(&mut p, PaneId::Left, root.join("missing").to_str().unwrap());
    assert_eq!(p.panes[0].visible_storage_kind(), StorageKind::Unknown);
    p.apply(Event::StorageKindUpdated {
        pane: PaneId::Left,
        navigation: network,
        kind: StorageKind::LocalSsd,
    });
    p.apply(Event::ListingFailed {
        pane: PaneId::Left,
        navigation: other,
        error: FsError::from_io("list", &root, io::ErrorKind::NotFound.into()),
    });
    assert_eq!(p.panes[0].visible_storage_kind(), StorageKind::Network);
    p.apply(Event::StorageKindUpdated {
        pane: PaneId::Left,
        navigation: other,
        kind: StorageKind::Unknown,
    });
    assert_eq!(p.panes[0].visible_storage_kind(), StorageKind::Network);
}

#[test]
fn failed_navigation_cannot_retarget_a_breadcrumb_from_the_failed_path() {
    let root = test_root();
    let original = root.join("original").join("child");
    let failed = root.join("missing").join("child");
    let mut p = Presenter::new(original.clone(), root);
    let navigation = begin(&mut p, PaneId::Left, original.to_str().unwrap());
    finish(&mut p, PaneId::Left, navigation, vec![]);
    let navigation = begin(&mut p, PaneId::Left, failed.to_str().unwrap());
    let parts = p.path_parts(PaneId::Left);
    let stale = parts[parts.len() - 2].identity.clone();
    p.apply(Event::ListingFailed {
        pane: PaneId::Left,
        navigation,
        error: FsError::from_io("list", &failed, io::ErrorKind::NotFound.into()),
    });
    assert_eq!(p.panes[0].display_path(), original);
    assert!(!p.navigate_path_part(PaneId::Left, &stale, &|_| panic!("old path target")));
}

#[test]
fn selection_keys_advance_from_parent_without_selecting_or_measuring_it() {
    for key in [8, 9] {
        let path = test_root().join("child");
        let mut p = Presenter::new(path.clone(), path.clone());
        let generation = begin(&mut p, PaneId::Left, path.to_str().unwrap());
        finish(
            &mut p,
            PaneId::Left,
            generation,
            vec![Entry::new(
                EntryId(1),
                path.join("folder"),
                EntryKind::Directory,
            )],
        );
        assert!(p.panes[0].parent_cursor);
        p.move_cursor(PaneId::Left, key, &|_| panic!("parent is not an entry"));
        assert_eq!(p.panes[0].cursor, Some(EntryId(1)));
        assert!(!p.panes[0].parent_cursor);
    }
}

#[test]
fn directory_detail_rejects_old_estimate_after_value_changes_and_keeps_partial_marker() {
    let root = test_root();
    let directory = root.join("folder");
    let mut p = Presenter::new(root.clone(), root.clone());
    let generation = begin(&mut p, PaneId::Left, root.to_str().unwrap());
    finish(
        &mut p,
        PaneId::Left,
        generation,
        vec![Entry::new(
            EntryId(1),
            directory.clone(),
            EntryKind::Directory,
        )],
    );
    p.panes[0].folder_sizes.insert(
        directory.clone(),
        FolderSize {
            bytes: 123,
            finished: false,
            ..Default::default()
        },
    );
    let previous = p
        .panel_details(PaneId::Left)
        .into_iter()
        .find(|field| field.label == "Bytes")
        .unwrap();
    assert_eq!(previous.value, "≈ 123");
    p.panes[0].folder_sizes.insert(
        directory,
        FolderSize {
            bytes: 456,
            finished: true,
            errors: 1,
            ..Default::default()
        },
    );
    assert!(
        !p.copy_panel_detail(PaneId::Left, &previous.identity, &|_| panic!(
            "outdated value"
        ))
    );
    let current = p
        .panel_details(PaneId::Left)
        .into_iter()
        .find(|field| field.label == "Bytes")
        .unwrap();
    assert_eq!(current.value, "≈ 456", "completed partial is not exact");
}

#[test]
fn pending_filtered_search_cannot_select_or_measure_a_hidden_cursor() {
    for key in [8, 9] {
        let path = test_root().join("child");
        let mut p = Presenter::new(path.clone(), path.clone());
        let generation = begin(&mut p, PaneId::Left, path.to_str().unwrap());
        finish(
            &mut p,
            PaneId::Left,
            generation,
            vec![Entry::new(
                EntryId(1),
                path.join("old folder"),
                EntryKind::Directory,
            )],
        );
        p.move_cursor(PaneId::Left, 3, &|_| true);
        p.preferences.values.quick_search_filter = true;
        p.change_search_query(PaneId::Left, "missing", &|_| true);
        assert!(p.panes[0].search_view_waiting());
        assert_eq!(p.panes[0].display_len(), 0);
        assert_eq!(p.panes[0].cursor, Some(EntryId(1)));
        p.move_cursor(PaneId::Left, key, &|_| {
            panic!("hidden entry cannot be selected")
        });
        assert_eq!(p.panes[0].cursor, Some(EntryId(1)));
        assert!(p.panes[0].entry_size_scans.is_empty());
    }
}

#[test]
fn refresh_carries_target_scan_intent_until_the_ui_accepts_its_completion() {
    let root = test_root();
    let mut p = Presenter::new(root.clone(), root.clone());
    let navigation = begin(&mut p, PaneId::Left, root.to_str().unwrap());
    let entries = vec![Entry::new(
        EntryId(1),
        root.join("folder"),
        EntryKind::Directory,
    )];
    finish(&mut p, PaneId::Left, navigation, entries.clone());
    p.move_cursor(PaneId::Left, 8, &|_| true);
    let scan = p.panes[0].entry_size_scans[0].scan;
    let sent = RefCell::new(Vec::new());
    assert!(p.navigate(PaneId::Left, root.clone(), None, &|command| {
        sent.borrow_mut().push(command);
        true
    }));
    assert!(sent.borrow().iter().any(|command| matches!(command,
        Command::Refresh { scan: Some(id), .. } if *id == scan)));
    let refreshed = NavigationId(p.panes[0].generation);
    finish(&mut p, PaneId::Left, refreshed, entries);
    p.apply(Event::FolderSizesFinished {
        pane: PaneId::Left,
        navigation,
        scan,
        cancelled: false,
    });
    sent.borrow_mut().clear();
    assert!(p.navigate(PaneId::Left, root, None, &|command| {
        sent.borrow_mut().push(command);
        true
    }));
    assert!(
        sent.borrow()
            .iter()
            .any(|command| matches!(command, Command::Refresh { scan: None, .. }))
    );
}
