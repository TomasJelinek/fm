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

fn polling_only(path: &str) -> WatchStatus {
    WatchStatus::PollingOnly(FsError::from_io(
        "watch directory",
        Path::new(path),
        io::Error::other("OS file watch limit reached"),
    ))
}

fn flat_view(navigation: NavigationId) -> WatchedView {
    WatchedView::Flat {
        pane: PaneId::Left,
        navigation,
    }
}

fn watch_text(p: &Presenter) -> Option<&'static str> {
    p.watch_notice(PaneId::Left).map(|notice| notice.text)
}

#[test]
fn a_watch_status_is_shown_non_modally_only_for_the_rows_it_concerns() {
    let mut p = Presenter::new("left".into(), "right".into());
    let first = begin(&mut p, PaneId::Left, "left");
    finish(&mut p, PaneId::Left, first, vec![entry(1, "left/a")]);
    p.apply(Event::WatchStatus {
        view: flat_view(first),
        status: polling_only("left"),
    });
    let notice = p
        .watch_notice(PaneId::Left)
        .expect("the current rows are only checked periodically");
    let polling = notice.text;
    assert!(polling.contains("about every 15 seconds"), "{polling}");
    assert!(
        notice.details.contains("OS file watch limit reached"),
        "the registration error explains it: {}",
        notice.details
    );
    assert!(p.panes[0].error.is_empty(), "never a panel error");
    assert!(p.desktop.notice.is_none(), "never a dialog");
    assert!(p.watch_notice(PaneId::Right).is_none());
    // The rows stay shown while another folder loads and after it failed.
    let other = begin(&mut p, PaneId::Left, "other");
    assert_eq!(watch_text(&p), Some(polling));
    p.apply(Event::ListingFailed {
        pane: PaneId::Left,
        navigation: other,
        error: FsError::from_io(
            "list",
            Path::new("other"),
            io::Error::from(io::ErrorKind::NotFound),
        ),
    });
    assert_eq!(watch_text(&p), Some(polling));
    p.apply(Event::WatchStatus {
        view: flat_view(first),
        status: WatchStatus::Native,
    });
    assert_eq!(watch_text(&p), None, "Native clears it");
    p.apply(Event::WatchStatus {
        view: flat_view(first),
        status: WatchStatus::Restarting,
    });
    let restarting = p.watch_notice(PaneId::Left).unwrap();
    assert!(
        restarting.text.contains("at most once a minute"),
        "{}",
        restarting.text
    );
    assert!(restarting.details.is_empty());
    // Another accepted folder is a new view, watched natively until reported
    // otherwise; a late status of the replaced rows is not shown.
    let next = begin(&mut p, PaneId::Left, "next");
    finish(&mut p, PaneId::Left, next, vec![entry(2, "next/b")]);
    assert_eq!(watch_text(&p), None);
    p.apply(Event::WatchStatus {
        view: flat_view(first),
        status: polling_only("left"),
    });
    assert_eq!(watch_text(&p), None);
    assert!(
        !p.take_retired().is_empty(),
        "replaced statuses are retired"
    );
}

#[test]
fn a_tree_watch_status_needs_the_tree_navigation_and_session() {
    let (mut p, root) = super::tree::fixture();
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(super::tree::publication(
            root, 1, false, false,
        )))),
        &|_| true,
    );
    let stamp = p.panes[0].tree_view().expect("an adopted tree").stamp;
    // The flat rows the tree was opened from are no longer shown.
    p.apply(Event::WatchStatus {
        view: flat_view(stamp.navigation),
        status: polling_only("tree"),
    });
    assert_eq!(watch_text(&p), None);
    p.apply(Event::WatchStatus {
        view: WatchedView::Tree(TreeViewStamp {
            session: TreeSessionId(stamp.session.0 + 1),
            ..stamp
        }),
        status: polling_only("tree"),
    });
    assert_eq!(watch_text(&p), None, "another session");
    // The projection revision can advance while a status waits.
    p.apply(Event::WatchStatus {
        view: WatchedView::Tree(TreeViewStamp {
            revision: TreeRevision(stamp.revision.0 + 5),
            ..stamp
        }),
        status: polling_only("tree"),
    });
    assert!(watch_text(&p).is_some());
}

#[test]
fn a_watch_hint_keeps_a_pending_tree_disclosure_and_then_refreshes_that_tree() {
    // The user disclosed a folder; its tree is not published yet.
    let (mut p, root) = super::tree::fixture();
    let navigation = p.panes[0].successful_generation.unwrap();
    assert!(
        !p.refresh_if_changed(PaneId::Left, navigation, &|command| panic!(
            "a re-read would discard the pending disclosure: {command:?}"
        )),
        "the hint waits for the tree"
    );
    assert!(!p.panes[0].loading);
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(super::tree::publication(
            root, 1, true, false,
        )))),
        &|_| true,
    );
    let view = p.panes[0]
        .tree_view()
        .expect("the disclosed tree is adopted")
        .clone();
    let refreshed = RefCell::new(Vec::new());
    p.flush_selection(&|command| {
        if let Command::Tree(command) = &command
            && let TreeCommand::RefreshLoaded(request) = command.as_ref()
        {
            refreshed.borrow_mut().push(request.view.clone());
        }
        assert!(
            !matches!(command, Command::Refresh { .. } | Command::Navigate { .. }),
            "the tree is kept: {command:?}"
        );
        true
    });
    let refreshed = refreshed.into_inner();
    assert_eq!(refreshed.len(), 1, "the retained hint refreshes the tree");
    assert!(Arc::ptr_eq(&refreshed[0], &view));
    assert!(Arc::ptr_eq(p.panes[0].tree_view().unwrap(), &view));
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
    let presented = |p: &Presenter, path: &str| {
        let pane = &p.panes[0];
        let entry = pane
            .entries
            .iter()
            .find(|e| e.path.as_path() == Path::new(path))
            .unwrap();
        crate::size_presentation::folder_size(
            pane.folder_size(entry),
            pane.folder_size_running(entry),
            pane.size_scan_attempted && pane.tree_view().is_none(),
            p.preferences.values.size_display,
            &p.number_locale,
        )
        .0
    };
    // (scan scope retained after cancellation, folders listed by the re-read, attempted after it)
    for (cancelled, relisted, attempted) in [
        (false, &["left/sized", "left/new"][..], false),
        (true, &["left/sized", "left/unreached"][..], true),
        (
            true,
            &["left/sized", "left/unreached", "left/new"][..],
            false,
        ),
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
        assert_eq!(p.panes[0].size_scan_attempted, attempted, "{relisted:?}");
        if relisted.contains(&"left/new") {
            assert_eq!(presented(&p, "left/new"), "<DIR>", "never part of a scan");
        }
        if attempted {
            assert_eq!(
                presented(&p, "left/unreached"),
                "—",
                "the cancelled scan did not finish it"
            );
        }
        assert_ne!(
            presented(&p, "left/sized"),
            "<DIR>",
            "the computed size stays"
        );
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
