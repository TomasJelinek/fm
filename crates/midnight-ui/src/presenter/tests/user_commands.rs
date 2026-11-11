use super::*;

fn loaded(files: bool) -> (Presenter, PathBuf) {
    let root = std::env::temp_dir().join("commands 文档");
    let mut p = Presenter::new(root.clone(), root.join("other"));
    let navigation = begin(&mut p, PaneId::Left, root.to_str().unwrap());
    finish(
        &mut p,
        PaneId::Left,
        navigation,
        if files {
            vec![
                Entry::new(EntryId(1), root.join("focus 日本語.txt"), EntryKind::File),
                Entry::new(EntryId(2), root.join("second.txt"), EntryKind::File),
            ]
        } else {
            vec![]
        },
    );
    p.preferences.loaded = true;
    p.preferences.values.user_commands = Arc::new(vec![UserCommand {
        id: UserCommandId::new("test-command").unwrap(),
        name: "Test command".into(),
        enabled: true,
        toolbar_label: None,
        icon: None,
        working_directory: "${active.path}".into(),
        launch: UserCommandLaunch::Single,
        invocation: UserCommandInvocation::Program {
            executable: root.join("fixture-tool"),
            arguments: vec![],
        },
    }]);
    if files {
        p.focus_display_row(PaneId::Left, 1);
    }
    (p, root)
}

#[test]
fn literal_command_in_empty_folder_captures_locations_without_parent_target() {
    let (mut p, root) = loaded(false);
    let sent = RefCell::new(Vec::new());
    p.request_user_command(PaneId::Left, "test-command", false, &|c| {
        sent.borrow_mut().push(c);
        true
    });
    let commands = sent.borrow();
    let Command::ExecuteUserCommand(request) = &commands[0] else {
        panic!()
    };
    assert_eq!(request.context.active, Some(root.clone()));
    assert_eq!(request.context.other, Some(root.join("other")));
    assert!(request.context.focus.is_none());
    assert!(request.context.files.as_ref().unwrap().fallback.is_none());
    assert!(request.context.files.as_ref().unwrap().selection.is_empty());
}

#[test]
fn command_waits_for_exact_selection_and_retains_focus_other_panel_and_configuration() {
    let (mut p, root) = loaded(true);
    let sent = RefCell::new(Vec::new());
    let send = |c| {
        sent.borrow_mut().push(c);
        true
    };
    p.bulk_selection(PaneId::Left, SelectionOperation::All, &send);
    let Command::Selection(request) = sent.borrow_mut().pop().unwrap() else {
        panic!()
    };
    let configuration = p.preferences.values.user_commands.clone();
    p.request_user_command(PaneId::Left, "test-command", false, &send);
    assert!(sent.borrow().is_empty());
    p.focus_display_row(PaneId::Left, 2);
    p.panes[1].path = root.join("changed other");
    p.preferences.values.user_commands = Arc::new(vec![]);
    let selection =
        Arc::new(SelectionSnapshot::from_bits(request.view.clone().unwrap(), vec![3]).unwrap());
    p.apply_with_sender(
        Event::SelectionChanged {
            pane: PaneId::Left,
            navigation: request.navigation,
            revision: request.revision,
            selection: selection.clone(),
            deferred: Default::default(),
        },
        &send,
    );
    let commands = sent.borrow();
    let Command::ExecuteUserCommand(command) = &commands[0] else {
        panic!()
    };
    assert!(Arc::ptr_eq(&command.configuration, &configuration));
    assert!(Arc::ptr_eq(
        &command.context.files.as_ref().unwrap().selection,
        &selection
    ));
    assert_eq!(command.context.other, Some(root.join("other")));
    assert_eq!(
        command.context.focus.as_ref().unwrap().path,
        root.join("focus 日本語.txt")
    );
}

#[test]
fn disabled_and_deleted_commands_do_not_dispatch_and_queue_refusal_never_creates_job() {
    let (mut p, _) = loaded(true);
    p.request_user_command(PaneId::Left, "removed", false, &|_| {
        panic!("deleted command")
    });
    Arc::make_mut(&mut p.preferences.values.user_commands)[0].enabled = false;
    p.request_user_command(PaneId::Left, "test-command", false, &|_| {
        panic!("disabled command")
    });
    Arc::make_mut(&mut p.preferences.values.user_commands)[0].enabled = true;
    p.request_user_command(PaneId::Left, "test-command", false, &|_| false);
    assert!(p.command_launches.jobs.is_empty());
    p.request_user_command(PaneId::Left, "test-command", false, &|c| {
        assert!(matches!(c, Command::ExecuteUserCommand(r) if r.id.0 == 2));
        true
    });
    assert_eq!(p.command_launches.jobs.len(), 1);
}

#[test]
fn preview_and_cancel_are_explicit_and_duplicate_rejection_does_not_finish_accepted_launch() {
    let (mut p, _) = loaded(true);
    p.request_user_command(PaneId::Left, "test-command", true, &|c| {
        assert!(matches!(c, Command::PreviewUserCommand(_)));
        true
    });
    let id = p.command_launches.jobs[0].id;
    let command = p.command_launches.jobs[0].command.clone();
    let event = |update| {
        Event::UserCommand(Box::new(UserCommandEvent {
            request: id,
            command: command.clone(),
            update,
        }))
    };
    p.apply(event(UserCommandUpdate::Queued));
    p.apply(event(UserCommandUpdate::Rejected(FsError::from_io(
        "duplicate command",
        Path::new(""),
        io::ErrorKind::AlreadyExists.into(),
    ))));
    assert!(!p.command_launches.jobs[0].finished);
    let token = format!("command-job:{}", id.0);
    p.cancel_user_command(&token, &|_| false);
    assert!(!p.command_launches.jobs[0].stopping);
    p.cancel_user_command(&token, &|c| {
        assert!(matches!(c, Command::CancelUserCommand(actual) if actual == id));
        true
    });
    assert!(p.command_launches.jobs[0].stopping);
    p.apply(event(UserCommandUpdate::Finished(UserCommandSummary {
        planned: 2,
        started: 1,
        not_started: 1,
        cancelled: true,
        error: None,
    })));
    assert!(p.command_launches.jobs[0].finished);
    assert_eq!(p.command_launches.jobs[0].started, 1);
    p.apply(event(UserCommandUpdate::Started {
        index: 9,
        process_id: 100,
    }));
    assert_eq!(p.command_launches.jobs[0].started, 1);
    assert!(!p.take_retired().is_empty());
}

#[test]
fn out_of_order_panel_selection_replies_still_submit_increasing_backend_ids() {
    let (mut p, root) = loaded(true);
    let other = root.join("other");
    let navigation = begin(&mut p, PaneId::Right, other.to_str().unwrap());
    finish(
        &mut p,
        PaneId::Right,
        navigation,
        vec![Entry::new(
            EntryId(3),
            other.join("third.txt"),
            EntryKind::File,
        )],
    );
    p.focus_display_row(PaneId::Right, 1);
    let sent = RefCell::new(Vec::new());
    let send = |c| {
        sent.borrow_mut().push(c);
        true
    };
    p.bulk_selection(PaneId::Left, SelectionOperation::All, &send);
    let Command::Selection(selection) = sent.borrow_mut().pop().unwrap() else {
        panic!()
    };
    assert!(p.request_user_command(PaneId::Left, "test-command", false, &send));
    assert!(p.request_user_command(PaneId::Right, "test-command", false, &send));
    p.apply_with_sender(
        Event::SelectionChanged {
            pane: PaneId::Left,
            navigation: selection.navigation,
            revision: selection.revision,
            selection: Arc::new(
                SelectionSnapshot::from_bits(selection.view.unwrap(), vec![3]).unwrap(),
            ),
            deferred: Default::default(),
        },
        &send,
    );
    let requests = sent.borrow();
    let ids: Vec<_> = requests
        .iter()
        .filter_map(|c| match c {
            Command::ExecuteUserCommand(r) => Some(r.id.0),
            _ => None,
        })
        .collect();
    assert_eq!(
        ids,
        vec![1, 2],
        "gesture order must not make later selection replies look like replayed IDs"
    );
}

#[test]
fn successful_preview_is_terminal_and_next_activation_retires_its_lease_on_worker() {
    let (mut p, _) = loaded(true);
    assert!(p.request_user_command(PaneId::Left, "test-command", true, &|_| true));
    let id = p.command_launches.jobs[0].id;
    let command = p.command_launches.jobs[0].command.clone();
    let preview = Arc::new(UserCommandPreview {
        processes: Arc::from([]),
        retention: UserCommandRetention::new(()),
    });
    let weak = Arc::downgrade(&preview);
    p.apply(Event::UserCommand(Box::new(UserCommandEvent {
        request: id,
        command,
        update: UserCommandUpdate::Preview(preview),
    })));
    assert!(p.command_launches.jobs[0].finished);
    assert!(
        !p.has_live_operations(),
        "Preview is a terminal result, without a later Finished event"
    );
    assert_eq!(p.command_launches.preview_ready, Some(id));
    assert!(p.request_user_command(PaneId::Left, "test-command", false, &|_| true));
    assert!(p.command_launches.jobs[0].preview.is_none());
    assert!(p.command_launches.preview_ready.is_none());
    assert!(
        weak.upgrade().is_some(),
        "large preview data must not be destroyed on the UI callback"
    );
    let retired = p.take_retired();
    std::thread::spawn(move || drop(retired)).join().unwrap();
    assert!(weak.upgrade().is_none());
}

#[test]
fn rejected_unsaved_preview_defers_last_configuration_owner_to_reclamation() {
    let (mut p, _) = loaded(false);
    let configuration = std::mem::replace(
        &mut p.preferences.values.user_commands,
        Arc::new(Vec::new()),
    );
    let weak = Arc::downgrade(&configuration);
    assert!(!p.preview_user_command_configuration(
        PaneId::Left,
        configuration,
        "test-command",
        &|_| false,
    ));
    assert!(
        weak.upgrade().is_some(),
        "queue refusal must not destroy an unsaved configuration on the UI callback"
    );
    let retired = p.take_retired();
    std::thread::spawn(move || drop(retired)).join().unwrap();
    assert!(weak.upgrade().is_none());
}

#[test]
fn disabled_command_allows_explicit_preview_but_never_execution() {
    let (mut p, _) = loaded(false);
    Arc::make_mut(&mut p.preferences.values.user_commands)[0].enabled = false;
    assert!(
        !p.request_user_command(PaneId::Left, "test-command", false, &|_| {
            panic!("disabled command executed")
        })
    );
    assert!(
        p.request_user_command(PaneId::Left, "test-command", true, &|command| {
            assert!(matches!(command, Command::PreviewUserCommand(_)));
            true
        })
    );
}

#[test]
fn jobs_clear_finished_commands_preserves_active_and_error_receipts() {
    let (mut p, _) = loaded(true);
    for index in 0..3 {
        assert!(p.request_user_command(PaneId::Left, "test-command", false, &|_| true));
        if index == 2 {
            continue;
        }
        let job = p.command_launches.jobs.last().unwrap();
        p.apply(Event::UserCommand(Box::new(UserCommandEvent {
            request: job.id,
            command: job.command.clone(),
            update: UserCommandUpdate::Finished(UserCommandSummary {
                planned: 1,
                started: u64::from(index == 0),
                not_started: u64::from(index != 0),
                cancelled: false,
                error: (index == 1).then(|| {
                    FsError::from_io(
                        "launch",
                        std::path::Path::new("program"),
                        std::io::Error::other("Launch denied"),
                    )
                }),
            }),
        })));
    }
    p.clear_finished_jobs(true);
    assert_eq!(p.command_launches.jobs.len(), 2);
    assert!(p.command_launches.jobs[0].error.is_some());
    assert!(!p.command_launches.jobs[1].finished);
    p.clear_finished_jobs(false);
    assert_eq!(p.command_launches.jobs.len(), 1);
    assert!(!p.command_launches.jobs[0].finished);
    assert!(!p.take_retired().is_empty());
}

#[test]
fn completed_command_history_survives_active_admission_until_explicit_clear() {
    let (mut p, _) = loaded(false);
    let mut completed = Vec::new();
    for _ in 0..40 {
        assert!(
            p.request_user_command(PaneId::Left, "test-command", false, &|command| {
                assert!(matches!(command, Command::ExecuteUserCommand(_)));
                true
            })
        );
        let job = p.command_launches.jobs.last().unwrap();
        let id = job.id;
        let command = job.command.clone();
        completed.push(id);
        p.apply(Event::UserCommand(Box::new(UserCommandEvent {
            request: id,
            command,
            update: UserCommandUpdate::Finished(UserCommandSummary {
                planned: 1,
                started: 1,
                not_started: 0,
                cancelled: false,
                error: None,
            }),
        })));
    }
    assert_eq!(p.command_launches.jobs.len(), 40);
    assert!(p.command_launches.jobs.iter().all(|job| job.finished));
    assert_eq!(
        p.command_launches
            .jobs
            .iter()
            .map(|job| job.id)
            .collect::<Vec<_>>(),
        completed
    );

    for _ in 0..32 {
        assert!(p.request_user_command(PaneId::Left, "test-command", false, &|_| true));
    }
    assert!(
        !p.request_user_command(PaneId::Left, "test-command", false, &|_| {
            panic!("the active launch limit must still reject another launch")
        })
    );
    assert_eq!(p.command_launches.jobs.len(), 72);
    p.clear_finished_jobs(false);
    assert_eq!(p.command_launches.jobs.len(), 32);
    assert!(p.command_launches.jobs.iter().all(|job| !job.finished));
    assert!(
        p.command_launches
            .jobs
            .iter()
            .all(|job| !completed.contains(&job.id))
    );

    p.apply(Event::UserCommand(Box::new(UserCommandEvent {
        request: completed[0],
        command: UserCommandId::new("test-command").unwrap(),
        update: UserCommandUpdate::Started {
            index: 99,
            process_id: 100,
        },
    })));
    assert_eq!(p.command_launches.jobs.len(), 32);
    assert!(p.command_launches.jobs.iter().all(|job| job.started == 0));
}
