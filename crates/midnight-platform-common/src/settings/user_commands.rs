//! Strict, ordered schema17 DTOs. Native paths use the existing lossless per-OS codec.
use super::{NativePath, invalid};
use midnight_core::*;
use serde::{Deserialize, Serialize};
use std::io;

#[derive(Serialize, Deserialize)]
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
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Launch {
    Single,
    PerItem,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "kebab-case", deny_unknown_fields)]
enum Invocation {
    Program {
        executable: NativePath,
        arguments: Vec<String>,
    },
    Terminal {
        shell: String,
        script: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        variables: Vec<String>,
    },
}
impl StoredUserCommand {
    fn into_command(self) -> io::Result<UserCommand> {
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
                Invocation::Program {
                    executable,
                    arguments,
                } => UserCommandInvocation::Program {
                    executable: executable.into_path()?,
                    arguments,
                },
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
        };
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
                UserCommandInvocation::Program {
                    executable,
                    arguments,
                } => Invocation::Program {
                    executable: NativePath::from_path(executable),
                    arguments,
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
pub(super) fn decode(
    version: u32,
    commands: Option<Vec<StoredUserCommand>>,
) -> io::Result<Vec<UserCommand>> {
    let commands = match (version, commands) {
        (1..=16, None) => return Ok(Vec::new()),
        (17..=super::CURRENT_PREFERENCES_VERSION, Some(commands))
            if commands.len() <= MAX_USER_COMMANDS =>
        {
            commands
        }
        _ => return Err(invalid("missing, unexpected or excessive Custom Commands")),
    }
    .into_iter()
    .map(StoredUserCommand::into_command)
    .collect::<io::Result<Vec<_>>>()?;
    validate_user_commands(&commands).map_err(invalid)?;
    Ok(commands)
}
