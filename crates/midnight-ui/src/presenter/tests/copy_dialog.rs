use super::*;

fn loaded() -> (Presenter, PathBuf) {
    let root = std::env::temp_dir().join("F5 文档");
    let mut p = Presenter::new(root.clone(), root.join("target"));
    let navigation = begin(&mut p, PaneId::Left, root.to_str().unwrap());
    finish(
        &mut p,
        PaneId::Left,
        navigation,
        vec![
            Entry::new(EntryId(1), root.join("first.txt"), EntryKind::File),
            Entry::new(EntryId(2), root.join("other.txt"), EntryKind::File),
        ],
    );
    p.focus_display_row(PaneId::Left, 1);
    (p, root)
}

pub(in crate::presenter) fn assert_native_destination_is_preserved(name: std::ffi::OsString) {
    let (mut p, root) = loaded();
    let target = root.join(name);
    p.panes[PaneId::Right.index()].path = target.clone();
    p.request_copy_dialog(PaneId::Left, &|_| true);
    let token = p.copies.draft.as_ref().unwrap().token.clone();
    p.confirm_copy_dialog(&token, &target.to_string_lossy(), &|command| {
        assert!(matches!(command, Command::EnqueueSelectedCopy(request) if request.destination == target));
        true
    });
}

#[test]
fn copy_dialog_captures_selection_and_other_panel_path_before_selection_finishes() {
    let (mut p, root) = loaded();
    let commands = RefCell::new(Vec::new());
    let send = |command| {
        commands.borrow_mut().push(command);
        true
    };
    p.bulk_selection(PaneId::Left, SelectionOperation::All, &send);
    let Command::Selection(request) = commands.borrow_mut().pop().unwrap() else {
        unreachable!()
    };
    p.request_copy_dialog(PaneId::Left, &send);
    assert!(p.copies.draft.is_none());
    p.navigate(PaneId::Right, root.join("changed target"), None, &send);
    p.focus_display_row(PaneId::Left, 2);
    let selection =
        Arc::new(SelectionSnapshot::from_bits(request.view.clone().unwrap(), vec![3]).unwrap());
    p.apply_with_sender(
        Event::SelectionChanged {
            pane: PaneId::Left,
            navigation: request.navigation,
            revision: request.revision,
            selection: selection.clone(),
            deferred: false,
        },
        &send,
    );
    let draft = p.copies.draft.as_ref().unwrap();
    assert!(Arc::ptr_eq(&draft.files.selection, &selection));
    assert_eq!(draft.destination, root.join("target"));
    assert_eq!(draft.files.fallback, Some(root.join("first.txt")));
    let token = draft.token.clone();
    let destination = draft.destination.to_string_lossy().into_owned();
    p.navigate(PaneId::Left, root.join("new source"), None, &send);
    commands.borrow_mut().clear();
    p.confirm_copy_dialog(&token, &destination, &send);
    p.confirm_copy_dialog(&token, &destination, &send);
    let commands = commands.borrow();
    assert_eq!(commands.len(), 1);
    let Command::EnqueueSelectedCopy(copy) = &commands[0] else {
        panic!("direct copy, no clipboard")
    };
    assert!(Arc::ptr_eq(&copy.files.selection, &selection));
    assert_eq!(copy.destination, root.join("target"));
    assert!(p.copies.draft.is_none());
    assert!(p.has_live_operations());
}

#[test]
fn copy_dialog_cancel_and_invalid_or_stale_confirmation_never_queue_work() {
    let (mut p, _) = loaded();
    p.request_copy_dialog(PaneId::Left, &|_| {
        panic!("opening the dialog performs no I/O")
    });
    let token = p.copies.draft.as_ref().unwrap().token.clone();
    let no_send = |_: Command| -> bool { panic!("no copy may be queued") };
    p.confirm_copy_dialog("stale", "", &no_send);
    p.confirm_copy_dialog(&token, "", &no_send);
    assert!(!p.copies.draft.as_ref().unwrap().error.is_empty());
    p.cancel_copy_dialog("stale");
    assert!(p.copies.draft.is_some());
    p.cancel_copy_dialog(&token);
    p.confirm_copy_dialog(&token, "anything", &no_send);
    assert!(p.copies.draft.is_none());
    assert!(!p.has_live_operations());
    assert!(!p.take_retired().is_empty());
}

#[test]
fn copy_dialog_rejected_admission_keeps_the_draft_for_retry() {
    let (mut p, root) = loaded();
    p.request_copy_dialog(PaneId::Left, &|_| true);
    let token = p.copies.draft.as_ref().unwrap().token.clone();
    let target = root.join("edited destination");
    p.confirm_copy_dialog(&token, &target.to_string_lossy(), &|_| false);
    assert!(p.copies.draft.is_some());
    assert!(!p.has_live_operations());
    p.confirm_copy_dialog(&token, &target.to_string_lossy(), &|command| {
        assert!(matches!(command, Command::EnqueueSelectedCopy(request) if request.destination == target));
        true
    });
    assert!(p.copies.draft.is_none());
}
