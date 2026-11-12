use super::*;

#[test]
fn archive_size_hotkey_captures_exact_provider_root_without_native_io() {
    let mut p = super::archives::archive_pane();
    let commands = RefCell::new(Vec::new());
    p.folder_sizes(PaneId::Left, &|command| {
        commands.borrow_mut().push(command);
        true
    });
    let commands = commands.borrow();
    let [
        Command::ArchiveSizes(ArchiveSizeCommand::Compute {
            root, selection, ..
        }),
    ] = commands.as_slice()
    else {
        panic!("archive folder sizes must use the provider command: {commands:?}");
    };
    assert_eq!(root.pane, PaneId::Left);
    assert!(Arc::ptr_eq(&root.entries, &p.panes[0].entries));
    assert!(Arc::ptr_eq(&root.index, &p.panes[0].index));
    assert!(selection.is_none());
}

#[test]
fn space_measures_the_archive_folder_then_advances_to_next_row() {
    let mut p = super::archives::archive_pane();
    p.focus_display_row(PaneId::Left, 1);
    let commands = RefCell::new(Vec::new());
    p.move_cursor(PaneId::Left, 8, &|command| {
        commands.borrow_mut().push(command);
        true
    });
    assert!(commands.borrow().iter().any(|command| matches!(
        command,
        Command::ArchiveSizes(ArchiveSizeCommand::Entry {
            entry: EntryId(2),
            ..
        })
    )));
    assert_eq!(p.panes[0].cursor, Some(EntryId(3)));
}

fn start(p: &mut Presenter) -> FolderSizeScanId {
    let request = RefCell::new(None);
    p.folder_sizes(PaneId::Left, &|command| {
        *request.borrow_mut() = Some(command);
        true
    });
    let Some(Command::ArchiveSizes(ArchiveSizeCommand::Compute { scan, .. })) =
        request.into_inner()
    else {
        panic!("expected an archive scan");
    };
    scan
}

fn sizes(p: &mut Presenter, mount: u64, version: u64, bytes: u64) {
    p.apply(Event::ArchiveSizes(Box::new(ArchiveSizeEvent::Snapshot {
        pane: PaneId::Left,
        scan: None,
        sizes: Arc::new(ArchiveSizeSnapshot::new(
            mount,
            version,
            HashMap::from([(
                VirtualEntryId { mount, entry: 2 },
                FolderSize {
                    bytes,
                    finished: true,
                    ..Default::default()
                },
            )]),
        )),
    })));
}

fn select(p: &mut Presenter, ids: &[u64]) {
    let mut totals = ArchiveSelectionTotals::default();
    for id in ids {
        let entry = p.panes[0].entry_by_id(EntryId(*id)).unwrap();
        totals.change(entry.kind, entry.metadata.size, true);
    }
    let archive = p.panes[0].archive.as_mut().unwrap();
    archive.selection_revision.0 += 1;
    archive.selection_pending = true;
    let revision = archive.selection_revision;
    p.apply(Event::ArchiveSelection {
        pane: PaneId::Left,
        navigation: NavigationId(p.panes[0].generation),
        revision,
        view: p.panes[0].display_index().clone(),
        result: Ok(ArchiveSelectionResult {
            selected: Arc::new(ids.iter().map(|id| EntryId(*id)).collect()),
            totals,
        }),
    });
}

#[test]
fn selected_archive_files_never_fall_back_to_scanning_all_folders() {
    for selected in [vec![2], vec![3], vec![2, 4]] {
        let mut p = super::archives::archive_pane();
        select(&mut p, &selected);
        let request = RefCell::new(None);
        p.folder_sizes(PaneId::Left, &|command| {
            *request.borrow_mut() = Some(command);
            true
        });
        let Some(Command::ArchiveSizes(ArchiveSizeCommand::Compute {
            selection: Some(ids),
            ..
        })) = request.into_inner()
        else {
            panic!("selection must be captured")
        };
        assert_eq!(*ids, selected.into_iter().map(EntryId).collect());
        assert!(Arc::ptr_eq(
            &ids,
            &p.panes[0].archive.as_ref().unwrap().selected
        ));
    }
}

#[test]
fn archive_sizes_ignore_other_mounts_and_old_versions_without_native_path_aliasing() {
    let mut p = super::archives::archive_pane();
    sizes(&mut p, 42, 2, 123);
    sizes(&mut p, 99, 9, 999);
    sizes(&mut p, 42, 1, 777);
    let pane = &p.panes[0];
    let folder = pane.entry_by_id(EntryId(2)).unwrap();
    let file = pane.entry_by_id(EntryId(3)).unwrap();
    assert_eq!(
        folder.path, file.path,
        "virtual display paths are not identities"
    );
    assert_eq!(pane.folder_size(folder).unwrap().bytes, 123);
    assert!(pane.folder_size(file).is_none());
    assert!(
        p.take_retired().len() >= 3,
        "retire large snapshots off the UI thread"
    );
}

#[test]
fn archive_resort_keeps_calculation_cursor_and_ready_sizes() {
    let mut p = super::archives::archive_pane();
    p.focus_display_row(PaneId::Left, 1);
    let scan = start(&mut p);
    let entries = p.panes[0].entries.clone();
    let location = p.panes[0].archive.as_ref().unwrap().location.clone();
    let commands = RefCell::new(Vec::new());
    assert!(p.sort(PaneId::Left, SortColumn::Size, &|command| {
        commands.borrow_mut().push(command);
        true
    }));
    assert!(
        !commands
            .borrow()
            .iter()
            .any(|c| matches!(c, Command::ArchiveSizes(ArchiveSizeCommand::Cancel { .. })))
    );
    sizes(&mut p, 42, 1, 456);
    p.apply(Event::ArchiveListed {
        pane: PaneId::Left,
        navigation: NavigationId(p.panes[0].generation),
        result: Ok(ArchiveListing {
            location,
            index: Arc::new(DirectoryIndex::from_entries(&entries)),
            entries,
        }),
    });
    assert_eq!(p.panes[0].cursor, Some(EntryId(2)));
    let folder = p.panes[0].entry_by_id(EntryId(2)).unwrap();
    assert_eq!(p.panes[0].folder_size(folder).unwrap().bytes, 456);
    p.folder_sizes(PaneId::Left, &|command| {
        assert!(matches!(command, Command::ArchiveSizes(
            ArchiveSizeCommand::Cancel { scan: id, .. }) if id == scan));
        true
    });
}

#[test]
fn late_scan_completion_or_rejection_does_not_clear_new_archive_scan() {
    let mut p = super::archives::archive_pane();
    let old = start(&mut p);
    p.apply(Event::ArchiveSizes(Box::new(ArchiveSizeEvent::Finished {
        pane: PaneId::Left,
        scan: old,
        cancelled: false,
    })));
    let new = start(&mut p);
    assert!(new > old);
    p.apply(Event::ArchiveSizes(Box::new(ArchiveSizeEvent::Rejected {
        pane: PaneId::Left,
        scan: Some(old),
        message: "obsolete".into(),
    })));
    p.apply(Event::ArchiveSizes(Box::new(ArchiveSizeEvent::Finished {
        pane: PaneId::Left,
        scan: old,
        cancelled: true,
    })));
    assert!(!p.panes[0].error.contains("obsolete"));
    p.folder_sizes(PaneId::Left, &|command| {
        assert!(matches!(command, Command::ArchiveSizes(
            ArchiveSizeCommand::Cancel { scan, .. }) if scan == new));
        true
    });
}

#[test]
fn archive_queue_rejection_is_retryable_and_pending_selection_is_not_scanned() {
    let mut p = super::archives::archive_pane();
    p.folder_sizes(PaneId::Left, &|_| false);
    assert!(p.panes[0].error.contains("queue is busy"));
    p.panes[0].archive.as_mut().unwrap().selection_pending = true;
    p.folder_sizes(PaneId::Left, &|_| panic!("selection not settled"));
    p.panes[0].archive.as_mut().unwrap().selection_pending = false;
    start(&mut p);
}

#[test]
fn insert_parent_and_file_space_do_not_request_archive_folder_scans() {
    for (row, key) in [(0, 8), (2, 8), (1, 9)] {
        let mut p = super::archives::archive_pane();
        p.focus_display_row(PaneId::Left, row);
        p.move_cursor(PaneId::Left, key, &|command| {
            assert!(!matches!(command, Command::ArchiveSizes(_)));
            true
        });
    }
}

fn observe(p: &mut Presenter) -> (SelectionSizeId, NavigationId, SelectionRevision) {
    let command = RefCell::new(None);
    p.flush_selection_sizes(&|value| {
        *command.borrow_mut() = Some(value);
        true
    });
    let Some(Command::ArchiveSizes(ArchiveSizeCommand::Observe {
        request,
        root,
        revision,
        ..
    })) = command.into_inner()
    else {
        panic!("expected archive observer")
    };
    (request, root.navigation, revision)
}

fn summary(
    p: &mut Presenter,
    observer: (SelectionSizeId, NavigationId, SelectionRevision),
    version: u64,
    bytes: u64,
) {
    p.apply(Event::ArchiveSizes(Box::new(ArchiveSizeEvent::Selected {
        pane: PaneId::Left,
        request: observer.0,
        navigation: observer.1,
        revision: observer.2,
        version,
        summary: SelectedFolderSizes {
            folders: 1,
            complete: 1,
            complete_bytes: Some(bytes),
            partial: 0,
            partial_bytes: Some(0),
        },
    })));
}

#[test]
fn selected_archive_folder_totals_are_versioned_and_clear_on_invalidation() {
    let mut p = super::archives::archive_pane();
    select(&mut p, &[2, 3]);
    let first = observe(&mut p);
    summary(&mut p, first, 2, 42);
    summary(&mut p, first, 1, 777);
    assert!(
        p.panel_details(PaneId::Left)
            .iter()
            .any(|f| f.value.contains("42 bytes"))
    );
    p.flush_selection_sizes(&|_| panic!("same observer cannot resubmit every refresh"));
    p.apply(Event::ArchiveSizes(Box::new(
        ArchiveSizeEvent::Invalidated {
            pane: PaneId::Left,
            mount: 99,
        },
    )));
    assert!(p.panes[0].selected_folder_sizes().is_some());
    p.apply(Event::ArchiveSizes(Box::new(
        ArchiveSizeEvent::Invalidated {
            pane: PaneId::Left,
            mount: 42,
        },
    )));
    assert!(p.panes[0].selected_folder_sizes().is_none());
    summary(&mut p, first, 2, 42);
    assert!(
        p.panes[0].selected_folder_sizes().is_none(),
        "late cached summary is invalid"
    );
}

#[test]
fn snapshot_ack_survives_navigation_and_retries_a_full_command_queue() {
    let mut p = super::archives::archive_pane();
    sizes(&mut p, 42, 3, 42);
    let archive = p.panes[0].archive.take();
    p.retired.push(Box::new(archive));
    p.flush_selection_sizes(&|command| {
        assert!(matches!(
            command,
            Command::ArchiveSizes(ArchiveSizeCommand::Acknowledge {
                pane: PaneId::Left,
                mount: 42,
                version: 3
            })
        ));
        false
    });
    let seen = RefCell::new(0);
    p.flush_selection_sizes(&|command| {
        assert!(matches!(
            command,
            Command::ArchiveSizes(ArchiveSizeCommand::Acknowledge {
                pane: PaneId::Left,
                mount: 42,
                version: 3
            })
        ));
        *seen.borrow_mut() += 1;
        true
    });
    assert_eq!(*seen.borrow(), 1);
    p.flush_selection_sizes(&|_| panic!("already acknowledged"));
}

#[test]
fn unavailable_mount_snapshot_is_retired_and_acknowledged() {
    let mut p = super::archives::archive_pane();
    sizes(&mut p, 99, 10, 999);
    p.flush_selection_sizes(&|command| {
        assert!(matches!(
            command,
            Command::ArchiveSizes(ArchiveSizeCommand::Acknowledge {
                mount: 99,
                version: 10,
                ..
            })
        ));
        true
    });
    let folder = p.panes[0].entry_by_id(EntryId(2)).unwrap();
    assert!(p.panes[0].folder_size(folder).is_none());
}

#[derive(Debug)]
struct SecondMount;
impl ArchiveMount for SecondMount {
    fn id(&self) -> u64 {
        43
    }
    fn list(&self, _: u64, _: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        panic!("UI listing")
    }
    fn read(&self, _: u64, _: u64, _: u32, _: &Cancellation) -> Result<Vec<u8>, FsError> {
        panic!("UI content read")
    }
    fn validate(&self) -> Result<(), FsError> {
        panic!("UI source validation")
    }
}

#[test]
fn opening_another_archive_keeps_observer_request_ids_monotonic() {
    let mut p = super::archives::archive_pane();
    select(&mut p, &[2]);
    let first = observe(&mut p);
    let mut location = p.panes[0].archive.as_ref().unwrap().location.clone();
    location.mount = Some(Arc::new(SecondMount));
    location.address.container = test_root().join("another.zip");
    let entries: Arc<[Entry]> = p.panes[0]
        .entries
        .iter()
        .map(|entry| {
            let mut node = entry.virtual_node.clone().unwrap();
            node.id.mount = 43;
            Entry::from_archive(node)
        })
        .collect();
    assert!(
        p.archive_navigate(PaneId::Left, ArchiveAction::List(location.clone()), &|_| {
            true
        })
    );
    p.apply(Event::ArchiveListed {
        pane: PaneId::Left,
        navigation: NavigationId(p.panes[0].generation),
        result: Ok(ArchiveListing {
            location,
            index: Arc::new(DirectoryIndex::from_entries(&entries)),
            entries,
        }),
    });
    select(&mut p, &[2]);
    let next = observe(&mut p);
    assert!(
        next.0 > first.0,
        "the backend pane rejects reused request IDs"
    );
    summary(&mut p, first, 99, 999);
    assert!(p.panes[0].selected_folder_sizes().is_none());
    summary(&mut p, next, 1, 123);
    assert_eq!(
        p.panes[0].selected_folder_sizes().unwrap().complete_bytes,
        Some(123)
    );
}

#[test]
fn running_scan_does_not_mark_unrelated_folders_after_navigation() {
    let mut p = super::archives::archive_pane();
    start(&mut p);
    let folder = p.panes[0].entry_by_id(EntryId(2)).unwrap().clone();
    assert!(p.panes[0].folder_size_running(&folder));
    let mut node = folder.virtual_node.clone().unwrap();
    node.id.entry = 20;
    node.parent = 10;
    let unrelated = Entry::from_archive(node);
    assert!(!p.panes[0].folder_size_running(&unrelated));
}

#[test]
fn flat_folder_summary_is_not_reused_by_an_inline_tree_selection() {
    let mut p = super::archives::archive_pane();
    select(&mut p, &[2]);
    let observer = observe(&mut p);
    summary(&mut p, observer, 1, 42);
    assert!(p.panes[0].selected_folder_sizes().is_some());
    let root = TreeRootSnapshot {
        pane: PaneId::Left,
        navigation: p.panes[0].displayed_generation(),
        entries: p.panes[0].entries.clone(),
        index: p.panes[0].index.clone(),
    };
    let original = root.entries[0].clone();
    let identity = p.panes[0].row_identity(&original);
    p.disclose_tree(PaneId::Left, &identity, &|_| true);
    let mut entry = original.clone();
    entry.id = EntryId(7);
    let stamp = TreeViewStamp {
        pane: PaneId::Left,
        navigation: root.navigation,
        session: TreeSessionId(5),
        revision: TreeRevision(1),
    };
    let view = Arc::new(
        TreeProjection::new(
            root,
            stamp,
            TreeLayoutRevision(1),
            vec![TreeDisplayRow {
                id: TreeRowId(7),
                entry,
                parent: None,
                depth: 0,
                disclosure: TreeDisclosure::Collapsed,
                expanded: false,
                selectable: true,
                navigable: true,
                context_only: false,
                error: None,
                error_details: None,
                original_root_entry: Some(original.id),
                prefix_end: 0,
                search_match: false,
            }],
        )
        .unwrap(),
    );
    let selected = Arc::new(TreeSelectionSnapshot::from_bits(view.clone(), vec![1]).unwrap());
    let publication = TreePublication::new(view, SelectionRevision(1), selected).unwrap();
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(publication))),
        &|_| true,
    );
    assert!(p.panes[0].tree_view().is_some());
    assert!(p.panes[0].selected_folder_sizes().is_none());
    summary(&mut p, observer, 2, 99);
    assert!(p.panes[0].selected_folder_sizes().is_none());
    assert!(
        !p.panel_details(PaneId::Left)
            .iter()
            .any(|field| field.value.contains("42 bytes") || field.value.contains("99 bytes"))
    );
}
