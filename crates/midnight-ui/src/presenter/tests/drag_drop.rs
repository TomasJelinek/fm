use super::*;

fn loaded() -> (Presenter, PathBuf) {
    let base = test_root().join("drag 文档 & $(literal)");
    let mut presenter = Presenter::new(base.clone(), test_root().join("target"));
    let navigation = begin(&mut presenter, PaneId::Left, base.to_str().unwrap());
    finish(
        &mut presenter,
        PaneId::Left,
        navigation,
        vec![
            Entry::new(EntryId(1), base.join("one"), EntryKind::File),
            Entry::new(EntryId(2), base.join("two"), EntryKind::File),
            Entry::new(EntryId(3), base.join("three"), EntryKind::File),
        ],
    );
    (presenter, base)
}

fn identity(presenter: &Presenter, entry: u64) -> String {
    let p = &presenter.panes[0];
    format!(
        "{}:{}:{}:entry:{entry}",
        p.generation,
        p.displayed_generation().0,
        p.search.view_revision
    )
}

#[test]
fn dragging_selected_group_preserves_selection_and_unselected_row_targets_itself() {
    let (mut p, base) = loaded();
    set_selected(&mut p, PaneId::Left, &[EntryId(1), EntryId(2)]);
    let selected = p.panes[0].selected.clone();
    let commands = RefCell::new(Vec::new());
    let send = |command| {
        commands.borrow_mut().push(command);
        true
    };
    assert!(p.begin_file_drag(PaneId::Left, 2, &identity(&p, 2), &send));
    let Command::PrepareDrag { id, files, .. } = commands.borrow_mut().remove(0) else {
        panic!("selected group must prepare on the worker");
    };
    assert!(Arc::ptr_eq(&files.selection, &selected));
    assert_eq!(files.fallback.as_ref(), Some(&base.join("two")));
    p.cancel_file_drag(&send);
    commands.borrow_mut().clear();
    assert!(p.begin_file_drag(PaneId::Left, 3, &identity(&p, 3), &send));
    let Command::PrepareDrag {
        id: next, files, ..
    } = commands.borrow_mut().remove(0)
    else {
        panic!("unselected row must prepare itself");
    };
    assert_ne!(next, id);
    assert!(files.selection.is_empty());
    assert_eq!(files.fallback.as_ref(), Some(&base.join("three")));
    assert!(Arc::ptr_eq(&p.panes[0].selected, &selected));
}

#[test]
fn drag_waits_for_exact_pending_selection_then_rejects_late_preparation_after_release() {
    let (mut p, _) = loaded();
    let commands = RefCell::new(Vec::new());
    let send = |command| {
        commands.borrow_mut().push(command);
        true
    };
    p.bulk_selection(PaneId::Left, SelectionOperation::All, &send);
    let Command::Selection(all) = commands.borrow_mut().remove(0) else {
        panic!()
    };
    assert!(p.begin_file_drag(PaneId::Left, 1, &identity(&p, 1), &send));
    assert!(commands.borrow().is_empty());
    p.apply_with_sender(
        Event::SelectionChanged {
            pane: PaneId::Left,
            navigation: all.navigation,
            revision: all.revision,
            selection: Arc::new(SelectionSnapshot::from_bits(all.view.unwrap(), vec![7]).unwrap()),
            deferred: false,
        },
        &send,
    );
    let id = commands
        .borrow()
        .iter()
        .find_map(|command| match command {
            Command::PrepareDrag { id, files, .. } => {
                assert_eq!(files.selection.len(), 3);
                Some(*id)
            }
            _ => None,
        })
        .unwrap();
    p.release_file_drag(None, &send);
    p.apply_with_sender(
        Event::DragDrop(DragDropEvent::Prepared { id, item_count: 3 }),
        &send,
    );
    assert!(
        p.drag_native_ready().is_none(),
        "release must disarm delayed preparation"
    );
    assert!(
        !commands
            .borrow()
            .iter()
            .any(|command| matches!(command, Command::StartDrag(_)))
    );
}

#[test]
fn accepted_drop_retries_one_operation_identity_and_keeps_captured_destination() {
    let (mut p, _) = loaded();
    let destination = Arc::new(test_root().join("captured target"));
    let event = Event::DragDrop(DragDropEvent::DropAccepted {
        token: DropToken(7),
        pane: PaneId::Right,
        item_count: 2,
        destination: destination.clone(),
    });
    p.apply_with_sender(event.clone(), &|_| false);
    assert_eq!(p.copies.progress.len(), 1);
    let id = p.copies.progress[0].id;
    assert_eq!(p.copies.progress[0].destination_directory, *destination);
    p.apply_with_sender(event, &|_| false);
    assert_eq!(
        p.copies.progress.len(),
        1,
        "replayed token cannot create another UI job"
    );
    p.panes[1].path = test_root().join("new navigation");
    let sent = RefCell::new(Vec::new());
    p.flush_drag_commands(&|command| {
        sent.borrow_mut().push(command);
        true
    });
    let sent = sent.borrow();
    let [Command::AcceptDrop { token, id: actual }] = sent.as_slice() else {
        panic!("one admitted drop must enqueue exactly one handoff");
    };
    assert_eq!(*token, DropToken(7));
    assert_eq!(*actual, id);
    assert_eq!(p.copies.progress[0].destination_directory, *destination);
    p.flush_drag_commands(&|_| panic!("accepted handoff must not repeat"));
}

#[test]
fn stale_rows_parent_and_changed_projection_never_prepare_a_drag() {
    let (mut p, _) = loaded();
    let token = identity(&p, 1);
    assert!(!p.begin_file_drag(PaneId::Left, 0, &token, &|_| panic!()));
    p.panes[0].search.view_revision += 1;
    assert!(!p.begin_file_drag(PaneId::Left, 1, &token, &|_| panic!()));
    assert!(!p.begin_file_drag(PaneId::Left, usize::MAX, &token, &|_| panic!()));
}

#[test]
fn exhausted_operation_ids_reject_an_admitted_drop_without_creating_a_job() {
    let (mut p, _) = loaded();
    p.operations.next_id = u64::MAX;
    let event = Event::DragDrop(DragDropEvent::DropAccepted {
        token: DropToken(7),
        pane: PaneId::Right,
        item_count: 1,
        destination: Arc::new(test_root().join("captured target")),
    });
    let sent = RefCell::new(Vec::new());
    p.apply_with_sender(event.clone(), &|command| {
        sent.borrow_mut().push(command);
        true
    });
    assert!(p.copies.progress.is_empty());
    assert_eq!(p.operations.next_id, u64::MAX);
    assert!(matches!(
        sent.borrow().as_slice(),
        [Command::RejectDrop(DropToken(7))]
    ));
    p.apply_with_sender(event, &|_| panic!("a replayed token must stay retired"));
}

#[test]
fn unavailable_native_gesture_keeps_the_next_gesture_eligible_for_native_dragging() {
    let (mut p, _) = loaded();
    let sent = RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    p.drag_drop.supported = Some(true);
    let mut previous = None;
    for attempt in 0..2 {
        assert!(p.begin_file_drag(PaneId::Left, 1, &identity(&p, 1), &send));
        let id = sent
            .borrow()
            .iter()
            .rev()
            .find_map(|command| match command {
                Command::PrepareDrag { id, .. } => Some(*id),
                _ => None,
            })
            .unwrap();
        assert_ne!(Some(id), previous);
        p.apply_with_sender(
            Event::DragDrop(DragDropEvent::Prepared { id, item_count: 1 }),
            &send,
        );
        assert_eq!(p.drag_native_ready(), Some(id), "native attempt {attempt}");
        assert!(p.arm_native_drag(id));
        p.apply_with_sender(
            Event::DragDrop(DragDropEvent::Finished {
                id,
                outcome: DragFinish::Unavailable,
            }),
            &send,
        );
        assert_eq!(p.drag_native_ready(), None, "do not retry the same gesture");
        p.release_file_drag(None, &send);
        previous = Some(id);
    }
}
