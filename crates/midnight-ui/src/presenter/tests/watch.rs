use super::*;

#[test]
fn watch_acknowledges_only_committed_snapshots_and_validates_hint_generation() {
    let mut p = Presenter::new("left".into(), "right".into());
    assert!(p.visible_directory(PaneId::Left).is_none());
    let generation = begin(&mut p, PaneId::Left, "left");
    assert!(p.visible_directory(PaneId::Left).is_none());
    finish(
        &mut p,
        PaneId::Left,
        generation,
        vec![entry(7, "left/a"), entry(8, "left/b")],
    );
    select_row(&mut p, PaneId::Left, 1, true);
    let visible = p.visible_directory(PaneId::Left).unwrap();
    assert_eq!(visible.navigation, generation);
    assert_eq!(visible.path, PathBuf::from("left"));
    assert!(Arc::ptr_eq(&visible.entries, &p.panes[0].entries));
    assert!(
        !p.refresh_if_changed(PaneId::Left, NavigationId(generation.0 - 1), &|_| panic!(
            "stale hints must not send"
        ))
    );
    assert!(!p.refresh_if_changed(PaneId::Left, generation, &|_| false));
    assert_eq!(p.panes[0].cursor, Some(EntryId(8)));
    assert!(selected_ids(&p, PaneId::Left).contains(&EntryId(8)));
    assert!(Arc::ptr_eq(
        &visible.entries,
        &p.visible_directory(PaneId::Left).unwrap().entries
    ));
    let sent = RefCell::new(None);
    assert!(p.refresh_if_changed(PaneId::Left, generation, &|command| {
        *sent.borrow_mut() = Some(command);
        true
    }));
    assert!(
        matches!(sent.into_inner().unwrap(), Command::Refresh { pane: PaneId::Left, path, .. } if path == Path::new("left"))
    );
    assert!(p.visible_directory(PaneId::Left).is_none());
    assert!(!p.refresh_if_changed(PaneId::Left, generation, &|_| panic!(
        "pending refresh must not restart"
    )));
    assert_eq!(p.panes[0].rows().len(), 2);
    p.apply(Event::DirectoryChanged {
        pane: PaneId::Left,
        navigation: generation,
    });
    assert!(p.panes[0].loading);
}

#[test]
fn failed_same_path_refresh_keeps_interactions_and_restores_old_ack_generation() {
    let mut p = Presenter::new("left".into(), "right".into());
    let generation = begin(&mut p, PaneId::Left, "left");
    finish(
        &mut p,
        PaneId::Left,
        generation,
        vec![entry(7, "left/a"), entry(8, "left/b")],
    );
    let visible = p.visible_directory(PaneId::Left).unwrap();
    let refresh = begin(&mut p, PaneId::Left, "left");
    select_row(&mut p, PaneId::Left, 1, true);
    p.apply(Event::ListingFailed {
        pane: PaneId::Left,
        navigation: refresh,
        error: FsError::from_io(
            "list",
            Path::new("left"),
            io::Error::from(io::ErrorKind::PermissionDenied),
        ),
    });
    assert_eq!(p.panes[0].cursor, Some(EntryId(8)));
    assert!(selected_ids(&p, PaneId::Left).contains(&EntryId(8)));
    let restored = p.visible_directory(PaneId::Left).unwrap();
    assert_eq!(restored.navigation, generation);
    assert!(Arc::ptr_eq(&visible.entries, &restored.entries));
}

#[test]
fn leaving_a_staged_refresh_then_failing_restores_the_latest_committed_selection() {
    let mut p = Presenter::new("left".into(), "right".into());
    let generation = begin(&mut p, PaneId::Left, "left");
    finish(
        &mut p,
        PaneId::Left,
        generation,
        vec![entry(7, "left/a"), entry(8, "left/b")],
    );
    begin(&mut p, PaneId::Left, "left");
    select_row(&mut p, PaneId::Left, 1, true);
    let other = begin(&mut p, PaneId::Left, "other");
    p.apply(Event::ListingFailed {
        pane: PaneId::Left,
        navigation: other,
        error: FsError::from_io(
            "list",
            Path::new("other"),
            io::Error::from(io::ErrorKind::NotFound),
        ),
    });
    assert_eq!(p.panes[0].cursor, Some(EntryId(8)));
    assert_eq!(selected_ids(&p, PaneId::Left), HashSet::from([EntryId(8)]));
    assert_eq!(
        p.visible_directory(PaneId::Left).unwrap().navigation,
        generation
    );
}

fn file(id: u64, name: &str) -> Entry {
    Entry::new(EntryId(id), PathBuf::from(name), EntryKind::File)
}

fn watch_refresh(p: &mut Presenter) -> NavigationId {
    let committed = p.panes[0].successful_generation.unwrap();
    let sent = RefCell::new(None);
    assert!(p.refresh_if_changed(PaneId::Left, committed, &|command| {
        *sent.borrow_mut() = Some(command);
        true
    }));
    match sent.into_inner() {
        Some(Command::Refresh { navigation, .. }) => navigation,
        _ => panic!("a same-folder watch hint sends a refresh"),
    }
}

#[test]
fn same_folder_refresh_drops_its_hidden_stream_instead_of_indexing_it() {
    let mut p = Presenter::new("left".into(), "right".into());
    let first = begin(&mut p, PaneId::Left, "left");
    finish(&mut p, PaneId::Left, first, vec![file(1, "left/a")]);
    let refresh = watch_refresh(&mut p);
    p.apply(Event::EntriesBatch {
        pane: PaneId::Left,
        navigation: refresh,
        entries: (2..130).map(|id| file(id, &format!("left/{id}"))).collect(),
    });
    p.apply(Event::MetadataBatch {
        pane: PaneId::Left,
        navigation: refresh,
        entries: vec![(EntryId(2), EntryMetadata::default())],
    });
    let pane = &p.panes[0];
    assert!(pane.streaming.is_empty() && pane.streaming_index.is_empty());
    assert!(pane.metadata.is_empty());
    assert_eq!(pane.rows().len(), 1, "the committed rows stay visible");
}

#[test]
fn caret_stays_near_a_focused_entry_that_a_refresh_removed() {
    let rows = |names: &[&str]| -> Vec<Entry> {
        names
            .iter()
            .enumerate()
            .map(|(id, name)| file(id as u64 + 1, &format!("left/{name}")))
            .collect()
    };
    for (focused, remaining, expected) in [
        ("c", &["a", "b", "d", "e"][..], "left/d"),
        ("e", &["a", "b", "c", "d"][..], "left/d"),
        ("c", &["x", "y", "z"][..], "left/z"),
    ] {
        let mut p = Presenter::new("left".into(), "right".into());
        let first = begin(&mut p, PaneId::Left, "left");
        finish(
            &mut p,
            PaneId::Left,
            first,
            rows(&["a", "b", "c", "d", "e"]),
        );
        let position = p.panes[0]
            .entries
            .iter()
            .position(|e| e.path.ends_with(focused));
        let row = position.unwrap() + p.panes[0].parent_offset();
        p.focus_display_row(PaneId::Left, row);
        let refresh = watch_refresh(&mut p);
        let mut renumbered = rows(remaining);
        for entry in &mut renumbered {
            entry.id.0 += 100;
        }
        finish(&mut p, PaneId::Left, refresh, renumbered);
        let pane = &p.panes[0];
        let cursor = pane
            .cursor
            .and_then(|id| pane.entry_by_id(id))
            .map(|e| &e.path);
        assert_eq!(cursor, Some(&PathBuf::from(expected)), "focused {focused}");
    }
}

#[test]
fn watch_refresh_keeps_computed_folder_sizes_but_explicit_refresh_discards_them() {
    let size = FolderSize {
        bytes: 42,
        files: 1,
        skipped_links: 0,
        errors: 0,
        finished: true,
        warnings: None,
    };
    let mut p = Presenter::new("left".into(), "right".into());
    let first = begin(&mut p, PaneId::Left, "left");
    finish(
        &mut p,
        PaneId::Left,
        first,
        vec![entry(1, "left/kept"), entry(2, "left/removed")],
    );
    for name in ["left/kept", "left/removed"] {
        p.panes[0].folder_sizes.insert(name.into(), size.clone());
    }
    p.panes[0].size_scan_attempted = true;
    let refresh = watch_refresh(&mut p);
    assert_eq!(p.panes[0].folder_sizes.len(), 2, "kept while loading");
    finish(&mut p, PaneId::Left, refresh, vec![entry(3, "left/kept")]);
    assert_eq!(
        p.panes[0].folder_sizes.keys().collect::<Vec<_>>(),
        [&PathBuf::from("left/kept")],
        "only vanished folders lose their size"
    );
    assert!(p.panes[0].size_scan_attempted);
    begin(&mut p, PaneId::Left, "left");
    assert!(p.panes[0].folder_sizes.is_empty(), "Ctrl+R discards them");
}

#[test]
fn folders_the_attempted_scan_never_covered_are_not_reported_as_unfinished() {
    use slint::Model;
    // (scan scope retained after cancellation, folders listed by the re-read)
    for (cancelled, relisted) in [
        (false, &["left/sized", "left/new"][..]),
        (true, &["left/sized", "left/unreached"][..]),
        (true, &["left/sized", "left/unreached", "left/new"][..]),
    ] {
        let mut p = Presenter::new("left".into(), "right".into());
        let first = begin(&mut p, PaneId::Left, "left");
        finish(
            &mut p,
            PaneId::Left,
            first,
            vec![entry(1, "left/sized"), entry(2, "left/unreached")],
        );
        let pane = &mut p.panes[0];
        pane.folder_sizes.insert(
            "left/sized".into(),
            FolderSize {
                bytes: 42,
                files: 1,
                finished: true,
                ..Default::default()
            },
        );
        pane.size_scan_attempted = true;
        if cancelled {
            pane.size_scan_scope = Some(SizeScanScope {
                navigation: first,
                path: "left".into(),
                entries: pane.entries.clone(),
                index: pane.index.clone(),
                selection: None,
            });
        }
        let refresh = watch_refresh(&mut p);
        let rows = relisted
            .iter()
            .enumerate()
            .map(|(id, path)| entry(id as u64 + 1, path))
            .collect();
        finish(&mut p, PaneId::Left, refresh, rows);
        // Each folder keeps its own state in the panel row model: a new folder is
        // not calculated, while one the scan did not reach stays unfinished.
        let model = crate::Rows::new(std::rc::Rc::new(RefCell::new(p)), PaneId::Left);
        let size = |name: &str| {
            (0..model.row_count())
                .filter_map(|row| model.row_data(row))
                .find(|row| row.name == name)
                .unwrap_or_else(|| panic!("no {name} row"))
                .size
                .to_string()
        };
        if relisted.contains(&"left/new") {
            assert_eq!(size("new"), "<DIR>", "never part of a scan: {relisted:?}");
        }
        if relisted.contains(&"left/unreached") {
            assert_eq!(
                size("unreached"),
                "—",
                "the cancelled scan did not finish it: {relisted:?}"
            );
        }
        assert_ne!(size("sized"), "<DIR>", "the computed size stays");
    }
}

#[test]
fn resort_keeps_a_retained_watch_hint_until_the_folder_is_read_again() {
    let mut p = Presenter::new("left".into(), "right".into());
    let first = begin(&mut p, PaneId::Left, "left");
    finish(&mut p, PaneId::Left, first, vec![entry(1, "left/a")]);
    let committed = p.panes[0].successful_generation.unwrap();
    assert!(
        !p.refresh_if_changed(PaneId::Left, committed, &|_| false),
        "a full queue retains the hint"
    );
    let sent = RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    assert!(p.sort(PaneId::Left, SortColumn::Size, &send));
    let resort = sent
        .borrow()
        .iter()
        .find_map(|command| match command {
            Command::Resort { navigation, .. } => Some(*navigation),
            _ => None,
        })
        .expect("a re-sort of the committed rows");
    let reread = |sent: &RefCell<Vec<Command>>| {
        sent.borrow().iter().any(|command| {
            matches!(
                command,
                Command::Refresh {
                    pane: PaneId::Left,
                    ..
                }
            )
        })
    };
    p.flush_selection(&send);
    assert!(!reread(&sent), "nothing is re-read while the re-sort loads");
    finish(&mut p, PaneId::Left, resort, vec![entry(1, "left/a")]);
    p.flush_selection(&send);
    assert!(
        reread(&sent),
        "the hint re-reads the folder after the re-sort"
    );
}

#[test]
fn native_history_keeps_the_same_bound_as_archive_and_network_history() {
    let mut p = Presenter::new("left".into(), "right".into());
    for index in 0..MAX_HISTORY + 5 {
        let path = format!("folder-{index}");
        let navigation = begin(&mut p, PaneId::Left, &path);
        finish(&mut p, PaneId::Left, navigation, vec![]);
    }
    let pane = &p.panes[0];
    assert_eq!(pane.history.len(), MAX_HISTORY);
    assert_eq!(pane.history_index, Some(MAX_HISTORY - 1));
    assert_eq!(pane.history[0], PathBuf::from("folder-5"));
    assert_eq!(
        pane.history.last(),
        Some(&PathBuf::from(format!("folder-{}", MAX_HISTORY + 4)))
    );
}
