//! Explicit external-editor configuration. Neither executable nor argv uses shell syntax.
use crate::{FilterMode, FsError, PanelFilter, TerminalProcess};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

pub const MAX_EDITOR_RULES: usize = 64;

pub const MAX_EDITOR_ARGUMENTS: usize = 64;
pub const MAX_EDITOR_ARGUMENT_BYTES: usize = 32 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorPreferences {
    /// None asks the backend for its safe platform default.
    pub executable: Option<PathBuf>,
    pub arguments: Vec<String>,
    /// First enabled matching rule wins; these legacy fields remain the fallback.
    pub rules: Vec<EditorRule>,
}

impl Default for EditorPreferences {
    fn default() -> Self {
        Self {
            executable: None,
            arguments: vec!["{file}".into()],
            rules: Vec::new(),
        }
    }
}

impl EditorPreferences {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.rules.len() > MAX_EDITOR_RULES {
            return Err("At most 64 editor rules are supported.");
        }
        for rule in &self.rules {
            rule.validate()?;
        }
        if let Some(executable) = &self.executable {
            if !executable.is_absolute() || executable.as_os_str().as_encoded_bytes().contains(&0) {
                return Err("Editor executable must be an absolute native path without NUL.");
            }
        } else if self.arguments != ["{file}"] {
            return Err("Custom editor arguments require an explicit executable.");
        }
        if self.arguments.len() > MAX_EDITOR_ARGUMENTS
            || self.arguments.iter().map(String::len).sum::<usize>() > MAX_EDITOR_ARGUMENT_BYTES
        {
            return Err("Editor arguments exceed the 64 argument or 32 KiB limit.");
        }
        if self
            .arguments
            .iter()
            .any(|argument| argument.contains('\0'))
        {
            return Err("Editor arguments cannot contain NUL.");
        }
        if !self.arguments.iter().any(|argument| argument == "{file}") {
            return Err("Editor arguments must contain {file} as a complete argument.");
        }
        Ok(())
    }
}

/// Ordered filename association. Pattern compilation is restricted to backend workers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorRule {
    pub enabled: bool,
    pub mode: FilterMode,
    pub expression: String,
    pub case_sensitive: bool,
    pub executable: Option<PathBuf>,
    pub arguments: Vec<String>,
    /// None retains the safe executable-directory default.
    pub working_directory: Option<PathBuf>,
}

impl Default for EditorRule {
    fn default() -> Self {
        Self {
            enabled: true,
            mode: FilterMode::Masks,
            expression: "*".into(),
            case_sensitive: false,
            executable: None,
            arguments: vec!["{file}".into()],
            working_directory: None,
        }
    }
}

impl EditorRule {
    pub fn filter(&self) -> PanelFilter {
        PanelFilter {
            // Disabled rules still require syntactically valid saved patterns.
            enabled: true,
            mode: self.mode,
            expression: self.expression.clone(),
            case_sensitive: self.case_sensitive,
        }
    }

    /// Validate bounded data only; regex compilation belongs to the backend.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.mode == FilterMode::Expression {
            return Err("Editor rules support wildcards or regular expressions.");
        }
        if self.expression.trim().is_empty() {
            return Err("Enter a filename pattern for each editor rule.");
        }
        self.filter().validate_bounds()?;
        if let Some(directory) = &self.working_directory
            && (!directory.is_absolute() || directory.as_os_str().as_encoded_bytes().contains(&0))
        {
            return Err("Editor working directory must be an absolute native path without NUL.");
        }
        EditorPreferences {
            executable: self.executable.clone(),
            arguments: self.arguments.clone(),
            rules: Vec::new(),
        }
        .validate()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorCommand {
    pub executable: PathBuf,
    pub arguments: Vec<String>,
}

impl EditorCommand {
    /// Preserve the native file identity; substitute only a complete argv item.
    pub fn arguments_for(&self, file: &Path) -> Result<Vec<OsString>, &'static str> {
        EditorPreferences {
            executable: Some(self.executable.clone()),
            arguments: self.arguments.clone(),
            rules: Vec::new(),
        }
        .validate()?;
        if !file.is_absolute() || file.as_os_str().as_encoded_bytes().contains(&0) {
            return Err("Editing requires an absolute local file path without NUL.");
        }
        Ok(self
            .arguments
            .iter()
            .map(|argument| {
                if argument == "{file}" {
                    file.as_os_str().to_owned()
                } else {
                    OsString::from(argument)
                }
            })
            .collect())
    }
}

pub trait EditorService: Send + Sync + 'static {
    /// Discovery and executable validation are backend-only I/O.
    fn default_editor(&self) -> Result<EditorCommand, FsError>;
    /// Revalidate the regular local file on the worker; return a child for backend reaping.
    fn launch_editor(
        &self,
        file: &Path,
        editor: &EditorPreferences,
    ) -> Result<Box<dyn TerminalProcess>, FsError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_whole_file_argument_substitutes_without_shell_expansion() {
        let file = std::env::current_dir()
            .unwrap()
            .join("Žluťoučký 文档 & $x; ' \" e\u{301}.txt");
        let command = EditorCommand {
            executable: std::env::current_exe().unwrap(),
            arguments: vec![
                "--literal={file}".into(),
                "{file}".into(),
                "$HOME; & echo x".into(),
                "".into(),
            ],
        };
        assert_eq!(
            command.arguments_for(&file).unwrap(),
            vec![
                OsString::from("--literal={file}"),
                file.into_os_string(),
                OsString::from("$HOME; & echo x"),
                OsString::new()
            ]
        );
    }

    #[test]
    fn configuration_rejects_relative_executable_nul_and_absent_file_placeholder() {
        let exe = std::env::current_exe().unwrap();
        for value in [
            EditorPreferences {
                executable: Some("editor".into()),
                ..Default::default()
            },
            EditorPreferences {
                executable: Some(exe.clone()),
                arguments: vec!["prefix{file}".into()],
                rules: Vec::new(),
            },
            EditorPreferences {
                executable: Some(exe.clone()),
                arguments: vec!["{file}".into(), "x\0y".into()],
                rules: Vec::new(),
            },
            EditorPreferences {
                executable: Some(exe),
                arguments: vec!["{file}".into(); MAX_EDITOR_ARGUMENTS + 1],
                rules: Vec::new(),
            },
        ] {
            assert!(value.validate().is_err(), "accepted {value:?}");
        }
    }

    #[test]
    fn default_preferences_do_not_allow_unapplied_custom_arguments() {
        assert!(EditorPreferences::default().validate().is_ok());
        assert!(
            EditorPreferences {
                executable: None,
                arguments: vec!["--ignored".into(), "{file}".into()],
                rules: Vec::new(),
            }
            .validate()
            .is_err()
        );
    }
}
