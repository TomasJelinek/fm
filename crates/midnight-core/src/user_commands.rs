//! User-authored command configuration. Process preparation/execution belongs to backends.
use crate::{ShellFlavor, ShortcutBinding, UserCommandTemplate};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

pub const MAX_USER_COMMANDS: usize = 128;
pub const MAX_USER_COMMAND_ARGUMENTS: usize = 64;
pub const MAX_USER_COMMAND_EXPANDED_ARGUMENTS: usize = 4096 + MAX_USER_COMMAND_ARGUMENTS;
pub const MAX_USER_COMMAND_CONFIG_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct UserCommandId(String);
impl UserCommandId {
    pub fn new(value: impl Into<String>) -> Result<Self, &'static str> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        {
            return Err("Command IDs require 1..64 ASCII letters, digits, underscores or hyphens.");
        }
        Ok(Self(value))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
    pub fn action(&self) -> String {
        format!("user-command.{}", self.0)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UserCommandLaunch {
    Single,
    PerItem,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UserCommandInvocation {
    /// Inert ordered grouping item; never becomes a native process plan.
    Separator,
    Program {
        executable: PathBuf,
        arguments: Vec<String>,
    },
    /// Literal user-authored source; captured values arrive through native environment data.
    Terminal {
        shell: ShellFlavor,
        script: String,
        variables: Vec<crate::UserCommandVariable>,
    },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserCommand {
    pub id: UserCommandId,
    pub name: String,
    pub enabled: bool,
    pub toolbar_label: Option<String>,
    pub icon: Option<PathBuf>,
    pub working_directory: String,
    pub launch: UserCommandLaunch,
    pub invocation: UserCommandInvocation,
}
impl UserCommand {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.name.trim().is_empty()
            || self.name.len() > 512
            || self.name.chars().any(char::is_control)
        {
            return Err(
                "Command names require nonempty text up to 512 bytes without control characters.",
            );
        }
        if self
            .toolbar_label
            .as_ref()
            .is_some_and(|label| label.len() > 512 || label.chars().any(char::is_control))
        {
            return Err("Command toolbar text exceeds 512 bytes or contains control characters.");
        }
        if let Some(icon) = &self.icon {
            native_path(icon)?;
        }
        if !matches!(self.invocation, UserCommandInvocation::Separator) {
            let directory = UserCommandTemplate::parse(&self.working_directory)?;
            if self.working_directory.is_empty() || directory.is_group() {
                return Err("Working directory requires one scalar path template.");
            }
        }
        match &self.invocation {
            UserCommandInvocation::Separator => {}
            UserCommandInvocation::Program {
                executable,
                arguments,
            } => {
                native_path(executable)?;
                if arguments.len() > MAX_USER_COMMAND_ARGUMENTS {
                    return Err("Program commands accept at most 64 stored arguments.");
                }
                for argument in arguments {
                    if UserCommandTemplate::parse(argument)?.is_group()
                        && self.launch == UserCommandLaunch::PerItem
                    {
                        return Err(
                            "Grouped selected paths and per-item execution cannot be combined.",
                        );
                    }
                }
            }
            UserCommandInvocation::Terminal {
                script, variables, ..
            } => {
                if script.trim().is_empty() || script.contains('\0') {
                    return Err("Terminal source cannot be empty or contain NUL.");
                }
                let mut seen = HashSet::new();
                if variables.len() > 6
                    || variables.iter().any(|variable| {
                        variable.environment_key().is_none() || !seen.insert(*variable)
                    })
                {
                    return Err(
                        "Terminal context declarations require unique scalar variables only.",
                    );
                }
            }
        }
        if self.storage_bytes() > MAX_USER_COMMAND_CONFIG_BYTES {
            return Err("Command exceeds the 1 MiB configuration resource budget.");
        }
        Ok(())
    }
    fn storage_bytes(&self) -> usize {
        let mut bytes = std::mem::size_of::<Self>()
            + self.id.as_str().len()
            + self.name.len()
            + self.working_directory.len()
            + self.toolbar_label.as_ref().map_or(0, String::len)
            + self
                .icon
                .as_ref()
                .map_or(0, |path| path.as_os_str().as_encoded_bytes().len());
        bytes = bytes.saturating_add(match &self.invocation {
            UserCommandInvocation::Separator => 0,
            UserCommandInvocation::Program {
                executable,
                arguments,
            } => executable
                .as_os_str()
                .as_encoded_bytes()
                .len()
                .saturating_add(arguments.iter().fold(0usize, |sum, argument| {
                    sum.saturating_add(argument.len() + std::mem::size_of::<String>())
                })),
            UserCommandInvocation::Terminal { script, .. } => script.len(),
        });
        bytes
    }
}
fn native_path(path: &Path) -> Result<(), &'static str> {
    if !path.is_absolute() || path.as_os_str().as_encoded_bytes().contains(&0) {
        return Err("Command executable/icon must be an absolute native path without NUL.");
    }
    Ok(())
}
pub fn validate_user_commands(commands: &[UserCommand]) -> Result<(), &'static str> {
    if commands.len() > MAX_USER_COMMANDS {
        return Err("At most 128 Custom Commands can be configured.");
    }
    let mut ids = HashSet::new();
    let mut bytes = 0usize;
    for command in commands {
        command.validate()?;
        if !ids.insert(&command.id) {
            return Err("Custom Command IDs must be unique.");
        }
        bytes = bytes.saturating_add(command.storage_bytes());
        if bytes > MAX_USER_COMMAND_CONFIG_BYTES {
            return Err("Custom Commands exceed the 1 MiB aggregate configuration budget.");
        }
    }
    Ok(())
}

/// Apply only within an accepted configuration update; preserve every unrelated override.
pub fn remove_deleted_user_command_shortcuts(
    old: &[UserCommand],
    new: &[UserCommand],
    overrides: &mut Vec<ShortcutBinding>,
) {
    let kept: HashSet<_> = new
        .iter()
        .filter(|command| !matches!(command.invocation, UserCommandInvocation::Separator))
        .map(|command| &command.id)
        .collect();
    let removed: HashSet<_> = old
        .iter()
        .filter(|command| !kept.contains(&command.id))
        .map(|command| command.id.action())
        .collect();
    overrides.retain(|binding| !removed.contains(&binding.action));
}

impl crate::UserPreferences {
    /// Accept one complete command-list edit atomically, including its deleted or newly inert ID bindings.
    /// Callers persist this accepted preference value through the existing settings service.
    pub fn replace_user_commands(
        &mut self,
        commands: std::sync::Arc<Vec<UserCommand>>,
    ) -> Result<(), crate::FsError> {
        validate_user_commands(&commands).map_err(|message| {
            crate::FsError::from_io(
                "Custom Commands settings",
                Path::new(""),
                std::io::Error::new(std::io::ErrorKind::InvalidInput, message),
            )
        })?;
        let mut accepted = self.clone();
        remove_deleted_user_command_shortcuts(
            &self.user_commands,
            &commands,
            &mut accepted.shortcuts,
        );
        accepted.user_commands = commands;
        accepted.validate()?;
        *self = accepted;
        Ok(())
    }
}
#[cfg(test)]
mod separator_tests {
    use super::*;

    fn separator() -> UserCommand {
        UserCommand {
            id: UserCommandId::new("group").unwrap(),
            name: "Group".into(),
            enabled: true,
            toolbar_label: None,
            icon: None,
            working_directory: String::new(),
            launch: UserCommandLaunch::Single,
            invocation: UserCommandInvocation::Separator,
        }
    }
    #[test]
    fn separator_skips_process_fields_but_retains_names_ids_and_configuration_budgets() {
        let mut item = separator();
        assert!(item.validate().is_ok());
        item.working_directory = "${unknown}".into();
        assert!(
            item.validate().is_ok(),
            "inert saved mode fields are not executable templates"
        );
        item.name.clear();
        assert!(item.validate().is_err());
        item.name = "Group".into();
        item.working_directory = "x".repeat(MAX_USER_COMMAND_CONFIG_BYTES);
        assert!(item.validate().is_err());
        assert!(validate_user_commands(&[separator(), separator()]).is_err());
        assert!(validate_user_commands(&vec![separator(); MAX_USER_COMMANDS + 1]).is_err());
    }
    #[test]
    fn replacing_a_command_with_separator_removes_only_its_shortcut() {
        let mut previous = separator();
        previous.working_directory = "${active.path}".into();
        previous.invocation = UserCommandInvocation::Program {
            executable: std::env::current_exe().unwrap(),
            arguments: Vec::new(),
        };
        let mut unrelated = previous.clone();
        unrelated.id = UserCommandId::new("other").unwrap();
        let old = vec![previous, unrelated.clone()];
        let mut shortcuts = vec![
            ShortcutBinding {
                action: old[0].id.action(),
                keys: vec!["Ctrl+Alt+Shift+F11".into()],
            },
            ShortcutBinding {
                action: unrelated.id.action(),
                keys: vec!["Ctrl+Alt+Shift+F12".into()],
            },
        ];
        let retained = shortcuts[1].clone();
        remove_deleted_user_command_shortcuts(&old, &[separator(), unrelated], &mut shortcuts);
        assert_eq!(shortcuts, vec![retained]);
    }
}
