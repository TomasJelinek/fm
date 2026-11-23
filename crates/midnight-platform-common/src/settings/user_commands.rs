//! Strict, ordered schema17 DTOs. Native paths use the existing lossless per-OS codec.
use super::{NativePath, invalid};
use midnight_core::*;
use serde::{Deserialize, Serialize};
use std::io;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StoredUserCommand {
    id: String,
    name: String,
    enabled: bool,
    #[serde(
        default,
        deserialize_with = "super::present",
        skip_serializing_if = "Option::is_none"
    )]
    toolbar_label: Option<String>,
    #[serde(
        default,
        deserialize_with = "super::present",
        skip_serializing_if = "Option::is_none"
    )]
    icon: Option<NativePath>,
    working_directory: String,
    launch: Launch,
    invocation: Invocation,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Launch {
    Single,
    PerItem,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "kebab-case", deny_unknown_fields)]
enum Invocation {
    Separator {},
    Program {
        executable: NativePath,
        arguments: Vec<String>,
        /// Schema 39: where the program's output goes; absent means hidden.
        #[serde(
            default,
            deserialize_with = "super::present",
            skip_serializing_if = "Option::is_none"
        )]
        output: Option<StoredOutput>,
    },
    Terminal {
        shell: String,
        script: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        variables: Vec<String>,
    },
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredOutput {
    window: OutputWindow,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    close_on_success: bool,
}
#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum OutputWindow {
    Hidden,
    Output,
    Console,
}
impl StoredOutput {
    fn from_output(output: UserCommandOutput) -> Option<Self> {
        (output != UserCommandOutput::default()).then_some(Self {
            window: match output.mode {
                UserCommandOutputMode::Hidden => OutputWindow::Hidden,
                UserCommandOutputMode::Window => OutputWindow::Output,
                UserCommandOutputMode::Console => OutputWindow::Console,
            },
            close_on_success: output.close_on_success,
        })
    }
    fn into_output(self) -> UserCommandOutput {
        UserCommandOutput {
            mode: match self.window {
                OutputWindow::Hidden => UserCommandOutputMode::Hidden,
                OutputWindow::Output => UserCommandOutputMode::Window,
                OutputWindow::Console => UserCommandOutputMode::Console,
            },
            close_on_success: self.close_on_success,
        }
    }
}
impl StoredUserCommand {
    /// Whether this record needs schema 39 (an output choice).
    fn has_output(&self) -> bool {
        matches!(
            &self.invocation,
            Invocation::Program {
                output: Some(_),
                ..
            }
        )
    }
    fn into_command(self) -> io::Result<UserCommand> {
        let mut output = UserCommandOutput::default();
        let command = UserCommand {
            id: UserCommandId::new(self.id).map_err(invalid)?,
            name: self.name,
            enabled: self.enabled,
            toolbar_label: self.toolbar_label,
            icon: self.icon.map(NativePath::into_path).transpose()?,
            working_directory: self.working_directory,
            launch: match self.launch {
                Launch::Single => UserCommandLaunch::Single,
                Launch::PerItem => UserCommandLaunch::PerItem,
            },
            invocation: match self.invocation {
                Invocation::Separator {} => UserCommandInvocation::Separator,
                Invocation::Program {
                    executable,
                    arguments,
                    output: stored,
                } => {
                    output =
                        stored.map_or_else(UserCommandOutput::default, StoredOutput::into_output);
                    UserCommandInvocation::Program {
                        executable: executable.into_path()?,
                        arguments,
                    }
                }
                Invocation::Terminal {
                    shell,
                    script,
                    variables,
                } => UserCommandInvocation::Terminal {
                    shell: match shell.as_str() {
                        "powershell" => ShellFlavor::PowerShell,
                        "cmd" => ShellFlavor::Cmd,
                        "posix" => ShellFlavor::Posix,
                        _ => return Err(invalid("unknown Custom Command shell")),
                    },
                    script,
                    variables: variables
                        .into_iter()
                        .map(|name| UserCommandVariable::parse(&name).map_err(invalid))
                        .collect::<io::Result<_>>()?,
                },
            },
            output: UserCommandOutput::default(),
        };
        let command = UserCommand { output, ..command };
        command.validate().map_err(invalid)?;
        Ok(command)
    }
}
impl From<UserCommand> for StoredUserCommand {
    fn from(command: UserCommand) -> Self {
        Self {
            id: command.id.as_str().to_owned(),
            name: command.name,
            enabled: command.enabled,
            toolbar_label: command.toolbar_label,
            icon: command.icon.map(NativePath::from_path),
            working_directory: command.working_directory,
            launch: match command.launch {
                UserCommandLaunch::Single => Launch::Single,
                UserCommandLaunch::PerItem => Launch::PerItem,
            },
            invocation: match command.invocation {
                UserCommandInvocation::Separator => Invocation::Separator {},
                UserCommandInvocation::Program {
                    executable,
                    arguments,
                } => Invocation::Program {
                    executable: NativePath::from_path(executable),
                    arguments,
                    output: StoredOutput::from_output(command.output),
                },
                UserCommandInvocation::Terminal {
                    shell,
                    script,
                    variables,
                } => Invocation::Terminal {
                    shell: match shell {
                        ShellFlavor::PowerShell => "powershell",
                        ShellFlavor::Cmd => "cmd",
                        ShellFlavor::Posix => "posix",
                    }
                    .to_owned(),
                    script,
                    variables: variables
                        .into_iter()
                        .map(|variable| variable.name().to_owned())
                        .collect(),
                },
            },
        }
    }
}

/// The system whose terminal runs this shell, as each desktop adapter allows: the
/// Windows terminal runs PowerShell and Command Prompt scripts and refuses POSIX
/// ones, the Linux terminal runs only POSIX shell scripts.
fn shell_system(shell: &str) -> Option<(PathSystem, &'static str)> {
    match shell {
        "powershell" => Some((PathSystem::Windows, "PowerShell")),
        "cmd" => Some((PathSystem::Windows, "Command Prompt")),
        "posix" => Some((PathSystem::Unix, "POSIX shell")),
        _ => None,
    }
}

/// The other system, as seen from `current`, when a command's working folder or shell
/// belongs to it, with the text that says so: a working folder that is absolute only
/// in that system's syntax, or a shell only that system's terminal runs.
fn foreign_place(
    current: PathSystem,
    working_directory: &str,
    shell: Option<&str>,
) -> Option<super::native_path::Foreign> {
    if let Some(system) = PathSystem::foreign_for(current, working_directory) {
        return Some((system, working_directory.to_owned()));
    }
    shell
        .and_then(shell_system)
        .filter(|(system, _)| *system != current)
        .map(|(system, name)| (system, name.to_owned()))
}

/// In memory only, as a kept command's `ForeignSetting::stored`: its stored record and
/// the key binding its own system gives it. In a settings file that binding stays in
/// the shortcut list, where `with_kept_bindings` writes it back.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct KeptUserCommand {
    command: StoredUserCommand,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    keys: Option<Vec<String>>,
}

impl StoredUserCommand {
    /// What makes this command the other system's, if anything: its program, its
    /// working folder, its shell or its icon, in that order.
    fn foreign(&self) -> io::Result<Option<super::native_path::Foreign>> {
        if let Invocation::Program { executable, .. } = &self.invocation
            && let Some(foreign) = executable.foreign()?
        {
            return Ok(Some(foreign));
        }
        // A separator runs nothing: neither a working folder nor a shell matters.
        let (runs, shell) = match &self.invocation {
            Invocation::Separator {} => (false, None),
            Invocation::Program { .. } => (true, None),
            Invocation::Terminal { shell, .. } => (true, Some(shell.as_str())),
        };
        if runs
            && let Some(foreign) =
                foreign_place(PathSystem::CURRENT, &self.working_directory, shell)
        {
            return Ok(Some(foreign));
        }
        Ok(self
            .icon
            .as_ref()
            .map(NativePath::foreign)
            .transpose()?
            .flatten())
    }

    /// This command as validation sees it: foreign paths replaced by native stand-ins.
    fn validation_command(mut self) -> io::Result<UserCommand> {
        if let Invocation::Program { executable, .. } = &mut self.invocation {
            *executable = executable.validation_copy()?;
        }
        self.icon = self
            .icon
            .as_ref()
            .map(NativePath::validation_copy)
            .transpose()?;
        self.into_command()
    }

    /// Keep this record verbatim after validating everything except its foreign paths.
    fn into_foreign(
        self,
        position: usize,
        (system, location): super::native_path::Foreign,
    ) -> io::Result<ForeignSetting> {
        let stored = serde_json::to_string(&KeptUserCommand {
            command: self.clone(),
            keys: None,
        })
        .map_err(invalid)?
        .into();
        let command = self.validation_command()?;
        Ok(ForeignSetting {
            kind: ForeignSettingKind::UserCommand {
                position,
                id: command.id,
                name: command.name,
            },
            system,
            location,
            stored,
        })
    }
}

/// Move the key binding of each kept command out of the active ones, into the kept
/// command's record: here it would name no command, so it does nothing until the
/// command returns to its own system. Returns how many bindings moved. A command
/// bound twice is refused, as its own system refuses it.
pub(super) fn keep_bindings(
    kept: &mut [ForeignSetting],
    shortcuts: &mut Vec<ShortcutBinding>,
) -> io::Result<usize> {
    let mut moved = 0;
    for setting in kept {
        let ForeignSettingKind::UserCommand { id, .. } = &setting.kind else {
            continue;
        };
        let action = id.action();
        let Some(index) = shortcuts
            .iter()
            .position(|binding| binding.action == action)
        else {
            continue;
        };
        if shortcuts[index + 1..]
            .iter()
            .any(|binding| binding.action == action)
        {
            return Err(invalid(
                "repeated key binding of a Custom Command from another system",
            ));
        }
        let binding = shortcuts.remove(index);
        let mut record: KeptUserCommand = serde_json::from_str(&setting.stored).map_err(invalid)?;
        record.keys = Some(binding.keys);
        setting.stored = serde_json::to_string(&record).map_err(invalid)?.into();
        moved += 1;
    }
    Ok(moved)
}

/// The active key bindings followed by those kept commands keep, as a settings file
/// holds them. The file must stay valid on the kept commands' own system, which reads
/// those bindings beside the others: a key that a binding made here took (or one that
/// no longer fits the limits) is dropped from the kept binding, and the kept binding
/// with it once no key is left.
pub(super) fn with_kept_bindings(
    mut shortcuts: Vec<ShortcutBinding>,
    commands: &[UserCommand],
    foreign: &[ForeignSetting],
) -> io::Result<Vec<ShortcutBinding>> {
    let mut all = commands.to_vec();
    let mut kept = Vec::new();
    for setting in foreign {
        if !matches!(setting.kind, ForeignSettingKind::UserCommand { .. }) {
            continue;
        }
        let record: KeptUserCommand = serde_json::from_str(&setting.stored).map_err(invalid)?;
        if let Some(keys) = record.keys {
            let command = record.command.validation_command()?;
            kept.push(ShortcutBinding {
                action: command.id.action(),
                keys,
            });
            all.push(command);
        }
    }
    if kept.is_empty() {
        return Ok(shortcuts);
    }
    let valid =
        |bindings: &[ShortcutBinding]| ShortcutMap::with_user_commands(bindings, &all).is_ok();
    let active = shortcuts.len();
    shortcuts.extend(kept.iter().cloned());
    if valid(&shortcuts) {
        return Ok(shortcuts);
    }
    shortcuts.truncate(active);
    for binding in kept {
        let unbound = binding.keys.is_empty();
        shortcuts.push(ShortcutBinding {
            action: binding.action,
            keys: Vec::new(),
        });
        let last = shortcuts.len() - 1;
        for key in binding.keys {
            shortcuts[last].keys.push(key);
            if !valid(&shortcuts) {
                shortcuts[last].keys.pop();
            }
        }
        let keep = if unbound {
            valid(&shortcuts)
        } else {
            !shortcuts[last].keys.is_empty()
        };
        if !keep {
            shortcuts.pop();
        }
    }
    Ok(shortcuts)
}

/// Commands whose paths, working folder or shell belong to another system are kept
/// inactive, in order.
pub(super) fn decode(
    version: u32,
    commands: Option<Vec<StoredUserCommand>>,
) -> io::Result<(Vec<UserCommand>, Vec<ForeignSetting>)> {
    let stored = match (version, commands) {
        (1..=16, None) => return Ok((Vec::new(), Vec::new())),
        (17..=super::CURRENT_PREFERENCES_VERSION, Some(commands))
            if commands.len() <= MAX_USER_COMMANDS =>
        {
            commands
        }
        _ => return Err(invalid("missing, unexpected or excessive Custom Commands")),
    };
    // Schema 39 added the output choice, and a schema 39 file has one.
    let outputs = stored.iter().any(StoredUserCommand::has_output);
    if (version < 39 && outputs) || (version == 39 && !outputs) {
        return Err(invalid(
            "missing or unexpected Custom Command output choice",
        ));
    }
    let (mut commands, mut foreign) = (Vec::new(), Vec::new());
    for (position, command) in stored.into_iter().enumerate() {
        match command.foreign()? {
            Some(found) => foreign.push(command.into_foreign(position, found)?),
            None => commands.push(command.into_command()?),
        }
    }
    validate_user_commands(&commands).map_err(invalid)?;
    Ok((commands, foreign))
}

/// Whether a command kept for another system has an output choice (schema 39).
pub(super) fn kept_output(foreign: &[ForeignSetting]) -> io::Result<bool> {
    for setting in foreign {
        if matches!(setting.kind, ForeignSettingKind::UserCommand { .. }) {
            let kept: KeptUserCommand = serde_json::from_str(&setting.stored).map_err(invalid)?;
            if kept.command.has_output() {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Return kept commands to their stored positions, around the native ones. Their key
/// bindings return through `with_kept_bindings`.
pub(super) fn merge(
    mut commands: Vec<StoredUserCommand>,
    foreign: &[ForeignSetting],
) -> io::Result<Vec<StoredUserCommand>> {
    for (position, stored) in super::foreign::positioned(foreign, |kind| match kind {
        ForeignSettingKind::UserCommand { position, .. } => Some(*position),
        _ => None,
    }) {
        let kept: KeptUserCommand = serde_json::from_str(stored).map_err(invalid)?;
        commands.insert(position.min(commands.len()), kept.command);
    }
    Ok(commands)
}

#[cfg(test)]
#[path = "user_commands_tests.rs"]
mod tests;
