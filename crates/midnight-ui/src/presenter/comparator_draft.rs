//! Retained comparator form state. Validation and matching remain in Core/backend.
use midnight_core::{
    FileComparatorDiagnostic, FileComparatorField, FileComparatorPreferences,
    FileComparatorProgram, FileComparatorRule, FileComparatorTarget, FilterMode,
    MAX_FILE_COMPARATOR_RULES,
};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RowId(u64);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Target {
    #[default]
    Automatic,
    Text,
    Images,
    External,
}
impl Target {
    fn from_index(index: i32) -> Option<Self> {
        match index {
            0 => Some(Self::Automatic),
            1 => Some(Self::Text),
            2 => Some(Self::Images),
            3 => Some(Self::External),
            _ => None,
        }
    }
    pub(crate) fn index(self) -> i32 {
        match self {
            Self::Automatic => 0,
            Self::Text => 1,
            Self::Images => 2,
            Self::External => 3,
        }
    }
}

#[derive(Debug)]
pub(crate) struct ExternalDraft {
    pub(crate) executable: String,
    pub(crate) native_executable: Option<PathBuf>,
    pub(crate) arguments: Vec<String>,
    pub(crate) working_directory: String,
    pub(crate) native_working_directory: Option<PathBuf>,
}
impl Default for ExternalDraft {
    fn default() -> Self {
        Self {
            executable: String::new(),
            native_executable: None,
            arguments: vec!["{left}".into(), "{right}".into()],
            working_directory: String::new(),
            native_working_directory: None,
        }
    }
}

#[derive(Debug)]
pub(crate) struct RuleDraft {
    pub(crate) id: RowId,
    pub(crate) enabled: bool,
    pub(crate) mode: FilterMode,
    pub(crate) expression: String,
    pub(crate) case_sensitive: bool,
    pub(crate) target: Target,
    // Hidden external fields survive target/row/topic changes within Options.
    pub(crate) external: ExternalDraft,
}
impl RuleDraft {
    fn new(id: RowId) -> Self {
        Self {
            id,
            enabled: true,
            mode: FilterMode::Masks,
            expression: "*".into(),
            case_sensitive: false,
            target: Target::Automatic,
            external: ExternalDraft::default(),
        }
    }
    fn from_rule(id: RowId, rule: &FileComparatorRule) -> Self {
        let mut draft = Self {
            enabled: rule.enabled,
            mode: rule.mode,
            expression: rule.expression.clone(),
            case_sensitive: rule.case_sensitive,
            ..Self::new(id)
        };
        draft.target = match &rule.target {
            FileComparatorTarget::Automatic => Target::Automatic,
            FileComparatorTarget::BuiltInText => Target::Text,
            FileComparatorTarget::BuiltInImages => Target::Images,
            FileComparatorTarget::External(program) => {
                draft.external = ExternalDraft {
                    executable: program.executable.to_string_lossy().into_owned(),
                    native_executable: Some(program.executable.clone()),
                    arguments: program.arguments.clone(),
                    working_directory: program
                        .working_directory
                        .as_deref()
                        .map_or_else(String::new, |path| path.to_string_lossy().into_owned()),
                    native_working_directory: program.working_directory.clone(),
                };
                Target::External
            }
        };
        draft
    }
    pub(crate) fn form(&self) -> crate::ComparatorOptionsDraft {
        crate::ComparatorOptionsDraft {
            enabled: self.enabled,
            mode: i32::from(self.mode == FilterMode::Regex),
            expression: self.expression.as_str().into(),
            case_sensitive: self.case_sensitive,
            target: self.target.index(),
            executable: self.external.executable.as_str().into(),
            working_directory: self.external.working_directory.as_str().into(),
        }
    }
    fn edit(&mut self, form: crate::ComparatorOptionsDraft) -> bool {
        let Some(target) = Target::from_index(form.target) else {
            return false;
        };
        let mode = match form.mode {
            0 => FilterMode::Masks,
            1 => FilterMode::Regex,
            _ => return false,
        };
        let mut changed = self.enabled != form.enabled
            || self.mode != mode
            || self.case_sensitive != form.case_sensitive
            || self.target != target;
        self.enabled = form.enabled;
        self.mode = mode;
        self.case_sensitive = form.case_sensitive;
        self.target = target;
        changed |= text(&mut self.expression, form.expression.as_str());
        changed |= text(&mut self.external.executable, form.executable.as_str());
        changed |= text(
            &mut self.external.working_directory,
            form.working_directory.as_str(),
        );
        changed
    }
    fn candidate(&self) -> Result<FileComparatorRule, (FileComparatorField, &'static str)> {
        let target = match self.target {
            Target::Automatic => FileComparatorTarget::Automatic,
            Target::Text => FileComparatorTarget::BuiltInText,
            Target::Images => FileComparatorTarget::BuiltInImages,
            Target::External => FileComparatorTarget::External(FileComparatorProgram {
                executable: native_path(
                    &self.external.executable,
                    self.external.native_executable.as_deref(),
                    false,
                )
                .map_err(|message| (FileComparatorField::Executable, message))?,
                arguments: self.external.arguments.clone(),
                working_directory: if self.external.working_directory.is_empty() {
                    None
                } else {
                    Some(
                        native_path(
                            &self.external.working_directory,
                            self.external.native_working_directory.as_deref(),
                            true,
                        )
                        .map_err(|message| (FileComparatorField::WorkingDirectory, message))?,
                    )
                },
            }),
        };
        Ok(FileComparatorRule {
            enabled: self.enabled,
            mode: self.mode,
            expression: self.expression.clone(),
            case_sensitive: self.case_sensitive,
            target,
        })
    }
    pub(crate) fn label(&self, index: usize) -> String {
        let target = match self.target {
            Target::Automatic => "Automatic".into(),
            Target::Text => "Built-in text".into(),
            Target::Images => "Built-in image".into(),
            Target::External if self.external.executable.is_empty() => {
                "External program (choose executable)".into()
            }
            Target::External => format!("External: {}", self.external.executable),
        };
        format!(
            "{}. {}{} → {}",
            index + 1,
            if self.enabled { "" } else { "[Off] " },
            self.expression,
            target
        )
    }
}

#[derive(Debug)]
pub(crate) struct Draft {
    pub(crate) rules: Vec<RuleDraft>,
    pub(crate) selected: Option<RowId>,
    next_id: u64,
}
impl Draft {
    pub(crate) fn from_preferences(preferences: &FileComparatorPreferences) -> Self {
        Self {
            rules: preferences
                .rules
                .iter()
                .enumerate()
                .map(|(index, rule)| RuleDraft::from_rule(RowId(index as u64 + 1), rule))
                .collect(),
            selected: None,
            next_id: preferences.rules.len() as u64 + 1,
        }
    }
    pub(crate) fn selected_index(&self) -> Option<usize> {
        self.selected
            .and_then(|id| self.rules.iter().position(|rule| rule.id == id))
    }
    pub(crate) fn selected_rule(&self) -> Option<&RuleDraft> {
        self.selected_index()
            .and_then(|index| self.rules.get(index))
    }
    pub(crate) fn selected_rule_mut(&mut self) -> Option<&mut RuleDraft> {
        self.selected_index()
            .and_then(|index| self.rules.get_mut(index))
    }
    pub(crate) fn select(&mut self, row: usize) -> bool {
        if row > self.rules.len() {
            return false;
        }
        let selected = self.rules.get(row).map(|rule| rule.id);
        if self.selected == selected {
            return false;
        }
        self.selected = selected;
        true
    }
    pub(crate) fn action(&mut self, action: i32) -> bool {
        match action {
            0 if self.rules.len() < MAX_FILE_COMPARATOR_RULES => {
                let Some(next_id) = self.next_id.checked_add(1) else {
                    return false;
                };
                let rule = RuleDraft::new(RowId(self.next_id));
                self.next_id = next_id;
                self.selected = Some(rule.id);
                self.rules.push(rule);
            }
            1 => {
                let Some(index) = self.selected_index() else {
                    return false;
                };
                self.rules.remove(index);
                self.selected = self
                    .rules
                    .get(index.min(self.rules.len().saturating_sub(1)))
                    .map(|rule| rule.id);
            }
            2 | 3 => {
                let Some(index) = self.selected_index() else {
                    return false;
                };
                let next = if action == 2 {
                    index.checked_sub(1)
                } else {
                    index.checked_add(1)
                };
                let Some(next) = next.filter(|next| *next < self.rules.len()) else {
                    return false;
                };
                self.rules.swap(index, next);
            }
            _ => return false,
        }
        true
    }
    pub(crate) fn edit_selected(&mut self, form: crate::ComparatorOptionsDraft) -> bool {
        self.selected_rule_mut().is_some_and(|rule| rule.edit(form))
    }
    pub(crate) fn change_arguments(&mut self, operation: i32, row: i32, value: &str) -> bool {
        self.selected_rule_mut().is_some_and(|rule| {
            rule.target == Target::External
                && crate::argument_rows::change(&mut rule.external.arguments, operation, row, value)
        })
    }
    pub(crate) fn picked(&mut self, path: PathBuf, directory: bool) -> bool {
        let Some(rule) = self
            .selected_rule_mut()
            .filter(|rule| rule.target == Target::External)
        else {
            return false;
        };
        let (display, native) = if directory {
            (
                &mut rule.external.working_directory,
                &mut rule.external.native_working_directory,
            )
        } else {
            (
                &mut rule.external.executable,
                &mut rule.external.native_executable,
            )
        };
        let picked_display = path.to_string_lossy();
        if native
            .as_ref()
            .is_some_and(|previous| previous.as_os_str() == path.as_os_str())
            && display.as_str() == picked_display.as_ref()
        {
            return false;
        }
        *display = picked_display.into_owned();
        *native = Some(path);
        true
    }
    pub(crate) fn candidate(&self) -> Result<FileComparatorPreferences, FileComparatorDiagnostic> {
        let rules = self
            .rules
            .iter()
            .enumerate()
            .map(|(index, rule)| {
                rule.candidate()
                    .map_err(|(field, message)| FileComparatorDiagnostic {
                        rule: Some(index),
                        field,
                        message: message.into(),
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let preferences = FileComparatorPreferences { rules };
        preferences.validate()?;
        Ok(preferences)
    }
    pub(crate) fn row_order(&self) -> Vec<RowId> {
        self.rules.iter().map(|rule| rule.id).collect()
    }
}

fn text(target: &mut String, value: &str) -> bool {
    if target == value {
        return false;
    }
    target.clear();
    target.push_str(value);
    true
}

fn native_path(
    text: &str,
    previous: Option<&Path>,
    directory: bool,
) -> Result<PathBuf, &'static str> {
    if let Some(path) = previous {
        if path.to_string_lossy() == text {
            return Ok(path.to_owned());
        }
        if (if directory { Some(path) } else { path.parent() })
            .is_some_and(|parent| crate::native_path_edit::ambiguous_edit(parent, text))
        {
            return Err(
                "This native path cannot be represented by its displayed text. Browse for a path, or enter another complete path.",
            );
        }
    }
    Ok(PathBuf::from(text))
}

#[cfg(test)]
#[path = "comparator_draft_tests.rs"]
mod tests;
