use super::*;
use std::cell::RefCell;

fn entry(command: &str, shell: ShellFlavor) -> CommandHistoryEntry {
    CommandHistoryEntry {
        command: command.into(),
        shell,
    }
}

#[test]
fn rotation_handles_empty_single_and_replaced_history_without_changing_terminal_mode() {
    let mut p = Presenter::new("left".into(), "right".into());
    p.desktop.shell = Some(ShellFlavor::Posix);
    p.desktop.terminal_mode = TerminalMode::Elevated;
    assert_eq!(p.rotate_command_history(false, "draft"), None);
    p.record_command_history(entry("one 文档", ShellFlavor::Posix));
    assert_eq!(
        p.rotate_command_history(false, "draft").as_deref(),
        Some("one 文档")
    );
    for older in [false, true] {
        assert_eq!(
            p.rotate_command_history(older, "one 文档").as_deref(),
            Some("one 文档")
        );
        assert_eq!(p.effective_terminal_mode(), TerminalMode::Elevated);
        assert_eq!(p.effective_shell(), Some(ShellFlavor::Posix));
        assert!(p.desktop.terminal_pending.is_none());
    }
    p.clear_command_history(&|_| true);
    assert_eq!(p.rotate_command_history(true, "one 文档"), None);
    p.record_command_history(entry("replacement", ShellFlavor::Posix));
    assert_eq!(
        p.rotate_command_history(true, "one 文档").as_deref(),
        Some("replacement")
    );
}

#[test]
fn local_history_and_byte_setting_survive_late_load_without_losing_saved_commands() {
    let mut p = Presenter::new("left".into(), "right".into());
    let sent = RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    p.change_size_display(true, &send);
    p.record_command_history(entry("echo 文档", ShellFlavor::PowerShell));
    p.flush_loaded_preferences(&send);
    assert!(
        sent.borrow().is_empty(),
        "initial disk read must finish before saving"
    );
    p.preferences_loaded(Ok(UserPreferences {
        dark_theme: PreferenceTheme::Dark,
        command_history: vec![
            entry("echo 文档", ShellFlavor::Cmd),
            entry("older", ShellFlavor::Cmd),
        ],
        ..Default::default()
    }));
    assert_eq!(p.preferences.values.size_display, SizeDisplay::Bytes);
    assert_eq!(p.preferences.values.dark_theme, PreferenceTheme::Dark);
    assert_eq!(
        p.preferences.values.command_history,
        vec![
            entry("echo 文档", ShellFlavor::PowerShell),
            entry("older", ShellFlavor::Cmd)
        ]
    );
    p.flush_loaded_preferences(&send);
    assert!(
        matches!(&sent.borrow()[0], Command::SavePreferences { preferences, .. }
        if preferences.command_history == p.preferences.values.command_history && preferences.size_display == SizeDisplay::Bytes)
    );
}

#[test]
fn clear_before_load_does_not_resurrect_saved_history_and_new_commands_remain() {
    let mut p = Presenter::new("left".into(), "right".into());
    p.clear_command_history(&|_| panic!("cannot save before loading"));
    p.record_command_history(entry("new", ShellFlavor::Posix));
    p.preferences_loaded(Ok(UserPreferences {
        command_history: vec![entry("old", ShellFlavor::Posix)],
        ..Default::default()
    }));
    assert_eq!(
        p.preferences.values.command_history,
        vec![entry("new", ShellFlavor::Posix)]
    );
}

#[test]
fn recall_preserves_exact_text_and_shell_without_reviving_sudo_or_executing() {
    let mut p = Presenter::new("left".into(), "right".into());
    p.desktop.shell = Some(ShellFlavor::Posix);
    p.desktop.terminal_mode = TerminalMode::Elevated;
    let command = "  printf '%s' '文档 👩🏽‍💻'  ";
    p.record_command_history(entry(command, ShellFlavor::Posix));
    let token = p.preferences.values.command_history[0].command.clone();
    assert_eq!(token, command);
    assert_eq!(p.recall_command(&token).as_deref(), Some(command));
    assert_eq!(p.effective_terminal_mode(), TerminalMode::Standard);
    assert_eq!(p.effective_shell(), Some(ShellFlavor::Posix));
    assert!(p.desktop.requests.is_empty());
    p.clear_command_history(&|_| true);
    assert_eq!(
        p.recall_command(&token),
        None,
        "stale choice cannot select a replacement row"
    );
}

#[test]
fn failed_launch_is_not_remembered_and_success_uses_frozen_backend_text() {
    let mut p = Presenter::new("left".into(), "right".into());
    p.desktop.shell = Some(ShellFlavor::PowerShell);
    let sent = RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    assert!(p.submit_command("failed", &send));
    let first = p.desktop.terminal_pending.unwrap();
    p.apply(Event::DesktopActionFinished {
        request: first,
        result: Err(FsError::from_io(
            "launch",
            std::path::Path::new(""),
            std::io::ErrorKind::NotFound.into(),
        )),
    });
    assert!(p.preferences.values.command_history.is_empty());
    assert!(p.submit_command("echo original", &send));
    let second = p.desktop.terminal_pending.unwrap();
    p.track_draft(true);
    p.apply(Event::DesktopActionFinished {
        request: second,
        result: Ok(DesktopActionResult::TerminalStarted {
            process_id: 1,
            history: entry("echo original", ShellFlavor::PowerShell),
        }),
    });
    assert_eq!(
        p.preferences.values.command_history,
        vec![entry("echo original", ShellFlavor::PowerShell)]
    );
    p.apply(Event::DesktopActionFinished {
        request: second,
        result: Ok(DesktopActionResult::TerminalStarted {
            process_id: 1,
            history: entry("stale duplicate", ShellFlavor::PowerShell),
        }),
    });
    assert_eq!(p.preferences.values.command_history.len(), 1);
}

#[test]
fn recall_rejects_a_shell_from_another_platform_and_keeps_current_draft_grammar() {
    let mut p = Presenter::new("left".into(), "right".into());
    p.desktop.shell = Some(ShellFlavor::Posix);
    p.track_draft(true);
    p.record_command_history(entry("echo %PATH%", ShellFlavor::Cmd));
    assert!(p.recall_command("echo %PATH%").is_none());
    assert_eq!(p.effective_shell(), Some(ShellFlavor::Posix));
    assert!(p.desktop.terminal_pending.is_none());
}

fn saved_histories(sent: &RefCell<Vec<Command>>) -> Vec<Vec<CommandHistoryEntry>> {
    sent.borrow()
        .iter()
        .filter_map(|command| match command {
            Command::SavePreferences { preferences, .. } => {
                Some(preferences.command_history.clone())
            }
            _ => None,
        })
        .collect()
}

#[test]
fn commands_run_while_history_is_not_remembered_stay_session_only() {
    let secret = "curl -H 'Authorization: Bearer 文档'";
    let mut p = Presenter::new("left".into(), "right".into());
    p.preferences_loaded(Ok(UserPreferences {
        command_history: vec![entry("saved", ShellFlavor::Posix)],
        ..Default::default()
    }));
    let sent = RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    p.change_remember_command_history(false, &send);
    p.preferences_saved(p.preferences.revision, Ok(()));
    sent.borrow_mut().clear();
    let revision = p.preferences.revision;
    p.record_command_history(entry(secret, ShellFlavor::Posix));
    p.flush_loaded_preferences(&send);
    assert!(
        saved_histories(&sent).is_empty(),
        "the stored history does not change while it is not remembered"
    );
    assert_ne!(p.preferences.revision, revision, "recall still refreshes");
    assert_eq!(p.rotate_command_history(true, "").as_deref(), Some(secret));

    p.change_remember_command_history(true, &send);
    let saved = saved_histories(&sent);
    assert!(!saved.is_empty());
    assert!(
        saved
            .iter()
            .all(|history| history.iter().all(|entry| entry.command != secret)),
        "enabling the option later must not persist the earlier command"
    );
    sent.borrow_mut().clear();
    // The same command and shell already head the recall list.
    p.record_command_history(entry(secret, ShellFlavor::Posix));
    p.flush_loaded_preferences(&send);
    assert!(
        saved_histories(&sent)
            .last()
            .is_some_and(|history| history.first() == Some(&entry(secret, ShellFlavor::Posix))),
        "running it again while remembering authorizes persistence"
    );
}

#[test]
fn session_only_command_supersedes_a_save_in_flight_so_its_result_completes() {
    let mut p = Presenter::new("left".into(), "right".into());
    p.preferences_loaded(Ok(UserPreferences {
        remember_command_history: false,
        ..Default::default()
    }));
    let sent = RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    p.change_panel_split(0.4, &send);
    let in_flight = p.preferences.revision;
    p.record_command_history(entry("ls -la", ShellFlavor::Posix));
    p.flush_loaded_preferences(&send);
    let revisions = sent
        .borrow()
        .iter()
        .filter_map(|command| match command {
            Command::SavePreferences { revision, .. } => Some(*revision),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(revisions.len(), 2, "the new revision is saved too");
    assert!(
        saved_histories(&sent).iter().all(Vec::is_empty),
        "the session-only command is not stored"
    );
    p.preferences_saved(in_flight, Ok(()));
    assert!(p.preferences.unsaved, "the newer save is still running");
    p.preferences_saved(revisions[1], Ok(()));
    assert_eq!(p.preferences_save_status(), "Changes saved");
    assert!(p.pending_preferences_save().is_none());
}
