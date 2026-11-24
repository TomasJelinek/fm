use super::*;
use crate::presenter::copy_selection::LOCAL_DESELECTION;

fn loaded_copy() -> (Presenter, PathBuf) {
    let root = std::env::temp_dir().join("copy-selection 文档");
    let mut p = Presenter::new(root.clone(), root.join("target"));
    let nav = begin(&mut p, PaneId::Left, root.to_str().unwrap());
    finish(
        &mut p,
        PaneId::Left,
        nav,
        vec![
            Entry::new(EntryId(1), root.join("first"), EntryKind::File),
            Entry::new(EntryId(2), root.join("second"), EntryKind::File),
        ],
    );
    set_selected(&mut p, PaneId::Left, &[EntryId(1), EntryId(2)]);
    p.panes[0].selection_rebase_pending = false;
    p.start_copy(
        FileClipboard {
            paths: vec![root.join("first"), root.join("second")],
            intent: FileClipboardIntent::Copy,
        },
        root.join("target"),
        &|_| true,
    );
    (p, root)
}

fn completed(p: &mut Presenter, root: &Path) {
    let id = p.copies.progress[0].id;
    p.apply(Event::Copy(CopyEvent::RootsCompleted {
        id,
        sources: vec![CompletedCopySource::Native(root.join("first"))],
    }));
}

#[test]
fn copy_completion_deselects_only_finished_item_before_job_ends() {
    let (mut p, root) = loaded_copy();
    let cursor = p.panes[0].cursor;
    completed(&mut p, &root);
    p.flush_selection(&|_| true);
    assert!(!p.panes[0].selection_visible(p.panes[0].entry_by_id(EntryId(1)).unwrap()));
    assert!(p.panes[0].selection_visible(p.panes[0].entry_by_id(EntryId(2)).unwrap()));
    assert_eq!(p.panes[0].cursor, cursor);
    assert!(!p.copies.progress[0].state.is_terminal());
}

#[test]
fn copy_completion_retries_command_backpressure() {
    let (mut p, root) = loaded_copy();
    completed(&mut p, &root);
    p.flush_selection(&|_| false);
    p.flush_selection(&|_| true);
    assert!(!p.panes[0].selection_visible(p.panes[0].entry_by_id(EntryId(1)).unwrap()));
}

#[test]
fn copy_completion_does_not_clear_a_different_navigation_context() {
    let (mut p, root) = loaded_copy();
    p.panes[0].selection_context = NavigationId(999);
    completed(&mut p, &root);
    p.flush_selection(&|_| true);
    assert!(p.panes[0].selection_visible(p.panes[0].entry_by_id(EntryId(1)).unwrap()));
}

fn remove_request(p: &mut Presenter) -> SelectionRequest {
    let sent = RefCell::new(None);
    p.flush_selection(&|command| {
        if let Command::Selection(request) = command
            && matches!(request.action, SelectionAction::Remove(_))
        {
            *sent.borrow_mut() = Some(request);
        }
        true
    });
    sent.into_inner()
        .expect("completion must queue idempotent removal")
}

#[test]
fn copy_completion_retries_backend_refusal_and_survives_terminal_progress() {
    let (mut p, root) = loaded_copy();
    completed(&mut p, &root);
    let request = remove_request(&mut p);
    p.apply(Event::SelectionFailed {
        pane: request.pane,
        navigation: request.navigation,
        revision: request.revision,
        error: FsError::from_io(
            "selection",
            &root,
            io::Error::from(io::ErrorKind::WouldBlock),
        ),
    });
    let mut done = (*p.copies.progress[0]).clone();
    done.state = CopyOperationState::Completed;
    p.apply(Event::Copy(CopyEvent::Progress(Arc::new(done))));
    let retried = remove_request(&mut p);
    assert!(retried.revision > request.revision);
    let SelectionAction::Remove(paths) = retried.action else {
        panic!()
    };
    assert_eq!(paths.as_ref(), &HashSet::from([root.join("first")]));
}

#[test]
fn copy_completion_recovers_when_backend_coalesces_rejection_to_later_revision() {
    let (mut p, root) = loaded_copy();
    completed(&mut p, &root);
    let request = remove_request(&mut p);
    assert!(p.send_selection(PaneId::Left, SelectionAction::Rebase, &|_| true));
    let later = p.panes[0].selection_revision;
    assert!(later > request.revision);
    p.apply(Event::SelectionFailed {
        pane: PaneId::Left,
        navigation: request.navigation,
        revision: later,
        error: FsError::from_io(
            "selection",
            &root,
            io::Error::from(io::ErrorKind::WouldBlock),
        ),
    });
    assert!(remove_request(&mut p).revision > later);
}

#[test]
fn acknowledged_completion_does_not_undo_later_user_reselection() {
    let (mut p, root) = loaded_copy();
    completed(&mut p, &root);
    let request = remove_request(&mut p);
    let remaining =
        Arc::new(SelectionSnapshot::from_bits(p.panes[0].index.clone(), vec![2]).unwrap());
    p.apply(Event::SelectionChanged {
        pane: PaneId::Left,
        navigation: request.navigation,
        revision: request.revision,
        selection: remaining,
        deferred: Default::default(),
    });
    set_selected(&mut p, PaneId::Left, &[EntryId(1), EntryId(2)]);
    p.flush_selection(&|command| {
        assert!(!matches!(
            command,
            Command::Selection(SelectionRequest {
                action: SelectionAction::Remove(_),
                ..
            })
        ));
        true
    });
    assert!(p.panes[0].selection_visible(p.panes[0].entry_by_id(EntryId(1)).unwrap()));
}

#[test]
fn completion_uses_raw_paths_after_refresh_recycles_row_ids() {
    let (mut p, root) = loaded_copy();
    completed(&mut p, &root);
    let first = remove_request(&mut p);
    let nav = begin(&mut p, PaneId::Left, root.to_str().unwrap());
    finish(
        &mut p,
        PaneId::Left,
        nav,
        vec![
            Entry::new(EntryId(1), root.join("second"), EntryKind::File),
            Entry::new(EntryId(2), root.join("first"), EntryKind::File),
        ],
    );
    let request = remove_request(&mut p);
    assert_ne!(request.navigation, first.navigation);
    assert!(!p.panes[0].selection_visible(p.panes[0].entry_by_id(EntryId(2)).unwrap()));
    let SelectionAction::Remove(paths) = request.action else {
        panic!()
    };
    assert!(paths.contains(&root.join("first")));
    assert!(!paths.contains(&root.join("second")));
}

fn archive_root_completed(p: &mut Presenter, id: OperationId, mount: u64, entry: u64) {
    p.apply(Event::Copy(CopyEvent::RootsCompleted {
        id,
        sources: vec![CompletedCopySource::Archive(VirtualEntryId {
            mount,
            entry,
        })],
    }));
}

/// The archive selection command a flush sends, if any.
fn flushed_archive_select(p: &mut Presenter) -> Option<Command> {
    let sent = RefCell::new(None);
    p.flush_selection(&|command| {
        if matches!(command, Command::ArchiveSelect { .. }) {
            *sent.borrow_mut() = Some(command);
        }
        true
    });
    sent.into_inner()
}

fn removed_archive_ids(command: &Command) -> HashSet<EntryId> {
    let Command::ArchiveSelect {
        action: ArchiveSelectionAction::Remove(ids),
        ..
    } = command
    else {
        panic!("archive completion must send a removal")
    };
    ids.as_ref().clone()
}

/// Select the archive pane's members `ids` with their totals, as a row toggle does. The
/// returned clone shares the selection, as a copy's captured request does.
fn select_archive_members(p: &mut Presenter, ids: &[u64]) -> Arc<HashSet<EntryId>> {
    let mut totals = ArchiveSelectionTotals::default();
    for id in ids {
        let entry = p.panes[0].entry_by_id(EntryId(*id)).unwrap();
        totals.change(entry.kind, entry.metadata.size, true);
    }
    let a = p.panes[0].archive.as_mut().unwrap();
    a.selected = Arc::new(ids.iter().map(|id| EntryId(*id)).collect());
    a.totals = totals;
    a.selected.clone()
}

fn archive_selected(p: &Presenter) -> HashSet<EntryId> {
    (*p.panes[0].archive.as_ref().unwrap().selected).clone()
}

/// An archive pane whose selection (entries 3 and 4) a copy's captured request shares,
/// so the first completed root (entry 3) is removed by the backend, not on the UI
/// thread. Its answer leaves the panel the only owner of its selection.
fn archive_with_acknowledged_completion() -> (Presenter, OperationId) {
    let mut p = super::archives::archive_pane();
    let id = OperationId(41);
    p.track_copy_selection(id);
    let captured = select_archive_members(&mut p, &[3, 4]);
    p.panes[0].archive.as_mut().unwrap().selection_pending = true;
    // Identical labels in this fixture have different provider entry identities.
    archive_root_completed(&mut p, id, 42, 3);
    assert!(
        flushed_archive_select(&mut p).is_none(),
        "a pending archive selection defers the removal"
    );
    p.panes[0].archive.as_mut().unwrap().selection_pending = false;
    let first = flushed_archive_select(&mut p).expect("a shared selection's removal is sent");
    assert_eq!(removed_archive_ids(&first), HashSet::from([EntryId(3)]));
    assert_eq!(
        archive_selected(&p),
        HashSet::from([EntryId(3), EntryId(4)])
    );
    drop(captured);
    let Command::ArchiveSelect {
        navigation,
        revision,
        view,
        mut totals,
        ..
    } = first
    else {
        unreachable!()
    };
    totals.change(EntryKind::File, Some(5), false);
    p.apply(Event::ArchiveSelection {
        pane: PaneId::Left,
        navigation,
        revision,
        view,
        result: Ok(ArchiveSelectionResult {
            selected: Arc::new(HashSet::from([EntryId(4)])),
            totals,
        }),
    });
    (p, id)
}

#[test]
fn archive_completion_uses_mount_identity_and_waits_for_pending_selection() {
    let (mut p, id) = archive_with_acknowledged_completion();
    // Another mount's entry 4 is not this pane's entry 4.
    archive_root_completed(&mut p, id, 777, 4);
    assert!(flushed_archive_select(&mut p).is_none());
    assert_eq!(archive_selected(&p), HashSet::from([EntryId(4)]));
    archive_root_completed(&mut p, id, 42, 4);
    assert!(
        flushed_archive_select(&mut p).is_none(),
        "the panel's own selection changes in place"
    );
    assert!(
        archive_selected(&p).is_empty(),
        "this mount's completion is removed"
    );
    let totals = p.panes[0].archive.as_ref().unwrap().totals;
    assert_eq!((totals.files(), totals.file_bytes()), (0, Some(0)));
}

#[test]
fn archive_navigation_discards_a_queued_completion_removal() {
    let (mut p, id) = archive_with_acknowledged_completion();
    archive_root_completed(&mut p, id, 42, 4);
    // Navigating away discards the queued removal, even if a different location
    // has the same IDs; the positive control above removes this exact member.
    p.panes[0].archive.as_mut().unwrap().location.directory = 9;
    assert!(flushed_archive_select(&mut p).is_none());
    assert_eq!(archive_selected(&p), HashSet::from([EntryId(4)]));
}

/// Copied members leave a selection that only the panel holds in place, on the UI
/// thread and without a backend round trip, as a row toggle does (WIN-8): the same
/// set, a new revision, and totals reduced by each removed member.
#[test]
fn copied_archive_members_leave_the_panels_own_selection_in_place() {
    let mut p = super::archives::archive_pane();
    let id = OperationId(43);
    p.track_copy_selection(id);
    drop(select_archive_members(&mut p, &[2, 3, 4]));
    let (set, revision) = {
        let a = p.panes[0].archive.as_ref().unwrap();
        (Arc::as_ptr(&a.selected), a.selection_revision)
    };
    p.apply(Event::Copy(CopyEvent::RootsCompleted {
        id,
        sources: [2, 3]
            .map(|entry| CompletedCopySource::Archive(VirtualEntryId { mount: 42, entry }))
            .into(),
    }));
    assert!(flushed_archive_select(&mut p).is_none());
    let a = p.panes[0].archive.as_ref().unwrap();
    assert_eq!(*a.selected, HashSet::from([EntryId(4)]));
    assert!(
        std::ptr::eq(Arc::as_ptr(&a.selected), set),
        "changed in place"
    );
    assert!(a.selection_revision > revision);
    assert_eq!(
        (a.totals.files(), a.totals.folders(), a.totals.file_bytes()),
        (1, 0, Some(5))
    );
    assert!(!p.copies.selection.pending_for(PaneId::Left));
}

/// 100,001 copied members leave a selection that a copy's request shares: one round
/// trip copies the set once, and the rest is removed in place in bounded flushes.
#[test]
fn a_large_shared_archive_selection_is_copied_once_then_changed_in_place() {
    const COUNT: u64 = 100_001;
    let (mut p, id, captured) = large_archive_selection(COUNT);
    p.apply(Event::Copy(CopyEvent::RootsCompleted {
        id,
        sources: (2..COUNT + 2)
            .map(|entry| CompletedCopySource::Archive(VirtualEntryId { mount: 42, entry }))
            .collect(),
    }));
    let first = flushed_archive_select(&mut p).expect("a shared selection goes to the backend");
    let Command::ArchiveSelect {
        navigation,
        revision,
        view,
        action: ArchiveSelectionAction::Remove(ids),
        mut totals,
        ..
    } = first
    else {
        unreachable!()
    };
    let mut next = (*captured).clone();
    for id in ids.iter() {
        next.remove(id);
        totals.change(EntryKind::File, Some(1), false);
    }
    drop(captured);
    p.apply(Event::ArchiveSelection {
        pane: PaneId::Left,
        navigation,
        revision,
        view,
        result: Ok(ArchiveSelectionResult {
            selected: Arc::new(next),
            totals,
        }),
    });
    let mut flushes = 0;
    while p.copies.selection.pending_for(PaneId::Left) {
        assert!(
            flushed_archive_select(&mut p).is_none(),
            "an unshared selection is changed in place"
        );
        flushes += 1;
        assert!(flushes <= COUNT as usize / LOCAL_DESELECTION + 1);
    }
    let a = p.panes[0].archive.as_ref().unwrap();
    assert!(a.selected.is_empty());
    assert_eq!((a.totals.files(), a.totals.file_bytes()), (0, Some(0)));
}

#[test]
fn archive_navigation_gets_a_new_selection_context_even_when_returning() {
    let mut p = super::archives::archive_pane();
    let old = p.panes[0].selection_context;
    let location = p.panes[0].archive.as_ref().unwrap().location.clone();
    let mut child = location.clone();
    child.directory = 2;
    child.trail.push((2, "folder".into()));
    child.address.components.push(b"folder".to_vec());
    assert!(p.archive_navigate(PaneId::Left, ArchiveAction::List(child.clone()), &|_| true));
    assert_ne!(p.panes[0].selection_context, old);
    p.panes[0].archive.as_mut().unwrap().location = child;
    p.panes[0].loading = false;
    assert!(p.archive_navigate(PaneId::Left, ArchiveAction::List(location), &|_| true));
    assert_ne!(p.panes[0].selection_context, old);
}

#[test]
fn return_from_archive_does_not_revive_old_native_completion_context() {
    let (mut p, root) = loaded_copy();
    let old = p.panes[0].selection_context;
    p.panes[0].archive = super::archives::archive_pane().panes[0].archive.take();
    let nav = begin(&mut p, PaneId::Left, root.to_str().unwrap());
    finish(
        &mut p,
        PaneId::Left,
        nav,
        vec![
            Entry::new(EntryId(1), root.join("first"), EntryKind::File),
            Entry::new(EntryId(2), root.join("second"), EntryKind::File),
        ],
    );
    assert_ne!(p.panes[0].selection_context, old);
    set_selected(&mut p, PaneId::Left, &[EntryId(1), EntryId(2)]);
    completed(&mut p, &root);
    p.flush_selection(&|command| {
        assert!(!matches!(
            command,
            Command::Selection(SelectionRequest {
                action: SelectionAction::Remove(_),
                ..
            })
        ));
        true
    });
    assert!(p.panes[0].selection_visible(p.panes[0].entry_by_id(EntryId(1)).unwrap()));
}

#[derive(Debug)]
struct CompletionMount;
impl ArchiveMount for CompletionMount {
    fn id(&self) -> u64 {
        42
    }
    fn list(&self, _: u64, _: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        panic!("UI performed listing")
    }
    fn read(&self, _: u64, _: u64, _: u32, _: &Cancellation) -> Result<Vec<u8>, FsError> {
        panic!("UI performed reading")
    }
    fn validate(&self) -> Result<(), FsError> {
        Ok(())
    }
}

/// An archive pane listing `count` files (entries 2, 3, ...), all of them selected, and
/// a copy of them tracked for completion.
fn large_archive_selection(count: u64) -> (Presenter, OperationId, Arc<HashSet<EntryId>>) {
    let mut p = Presenter::new(test_root(), test_root());
    let path = p.panes[0].path.to_string_lossy().into_owned();
    let nav = begin(&mut p, PaneId::Left, &path);
    finish(&mut p, PaneId::Left, nav, vec![]);
    let entries: Arc<[Entry]> = (0..count)
        .map(|index| {
            Entry::from_archive(ArchiveNode {
                modified: None,
                id: VirtualEntryId {
                    mount: 42,
                    entry: index + 2,
                },
                parent: 1,
                raw_name: format!("{index:06}.txt").into_bytes().into(),
                display_name: format!("{index:06}.txt"),
                kind: EntryKind::File,
                size: Some(1),
                readable: true,
                access: Default::default(),
            })
        })
        .collect::<Vec<_>>()
        .into();
    let mount: Arc<dyn ArchiveMount> = Arc::new(CompletionMount);
    p.apply(Event::ArchiveListed {
        pane: PaneId::Left,
        navigation: nav,
        result: Ok(ArchiveListing {
            location: ArchiveLocation {
                mount: Some(mount),
                address: ArchiveAddress {
                    nested: Vec::new(),
                    format: None,
                    provider: "midnight.archive".into(),
                    container: test_root().join("flat.tar"),
                    source_version: vec![1],
                    components: vec![],
                },
                directory: 1,
                trail: vec![(1, "flat.tar".into())],
            },
            index: Arc::new(DirectoryIndex::from_entries(&entries)),
            entries: entries.clone(),
        }),
    });
    let selected: Arc<HashSet<EntryId>> = Arc::new(entries.iter().map(|entry| entry.id).collect());
    p.panes[0].archive.as_mut().unwrap().selected = selected.clone();
    let id = OperationId(77);
    p.track_copy_selection(id);
    (p, id, selected)
}

/// Manual measurement (WIN-8): what the presenter does for each copied member of a
/// selection of 100,001: the completion notices, and the selection edits that
/// deselect the copied members (the backend's answers are prepared outside the timer).
/// `cargo test -p midnight-ui --lib completing_a_large_archive_selection -- --ignored --nocapture`
#[test]
#[ignore = "manual measurement"]
fn completing_a_large_archive_selection() {
    const COUNT: u64 = 100_001;
    for batch in [1u64, 512] {
        let (mut p, id, _) = large_archive_selection(COUNT);
        let started = std::time::Instant::now();
        let mut entry = 2;
        while entry < COUNT + 2 {
            let end = (entry + batch).min(COUNT + 2);
            p.apply(Event::Copy(CopyEvent::RootsCompleted {
                id,
                sources: (entry..end)
                    .map(|entry| CompletedCopySource::Archive(VirtualEntryId { mount: 42, entry }))
                    .collect(),
            }));
            entry = end;
        }
        let applied = started.elapsed();
        println!(
            "PERF presenter: {COUNT} completions in notices of {batch} applied in {:.0} ms              ({:.2} us per completion)",
            applied.as_secs_f64() * 1000.0,
            applied.as_secs_f64() * 1e6 / COUNT as f64
        );
    }
    // Deselection, with the selection-size observer keeping the selection it was last
    // sent (as the backend's size lane does) or not: a shared selection costs a round
    // trip that copies it once (prepared here outside the timer); the panel's own
    // selection changes in place.
    for observer in [false, true] {
        let (mut p, id, captured) = large_archive_selection(COUNT);
        drop(captured);
        for entry in 2..COUNT + 2 {
            archive_root_completed(&mut p, id, 42, entry);
        }
        let observed = RefCell::new(None::<Arc<HashSet<EntryId>>>);
        let (mut flushes, mut trips) = (0u64, 0u64);
        let mut flushing = std::time::Duration::ZERO;
        while p.copies.selection.pending_for(PaneId::Left) {
            let sent = RefCell::new(None);
            let timer = std::time::Instant::now();
            p.flush_selection(&|command| {
                match command {
                    command @ Command::ArchiveSelect { .. } => *sent.borrow_mut() = Some(command),
                    Command::ArchiveSizes(ArchiveSizeCommand::Observe { selected, .. })
                        if observer =>
                    {
                        *observed.borrow_mut() = Some(selected)
                    }
                    _ => {}
                }
                true
            });
            flushing += timer.elapsed();
            flushes += 1;
            let Some(Command::ArchiveSelect {
                navigation,
                revision,
                view,
                selected,
                mut totals,
                action: ArchiveSelectionAction::Remove(ids),
                ..
            }) = sent.into_inner()
            else {
                continue;
            };
            trips += 1;
            let mut next = (*selected).clone();
            drop(selected);
            for id in ids.iter() {
                next.remove(id);
                totals.change(EntryKind::File, Some(1), false);
            }
            p.apply(Event::ArchiveSelection {
                pane: PaneId::Left,
                navigation,
                revision,
                view,
                result: Ok(ArchiveSelectionResult {
                    selected: Arc::new(next),
                    totals,
                }),
            });
        }
        assert!(p.panes[0].archive.as_ref().unwrap().selected.is_empty());
        println!(
            "PERF presenter: deselecting {COUNT} copied members (size observer kept: \
             {observer}): {flushes} flushes, {trips} backend copies of the set, \
             {:.1} ms in flushes ({:.3} ms per flush)",
            flushing.as_secs_f64() * 1000.0,
            flushing.as_secs_f64() * 1000.0 / flushes.max(1) as f64
        );
    }
}
