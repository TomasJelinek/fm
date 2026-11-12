use super::*;

fn ready() -> Presenter {
    let path = test_root().join("operations");
    let mut p = Presenter::new(path.clone(), path.clone());
    let generation = begin(&mut p, PaneId::Left, path.to_str().unwrap());
    finish(
        &mut p,
        PaneId::Left,
        generation,
        vec![
            Entry::new(EntryId(1), path.join("文档.txt"), EntryKind::File),
            Entry::new(EntryId(2), path.join("second.txt"), EntryKind::File),
        ],
    );
    p.panes[0].parent_cursor = false;
    p.panes[0].cursor = Some(EntryId(1));
    p
}
#[test]
fn parent_row_is_navigation_only_but_real_selection_can_be_deleted() {
    let mut p = ready();
    p.panes[0].parent_cursor = true;
    p.panes[0].cursor = None;
    for kind in [
        MutationKind::Rename,
        MutationKind::Delete(DeleteMode::Trash),
        MutationKind::Delete(DeleteMode::Permanent),
        MutationKind::EditMetadata,
    ] {
        p.start_operation(PaneId::Left, kind, &|_| {
            panic!("parent must not queue a job")
        });
    }
    p.copy_files(PaneId::Left, &|_| panic!("parent must not enter clipboard"));
    p.request_copy_dialog(PaneId::Left, &|_| panic!("parent must not be copied"));
    p.open_metadata(PaneId::Left, false, &|_| {
        panic!("parent cannot edit metadata")
    });
    assert!(!p.has_live_operations());
    set_selected(&mut p, PaneId::Left, &[EntryId(2)]);
    let selected_path = p.panes[0].entries[1].path.clone();
    p.start_operation(
        PaneId::Left,
        MutationKind::Delete(DeleteMode::Trash),
        &|command| {
            let Command::PrepareOperation(request) = command else {
                panic!()
            };
            assert_eq!(request.focused, None);
            assert_eq!(request.selection.len(), 1);
            assert!(request.selection.contains_path(&selected_path));
            true
        },
    );
}

#[test]
fn parent_navigation_keeps_location_based_creation_available() {
    for kind in [MutationKind::CreateDirectory, MutationKind::CreateFile] {
        let mut p = ready();
        p.panes[0].parent_cursor = true;
        p.panes[0].cursor = None;
        p.start_operation(PaneId::Left, kind, &|command| {
            let Command::PrepareOperation(request) = command else {
                panic!()
            };
            assert_eq!(request.kind, kind);
            assert!(request.focused.is_none());
            assert!(request.selection.is_empty());
            true
        });
        assert!(p.has_live_operations());
    }
}

#[test]
fn rename_captures_focus_and_delete_captures_selection_without_mutating() {
    for kind in [
        MutationKind::Rename,
        MutationKind::Delete(DeleteMode::Trash),
        MutationKind::CreateDirectory,
        MutationKind::CreateFile,
    ] {
        let mut p = ready();
        set_selected(&mut p, PaneId::Left, &[EntryId(2)]);
        let commands = RefCell::new(Vec::new());
        p.start_operation(PaneId::Left, kind, &|command| {
            commands.borrow_mut().push(command);
            true
        });
        let commands = commands.borrow();
        let Command::PrepareOperation(request) = &commands[0] else {
            panic!()
        };
        assert_eq!(request.kind, kind);
        assert_eq!(request.focused, Some(EntryId(1)));
        assert_eq!(request.selection.len(), 1);
        assert!(p.has_live_operations());
        assert!(p.operations.preview.is_none());
    }
}

#[test]
fn empty_file_creation_does_not_require_a_focused_entry() {
    let path = test_root().join("empty");
    let mut p = Presenter::new(path.clone(), path.clone());
    let navigation = begin(&mut p, PaneId::Left, path.to_str().unwrap());
    finish(&mut p, PaneId::Left, navigation, vec![]);
    let commands = RefCell::new(Vec::new());
    p.start_operation(PaneId::Left, MutationKind::CreateFile, &|command| {
        commands.borrow_mut().push(command);
        true
    });
    let commands = commands.borrow();
    let Command::PrepareOperation(request) = &commands[0] else {
        panic!()
    };
    assert_eq!(request.kind, MutationKind::CreateFile);
    assert_eq!(request.source.path, path);
    assert!(request.source.entries.is_empty());
    assert!(request.focused.is_none());
    assert!(p.has_live_operations());
}

fn created_file_refresh(p: &mut Presenter, destination: PathBuf) {
    created_entry_refresh(p, destination, MutationKind::CreateFile);
}

fn created_entry_refresh(p: &mut Presenter, destination: PathBuf, kind: MutationKind) {
    p.apply(Event::Operation(OperationEvent::Progress(Arc::new(
        OperationProgress {
            pause_requested: false,
            paused: false,
            discovery: Default::default(),
            id: OperationId(1),
            throughput: Arc::default(),
            kind,
            state: OperationState::Completed,
            total: 1,
            completed: 1,
            failed: 0,
            processed_entries: 1,
            current_path: None,
            message: String::new(),
            errors: vec![],
            source_directory: p.panes[0].path.clone(),
            renamed_to: Some(destination),
        },
    ))));
    p.refresh_operation_paths(&|_| true);
}

#[test]
fn created_external_directory_keeps_panel_path_search_and_caret_during_refresh() {
    let mut p = ready();
    p.preferences.values.quick_search_filter = true;
    p.change_search_query(PaneId::Left, "second", &|_| true);
    let source = p.panes[0].path.clone();
    let cursor = p.panes[0].cursor;
    created_entry_refresh(
        &mut p,
        test_root().join("elsewhere/new"),
        MutationKind::CreateDirectory,
    );
    assert_eq!(p.panes[0].path, source);
    assert_eq!(p.panes[0].search.query, "second");
    assert_eq!(p.panes[0].cursor, cursor);
    assert!(p.panes[0].preserve_viewport);
}

#[test]
fn created_file_is_focused_and_revealed_after_refresh_from_existing_or_parent_cursor() {
    for parent in [false, true] {
        let mut p = ready();
        p.panes[0].parent_cursor = parent;
        if parent {
            p.panes[0].cursor = None;
        }
        let target = p.panes[0].path.join("zz-new.txt");
        created_file_refresh(&mut p, target.clone());
        let navigation = NavigationId(p.panes[0].generation);
        let mut entries = p.panes[0].entries.to_vec();
        entries.push(Entry::new(EntryId(99), target, EntryKind::File));
        finish(&mut p, PaneId::Left, navigation, entries);
        assert_eq!(p.panes[0].cursor, Some(EntryId(99)));
        assert!(!p.panes[0].parent_cursor);
        assert!(
            !p.panes[0].preserve_viewport,
            "new file must be scrolled into view"
        );
    }
}

#[test]
fn created_file_reveal_leaves_search_but_respects_a_new_cursor_choice_during_refresh() {
    let mut p = ready();
    p.preferences.values.quick_search_filter = true;
    p.change_search_query(PaneId::Left, "second", &|_| true);
    let target = p.panes[0].path.join("zz-new.txt");
    created_file_refresh(&mut p, target.clone());
    assert!(
        p.panes[0].search.query.is_empty(),
        "an old filter must not hide the new file"
    );
    let navigation = NavigationId(p.panes[0].generation);
    // A user moving the caret after refresh starts wins over automatic reveal.
    p.panes[0].cursor = Some(EntryId(2));
    p.panes[0].parent_cursor = false;
    p.panes[0].pending.as_mut().unwrap().cursor_touched = true;
    let mut entries = p.panes[0].entries.to_vec();
    entries.push(Entry::new(EntryId(99), target, EntryKind::File));
    finish(&mut p, PaneId::Left, navigation, entries);
    assert_eq!(p.panes[0].cursor, Some(EntryId(2)));
}
#[test]
fn stale_confirmation_and_failed_enqueue_do_not_confirm_another_operation() {
    let mut p = ready();
    p.start_operation(PaneId::Left, MutationKind::Rename, &|_| false);
    assert!(!p.has_live_operations());
    p.start_operation(PaneId::Left, MutationKind::Rename, &|_| true);
    p.apply(Event::Operation(OperationEvent::Preview(Arc::new(
        OperationPreview {
            id: OperationId(1),
            kind: MutationKind::Rename,
            files: 1,
            folders: 0,
            file_bytes: None,
            sample: vec![MutationItem {
                path: p.panes[0].entries[0].path.clone(),
                kind: EntryKind::File,
                metadata: Default::default(),
            }],
        },
    ))));
    p.confirm_operation("999", "new.txt".into(), &|_| panic!("stale callback"));
    assert!(p.operations.preview.is_some());
    p.confirm_operation("1", "new.txt".into(), &|_| false);
    assert!(p.operations.preview.is_some());
    p.confirm_operation("1", "new.txt".into(), &|command| {
        assert!(matches!(
            command,
            Command::ConfirmOperation {
                id: OperationId(1),
                new_name: Some(_)
            }
        ));
        true
    });
    assert!(p.operations.preview.is_none());
}

#[test]
fn jobs_cancelled_delete_confirmation_is_hidden_until_worker_retires_and_never_becomes_history() {
    for before_preview in [true, false] {
        let mut p = ready();
        p.start_operation(
            PaneId::Left,
            MutationKind::Delete(DeleteMode::Permanent),
            &|_| true,
        );
        let id = p.operations.pending.unwrap();
        assert!(
            p.has_live_operations(),
            "shutdown must still cancel preparation"
        );
        assert!(crate::job_presentation::rows(&p).is_empty());
        if !before_preview {
            let progress = Arc::make_mut(&mut p.operations.progress[0]);
            progress.state = OperationState::AwaitingConfirmation;
            p.apply(Event::Operation(OperationEvent::Preview(Arc::new(
                OperationPreview {
                    id,
                    kind: MutationKind::Delete(DeleteMode::Permanent),
                    files: 1,
                    folders: 0,
                    file_bytes: None,
                    sample: vec![],
                },
            ))));
        }
        p.cancel_operation(
            &id.0.to_string(),
            &|command| matches!(command, Command::CancelOperation(actual) if actual == id),
        );
        let mut done = (*p.operations.progress[0]).clone();
        done.state = OperationState::Cancelled;
        p.apply(Event::Operation(OperationEvent::Progress(Arc::new(
            done.clone(),
        ))));
        p.apply(Event::Operation(OperationEvent::Progress(Arc::new(done))));
        assert!(crate::job_presentation::rows(&p).is_empty());
        assert!(p.operations.unconfirmed.is_empty());
        assert!(!p.has_live_operations());
        assert!(
            p.operations.refresh.is_empty(),
            "an unstarted cancellation should not refresh panels"
        );
    }
}

#[test]
fn jobs_preparation_errors_and_cancelled_confirmed_delete_remain_visible() {
    for accepted in [false, true] {
        let mut p = ready();
        p.start_operation(
            PaneId::Left,
            MutationKind::Delete(DeleteMode::Permanent),
            &|_| true,
        );
        let id = p.operations.pending.unwrap();
        p.apply(Event::Operation(OperationEvent::Preview(Arc::new(
            OperationPreview {
                id,
                kind: MutationKind::Delete(DeleteMode::Permanent),
                files: 1,
                folders: 0,
                file_bytes: None,
                sample: vec![],
            },
        ))));
        if accepted {
            p.confirm_operation(&id.0.to_string(), String::new(), &|_| true);
        }
        let mut done = (*p.operations.progress[0]).clone();
        if accepted {
            done.state = OperationState::Cancelled;
        } else {
            done.state = OperationState::Failed;
            done.failed = 1;
            done.errors.push(FsError::from_io(
                "prepare",
                &done.source_directory,
                io::Error::other("Cannot inspect selected file"),
            ));
        }
        p.apply(Event::Operation(OperationEvent::Progress(Arc::new(done))));
        assert_eq!(crate::job_presentation::rows(&p).len(), 1);
        assert!(p.operations.unconfirmed.is_empty());
        if !accepted {
            assert!(p.desktop.notice.is_some());
        }
    }
}
