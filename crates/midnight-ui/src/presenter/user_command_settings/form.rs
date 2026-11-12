//! Pure bounded input conversion. Native literals remain native until actually edited.
use midnight_core::*;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub(crate) struct Form {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub toolbar_label: String,
    pub icon: String,
    pub icon_edited: bool,
    pub working_directory: String,
    pub kind: i32,
    pub launch: i32,
    pub executable: String,
    pub executable_edited: bool,
    pub arguments: String,
    pub script: String,
    pub shell: i32,
    pub active_path: bool,
    pub other_path: bool,
    pub file_path: bool,
    pub file_name: bool,
    pub file_stem: bool,
    pub file_extension: bool,
}
impl Form {
    pub(crate) fn new(id: String) -> Self {
        Self {
            id,
            name: "New command".into(),
            enabled: true,
            toolbar_label: String::new(),
            icon: String::new(),
            icon_edited: false,
            working_directory: "${active.path}".into(),
            kind: 0,
            launch: 0,
            executable: String::new(),
            executable_edited: false,
            arguments: "[]".into(),
            script: String::new(),
            shell: 0,
            active_path: false,
            other_path: false,
            file_path: false,
            file_name: false,
            file_stem: false,
            file_extension: false,
        }
    }
    pub(crate) fn from_command(command: &UserCommand) -> Self {
        let mut form = Self::new(command.id.as_str().into());
        form.name = command.name.clone();
        form.enabled = command.enabled;
        form.toolbar_label = command.toolbar_label.clone().unwrap_or_default();
        form.icon = command
            .icon
            .as_ref()
            .map_or_else(String::new, |p| p.to_string_lossy().into_owned());
        form.working_directory = command.working_directory.clone();
        form.launch = i32::from(command.launch == UserCommandLaunch::PerItem);
        match &command.invocation {
            UserCommandInvocation::Program {
                executable,
                arguments,
            } => {
                form.executable = executable.to_string_lossy().into_owned();
                form.arguments =
                    serde_json::to_string_pretty(arguments).unwrap_or_else(|_| "[]".into());
            }
            UserCommandInvocation::Terminal {
                shell,
                script,
                variables,
            } => {
                form.kind = 1;
                form.script = script.clone();
                form.shell = match shell {
                    ShellFlavor::PowerShell => 0,
                    ShellFlavor::Posix => 1,
                    ShellFlavor::Cmd => 2,
                };
                for variable in variables {
                    form.declare(*variable);
                }
            }
        }
        form
    }
    #[cfg(test)]
    pub(crate) fn command(&self, original: Option<&UserCommand>) -> Result<UserCommand, String> {
        self.command_with_native(original, None, None)
    }
    pub(crate) fn command_with_native(
        &self,
        original: Option<&UserCommand>,
        executable: Option<&Path>,
        icon: Option<&Path>,
    ) -> Result<UserCommand, String> {
        let id = UserCommandId::new(self.id.clone()).map_err(str::to_owned)?;
        if original.is_some_and(|original| original.id != id) {
            return Err("This command draft is obsolete. Select the command again.".into());
        }
        let bytes = [
            &self.name,
            &self.toolbar_label,
            &self.icon,
            &self.working_directory,
        ]
        .into_iter()
        .fold(0usize, |total, text| total.saturating_add(text.len()));
        let active = if self.kind == 0 {
            self.executable.len().saturating_add(self.arguments.len())
        } else {
            self.script.len()
        };
        // Raw native display replacement and JSON escaping can expand stored data.
        if bytes.saturating_add(active) > MAX_USER_COMMAND_CONFIG_BYTES * 8 {
            return Err("Command form exceeds its 8 MiB input resource budget.".into());
        }
        let launch = match self.launch {
            0 => UserCommandLaunch::Single,
            1 => UserCommandLaunch::PerItem,
            _ => return Err("Choose Single or Per item execution.".into()),
        };
        let invocation = match self.kind {
            0 => {
                let old = original.and_then(|command| match &command.invocation {
                    UserCommandInvocation::Program { executable, .. } => Some(executable.as_path()),
                    _ => None,
                });
                UserCommandInvocation::Program {
                    executable: match executable.filter(|p| p.to_string_lossy() == self.executable)
                    {
                        Some(path) => path.to_path_buf(),
                        None => literal(&self.executable, self.executable_edited, old)?,
                    },
                    arguments: arguments(&self.arguments)?,
                }
            }
            1 => UserCommandInvocation::Terminal {
                shell: self.shell()?,
                script: self.script.clone(),
                variables: self.variables(),
            },
            _ => return Err("Choose Program or Terminal command type.".into()),
        };
        let command = UserCommand {
            id,
            name: self.name.clone(),
            enabled: self.enabled,
            toolbar_label: (!self.toolbar_label.is_empty()).then(|| self.toolbar_label.clone()),
            icon: if self.icon.is_empty() {
                None
            } else {
                Some(
                    if let Some(path) = icon.filter(|p| p.to_string_lossy() == self.icon) {
                        path.to_path_buf()
                    } else {
                        literal(
                            &self.icon,
                            self.icon_edited,
                            original.and_then(|c| c.icon.as_deref()),
                        )?
                    },
                )
            },
            working_directory: self.working_directory.clone(),
            launch,
            invocation,
        };
        command.validate().map_err(str::to_owned)?;
        Ok(command)
    }
    pub(crate) fn insert_variable(
        &mut self,
        field: i32,
        variable: UserCommandVariable,
    ) -> Result<(), String> {
        match field {
            0 => {
                if variable == UserCommandVariable::SelectedPaths {
                    return Err("Selected paths can only be a complete Program argument.".into());
                }
                self.working_directory
                    .push_str(&format!("${{{}}}", variable.name()));
            }
            1 => {
                let mut values = arguments(&self.arguments)?;
                if values.len() == MAX_USER_COMMAND_ARGUMENTS {
                    return Err("Program commands accept at most 64 stored arguments.".into());
                }
                values.push(format!("${{{}}}", variable.name()));
                self.arguments =
                    serde_json::to_string_pretty(&values).map_err(|error| error.to_string())?;
            }
            2 => {
                let key = variable
                    .environment_key()
                    .ok_or("Terminal accepts scalar context values only.")?;
                let reference = match self.shell()? {
                    ShellFlavor::PowerShell => format!("\"$env:{key}\""),
                    ShellFlavor::Posix => format!("\"${key}\""),
                    ShellFlavor::Cmd => format!("!{key}_ARG!"),
                };
                if !self.script.is_empty() && !self.script.ends_with(char::is_whitespace) {
                    self.script.push(' ');
                }
                self.script.push_str(&reference);
                self.declare(variable);
            }
            _ => return Err("Choose a variable insertion target.".into()),
        }
        Ok(())
    }
    fn shell(&self) -> Result<ShellFlavor, String> {
        match self.shell {
            0 => Ok(ShellFlavor::PowerShell),
            1 => Ok(ShellFlavor::Posix),
            2 => Ok(ShellFlavor::Cmd),
            _ => Err("Choose a terminal shell.".into()),
        }
    }
    fn variables(&self) -> Vec<UserCommandVariable> {
        [
            (self.active_path, UserCommandVariable::ActivePath),
            (self.other_path, UserCommandVariable::OtherPath),
            (self.file_path, UserCommandVariable::FilePath),
            (self.file_name, UserCommandVariable::FileName),
            (self.file_stem, UserCommandVariable::FileStem),
            (self.file_extension, UserCommandVariable::FileExtension),
        ]
        .into_iter()
        .filter_map(|(enabled, variable)| enabled.then_some(variable))
        .collect()
    }
    fn declare(&mut self, variable: UserCommandVariable) {
        match variable {
            UserCommandVariable::ActivePath => self.active_path = true,
            UserCommandVariable::OtherPath => self.other_path = true,
            UserCommandVariable::FilePath => self.file_path = true,
            UserCommandVariable::FileName => self.file_name = true,
            UserCommandVariable::FileStem => self.file_stem = true,
            UserCommandVariable::FileExtension => self.file_extension = true,
            UserCommandVariable::SelectedPaths => {}
        }
    }
}
fn literal(text: &str, edited: bool, original: Option<&Path>) -> Result<PathBuf, String> {
    if !edited
        && let Some(original) = original
        && original.to_string_lossy() == text
    {
        return Ok(original.to_path_buf());
    }
    if original
        .and_then(Path::parent)
        .is_some_and(|parent| crate::native_path_edit::ambiguous_edit(parent, text))
    {
        return Err(concat!(
            "The displayed executable or icon folder contains characters this editor cannot represent. ",
            "Enter another complete path without replacement characters."
        ).into());
    }
    Ok(PathBuf::from(text))
}
fn arguments(source: &str) -> Result<Vec<String>, String> {
    if source.len() > MAX_USER_COMMAND_CONFIG_BYTES * 8 {
        return Err("Program argument input is too large.".into());
    }
    // Reject oversized arrays before serde can allocate an unbounded Vec. Valid
    // arguments are strings, so only commas outside JSON strings are separators.
    let (mut quoted, mut escaped, mut separators) = (false, false, 0usize);
    for byte in source.bytes() {
        if quoted {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                quoted = false;
            }
        } else if byte == b'"' {
            quoted = true;
        } else if byte == b',' {
            separators += 1;
            if separators >= MAX_USER_COMMAND_ARGUMENTS {
                return Err("Program commands accept at most 64 stored arguments.".into());
            }
        }
    }
    let arguments: Vec<String> = serde_json::from_str(source)
        .map_err(|error| format!("Arguments require a JSON list of strings: {error}"))?;
    if arguments.len() > MAX_USER_COMMAND_ARGUMENTS {
        return Err("Program commands accept at most 64 stored arguments.".into());
    }
    Ok(arguments)
}

#[cfg(test)]
#[path = "form_tests.rs"]
mod tests;
