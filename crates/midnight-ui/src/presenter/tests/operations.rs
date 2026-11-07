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
            id: OperationId(1),
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
