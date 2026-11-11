//! Retained editor drafts. Matching, discovery, native pickers and writes stay in backend services.
use super::*;

#[derive(Clone, Debug)]
pub(crate) struct RuleDraft {
    pub enabled: bool,
    pub mode: FilterMode,
    pub expression: String,
    pub case_sensitive: bool,
    pub executable: String,
    pub arguments: Vec<String>,
    pub use_default: bool,
    pub native: Option<PathBuf>,
    pub working_directory: String,
    pub native_working_directory: Option<PathBuf>,
}

impl RuleDraft {
    fn from_rule(rule: &EditorRule) -> Self {
        Self {
            enabled: rule.enabled,
            mode: rule.mode,
            expression: rule.expression.clone(),
            case_sensitive: rule.case_sensitive,
            executable: display(rule.executable.as_deref()),
            arguments: rule.arguments.clone(),
            use_default: rule.executable.is_none(),
            native: rule.executable.clone(),
            working_directory: display(rule.working_directory.as_deref()),
            native_working_directory: rule.working_directory.clone(),
        }
    }
    fn candidate(&self) -> Result<EditorRule, String> {
        let rule = EditorRule {
            enabled: self.enabled,
            mode: self.mode,
            expression: self.expression.clone(),
            case_sensitive: self.case_sensitive,
            executable: if self.use_default {
                None
            } else {
                Some(native_path(
                    &self.executable,
                    self.native.as_deref(),
                    false,
                )?)
            },
            arguments: if self.use_default {
                vec!["{file}".into()]
            } else {
                self.arguments.clone()
            },
            working_directory: if self.working_directory.is_empty() {
                None
            } else {
                Some(native_path(
                    &self.working_directory,
                    self.native_working_directory.as_deref(),
                    true,
                )?)
            },
        };
        rule.validate().map_err(str::to_owned)?;
        Ok(rule)
    }
}

fn display(path: Option<&Path>) -> String {
    path.map_or_else(String::new, |path| path.to_string_lossy().into_owned())
}
fn native_path(text: &str, previous: Option<&Path>, directory: bool) -> Result<PathBuf, String> {
    if let Some(path) = previous {
        if path.to_string_lossy() == text {
            return Ok(path.to_owned());
        }
        if (if directory { Some(path) } else { path.parent() })
            .is_some_and(|parent| crate::native_path_edit::ambiguous_edit(parent, text))
        {
            return Err("This folder cannot be represented by its displayed text. Browse for a path, or enter another complete path.".into());
        }
    }
    Ok(PathBuf::from(text))
}

#[derive(Clone, Debug)]
pub(crate) struct Draft {
    // The original editor remains the fallback, even after associations are added.
    pub executable: String,
    pub arguments: Vec<String>,
    pub use_default: bool,
    pub native: Option<PathBuf>,
    pub original: EditorPreferences,
    pub rules: Vec<RuleDraft>,
    pub selected: Option<usize>,
}
impl Draft {
    fn from_preferences(editor: &EditorPreferences) -> Self {
        Self {
            executable: display(editor.executable.as_deref()),
            arguments: editor.arguments.clone(),
            use_default: editor.executable.is_none(),
            native: editor.executable.clone(),
            original: editor.clone(),
            rules: editor.rules.iter().map(RuleDraft::from_rule).collect(),
            selected: None,
        }
    }
    pub fn selected_form(&self) -> RuleDraft {
        self.selected
            .and_then(|index| self.rules.get(index))
            .cloned()
            .unwrap_or_else(|| RuleDraft {
                executable: self.executable.clone(),
                arguments: self.arguments.clone(),
                use_default: self.use_default,
                native: self.native.clone(),
                ..RuleDraft::from_rule(&EditorRule::default())
            })
    }
    pub fn picked(&mut self, path: PathBuf, working_directory: bool) {
        if let Some(rule) = self.selected.and_then(|index| self.rules.get_mut(index)) {
            if working_directory {
                rule.working_directory = display(Some(&path));
                rule.native_working_directory = Some(path);
            } else {
                rule.executable = display(Some(&path));
                rule.native = Some(path);
                rule.use_default = false;
            }
        } else if !working_directory {
            self.executable = display(Some(&path));
            self.native = Some(path);
            self.use_default = false;
        }
    }
    fn candidate(&self) -> Result<EditorPreferences, String> {
        let mut editor = if self.use_default {
            EditorPreferences::default()
        } else {
            EditorPreferences {
                executable: Some(native_path(
                    &self.executable,
                    self.native.as_deref(),
                    false,
                )?),
                arguments: self.arguments.clone(),
                rules: Vec::new(),
            }
        };
        editor.rules = self
            .rules
            .iter()
            .enumerate()
            .map(|(index, rule)| {
                rule.candidate()
                    .map_err(|error| format!("Editor rule {}: {error}", index + 1))
            })
            .collect::<Result<_, _>>()?;
        editor.validate().map_err(str::to_owned)?;
        Ok(editor)
    }
}

impl Presenter {
    pub(crate) fn editor_draft_loaded(&self) -> bool {
        self.preferences.loaded
    }
    pub(crate) fn discard_editor_draft(&mut self) {
        self.settings_picker.invalidate();
        if let Some(draft) = self.editor.draft.take() {
            self.retired.push(Box::new(draft));
        }
        self.editor.dirty = false;
        self.editor.error.clear();
        self.editor.revision = self.editor.revision.saturating_add(1);
    }
    pub(crate) fn editor_draft(&self) -> Draft {
        self.editor
            .draft
            .clone()
            .unwrap_or_else(|| Draft::from_preferences(&self.preferences.values.editor))
    }
    fn remember_editor(&mut self, draft: Draft) {
        self.settings_picker.invalidate();
        self.editor.draft = Some(draft);
        self.editor.dirty = true;
        self.editor.error.clear();
        self.editor.revision = self.editor.revision.saturating_add(1);
    }
    pub(crate) fn select_editor_rule(&mut self, row: i32) {
        let mut draft = self.editor_draft();
        let Ok(row) = usize::try_from(row) else {
            return;
        };
        if row > draft.rules.len() {
            return;
        }
        self.settings_picker.invalidate();
        draft.selected = (row < draft.rules.len()).then_some(row);
        self.editor.draft = Some(draft);
        self.editor.revision = self.editor.revision.saturating_add(1);
    }
    pub(crate) fn editor_rule_action(&mut self, action: i32) {
        let mut draft = self.editor_draft();
        match action {
            0 if draft.rules.len() < MAX_EDITOR_RULES => {
                // An immediately useful system-editor rule; no empty program or argv is introduced.
                draft
                    .rules
                    .push(RuleDraft::from_rule(&EditorRule::default()));
                draft.selected = Some(draft.rules.len() - 1);
            }
            1 => {
                let Some(index) = draft.selected else {
                    return;
                };
                draft.rules.remove(index);
                draft.selected = if draft.rules.is_empty() {
                    None
                } else {
                    Some(index.min(draft.rules.len() - 1))
                };
            }
            2 | 3 => {
                let Some(index) = draft.selected else {
                    return;
                };
                let next = if action == 2 {
                    index.checked_sub(1)
                } else {
                    index.checked_add(1)
                };
                let Some(next) = next.filter(|next| *next < draft.rules.len()) else {
                    return;
                };
                draft.rules.swap(index, next);
                draft.selected = Some(next);
            }
            _ => return,
        }
        self.remember_editor(draft);
    }
    pub(crate) fn edit_editor_form(&mut self, form: crate::EditorOptionsDraft) {
        let mut draft = self.editor_draft();
        if let Some(rule) = draft.selected.and_then(|index| draft.rules.get_mut(index)) {
            rule.enabled = form.enabled;
            rule.mode = if form.mode == 1 {
                FilterMode::Regex
            } else {
                FilterMode::Masks
            };
            rule.expression = form.expression.to_string();
            rule.case_sensitive = form.case_sensitive;
            rule.executable = form.executable.to_string();
            rule.use_default = form.use_default;
            rule.working_directory = form.working_directory.to_string();
        } else {
            draft.executable = form.executable.to_string();
            draft.use_default = form.use_default;
        }
        self.remember_editor(draft);
    }
    #[cfg(test)]
    pub(crate) fn edit_editor_draft(&mut self, executable: String, use_default: bool) {
        let mut draft = self.editor_draft();
        if draft.executable == executable && draft.use_default == use_default {
            return;
        }
        draft.executable = executable;
        draft.use_default = use_default;
        self.remember_editor(draft);
    }
    pub(crate) fn edit_editor_argument(&mut self, operation: i32, row: i32, text: &str) {
        let mut draft = self.editor_draft();
        let arguments = match draft.selected {
            Some(index) => &mut draft.rules[index].arguments,
            None => &mut draft.arguments,
        };
        if crate::argument_rows::change(arguments, operation, row, text) {
            self.remember_editor(draft);
        }
    }
    pub(crate) fn save_editor_draft(&mut self, send: &dyn Fn(Command) -> bool) {
        let draft = self.editor_draft();
        if !self.preferences.loaded || self.operations.closing {
            return;
        }
        if draft.original != self.preferences.values.editor {
            self.editor.error = "Editor settings changed while this draft was open. Discard changes to reload current settings.".into();
            return;
        }
        let editor = match draft.candidate() {
            Ok(value) => value,
            Err(error) => {
                self.editor.error = error;
                return;
            }
        };
        self.preferences.edited.editor = true;
        let mut values = self.preferences.values.clone();
        values.editor = editor.clone();
        self.update_preferences(values, send);
        if self.preferences.values.editor == editor {
            self.editor.draft = None;
            self.editor.dirty = false;
            self.editor.error.clear();
            self.editor.revision = self.editor.revision.saturating_add(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn literal_rows_and_default_toggle_survive_as_one_retained_draft() {
        let root = std::env::current_dir().unwrap();
        let mut p = Presenter::new(root.clone(), root);
        p.apply(Event::PreferencesLoaded(Ok(Box::default())));
        let exe = std::env::current_exe()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        p.edit_editor_draft(exe, false);
        p.edit_editor_argument(1, 0, "two words");
        p.edit_editor_argument(1, 0, "");
        let arguments = p.editor_draft().arguments;
        let text = p.editor_draft().executable;
        p.edit_editor_draft(text.clone(), true);
        p.edit_editor_draft(text, false);
        assert_eq!(p.editor_draft().arguments, arguments);
        p.save_editor_draft(&|_| true);
        assert!(!p.editor.dirty);
        assert_eq!(
            p.preferences.values.editor.arguments,
            ["{file}", "two words", ""]
        );
    }

    #[test]
    fn association_drafts_reorder_toggle_and_discard_without_replacing_fallback() {
        let root = std::env::current_dir().unwrap();
        let mut p = Presenter::new(root.clone(), root);
        let original = EditorPreferences {
            executable: Some(std::env::current_exe().unwrap()),
            ..Default::default()
        };
        p.apply(Event::PreferencesLoaded(Ok(Box::new(UserPreferences {
            editor: original.clone(),
            ..Default::default()
        }))));
        p.editor_rule_action(0);
        p.editor_rule_action(0);
        let mut draft = p.editor_draft();
        draft.rules[0].expression = "*.txt".into();
        draft.rules[1].expression = "*.rs".into();
        draft.rules[1].enabled = false;
        p.remember_editor(draft);
        p.editor_rule_action(2);
        p.select_editor_rule(2);
        assert!(p.editor_draft().selected.is_none());
        assert_eq!(
            p.editor_draft().executable,
            display(original.executable.as_deref())
        );
        p.save_editor_draft(&|_| true);
        assert_eq!(p.preferences.values.editor.executable, original.executable);
        assert_eq!(p.preferences.values.editor.rules[0].expression, "*.rs");
        assert!(!p.preferences.values.editor.rules[0].enabled);
        p.editor_rule_action(0);
        p.discard_editor_draft();
        assert_eq!(p.editor_draft().rules.len(), 2);
    }
}
