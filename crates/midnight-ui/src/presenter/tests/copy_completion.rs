use super::*;

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
    p.apply(Event::Copy(CopyEvent::RootCompleted {
        id,
        source: CompletedCopySource::Native(root.join("first")),
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

#[test]
fn archive_completion_uses_mount_identity_and_waits_for_pending_selection() {
    let mut p = super::archives::archive_pane();
    let id = OperationId(41);
    p.track_copy_selection(id);
    p.panes[0].archive.as_mut().unwrap().selection_pending = true;
    // Identical labels in this fixture have different provider entry identities.
    p.apply(Event::Copy(CopyEvent::RootCompleted {
        id,
        source: CompletedCopySource::Archive(VirtualEntryId {
            mount: 42,
            entry: 3,
        }),
    }));
    p.flush_selection(&|command| {
        assert!(!matches!(command, Command::ArchiveSelect { .. }));
        true
    });
    p.panes[0].archive.as_mut().unwrap().selection_pending = false;
    let sent = RefCell::new(None);
    p.flush_selection(&|command| {
        if matches!(command, Command::ArchiveSelect { .. }) {
            *sent.borrow_mut() = Some(command);
        }
        true
    });
    let Command::ArchiveSelect {
        action: ArchiveSelectionAction::Remove(ids),
        ..
    } = sent.into_inner().unwrap()
    else {
        panic!()
    };
    assert_eq!(ids.as_ref(), &HashSet::from([EntryId(3)]));
    p.apply(Event::Copy(CopyEvent::RootCompleted {
        id,
        source: CompletedCopySource::Archive(VirtualEntryId {
            mount: 777,
            entry: 4,
        }),
    }));
    // Navigating away discards queued removal, even if a different location has the same IDs.
    p.panes[0].archive.as_mut().unwrap().location.directory = 9;
    p.flush_selection(&|command| {
        assert!(!matches!(command, Command::ArchiveSelect { .. }));
        true
    });
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
