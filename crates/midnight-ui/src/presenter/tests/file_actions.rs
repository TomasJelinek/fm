use super::*;

fn loaded() -> (Presenter, PathBuf) {
    let base = std::env::temp_dir().join("menu 文档 & $(literal)");
    let mut p = Presenter::new(base.clone(), base.join("other"));
    let navigation = begin(&mut p, PaneId::Left, base.to_str().unwrap());
    finish(
        &mut p,
        PaneId::Left,
        navigation,
        vec![
            Entry::new(EntryId(1), base.join("a.txt"), EntryKind::File),
            Entry::new(EntryId(2), base.join("文件夹"), EntryKind::Directory),
            Entry::new(EntryId(3), base.join("c.txt"), EntryKind::File),
        ],
    );
    (p, base)
}

fn menu_request(commands: &RefCell<Vec<Command>>) -> DesktopRequestId {
    commands
        .borrow()
        .iter()
        .find_map(|command| match command {
            Command::ShowShellMenu { request, .. } => Some(*request),
            _ => None,
        })
        .expect("a native menu request")
}

#[test]
fn copy_waits_for_exact_selection_revision_and_keeps_its_captured_view() {
    let (mut p, base) = loaded();
    p.focus_display_row(PaneId::Left, 1);
    let commands = RefCell::new(Vec::new());
    let send = |command| {
        commands.borrow_mut().push(command);
        true
    };
    p.bulk_selection(PaneId::Left, SelectionOperation::All, &send);
    let all = match commands.borrow_mut().pop().unwrap() {
        Command::Selection(request) => request,
        _ => unreachable!(),
    };
    p.copy_files(PaneId::Left, &send);
    assert!(commands.borrow().is_empty());
    p.bulk_selection(PaneId::Left, SelectionOperation::Clear, &send);
    commands.borrow_mut().clear();
    p.focus_display_row(PaneId::Left, 3);
    let selection =
        Arc::new(SelectionSnapshot::from_bits(all.view.clone().unwrap(), vec![7]).unwrap());
    p.apply_with_sender(
        Event::SelectionChanged {
            pane: PaneId::Left,
            navigation: all.navigation,
            revision: all.revision,
            selection: selection.clone(),
            deferred: Default::default(),
        },
        &send,
    );
    let dispatched = commands.borrow();
    let Command::CopyFiles { files, .. } = &dispatched[0] else {
        panic!("copy must dispatch its captured selection")
    };
    assert!(Arc::ptr_eq(&files.selection, &selection));
    assert!(Arc::ptr_eq(&files.view, all.view.as_ref().unwrap()));
    assert_eq!(files.fallback.as_ref(), Some(&base.join("a.txt")));
    assert_eq!(files.selection.len(), 3);
}

#[test]
fn context_menu_preserves_selected_group_but_unselected_row_is_its_only_target() {
    let (mut p, base) = loaded();
    set_selected(&mut p, PaneId::Left, &[EntryId(1), EntryId(2)]);
    let selected = p.panes[0].selected.clone();
    let commands = RefCell::new(Vec::new());
    let send = |command| {
        commands.borrow_mut().push(command);
        true
    };
    p.request_entry_menu(PaneId::Left, 2, Default::default(), &send);
    let request = menu_request(&commands);
    {
        let captured = commands.borrow();
        let Command::ShowShellMenu { files, target, .. } = &captured[0] else {
            unreachable!()
        };
        assert!(Arc::ptr_eq(&files.selection, &selected));
        assert_eq!(files.fallback.as_ref(), Some(&base.join("文件夹")));
        assert_eq!(*target, ShellMenuTarget::Items);
    }
    p.apply_with_sender(
        Event::ShellMenuFinished {
            request,
            result: Ok(ShellMenuOutcome::Dismissed),
        },
        &send,
    );
    commands.borrow_mut().clear();
    p.request_entry_menu(PaneId::Left, 3, Default::default(), &send);
    let captured = commands.borrow();
    let Command::ShowShellMenu { files, .. } = &captured[0] else {
        unreachable!()
    };
    assert!(files.selection.is_empty());
    assert_eq!(files.fallback.as_ref(), Some(&base.join("c.txt")));
    assert!(Arc::ptr_eq(&p.panes[0].selected, &selected));
}

#[test]
fn native_copy_and_paste_keep_menu_targets_after_navigation_and_ignore_replayed_results() {
    for verb in [ShellMenuVerb::Copy, ShellMenuVerb::Paste] {
        let (mut p, base) = loaded();
        let commands = RefCell::new(Vec::new());
        let send = |command| {
            commands.borrow_mut().push(command);
            true
        };
        p.request_entry_menu(PaneId::Left, 1, Default::default(), &send);
        let request = menu_request(&commands);
        p.navigate(PaneId::Left, base.join("new destination"), None, &send);
        commands.borrow_mut().clear();
        let event = Event::ShellMenuFinished {
            request,
            result: Ok(ShellMenuOutcome::Managed(verb)),
        };
        p.apply_with_sender(event.clone(), &send);
        p.apply_with_sender(event, &send);
        let captured = commands.borrow();
        assert_eq!(captured.len(), 1, "one native result produces one action");
        match (&captured[0], verb) {
            (Command::CopyFiles { files, .. }, ShellMenuVerb::Copy) => {
                assert_eq!(files.fallback.as_ref(), Some(&base.join("a.txt")));
            }
            (Command::ReadFileClipboard { destination, .. }, ShellMenuVerb::Paste) => {
                assert_eq!(destination, &base);
            }
            _ => panic!("menu action must keep its captured target"),
        }
    }
}

#[test]
fn destructive_native_actions_reject_a_changed_navigation() {
    for verb in [ShellMenuVerb::Delete, ShellMenuVerb::Rename] {
        let (mut p, base) = loaded();
        let commands = RefCell::new(Vec::new());
        let send = |command| {
            commands.borrow_mut().push(command);
            true
        };
        p.request_entry_menu(PaneId::Left, 1, Default::default(), &send);
        let request = menu_request(&commands);
        p.navigate(PaneId::Left, base.join("elsewhere"), None, &send);
        commands.borrow_mut().clear();
        p.apply_with_sender(
            Event::ShellMenuFinished {
                request,
                result: Ok(ShellMenuOutcome::Managed(verb)),
            },
            &send,
        );
        assert!(commands.borrow().is_empty());
        assert!(
            p.desktop.notice.is_some(),
            "changed context must be reported"
        );
    }
}

#[test]
fn folder_menu_paste_targets_that_folder_unless_it_represents_a_group() {
    for selected in 0..3 {
        let (mut p, base) = loaded();
        if selected == 1 {
            set_selected(&mut p, PaneId::Left, &[EntryId(2)]);
        } else if selected == 2 {
            set_selected(&mut p, PaneId::Left, &[EntryId(1), EntryId(2)]);
        }
        let commands = RefCell::new(Vec::new());
        let send = |command| {
            commands.borrow_mut().push(command);
            true
        };
        p.request_entry_menu(PaneId::Left, 2, Default::default(), &send);
        let request = menu_request(&commands);
        commands.borrow_mut().clear();
        p.apply_with_sender(
            Event::ShellMenuFinished {
                request,
                result: Ok(ShellMenuOutcome::Managed(ShellMenuVerb::Paste)),
            },
            &send,
        );
        let captured = commands.borrow();
        let Command::ReadFileClipboard { destination, .. } = &captured[0] else {
            unreachable!()
        };
        assert_eq!(
            destination,
            &if selected == 2 {
                base
            } else {
                base.join("文件夹")
            }
        );
    }
}

#[test]
fn delete_uses_captured_unselected_target_even_after_cursor_and_selection_changes() {
    let (mut p, base) = loaded();
    set_selected(&mut p, PaneId::Left, &[EntryId(1), EntryId(2)]);
    let commands = RefCell::new(Vec::new());
    let send = |command| {
        commands.borrow_mut().push(command);
        true
    };
    p.request_entry_menu(PaneId::Left, 3, Default::default(), &send);
    let request = menu_request(&commands);
    p.focus_display_row(PaneId::Left, 1);
    set_selected(&mut p, PaneId::Left, &[EntryId(2)]);
    commands.borrow_mut().clear();
    p.apply_with_sender(
        Event::ShellMenuFinished {
            request,
            result: Ok(ShellMenuOutcome::Managed(ShellMenuVerb::Delete)),
        },
        &send,
    );
    let captured = commands.borrow();
    let Command::PrepareOperation(operation) = &captured[0] else {
        unreachable!()
    };
    assert_eq!(operation.kind, MutationKind::Delete(DeleteMode::Trash));
    assert!(operation.selection.is_empty());
    assert_eq!(operation.focused, Some(EntryId(3)));
    assert!(operation.view.by_path.contains_key(&base.join("c.txt")));
}

#[test]
fn unavailable_native_menu_exposes_application_actions_and_old_tokens_are_rejected() {
    let (mut p, base) = loaded();
    let commands = RefCell::new(Vec::new());
    let send = |command| {
        commands.borrow_mut().push(command);
        true
    };
    p.request_entry_menu(PaneId::Left, 2, Default::default(), &send);
    let request = menu_request(&commands);
    p.apply_with_sender(
        Event::ShellMenuFinished {
            request,
            result: Ok(ShellMenuOutcome::Unavailable),
        },
        &send,
    );
    let token = p.file_menu_token();
    assert!(!token.is_empty());
    commands.borrow_mut().clear();
    p.file_menu_action("stale", 1, &send);
    assert!(commands.borrow().is_empty());
    p.file_menu_action(&token, 1, &send);
    let captured = commands.borrow();
    let Command::CopyFiles { files, .. } = &captured[0] else {
        unreachable!()
    };
    assert_eq!(files.fallback.as_ref(), Some(&base.join("文件夹")));
    assert!(p.file_menu_token().is_empty());
}

#[test]
fn cut_publishes_the_captured_native_selection_without_mutation() {
    let (mut p, base) = loaded();
    let commands = RefCell::new(Vec::new());
    let send = |command| {
        commands.borrow_mut().push(command);
        true
    };
    p.request_entry_menu(PaneId::Left, 1, Default::default(), &send);
    let request = menu_request(&commands);
    commands.borrow_mut().clear();
    p.apply_with_sender(
        Event::ShellMenuFinished {
            request,
            result: Ok(ShellMenuOutcome::Managed(ShellMenuVerb::Cut)),
        },
        &send,
    );
    let commands = commands.borrow();
    let [Command::CutFiles { files, .. }] = commands.as_slice() else {
        panic!("Cut must publish captured native clipboard paths");
    };
    assert_eq!(files.fallback, Some(base.join("a.txt")));
    assert!(!p.has_live_operations());
}

#[test]
fn paste_response_keeps_destination_and_accepts_only_one_matching_reply() {
    let (mut p, base) = loaded();
    let commands = RefCell::new(Vec::new());
    let send = |command| {
        commands.borrow_mut().push(command);
        true
    };
    p.paste_files(PaneId::Left, &send);
    let request = match commands.borrow().last().unwrap() {
        Command::ReadFileClipboard { request, .. } => *request,
        _ => unreachable!(),
    };
    p.navigate(PaneId::Left, base.join("different"), None, &send);
    commands.borrow_mut().clear();
    let clipboard = FileClipboard {
        paths: vec![base.join("source.txt")],
        intent: FileClipboardIntent::Copy,
    };
    p.apply_with_sender(
        Event::ClipboardFilesRead {
            request: DesktopRequestId(request.0 + 1),
            destination: base.clone(),
            result: Ok(clipboard.clone()),
        },
        &send,
    );
    assert!(
        commands.borrow().is_empty(),
        "unrelated response cannot enqueue"
    );
    let event = Event::ClipboardFilesRead {
        request,
        destination: base.clone(),
        result: Ok(clipboard),
    };
    p.apply_with_sender(event.clone(), &send);
    p.apply_with_sender(event, &send);
    let captured = commands.borrow();
    assert_eq!(captured.len(), 1);
    assert!(matches!(&captured[0], Command::EnqueueCopy(copy) if copy.destination == base));
}

#[test]
fn mismatched_paste_destination_and_busy_queue_never_enqueue() {
    let (mut p, base) = loaded();
    p.paste_files(PaneId::Left, &|_| false);
    let commands = RefCell::new(Vec::new());
    let send = |command| {
        commands.borrow_mut().push(command);
        true
    };
    p.paste_files(PaneId::Left, &send);
    let request = match commands.borrow().last().unwrap() {
        Command::ReadFileClipboard { request, .. } => *request,
        _ => unreachable!(),
    };
    commands.borrow_mut().clear();
    p.apply_with_sender(
        Event::ClipboardFilesRead {
            request,
            destination: base.join("different"),
            result: Ok(FileClipboard {
                paths: vec![base.join("source.txt")],
                intent: FileClipboardIntent::Copy,
            }),
        },
        &send,
    );
    assert!(commands.borrow().is_empty());
    assert!(p.desktop.notice.is_some());
    p.paste_files(PaneId::Left, &send);
    assert!(matches!(
        commands.borrow().last(),
        Some(Command::ReadFileClipboard { .. })
    ));
}
