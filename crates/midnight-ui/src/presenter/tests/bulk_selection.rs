use super::*;

fn loaded() -> Presenter {
    let mut presenter = Presenter::new("left".into(), "right".into());
    let navigation = begin(&mut presenter, PaneId::Left, "left");
    finish(
        &mut presenter,
        PaneId::Left,
        navigation,
        vec![entry(1, "left/a"), entry(2, "left/b"), entry(3, "left/c")],
    );
    presenter
}

fn sent_selection(commands: &RefCell<Vec<Command>>) -> SelectionRequest {
    match commands.borrow_mut().pop().expect("selection command") {
        Command::Selection(request) => request,
        other => panic!("expected selection, got {other:?}"),
    }
}

fn ack(
    request: &SelectionRequest,
    basis: Arc<DirectoryIndex>,
    bits: Vec<u64>,
    deferred: bool,
) -> Event {
    Event::SelectionChanged {
        pane: request.pane,
        navigation: request.navigation,
        revision: request.revision,
        selection: Arc::new(SelectionSnapshot::from_bits(basis, bits).unwrap()),
        deferred,
    }
}

#[test]
fn bulk_work_is_queued_without_changing_cursor_and_stale_snapshots_are_retired() {
    let mut p = loaded();
    set_selected(&mut p, PaneId::Left, &[EntryId(2)]);
    let previous = p.panes[0].selected.clone();
    let cursor = p.panes[0].cursor;
    let commands = RefCell::new(Vec::new());
    let send = |command| {
        commands.borrow_mut().push(command);
        true
    };
    assert!(p.bulk_selection(PaneId::Left, SelectionOperation::All, &send));
    let all = sent_selection(&commands);
    assert!(Arc::ptr_eq(&all.seed, &previous));
    assert!(Arc::ptr_eq(all.view.as_ref().unwrap(), &p.panes[0].index));
    assert_eq!(
        p.panes[0].selected.len(),
        1,
        "only the backend commits selection"
    );
    assert_eq!(p.panes[0].cursor, cursor);
    p.apply(ack(&all, p.panes[0].index.clone(), vec![7], false));
    assert_eq!(p.panes[0].selected.len(), 3);
    assert!(p.panes[1].selected.is_empty());
    assert!(!p.bulk_selection(PaneId::Left, SelectionOperation::Clear, &|_| false));
    assert_eq!(
        p.panes[0].selected.len(),
        3,
        "queue refusal preserves visible selection"
    );
    assert!(p.bulk_selection(PaneId::Left, SelectionOperation::Clear, &send));
    let clear = sent_selection(&commands);
    p.apply(ack(&clear, p.panes[0].index.clone(), vec![0], false));
    p.apply(ack(&all, p.panes[0].index.clone(), vec![7], false));
    assert!(
        p.panes[0].selected.is_empty(),
        "old completion cannot undo a clear"
    );
    assert!(!p.take_retired().is_empty());
}

#[test]
fn properties_capture_exact_pending_all_even_if_clear_is_already_requested() {
    let mut p = loaded();
    let commands = RefCell::new(Vec::new());
    let send = |command| {
        commands.borrow_mut().push(command);
        true
    };
    p.bulk_selection(PaneId::Left, SelectionOperation::All, &send);
    let all = sent_selection(&commands);
    p.properties(PaneId::Left, &send);
    assert!(
        commands.borrow().is_empty(),
        "Properties waits for the intended revision"
    );
    p.bulk_selection(PaneId::Left, SelectionOperation::Clear, &send);
    let clear = sent_selection(&commands);
    p.apply_with_sender(ack(&all, p.panes[0].index.clone(), vec![7], false), &send);
    match commands.borrow_mut().pop().unwrap() {
        Command::ShowSelectionProperties {
            selection, view, ..
        } => {
            assert_eq!(selection.len(), 3);
            assert!(selection.contains_path(Path::new("left/a")));
            assert!(view.by_path.contains_key(Path::new("left/c")));
        }
        other => panic!("expected captured Properties, got {other:?}"),
    }
    p.apply_with_sender(ack(&clear, p.panes[0].index.clone(), vec![0], false), &send);
    assert!(p.panes[0].selected.is_empty());
    assert!(p.selection_properties[0].is_none());
}

#[test]
fn deferred_streaming_bulk_cannot_open_partial_properties_during_rebase_backpressure() {
    let mut p = Presenter::new("left".into(), "right".into());
    let navigation = begin(&mut p, PaneId::Left, "left");
    p.apply(Event::EntriesBatch {
        pane: PaneId::Left,
        navigation,
        entries: vec![entry(1, "left/a")],
    });
    let commands = RefCell::new(Vec::new());
    let send = |command| {
        commands.borrow_mut().push(command);
        true
    };
    p.bulk_selection(PaneId::Left, SelectionOperation::All, &send);
    let all = sent_selection(&commands);
    p.apply(ack(&all, Arc::new(DirectoryIndex::default()), vec![], true));
    p.properties(PaneId::Left, &send);
    assert!(commands.borrow().is_empty());
    finish(
        &mut p,
        PaneId::Left,
        navigation,
        vec![entry(1, "left/a"), entry(2, "left/b")],
    );
    p.panes[0].selection_rebase_pending = true;
    p.flush_selection(&|_| false);
    p.properties(PaneId::Left, &send);
    assert!(
        commands.borrow().is_empty(),
        "committed listing is not a completed selection"
    );
    p.flush_selection(&send);
    let rebase = sent_selection(&commands);
    p.apply(ack(&rebase, p.panes[0].index.clone(), vec![3], false));
    p.properties(PaneId::Left, &send);
    assert!(matches!(
        commands.borrow_mut().pop(),
        Some(Command::ShowSelectionProperties { .. })
    ));
}

#[test]
fn pending_streamed_toggle_properties_use_returned_selection_basis() {
    let mut p = Presenter::new("left".into(), "right".into());
    let navigation = begin(&mut p, PaneId::Left, "left");
    p.apply(Event::EntriesBatch {
        pane: PaneId::Left,
        navigation,
        entries: vec![entry(55, "left/文档")],
    });
    let commands = RefCell::new(Vec::new());
    let send = |command| {
        commands.borrow_mut().push(command);
        true
    };
    p.select_row(PaneId::Left, 0, true, &send);
    let toggle = sent_selection(&commands);
    p.properties(PaneId::Left, &send);
    let subset = Arc::new(DirectoryIndex::from_entries(p.panes[0].rows()));
    p.apply_with_sender(ack(&toggle, subset, vec![1], false), &send);
    match commands.borrow_mut().pop().unwrap() {
        Command::ShowSelectionProperties {
            selection, view, ..
        } => {
            assert!(selection.contains_path(Path::new("left/文档")));
            assert!(view.by_path.contains_key(Path::new("left/文档")));
        }
        other => panic!("expected Properties from returned basis, got {other:?}"),
    }
}

#[test]
fn navigation_cancels_waiting_properties_and_same_path_new_tab_has_new_context() {
    let mut p = loaded();
    let original_context = p.panes[0].selection_context;
    let commands = RefCell::new(Vec::new());
    let send = |command| {
        commands.borrow_mut().push(command);
        true
    };
    p.bulk_selection(PaneId::Left, SelectionOperation::All, &send);
    let all = sent_selection(&commands);
    p.properties(PaneId::Left, &send);
    assert_eq!(p.desktop.requests.len(), 1);
    assert!(p.new_tab(PaneId::Left, &send));
    assert_ne!(p.panes[0].selection_context, original_context);
    assert!(p.selection_properties[0].is_none());
    assert!(p.desktop.requests.is_empty());
    commands.borrow_mut().clear();
    p.apply_with_sender(ack(&all, all.view.clone().unwrap(), vec![7], false), &send);
    assert!(p.panes[0].selected.is_empty());
    assert!(commands.borrow().is_empty());
}

#[test]
fn failed_internal_rebase_is_retried_without_replaying_the_user_action() {
    let mut p = loaded();
    let commands = RefCell::new(Vec::new());
    let send = |command| {
        commands.borrow_mut().push(command);
        true
    };
    p.bulk_selection(PaneId::Left, SelectionOperation::All, &send);
    let all = sent_selection(&commands);
    p.apply_with_sender(
        Event::SelectionFailed {
            pane: PaneId::Left,
            navigation: all.navigation,
            revision: all.revision,
            error: FsError::from_io(
                "selection",
                Path::new("left"),
                io::ErrorKind::WouldBlock.into(),
            ),
        },
        &send,
    );
    let retry = sent_selection(&commands);
    assert!(matches!(retry.action, SelectionAction::Rebase));
    assert!(retry.revision > all.revision);
    assert_eq!(retry.context, all.context);
    assert!(!p.panes[0].error.is_empty());
}

#[test]
fn failed_destination_cancels_its_properties_and_drops_its_late_selection_ack() {
    let mut p = loaded();
    let commands = RefCell::new(Vec::new());
    let send = |command| {
        commands.borrow_mut().push(command);
        true
    };
    assert!(p.navigate(PaneId::Left, "elsewhere".into(), None, &send));
    let navigation = NavigationId(p.panes[0].generation);
    commands.borrow_mut().clear();
    p.apply(Event::EntriesBatch {
        pane: PaneId::Left,
        navigation,
        entries: vec![entry(88, "elsewhere/temporary")],
    });
    p.select_row(PaneId::Left, 0, true, &send);
    let toggle = sent_selection(&commands);
    p.properties(PaneId::Left, &send);
    assert!(p.selection_properties[0].is_some());
    let failed_basis = Arc::new(DirectoryIndex::from_entries(p.panes[0].rows()));
    p.apply_with_sender(
        Event::ListingFailed {
            pane: PaneId::Left,
            navigation,
            error: FsError::from_io(
                "list",
                Path::new("elsewhere"),
                io::ErrorKind::PermissionDenied.into(),
            ),
        },
        &send,
    );
    assert!(p.selection_properties[0].is_none());
    assert!(p.desktop.requests.is_empty());
    let rebase = sent_selection(&commands);
    assert!(matches!(rebase.action, SelectionAction::Rebase));
    p.apply_with_sender(ack(&toggle, failed_basis, vec![1], false), &send);
    assert!(p.panes[0].selected.is_empty());
    assert!(
        commands.borrow().is_empty(),
        "failed destination cannot launch Properties later"
    );
}
