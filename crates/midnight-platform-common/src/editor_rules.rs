//! Worker-only ordered editor matching. Display labels never become native paths.
use crate::filter::CompiledFilter;
use midnight_core::{EditorCommand, EditorPreferences, EditorRule};
use std::path::{Path, PathBuf};

pub struct SelectedEditor {
    /// None asks the native service for its safe platform default.
    pub command: Option<EditorCommand>,
    pub working_directory: Option<PathBuf>,
}

/// Compile every pattern before accepting preferences, including disabled rules.
pub fn validate(preferences: &EditorPreferences) -> Result<(), String> {
    preferences.validate().map_err(str::to_owned)?;
    for (index, rule) in preferences.rules.iter().enumerate() {
        compile(rule).map_err(|error| format!("Editor rule {}: {error}", index + 1))?;
    }
    Ok(())
}

/// First enabled filename match wins. Non-Unicode names use the existing fallback.
/// Matching never changes the exact native file passed to the editor.
pub fn select(preferences: &EditorPreferences, file: &Path) -> Result<SelectedEditor, String> {
    preferences.validate().map_err(str::to_owned)?;
    let name = file.file_name().filter(|name| name.to_str().is_some());
    for (index, rule) in preferences.rules.iter().enumerate() {
        let pattern =
            compile(rule).map_err(|error| format!("Editor rule {}: {error}", index + 1))?;
        if rule.enabled && name.is_some_and(|name| pattern.matches(name)) {
            return Ok(SelectedEditor {
                command: rule.executable.as_ref().map(|executable| EditorCommand {
                    executable: executable.clone(),
                    arguments: rule.arguments.clone(),
                }),
                working_directory: rule.working_directory.clone(),
            });
        }
    }
    Ok(SelectedEditor {
        command: preferences
            .executable
            .as_ref()
            .map(|executable| EditorCommand {
                executable: executable.clone(),
                arguments: preferences.arguments.clone(),
            }),
        working_directory: None,
    })
}

fn compile(rule: &EditorRule) -> Result<CompiledFilter, String> {
    CompiledFilter::new(&rule.filter())
}

#[cfg(test)]
mod tests {
    use super::*;
    use midnight_core::FilterMode;

    fn rule(expression: &str, executable: &str) -> EditorRule {
        EditorRule {
            expression: expression.into(),
            executable: Some(std::env::current_dir().unwrap().join(executable)),
            ..Default::default()
        }
    }

    #[test]
    fn priority_disabled_masks_regex_and_fallback_are_observable() {
        let mut preferences = EditorPreferences {
            executable: Some(std::env::current_exe().unwrap()),
            rules: vec![
                rule("*.rs;*.txt|secret*", "first.exe"),
                rule(r"\.rs$", "second.exe"),
            ],
            ..Default::default()
        };
        preferences.rules[1].mode = FilterMode::Regex;
        let target = Path::new("Žluťoučký 文档.RS");
        assert!(
            select(&preferences, target)
                .unwrap()
                .command
                .unwrap()
                .executable
                .ends_with("first.exe")
        );
        preferences.rules.swap(0, 1);
        assert!(
            select(&preferences, target)
                .unwrap()
                .command
                .unwrap()
                .executable
                .ends_with("second.exe")
        );
        preferences.rules[0].enabled = false;
        assert!(
            select(&preferences, target)
                .unwrap()
                .command
                .unwrap()
                .executable
                .ends_with("first.exe")
        );
        assert_eq!(
            select(&preferences, Path::new("secret.txt"))
                .unwrap()
                .command
                .unwrap()
                .executable,
            preferences.executable.clone().unwrap()
        );
        preferences.rules[1].case_sensitive = true;
        assert_eq!(
            select(&preferences, target)
                .unwrap()
                .command
                .unwrap()
                .executable,
            preferences.executable.unwrap()
        );
    }

    #[test]
    fn invalid_patterns_fail_with_the_rule_number_even_when_disabled() {
        let preferences = EditorPreferences {
            rules: vec![EditorRule {
                enabled: false,
                mode: FilterMode::Regex,
                expression: "[".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(
            validate(&preferences)
                .unwrap_err()
                .starts_with("Editor rule 1:")
        );
        assert!(select(&preferences, Path::new("file.txt")).is_err());
    }

    #[test]
    fn rule_preserves_literal_arguments_and_native_working_directory() {
        let directory = std::env::current_dir().unwrap().join("文档 & directory");
        let mut selected = rule("*", "editor.exe");
        selected.arguments = vec![
            "{file}".into(),
            "two words".into(),
            "\"quoted\";$HOME".into(),
            "".into(),
        ];
        selected.working_directory = Some(directory.clone());
        let preferences = EditorPreferences {
            rules: vec![selected.clone()],
            ..Default::default()
        };
        let result = select(&preferences, Path::new("文件.txt")).unwrap();
        assert_eq!(result.working_directory, Some(directory));
        assert_eq!(result.command.unwrap().arguments, selected.arguments);
    }
}
