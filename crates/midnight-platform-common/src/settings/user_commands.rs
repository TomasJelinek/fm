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
    Separator {},
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
                Invocation::Separator {} => UserCommandInvocation::Separator,
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
                UserCommandInvocation::Separator => Invocation::Separator {},
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
impl StoredUserCommand {
    /// The other system's program or icon path, if this command has one.
    fn foreign(&self) -> io::Result<Option<super::native_path::Foreign>> {
        if let Invocation::Program { executable, .. } = &self.invocation
            && let Some(foreign) = executable.foreign()?
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

    /// Keep this record verbatim after validating everything except its foreign paths.
    fn into_foreign(
        mut self,
        position: usize,
        (system, location): super::native_path::Foreign,
    ) -> io::Result<ForeignSetting> {
        let stored = serde_json::to_string(&self).map_err(invalid)?.into();
        if let Invocation::Program { executable, .. } = &mut self.invocation {
            *executable = executable.validation_copy()?;
        }
        self.icon = self
            .icon
            .as_ref()
            .map(NativePath::validation_copy)
            .transpose()?;
        let command = self.into_command()?;
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

/// Commands whose paths belong to another system are kept inactive, in order.
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

/// Return kept commands to their stored positions, around the native ones.
pub(super) fn merge(
    mut commands: Vec<StoredUserCommand>,
    foreign: &[ForeignSetting],
) -> io::Result<Vec<StoredUserCommand>> {
    for (position, stored) in super::foreign::positioned(foreign, |kind| match kind {
        ForeignSettingKind::UserCommand { position, .. } => Some(*position),
        _ => None,
    }) {
        let command = serde_json::from_str(stored).map_err(invalid)?;
        commands.insert(position.min(commands.len()), command);
    }
    Ok(commands)
}
