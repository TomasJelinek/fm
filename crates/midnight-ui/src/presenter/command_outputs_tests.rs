use super::*;
use std::cell::RefCell;

fn tool(output: UserCommandOutput) -> UserCommand {
    UserCommand {
        id: UserCommandId::new("git-clean").unwrap(),
        name: "git clean".into(),
        enabled: true,
        toolbar_label: None,
        icon: None,
        working_directory: "${active.path}".into(),
        launch: UserCommandLaunch::Single,
        invocation: UserCommandInvocation::Program {
            executable: std::env::current_exe().unwrap(),
            arguments: vec!["clean".into(), "-fxd".into()],
        },
        output,
    }
}

fn presenter() -> Presenter {
    let mut p = Presenter::new("left".into(), "right".into());
    p.preferences_loaded(Ok(UserPreferences {
        user_commands: Arc::new(vec![tool(UserCommandOutput::NEW_PROGRAM)]),
        ..Default::default()
    }));
    p
}

fn session(request: u64) -> Arc<UserCommandOutputSession> {
    UserCommandOutputSession::new(
        UserCommandRequestId(request),
        UserCommandId::new("git-clean").unwrap(),
        "git clean".into(),
        1,
    )
}

fn event(session: &Arc<UserCommandOutputSession>, opened: bool) -> Event {
    Event::UserCommandOutput(Box::new(UserCommandOutputEvent {
        session: session.clone(),
        opened,
    }))
}

#[test]
fn an_opened_session_gets_a_window_and_later_news_a_new_revision() {
    let mut p = presenter();
    let first = session(1);
    // News for a session without a window opens none (it was closed).
    p.apply(event(&first, false));
    assert!(p.command_output(UserCommandRequestId(1)).is_none());
    p.apply(event(&first, true));
    let revision = p.command_output(UserCommandRequestId(1)).unwrap().revision;
    p.apply(event(&first, false));
    assert_ne!(
        p.command_output(UserCommandRequestId(1)).unwrap().revision,
        revision
    );
    assert_eq!(p.command_launches.outputs.windows.len(), 1);
}

#[test]
fn beyond_the_window_limit_a_command_is_stopped_and_reported() {
    let mut p = presenter();
    for request in 1..=MAX_OUTPUT_SESSIONS as u64 {
        p.apply(event(&session(request), true));
    }
    let refused = session(99);
    p.apply(event(&refused, true));
    assert!(refused.stop_requested());
    assert!(p.command_output(UserCommandRequestId(99)).is_none());
    let (title, body) = p.desktop.notice.clone().unwrap();
    assert_eq!(title, "Custom Commands");
    assert!(body.contains("output windows are open"), "{body}");
}

#[test]
fn stop_and_closing_a_running_window_end_the_program_and_its_launches() {
    let mut p = presenter();
    let sent = RefCell::new(Vec::new());
    let send = |command: Command| {
        sent.borrow_mut().push(command);
        true
    };
    let running = session(1);
    p.apply(event(&running, true));
    p.stop_command_output(UserCommandRequestId(1), &send);
    assert!(running.stop_requested());
    assert!(matches!(
        sent.borrow().as_slice(),
        [Command::CancelUserCommand(UserCommandRequestId(1))]
    ));
    let other = session(2);
    p.apply(event(&other, true));
    p.close_command_output(UserCommandRequestId(2), &send);
    assert!(other.stop_requested());
    assert!(p.command_output(UserCommandRequestId(2)).is_none());
    // A finished one closes without a further request.
    let done = session(3);
    p.apply(event(&done, true));
    done.finish(false, None);
    sent.borrow_mut().clear();
    p.close_command_output(UserCommandRequestId(3), &send);
    p.stop_command_output(UserCommandRequestId(3), &send);
    assert!(!done.stop_requested());
    assert!(sent.borrow().is_empty());
}

#[test]
fn copy_all_and_clear_act_on_the_retained_lines() {
    let mut p = presenter();
    let sent = RefCell::new(Vec::new());
    let send = |command: Command| {
        sent.borrow_mut().push(command);
        true
    };
    let shown = session(1);
    p.apply(event(&shown, true));
    p.copy_command_output(UserCommandRequestId(1), &send);
    assert_eq!(
        p.command_output(UserCommandRequestId(1)).unwrap().status,
        "Nothing to copy."
    );
    shown.append(
        OutputStream::Stdout,
        vec!["Removing build/".into(), "Removing target/".into()],
        None,
    );
    p.copy_command_output(UserCommandRequestId(1), &send);
    assert_eq!(
        p.command_output(UserCommandRequestId(1)).unwrap().status,
        "Copied 2 lines."
    );
    assert!(
        !sent.borrow().is_empty(),
        "the clipboard worker got the text"
    );
    p.clear_command_output(UserCommandRequestId(1));
    assert!(shown.text().is_empty());
    assert!(
        p.command_output(UserCommandRequestId(1))
            .unwrap()
            .status
            .is_empty()
    );
}

#[test]
fn close_when_finished_successfully_is_remembered_with_the_command() {
    let mut p = presenter();
    let sent = RefCell::new(Vec::new());
    let send = |command: Command| {
        sent.borrow_mut().push(command);
        true
    };
    p.apply(event(&session(1), true));
    assert_eq!(
        p.command_output_closes_on_success(UserCommandRequestId(1)),
        Some(false)
    );
    p.set_command_output_close_on_success(UserCommandRequestId(1), true, &send);
    assert_eq!(
        p.command_output_closes_on_success(UserCommandRequestId(1)),
        Some(true)
    );
    assert!(sent.borrow().iter().any(|command| matches!(
        command,
        Command::SavePreferences { preferences, .. }
            if preferences.user_commands[0].output.close_on_success
    )));
    // A command that no longer shows an output window has no such choice.
    let mut values = p.preferences.values.clone();
    values.user_commands = Arc::new(vec![tool(UserCommandOutput::default())]);
    p.update_preferences(values, &send);
    assert_eq!(
        p.command_output_closes_on_success(UserCommandRequestId(1)),
        None
    );
    sent.borrow_mut().clear();
    p.set_command_output_close_on_success(UserCommandRequestId(1), false, &send);
    assert!(sent.borrow().is_empty());
}

#[test]
fn save_as_picks_saves_and_asks_before_replacing_an_existing_file() {
    let mut p = presenter();
    let sent = RefCell::new(Vec::new());
    let send = |command: Command| {
        sent.borrow_mut().push(command);
        true
    };
    let shown = session(1);
    shown.append(OutputStream::Stdout, vec!["Removing build/".into()], None);
    p.apply(event(&shown, true));
    p.save_command_output(UserCommandRequestId(1), PathPickerOwner::None, &send);
    let picker = match sent.borrow().last() {
        Some(Command::PickPath { request, input }) => {
            assert_eq!(input.kind, PathPickerKind::CommandOutputSave);
            *request
        }
        other => panic!("expected a picker, got {other:?}"),
    };
    assert!(p.command_output(UserCommandRequestId(1)).unwrap().saving());
    // A second Save as waits for the first.
    let count = sent.borrow().len();
    p.save_command_output(UserCommandRequestId(1), PathPickerOwner::None, &send);
    assert_eq!(sent.borrow().len(), count);
    let destination = std::env::temp_dir().join("output 文档.txt");
    p.apply_with_sender(
        Event::PathPicked {
            request: picker,
            result: Ok(Some(destination.clone())),
        },
        &send,
    );
    let save = match sent.borrow().last() {
        Some(Command::SaveUserCommandOutput(save)) => {
            assert_eq!(save.destination, destination);
            assert!(!save.overwrite, "picking grants no overwrite");
            assert!(Arc::ptr_eq(&save.session, &shown));
            save.request
        }
        other => panic!("expected a save, got {other:?}"),
    };
    p.apply(Event::UserCommandOutputSaved {
        request: save,
        result: Err(FsError::from_io(
            "save command output",
            &destination,
            std::io::ErrorKind::AlreadyExists.into(),
        )),
    });
    let prompt = p
        .command_output(UserCommandRequestId(1))
        .unwrap()
        .save_prompt()
        .unwrap();
    assert_eq!(prompt, "output 文档.txt already exists. Replace it?");
    p.confirm_command_output_save(UserCommandRequestId(1), true, &send);
    let replace = match sent.borrow().last() {
        Some(Command::SaveUserCommandOutput(save)) => {
            assert!(save.overwrite, "Replace consents");
            save.request
        }
        other => panic!("expected a replacing save, got {other:?}"),
    };
    p.apply(Event::UserCommandOutputSaved {
        request: replace,
        result: Ok(UserCommandOutputSaveReceipt {
            path: destination.clone(),
            bytes: 16,
            recovery: vec![],
        }),
    });
    let window = p.command_output(UserCommandRequestId(1)).unwrap();
    assert!(!window.saving());
    assert!(
        window.status.starts_with("Saved 16 bytes to "),
        "{}",
        window.status
    );
    // Cancel at the question saves nothing.
    p.save_command_output(UserCommandRequestId(1), PathPickerOwner::None, &send);
    let picker = match sent.borrow().last() {
        Some(Command::PickPath { request, .. }) => *request,
        other => panic!("expected a picker, got {other:?}"),
    };
    p.apply_with_sender(
        Event::PathPicked {
            request: picker,
            result: Ok(Some(destination.clone())),
        },
        &send,
    );
    let save = match sent.borrow().last() {
        Some(Command::SaveUserCommandOutput(save)) => save.request,
        other => panic!("expected a save, got {other:?}"),
    };
    p.apply(Event::UserCommandOutputSaved {
        request: save,
        result: Err(FsError::from_io(
            "save command output",
            &destination,
            std::io::ErrorKind::AlreadyExists.into(),
        )),
    });
    let count = sent.borrow().len();
    p.confirm_command_output_save(UserCommandRequestId(1), false, &send);
    assert_eq!(sent.borrow().len(), count);
    assert_eq!(
        p.command_output(UserCommandRequestId(1)).unwrap().status,
        "Not saved."
    );
}
