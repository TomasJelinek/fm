//! Bounded exact-text command history. Entries never store elevation or credentials separately.
use crate::ShellFlavor;
use std::collections::HashSet;

pub const MAX_COMMAND_HISTORY: usize = 100;
/// Aggregate retention budget, not a limit on commands that may be executed.
pub const MAX_COMMAND_HISTORY_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandHistoryEntry {
    pub command: String,
    pub shell: ShellFlavor,
}

pub fn validate_command_history(history: &[CommandHistoryEntry]) -> Result<(), &'static str> {
    if history.len() > MAX_COMMAND_HISTORY {
        return Err("At most 100 command history entries can be saved.");
    }
    let mut bytes = 0usize;
    let mut commands = HashSet::new();
    for entry in history {
        if entry.command.trim().is_empty() || entry.command.contains('\0') {
            return Err("Command history cannot contain blank commands or NUL.");
        }
        bytes = bytes.saturating_add(entry.command.len());
        if bytes > MAX_COMMAND_HISTORY_BYTES {
            return Err("Command history exceeds its 1 MiB retention budget.");
        }
        if !commands.insert(entry.command.as_str()) {
            return Err("Command history must not contain duplicate command text.");
        }
    }
    Ok(())
}

/// Retain the exact command once, newest first, updating its shell on re-execution.
/// Returns false for unchanged or unretainable input; never truncates command text.
pub fn remember_command(
    history: &mut Vec<CommandHistoryEntry>,
    entry: CommandHistoryEntry,
) -> bool {
    if entry.command.trim().is_empty()
        || entry.command.contains('\0')
        || entry.command.len() > MAX_COMMAND_HISTORY_BYTES
        || history.first() == Some(&entry)
    {
        return false;
    }
    history.retain(|old| old.command != entry.command);
    history.insert(0, entry);
    history.truncate(MAX_COMMAND_HISTORY);
    let mut bytes = 0usize;
    let retained = history
        .iter()
        .take_while(|entry| {
            bytes = bytes.saturating_add(entry.command.len());
            bytes <= MAX_COMMAND_HISTORY_BYTES
        })
        .count();
    history.truncate(retained);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(text: &str, shell: ShellFlavor) -> CommandHistoryEntry {
        CommandHistoryEntry {
            command: text.into(),
            shell,
        }
    }

    #[test]
    fn history_deduplicates_exact_text_across_shells_and_keeps_latest_shell() {
        let mut history = Vec::new();
        let literal = "  echo 文档 e\u{301} 👩🏽‍💻  ";
        assert!(remember_command(
            &mut history,
            command(literal, ShellFlavor::Cmd)
        ));
        assert!(remember_command(
            &mut history,
            command("next", ShellFlavor::Posix)
        ));
        assert!(remember_command(
            &mut history,
            command(literal, ShellFlavor::PowerShell)
        ));
        assert_eq!(
            history,
            vec![
                command(literal, ShellFlavor::PowerShell),
                command("next", ShellFlavor::Posix)
            ]
        );
        assert!(!remember_command(
            &mut history,
            command(literal, ShellFlavor::PowerShell)
        ));
        assert!(remember_command(
            &mut history,
            command(literal.trim(), ShellFlavor::Cmd)
        ));
        assert_eq!(
            history.len(),
            3,
            "spaces are part of the exact executed command"
        );
    }

    #[test]
    fn history_keeps_last_hundred_and_never_truncates_an_oversized_command() {
        let mut history = Vec::new();
        for index in 0..105 {
            remember_command(
                &mut history,
                command(&format!("echo {index}"), ShellFlavor::Posix),
            );
        }
        assert_eq!(history.len(), 100);
        assert_eq!(history[0].command, "echo 104");
        assert_eq!(history[99].command, "echo 5");
        let before = history.clone();
        for text in [
            " ".into(),
            "a\0b".into(),
            "x".repeat(MAX_COMMAND_HISTORY_BYTES + 1),
        ] {
            assert!(!remember_command(
                &mut history,
                command(&text, ShellFlavor::Posix)
            ));
            assert_eq!(history, before);
        }
        let large = "字".repeat(MAX_COMMAND_HISTORY_BYTES / 3);
        remember_command(&mut history, command(&large, ShellFlavor::Cmd));
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].command, large);
        assert!(validate_command_history(&history).is_ok());
    }
}
