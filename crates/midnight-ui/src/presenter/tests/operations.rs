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
                new_name: Some(_),
                ..
            }
        ));
        true
    });
    assert!(p.operations.preview.is_none());
}

#[test]
fn delete_waiting_for_a_native_listing_reports_the_refusal() {
    let mut p = ready();
    let path = test_root().join("other-folder");
    begin(&mut p, PaneId::Left, path.to_str().unwrap());
    assert!(p.visible_directory(PaneId::Left).is_none());

    p.start_operation(
        PaneId::Left,
        MutationKind::Delete(DeleteMode::Trash),
        &|_| panic!("Delete must wait for the current directory snapshot"),
    );
    assert!(!p.has_live_operations());
    let (title, message) = p.desktop.notice.as_ref().expect("Delete refusal");
    assert_eq!(title, "Delete unavailable");
    assert!(message.contains("file list"));
    assert!(p.desktop.notice_error);
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

#[test]
fn completion_refresh_follows_a_pane_that_moved_on_while_its_old_refresh_waited() {
    let mut p = Presenter::new("a".into(), "right".into());
    let first = begin(&mut p, PaneId::Left, "a");
    finish(&mut p, PaneId::Left, first, vec![]);
    let moved = begin(&mut p, PaneId::Left, "b");
    // A completion into A was queued while A was loading; then the pane moved on.
    p.operations
        .refresh
        .push((PaneId::Left, PathBuf::from("a"), None));
    p.queue_operation_refresh(PaneId::Left, Path::new("b"), None);
    assert_eq!(
        p.operations.refresh,
        [(PaneId::Left, PathBuf::from("b"), None)],
        "a newer completion replaces the stale target instead of being skipped"
    );
    let sent = RefCell::new(Vec::new());
    let entries: Vec<Entry> = vec![];
    p.apply_with_sender(
        Event::ListingFinished {
            pane: PaneId::Left,
            navigation: moved,
            index: Arc::new(DirectoryIndex::from_entries(&entries)),
            entries: entries.into(),
        },
        &|command| {
            sent.borrow_mut().push(command);
            true
        },
    );
    assert!(sent.borrow().iter().any(|command| matches!(command,
        Command::Refresh { pane: PaneId::Left, path, .. } if path == Path::new("b"))));
    assert!(p.operations.refresh.is_empty());
}

/// A terminal Rename of the focused entry to `renamed` with one issue.
fn renamed_with_issue(
    p: &mut Presenter,
    state: OperationState,
    renamed: &Path,
    issue: FsError,
) -> (String, String) {
    let completed = u64::from(state == OperationState::CompletedWithErrors);
    p.apply(Event::Operation(OperationEvent::Progress(Arc::new(
        OperationProgress {
            pause_requested: false,
            paused: false,
            discovery: Default::default(),
            id: OperationId(1),
            throughput: Arc::default(),
            kind: MutationKind::Rename,
            state,
            total: 1,
            completed,
            failed: 1,
            processed_entries: completed,
            current_path: None,
            message: issue.to_string(),
            errors: vec![issue],
            source_directory: p.panes[0].path.clone(),
            renamed_to: (completed == 1).then(|| renamed.to_owned()),
        },
    ))));
    p.desktop
        .notice
        .clone()
        .expect("terminal issues are reported")
}

#[test]
fn completed_with_errors_notice_names_completion_and_reveals_the_renamed_entry() {
    let mut p = ready();
    let renamed = p.panes[0].path.join("renamed.txt");
    let stage = p.panes[0].path.join(".midnight-mutation-1-2-3");
    // A completed Rename whose staging folder could not be removed (P2-9).
    let issue = FsError::from_io(
        "remove staging folder",
        &stage,
        io::Error::other(format!(
            "Renamed to {}, but its temporary staging folder remains at {} because it \
             could not be removed: Directory not empty (os error 39). The folder is no \
             longer needed, but another program may have placed entries in it.",
            renamed.display(),
            stage.display()
        )),
    );
    let (title, body) =
        renamed_with_issue(&mut p, OperationState::CompletedWithErrors, &renamed, issue);
    assert_eq!(title, "File operation completed with errors");
    assert!(body.contains("remains at"), "{body}");
    assert!(!body.contains("have not been undone"), "{body}");
    p.refresh_operation_paths(&|_| true);
    let navigation = NavigationId(p.panes[0].generation);
    let mut entries: Vec<Entry> = p.panes[0]
        .entries
        .iter()
        .filter(|entry| entry.id != EntryId(1))
        .cloned()
        .collect();
    entries.push(Entry::new(EntryId(99), renamed, EntryKind::File));
    finish(&mut p, PaneId::Left, navigation, entries);
    assert_eq!(p.panes[0].cursor, Some(EntryId(99)));
    // A Rename that did not complete is still titled as failed.
    let mut p = ready();
    let refused = FsError::from_io(
        "mutate entry",
        &p.panes[0].entries[0].path,
        io::Error::other("Permission denied (os error 13)"),
    );
    let renamed = p.panes[0].path.join("renamed.txt");
    let (title, _) = renamed_with_issue(&mut p, OperationState::Failed, &renamed, refused);
    assert_eq!(title, "File operation failed");
}

#[test]
fn a_new_empty_file_over_a_listed_file_asks_then_replaces() {
    let mut p = ready();
    let existing = p.panes[0]
        .entries
        .iter()
        .find(|entry| entry.kind == EntryKind::File)
        .expect("the fixture lists a file")
        .path
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    p.start_operation(PaneId::Left, MutationKind::CreateFile, &|_| true);
    p.apply(Event::Operation(OperationEvent::Preview(Arc::new(
        OperationPreview {
            id: OperationId(1),
            kind: MutationKind::CreateFile,
            files: 0,
            folders: 0,
            file_bytes: None,
            sample: vec![],
        },
    ))));
    p.confirm_operation("1", existing.clone(), &|_| {
        panic!("an existing name asks first")
    });
    assert!(p.operations.preview.is_some(), "the dialog stays open");
    assert_eq!(
        p.operations.replace_name,
        Some((OperationId(1), existing.clone()))
    );
    let sent = std::cell::RefCell::new(Vec::new());
    p.confirm_operation("1", existing.clone(), &|command| {
        sent.borrow_mut().push(command);
        true
    });
    assert!(matches!(
        sent.borrow().as_slice(),
        [Command::ConfirmOperation {
            id: OperationId(1),
            new_name: Some(name),
            replace_existing: true,
        }] if *name == existing
    ));
    assert!(p.operations.preview.is_none());
    assert!(p.operations.replace_name.is_none());
}
