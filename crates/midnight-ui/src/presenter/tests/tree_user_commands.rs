use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

fn loaded(selected: bool) -> Presenter {
    let (mut p, root) = tree::fixture();
    p.preferences.loaded = true;
    p.preferences.values.user_commands = Arc::new(vec![UserCommand {
        id: UserCommandId::new("tree-command").unwrap(),
        name: "Tree command".into(),
        enabled: true,
        toolbar_label: None,
        icon: None,
        working_directory: "${active.path}".into(),
        launch: UserCommandLaunch::Single,
        invocation: UserCommandInvocation::Program {
            executable: test_root().join("fixture-program"),
            arguments: vec![],
        },
    }]);
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(tree::publication(
            root, 1, true, selected,
        )))),
        &|_| true,
    );
    p.focus_display_row(PaneId::Left, 2);
    p
}
fn request(p: &mut Presenter) -> TreeUserCommandRequest {
    let result = RefCell::new(None);
    assert!(
        p.request_user_command(PaneId::Left, "tree-command", false, &|command| {
            if let Command::Tree(command) = command
                && let TreeCommand::UserCommand(request) = *command
            {
                *result.borrow_mut() = Some(request);
            } else {
                panic!("tree must not submit flat file arguments");
            }
            true
        })
    );
    result.into_inner().unwrap()
}
#[derive(Debug)]
struct Source {
    stamp: TreeViewStamp,
    request: TreeRequestId,
    drops: Arc<AtomicUsize>,
}
impl CapturedUserCommandSources for Source {
    fn stamp(&self) -> TreeViewStamp {
        self.stamp
    }
    fn request(&self) -> TreeRequestId {
        self.request
    }
    fn resolve(&self, _: &Cancellation) -> Result<ResolvedUserCommandSources, FsError> {
        panic!("UI resolved native arguments")
    }
    fn revalidate(&self, _: &Cancellation) -> Result<(), FsError> {
        panic!("UI validated native arguments")
    }
}
impl Drop for Source {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}
fn receipt(request: &TreeUserCommandRequest, drops: Arc<AtomicUsize>) -> Event {
    Event::Tree(Box::new(TreeEvent::UserCommandCaptured {
        stamp: request.view.stamp,
        request: request.request,
        retryable: false,
        result: Ok(Arc::new(Source {
            stamp: request.view.stamp,
            request: request.request,
            drops,
        })),
    }))
}
fn reclaim(p: &mut Presenter, drops: &AtomicUsize) {
    assert_eq!(
        drops.load(Ordering::SeqCst),
        0,
        "last provider ownership dropped on UI"
    );
    let retired = p.take_retired();
    std::thread::spawn(move || drop(retired)).join().unwrap();
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[test]
fn tree_user_command_capture_freezes_configuration_focus_other_directory_and_selected_marks() {
    let mut p = loaded(true);
    let configuration = p.preferences.values.user_commands.clone();
    let other = p.panes[1].display_path().to_owned();
    let request = request(&mut p);
    assert!(request.selection.contains_position(0));
    assert_eq!(request.focus, Some(TreeRowId(11)));
    p.focus_display_row(PaneId::Left, 1);
    p.panes[1].path = test_root().join("other changed");
    p.preferences.values.user_commands = Arc::new(vec![]);
    let sent = RefCell::new(None);
    let drops = Arc::new(AtomicUsize::new(0));
    p.apply_with_sender(receipt(&request, drops.clone()), &|command| {
        if let Command::ExecuteUserCommand(request) = command {
            *sent.borrow_mut() = Some(request);
        }
        true
    });
    let launch = sent.into_inner().unwrap();
    assert!(Arc::ptr_eq(&configuration, &launch.configuration));
    assert_eq!(launch.context.other, Some(other));
    assert!(launch.context.files.is_none() && launch.context.focus.is_none());
    assert_eq!(launch.captured.as_ref().unwrap().request(), request.request);
    p.clear_tree(PaneId::Left, Some(NavigationId(99)));
    assert!(
        !request.cancel.is_cancelled(),
        "accepted launch survives panel navigation"
    );
    drop(launch);
    reclaim(&mut p, &drops);
}

#[test]
fn tree_user_command_capture_queue_and_launch_transport_retry_same_owner_without_duplicate_jobs() {
    let mut p = loaded(true);
    assert!(p.request_user_command(PaneId::Left, "tree-command", false, &|_| false));
    assert!(p.command_launches.tree[0].is_some());
    let request = RefCell::new(None);
    p.flush_tree_user_commands(&|command| {
        if let Command::Tree(command) = command
            && let TreeCommand::UserCommand(value) = *command
        {
            *request.borrow_mut() = Some(value);
        }
        true
    });
    let request = request.into_inner().unwrap();
    let drops = Arc::new(AtomicUsize::new(0));
    p.apply_with_sender(receipt(&request, drops.clone()), &|_| false);
    assert!(p.command_launches.jobs.is_empty());
    assert!(p.command_launches.tree[0].is_some());
    let sent = RefCell::new(Vec::new());
    for _ in 0..3 {
        p.flush_tree_user_commands(&|command| {
            sent.borrow_mut().push(command);
            true
        });
    }
    assert_eq!(p.command_launches.jobs.len(), 1);
    assert_eq!(
        sent.borrow()
            .iter()
            .filter(|command| matches!(command, Command::ExecuteUserCommand(_)))
            .count(),
        1
    );
    assert!(p.command_launches.tree[0].is_none());
    sent.borrow_mut().clear();
    reclaim(&mut p, &drops);
}

#[test]
fn tree_user_command_pending_navigation_reprojection_shutdown_and_settings_edits_reject_late_capture()
 {
    for cause in 0..4 {
        let mut p = loaded(true);
        let request = if cause == 3 {
            let sent = RefCell::new(None);
            assert!(p.preview_user_command_configuration(
                PaneId::Left,
                p.preferences.values.user_commands.clone(),
                "tree-command",
                &|command| {
                    if let Command::Tree(command) = command
                        && let TreeCommand::UserCommand(request) = *command
                    {
                        *sent.borrow_mut() = Some(request);
                    }
                    true
                }
            ));
            sent.into_inner().unwrap()
        } else {
            request(&mut p)
        };
        match cause {
            0 => p.clear_tree(PaneId::Left, Some(NavigationId(99))),
            1 => p.apply_with_sender(
                Event::Tree(Box::new(TreeEvent::Published(tree::publication(
                    request.view.root.clone(),
                    2,
                    true,
                    true,
                )))),
                &|_| true,
            ),
            2 => p.request_operation_shutdown(&|_| true),
            _ => p.user_command_settings.edit_revision += 1,
        }
        let drops = Arc::new(AtomicUsize::new(0));
        p.apply_with_sender(receipt(&request, drops.clone()), &|command| {
            assert!(!matches!(
                command,
                Command::ExecuteUserCommand(_) | Command::PreviewUserCommand(_)
            ));
            true
        });
        assert!(request.cancel.is_cancelled());
        assert!(p.command_launches.jobs.is_empty());
        reclaim(&mut p, &drops);
    }
}

#[test]
fn tree_user_command_location_only_parent_never_becomes_a_file_argument() {
    let mut p = loaded(false);
    p.focus_display_row(PaneId::Left, 0);
    let request = request(&mut p);
    assert!(request.focus.is_none());
    assert!(request.selection.is_empty());
}

#[test]
fn tree_user_command_retryable_receipt_defers_and_terminal_refusal_does_not_loop() {
    let mut p = loaded(true);
    let request = request(&mut p);
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::UserCommandCaptured {
            stamp: request.view.stamp,
            request: request.request,
            retryable: true,
            result: Err(TreeDiagnostic::new("queue busy")),
        })),
        &|_| panic!("must not retry inside rejection delivery"),
    );
    let sent = RefCell::new(None);
    p.flush_tree_user_commands(&|command| {
        if let Command::Tree(command) = command
            && let TreeCommand::UserCommand(request) = *command
        {
            *sent.borrow_mut() = Some(request);
        }
        true
    });
    let next = sent.into_inner().unwrap();
    assert!(next.request.0 > request.request.0);
    assert!(Arc::ptr_eq(&request.view, &next.view));
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::UserCommandCaptured {
            stamp: next.view.stamp,
            request: next.request,
            retryable: false,
            result: Err(TreeDiagnostic::new("unsupported provider")),
        })),
        &|_| panic!("terminal refusal must not enqueue"),
    );
    assert!(p.command_launches.tree[0].is_none());
    p.flush_tree_user_commands(&|_| panic!("terminal refusal must not retry"));
}

#[test]
fn tree_user_command_waits_for_exact_selection_receipt_and_keeps_original_focus() {
    let mut p = loaded(true);
    assert!(p.send_tree_selection(
        PaneId::Left,
        TreeSelectionAction::Toggle(TreeRowId(11)),
        &|_| true
    ));
    let revision = p.panes[0].tree.revision;
    let view = p.panes[0].tree_view().unwrap().clone();
    assert!(
        p.request_user_command(PaneId::Left, "tree-command", false, &|_| panic!(
            "unsettled selection"
        ))
    );
    p.focus_display_row(PaneId::Left, 1);
    let selection = Arc::new(TreeSelectionSnapshot::from_bits(view.clone(), vec![3]).unwrap());
    let sent = RefCell::new(None);
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::SelectionChanged {
            view,
            revision,
            selection: selection.clone(),
        })),
        &|command| {
            if let Command::Tree(command) = command
                && let TreeCommand::UserCommand(request) = *command
            {
                *sent.borrow_mut() = Some(request);
            }
            true
        },
    );
    let request = sent.into_inner().unwrap();
    assert_eq!(request.selection_revision, revision);
    assert!(Arc::ptr_eq(&request.selection, &selection));
    assert_eq!(request.focus, Some(TreeRowId(11)));
}
