use super::*;

fn loaded() -> Presenter {
    loaded_selection(2)
}

fn loaded_selection(bits: u64) -> Presenter {
    let (mut presenter, root) = super::tree::fixture();
    let mut publication = super::tree::publication(root, 1, true, false);
    publication.selection =
        Arc::new(TreeSelectionSnapshot::from_bits(publication.view.clone(), vec![bits]).unwrap());
    presenter.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(publication))),
        &|_| true,
    );
    presenter
}

#[test]
fn nested_properties_captures_selected_occurrences_on_the_backend() {
    let mut presenter = loaded();
    let sent = RefCell::new(Vec::new());
    presenter.request_selection_properties(PaneId::Left, &|command| {
        sent.borrow_mut().push(command);
        true
    });
    assert!(sent.borrow().iter().any(|command|
        matches!(command, Command::Tree(command) if matches!(command.as_ref(), TreeCommand::Clipboard(_)))),
        "Alt+Enter must capture the native tree selection before showing Properties");
}

#[test]
fn nested_context_menu_captures_clicked_occurrence_without_changing_marks() {
    let mut presenter = loaded();
    let marked = presenter.panes[0]
        .tree
        .publication
        .as_ref()
        .unwrap()
        .selection
        .clone();
    let sent = RefCell::new(Vec::new());
    presenter.request_entry_menu(PaneId::Left, 1, Default::default(), &|command| {
        sent.borrow_mut().push(command);
        true
    });
    let sent = sent.borrow();
    let request = sent
        .iter()
        .find_map(|command| match command {
            Command::Tree(command) => match command.as_ref() {
                TreeCommand::Clipboard(request) => Some(request),
                _ => None,
            },
            _ => None,
        })
        .expect("the item menu must capture exact tree occurrences");
    assert_eq!(request.scope, TreeClipboardScope::Dragged(TreeRowId(7)));
    assert!(Arc::ptr_eq(&request.selection, &marked));
    assert!(request.selection.contains_position(1));
    assert!(!request.selection.contains_position(0));
}

#[derive(Debug)]
struct Opaque(FileClipboardIntent);
impl CapturedClipboardFiles for Opaque {
    fn intent(&self) -> FileClipboardIntent {
        self.0
    }
    fn resolve(&self, _: &Cancellation) -> Result<FileClipboard, FsError> {
        panic!("the presenter must not resolve provider targets")
    }
}

fn take_capture(sent: Vec<Command>) -> TreeClipboardRequest {
    sent.into_iter()
        .find_map(|command| match command {
            Command::Tree(command) => match *command {
                TreeCommand::Clipboard(request) => Some(request),
                _ => None,
            },
            _ => None,
        })
        .expect("capture request")
}

fn resolve(
    presenter: &mut Presenter,
    request: &TreeClipboardRequest,
) -> (Vec<Command>, std::sync::Weak<Opaque>) {
    let source = Arc::new(Opaque(request.intent));
    let weak = Arc::downgrade(&source);
    let sent = RefCell::new(Vec::new());
    presenter.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Clipboard {
            stamp: request.view.stamp,
            request: request.request,
            result: Ok(source),
        })),
        &|command| {
            sent.borrow_mut().push(command);
            true
        },
    );
    (sent.into_inner(), weak)
}

fn menu(presenter: &mut Presenter) -> (TreeClipboardRequest, DesktopRequestId) {
    let sent = RefCell::new(Vec::new());
    presenter.request_entry_menu(PaneId::Left, 1, Default::default(), &|command| {
        sent.borrow_mut().push(command);
        true
    });
    let capture = take_capture(sent.into_inner());
    let (commands, _) = resolve(presenter, &capture);
    let request = commands
        .iter()
        .find_map(|command| match command {
            Command::ShowCapturedShellMenu { request, .. } => Some(*request),
            _ => None,
        })
        .expect("native menu retains opaque source");
    (capture, request)
}

fn finish_menu(
    presenter: &mut Presenter,
    request: DesktopRequestId,
    outcome: ShellMenuOutcome,
) -> Vec<Command> {
    let sent = RefCell::new(Vec::new());
    presenter.apply_with_sender(
        Event::ShellMenuFinished {
            request,
            result: Ok(outcome),
        },
        &|command| {
            sent.borrow_mut().push(command);
            true
        },
    );
    sent.into_inner()
}

#[test]
fn properties_retains_opaque_source_through_completion_and_worker_reclamation() {
    let mut presenter = loaded();
    let sent = RefCell::new(Vec::new());
    presenter.request_selection_properties(PaneId::Left, &|command| {
        sent.borrow_mut().push(command);
        true
    });
    let capture = take_capture(sent.into_inner());
    let (commands, weak) = resolve(&mut presenter, &capture);
    let request = commands
        .iter()
        .find_map(|command| match command {
            Command::ShowCapturedProperties {
                request, cancel, ..
            } => {
                assert!(!cancel.is_cancelled());
                Some(*request)
            }
            _ => None,
        })
        .expect("captured Properties command");
    drop(commands);
    assert!(weak.upgrade().is_some());
    presenter.apply(Event::DesktopActionFinished {
        request,
        result: Ok(DesktopActionResult::PropertiesOpened),
    });
    assert!(presenter.file_actions.tree_clipboard.is_none());
    assert!(
        weak.upgrade().is_some(),
        "source must be queued for worker reclamation"
    );
    let retired = presenter.take_retired();
    std::thread::spawn(move || drop(retired)).join().unwrap();
    assert!(weak.upgrade().is_none());
}

#[test]
fn managed_menu_verbs_keep_unselected_clicked_row_instead_of_other_marks_or_caret() {
    for verb in [
        ShellMenuVerb::Copy,
        ShellMenuVerb::Cut,
        ShellMenuVerb::Rename,
        ShellMenuVerb::Delete,
    ] {
        let mut presenter = loaded();
        let (capture, request) = menu(&mut presenter);
        presenter.focus_display_row(PaneId::Left, 2);
        let commands = finish_menu(&mut presenter, request, ShellMenuOutcome::Managed(verb));
        match verb {
            ShellMenuVerb::Copy | ShellMenuVerb::Cut => {
                let next = take_capture(commands);
                assert_eq!(next.scope, TreeClipboardScope::Dragged(TreeRowId(7)));
                assert_eq!(next.intent, if verb == ShellMenuVerb::Cut { FileClipboardIntent::Move } else { FileClipboardIntent::Copy });
                assert!(next.selection.contains_position(1));
                assert!(!next.cancel.is_cancelled(), "new verb owns independent cancellation");
                assert!(capture.cancel.is_cancelled(), "finished menu capture is retired");
            }
            ShellMenuVerb::Rename => assert!(commands.iter().any(|command|
                matches!(command, Command::Tree(command) if matches!(command.as_ref(), TreeCommand::Rename(request) if request.row == TreeRowId(7))))),
            ShellMenuVerb::Delete => assert!(commands.iter().any(|command|
                matches!(command, Command::Tree(command) if matches!(command.as_ref(), TreeCommand::Delete(request) if request.scope == TreeClipboardScope::Dragged(TreeRowId(7)))))),
            _ => unreachable!(),
        }
        presenter.retire_all();
        let retired = presenter.take_retired();
        std::thread::spawn(move || drop(retired)).join().unwrap();
    }
}

#[test]
fn menu_rejects_selection_change_on_same_projection_without_retargeting() {
    let mut presenter = loaded();
    let (capture, request) = menu(&mut presenter);
    assert!(presenter.send_tree_selection(
        PaneId::Left,
        TreeSelectionAction::Toggle(TreeRowId(7)),
        &|_| true
    ));
    assert!(Arc::ptr_eq(
        presenter.panes[0].tree_view().unwrap(),
        &capture.view
    ));
    let commands = finish_menu(
        &mut presenter,
        request,
        ShellMenuOutcome::Managed(ShellMenuVerb::Delete),
    );
    assert!(
        !commands
            .iter()
            .any(|command| matches!(command, Command::Tree(_)))
    );
    assert!(
        presenter
            .desktop
            .notice
            .as_ref()
            .is_some_and(|notice| notice.0 == "Folder or selection changed")
    );
    assert!(capture.cancel.is_cancelled());
    let retired = presenter.take_retired();
    std::thread::spawn(move || drop(retired)).join().unwrap();
}

#[test]
fn fallback_open_uses_captured_row_and_accepts_only_correlated_backend_directory() {
    let mut presenter = loaded();
    let (capture, request) = menu(&mut presenter);
    finish_menu(&mut presenter, request, ShellMenuOutcome::Unavailable);
    let token = presenter.file_menu_token();
    assert!(!token.is_empty());
    let sent = RefCell::new(Vec::new());
    presenter.file_menu_action(&token, 0, &|command| {
        sent.borrow_mut().push(command);
        true
    });
    let commands = sent.into_inner();
    let request = commands
        .iter()
        .find_map(|command| match command {
            Command::OpenCapturedEntry {
                request,
                pane,
                navigation,
                row,
                ..
            } => {
                assert_eq!(
                    (*pane, *navigation, *row),
                    (PaneId::Left, capture.view.stamp.navigation, TreeRowId(7))
                );
                Some(*request)
            }
            _ => None,
        })
        .expect("captured Open");
    drop(commands);
    let path = test_root().join("backend-native-target");
    presenter.apply(Event::DesktopActionFinished {
        request,
        result: Ok(DesktopActionResult::OpenDirectory {
            pane: PaneId::Left,
            navigation: capture.view.stamp.navigation,
            path: path.clone(),
        }),
    });
    let sent = RefCell::new(Vec::new());
    presenter.dispatch_open_directories(&|command| {
        sent.borrow_mut().push(command);
        true
    });
    assert!(sent.borrow().iter().any(
        |command| matches!(command, Command::Navigate { path: target, .. } if target == &path)
    ));
    presenter.retire_all();
    let retired = presenter.take_retired();
    std::thread::spawn(move || drop(retired)).join().unwrap();
}

#[test]
fn paste_into_clicked_folder_waits_for_captured_backend_destination() {
    let mut presenter = loaded();
    let (_, request) = menu(&mut presenter);
    let commands = finish_menu(
        &mut presenter,
        request,
        ShellMenuOutcome::Managed(ShellMenuVerb::Paste),
    );
    let request = commands
        .iter()
        .find_map(|command| match command {
            Command::ReadCapturedFileClipboard { request, .. } => Some(*request),
            _ => None,
        })
        .expect("the destination must resolve on the clipboard worker");
    assert!(
        !commands
            .iter()
            .any(|command| matches!(command, Command::ReadFileClipboard { .. }))
    );
    drop(commands);
    presenter.apply_with_sender(
        Event::ClipboardFilesRead {
            request,
            destination: PathBuf::new(),
            result: Err(FsError::from_io(
                "read clipboard",
                &test_root(),
                std::io::ErrorKind::Interrupted.into(),
            )),
        },
        &|_| true,
    );
    assert!(presenter.file_actions.tree_clipboard.is_none());
    let retired = presenter.take_retired();
    std::thread::spawn(move || drop(retired)).join().unwrap();
}

#[test]
fn clicking_a_marked_child_preserves_the_whole_selected_batch_for_copy() {
    let mut presenter = loaded_selection(3);
    let sent = RefCell::new(Vec::new());
    presenter.request_entry_menu(PaneId::Left, 2, Default::default(), &|command| {
        sent.borrow_mut().push(command);
        true
    });
    let capture = take_capture(sent.into_inner());
    assert_eq!(capture.scope, TreeClipboardScope::Dragged(TreeRowId(11)));
    assert!(capture.selection.contains_position(0));
    assert!(capture.selection.contains_position(1));
    let (commands, _) = resolve(&mut presenter, &capture);
    let request = commands
        .iter()
        .find_map(|command| match command {
            Command::ShowCapturedShellMenu { request, .. } => Some(*request),
            _ => None,
        })
        .unwrap();
    drop(commands);
    let commands = finish_menu(
        &mut presenter,
        request,
        ShellMenuOutcome::Managed(ShellMenuVerb::Copy),
    );
    let next = take_capture(commands);
    assert_eq!(next.scope, capture.scope);
    assert!(Arc::ptr_eq(&next.selection, &capture.selection));
    presenter.retire_all();
    let retired = presenter.take_retired();
    std::thread::spawn(move || drop(retired)).join().unwrap();
}

#[test]
fn dismiss_and_navigation_discard_late_native_menu_actions() {
    for navigate in [false, true] {
        let mut presenter = loaded();
        let (capture, request) = menu(&mut presenter);
        if navigate {
            presenter.clear_tree(PaneId::Left, Some(NavigationId(99)));
        } else {
            finish_menu(&mut presenter, request, ShellMenuOutcome::Unavailable);
            let token = presenter.file_menu_token();
            presenter.dismiss_file_menu(&token);
        }
        assert!(capture.cancel.is_cancelled());
        let commands = finish_menu(
            &mut presenter,
            request,
            ShellMenuOutcome::Managed(ShellMenuVerb::Delete),
        );
        assert!(commands.is_empty());
        assert!(presenter.file_actions.tree_clipboard.is_none());
        let retired = presenter.take_retired();
        std::thread::spawn(move || drop(retired)).join().unwrap();
    }
}

#[test]
fn refused_open_dispatch_retries_the_same_source_then_clears_on_file_completion() {
    let mut presenter = loaded();
    let (_, request) = menu(&mut presenter);
    finish_menu(&mut presenter, request, ShellMenuOutcome::Unavailable);
    let token = presenter.file_menu_token();
    presenter.file_menu_action(&token, 0, &|command| {
        assert!(matches!(
            command,
            Command::OpenCapturedEntry {
                row: TreeRowId(7),
                ..
            }
        ));
        false
    });
    let source = Arc::downgrade(
        presenter
            .file_actions
            .tree_clipboard
            .as_ref()
            .unwrap()
            .source
            .as_ref()
            .unwrap(),
    );
    assert!(source.upgrade().is_some());
    presenter.focus_display_row(PaneId::Left, 2);
    let sent = RefCell::new(Vec::new());
    presenter.flush_tree_clipboard(&|command| {
        sent.borrow_mut().push(command);
        true
    });
    let commands = sent.into_inner();
    let request = commands
        .iter()
        .find_map(|command| match command {
            Command::OpenCapturedEntry {
                request,
                source: retry_source,
                row,
                ..
            } => {
                assert_eq!(*row, TreeRowId(7));
                assert!(Arc::ptr_eq(&source.upgrade().unwrap(), retry_source));
                Some(*request)
            }
            _ => None,
        })
        .expect("retry captured Open");
    drop(commands);
    presenter.apply(Event::DesktopActionFinished {
        request,
        result: Ok(DesktopActionResult::FileOpened),
    });
    assert!(presenter.file_actions.tree_clipboard.is_none());
    let retired = presenter.take_retired();
    std::thread::spawn(move || drop(retired)).join().unwrap();
    assert!(source.upgrade().is_none());
}
