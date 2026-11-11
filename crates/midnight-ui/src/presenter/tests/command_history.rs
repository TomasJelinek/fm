use super::*;
use std::cell::RefCell;

fn entry(command: &str, shell: ShellFlavor) -> CommandHistoryEntry {
    CommandHistoryEntry {
        command: command.into(),
        shell,
    }
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
        theme: PreferenceTheme::Dark,
        command_history: vec![
            entry("echo 文档", ShellFlavor::Cmd),
            entry("older", ShellFlavor::Cmd),
        ],
        ..Default::default()
    }));
    assert_eq!(p.preferences.values.size_display, SizeDisplay::Bytes);
    assert_eq!(p.preferences.values.theme, PreferenceTheme::Dark);
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
