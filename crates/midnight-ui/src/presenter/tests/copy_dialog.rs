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

pub(in crate::presenter) fn assert_ambiguous_destination_is_refused(name: std::ffi::OsString) {
    for moving in [false, true] {
        let (mut p, root) = loaded();
        let target = root.join(&name);
        p.panes[PaneId::Right.index()].path = target.clone();
        let source = p.panes[0].display_path().to_owned();
        let focus = p.panes[0].cursor;
        if moving {
            p.request_move_dialog(PaneId::Left, &|_| true);
        } else {
            p.request_copy_dialog(PaneId::Left, &|_| true);
        }
        let token = p.copies.draft.as_ref().unwrap().token.clone();
        let ambiguous = target.join("child").to_string_lossy().into_owned();
        p.confirm_copy_dialog(&token, &ambiguous, &|_| {
            panic!("Ambiguous destination queued work")
        });
        let draft = p
            .copies
            .draft
            .as_ref()
            .expect("Refusal must keep the draft");
        assert_eq!(draft.destination, target);
        assert!(!draft.error.is_empty());
        assert!(!p.has_live_operations());
        assert_eq!(p.panes[0].display_path(), source);
        assert_eq!(p.panes[0].cursor, focus);
        let unambiguous = root.join("explicit Unicode 日本語");
        p.confirm_copy_dialog(&token, unambiguous.to_str().unwrap(), &|command| {
            match command {
                Command::EnqueueSelectedCopy(request) => {
                    assert!(!moving);
                    assert_eq!(request.destination, unambiguous);
                }
                Command::EnqueueSelectedMove(request) => {
                    assert!(moving);
                    assert_eq!(request.destination, unambiguous);
                }
                _ => panic!("Unexpected command"),
            }
            true
        });
        assert!(p.copies.draft.is_none());
    }
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
    let crate::presenter::copy_dialog::CopyDraftInput::Native(files) = &draft.input else {
        panic!("native draft")
    };
    assert!(Arc::ptr_eq(&files.selection, &selection));
    assert_eq!(draft.destination, root.join("target"));
    assert_eq!(files.fallback, Some(root.join("first.txt")));
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

#[test]
fn move_clipboard_opens_a_confirmation_without_enqueuing_or_copying() {
    let (mut p, root) = loaded();
    let destination = root.join("captured destination 文档");
    let clipboard = FileClipboard {
        paths: vec![root.join("original.txt")],
        intent: FileClipboardIntent::Move,
    };
    p.start_copy(clipboard, destination.clone(), &|_| {
        panic!("Move clipboard must await explicit confirmation")
    });
    assert!(
        p.copies.draft.is_some(),
        "Move clipboard must open the destination confirmation"
    );
    assert_eq!(p.copies.draft.as_ref().unwrap().destination, destination);
    let token = p.copies.draft.as_ref().unwrap().token.clone();
    p.cancel_copy_dialog(&token);
    assert!(p.copies.draft.is_none());
    assert!(!p.has_live_operations());
}

#[test]
fn f6_move_captures_selection_and_destination_and_confirms_once() {
    let (mut p, root) = loaded();
    p.request_move_dialog(PaneId::Left, &|_| panic!("Opening F6 performs no I/O"));
    let draft = p.copies.draft.as_ref().unwrap();
    assert_eq!(draft.intent, TransferIntent::RecoverableMove);
    let token = draft.token.clone();
    let destination = draft.destination.clone();
    p.focus_display_row(PaneId::Left, 2);
    p.panes[PaneId::Right.index()].path = root.join("changed destination");
    let commands = RefCell::new(Vec::new());
    let send = |command| {
        commands.borrow_mut().push(command);
        true
    };
    p.confirm_copy_dialog(&token, &destination.to_string_lossy(), &send);
    p.confirm_copy_dialog(&token, &destination.to_string_lossy(), &send);
    let commands = commands.borrow();
    let [Command::EnqueueSelectedMove(request)] = commands.as_slice() else {
        panic!("F6 must submit one direct selected Move");
    };
    assert_eq!(request.destination, root.join("target"));
    assert_eq!(request.files.fallback, Some(root.join("first.txt")));
    assert_eq!(
        p.copies.progress.last().unwrap().intent,
        TransferIntent::RecoverableMove
    );
}

#[test]
fn clipboard_move_retains_shared_raw_paths_and_original_destination_after_busy_retry() {
    let (mut p, root) = loaded();
    let paths = vec![root.join("original 文档.txt")];
    let destination = root.join("target");
    p.start_copy(
        FileClipboard {
            paths: paths.clone(),
            intent: FileClipboardIntent::Move,
        },
        destination.clone(),
        &|_| panic!("confirmation first"),
    );
    let draft = p.copies.draft.as_ref().unwrap();
    let crate::presenter::copy_dialog::CopyDraftInput::Clipboard(files) = &draft.input else {
        panic!("shared raw clipboard draft");
    };
    let captured = Arc::downgrade(files);
    let token = draft.token.clone();
    p.panes[PaneId::Right.index()].path = root.join("other");
    p.confirm_copy_dialog(&token, &destination.to_string_lossy(), &|_| false);
    assert!(p.copies.draft.is_some());
    assert_eq!(
        captured.strong_count(),
        1,
        "the retained draft must be the independent raw-payload owner after queue refusal"
    );
    assert!(!p.has_live_operations());
    p.confirm_copy_dialog(&token, &destination.to_string_lossy(), &|command| {
        let Command::EnqueueClipboardMove(request) = command else {
            panic!("shared clipboard Move route");
        };
        assert!(Arc::ptr_eq(
            &request.clipboard,
            &captured.upgrade().unwrap()
        ));
        assert_eq!(request.clipboard.paths, paths);
        assert_eq!(request.destination, destination);
        true
    });
    assert!(p.copies.draft.is_none());
    assert_eq!(
        p.copies.progress.last().unwrap().intent,
        TransferIntent::RecoverableMove
    );
}

#[test]
fn f6_waits_for_the_exact_pending_selection_revision() {
    let (mut p, root) = loaded();
    let commands = RefCell::new(Vec::new());
    let send = |command| {
        commands.borrow_mut().push(command);
        true
    };
    p.bulk_selection(PaneId::Left, SelectionOperation::All, &send);
    let Command::Selection(request) = commands.borrow_mut().pop().unwrap() else {
        panic!("selection request");
    };
    p.request_move_dialog(PaneId::Left, &send);
    assert!(p.copies.draft.is_none());
    p.panes[PaneId::Right.index()].path = root.join("new target");
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
    assert_eq!(draft.destination, root.join("target"));
    let crate::presenter::copy_dialog::CopyDraftInput::Native(files) = &draft.input else {
        panic!("native selection");
    };
    assert!(Arc::ptr_eq(&files.selection, &selection));
    assert_eq!(draft.intent, TransferIntent::RecoverableMove);
}
