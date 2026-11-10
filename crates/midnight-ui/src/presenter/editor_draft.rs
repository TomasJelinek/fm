//! Retained editor draft; path browsing and persistence remain backend requests.
use super::*;

#[derive(Clone, Debug)]
pub(crate) struct Draft {
    pub executable: String,
    pub arguments: Vec<String>,
    pub use_default: bool,
    pub native: Option<PathBuf>,
    pub original: EditorPreferences,
}
impl Draft {
    fn from_preferences(editor: &EditorPreferences) -> Self {
        Self {
            executable: editor
                .executable
                .as_ref()
                .map_or_else(String::new, |p| p.to_string_lossy().into_owned()),
            arguments: editor.arguments.clone(),
            use_default: editor.executable.is_none(),
            native: editor.executable.clone(),
            original: editor.clone(),
        }
    }
    fn candidate(&self) -> Result<EditorPreferences, String> {
        if self.use_default {
            return Ok(EditorPreferences::default());
        }
        let executable = match &self.native {
            Some(path) if path.to_string_lossy() == self.executable => path.clone(),
            previous => {
                if previous
                    .as_deref()
                    .and_then(Path::parent)
                    .is_some_and(|parent| {
                        crate::native_path_edit::ambiguous_edit(parent, &self.executable)
                    })
                {
                    return Err("This folder cannot be represented by its displayed text. Browse for the executable, or enter another complete path.".into());
                }
                PathBuf::from(&self.executable)
            }
        };
        let editor = EditorPreferences {
            executable: Some(executable),
            arguments: self.arguments.clone(),
        };
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
    pub(crate) fn edit_editor_draft(&mut self, executable: String, use_default: bool) {
        let mut draft = self.editor_draft();
        if draft.executable == executable && draft.use_default == use_default {
            return;
        }
        self.settings_picker.invalidate();
        draft.executable = executable;
        draft.use_default = use_default;
        self.editor.draft = Some(draft);
        self.editor.dirty = true;
        self.editor.error.clear();
        self.editor.revision = self.editor.revision.saturating_add(1);
    }
    pub(crate) fn edit_editor_argument(&mut self, operation: i32, row: i32, text: &str) {
        let mut draft = self.editor_draft();
        if !crate::argument_rows::change(&mut draft.arguments, operation, row, text) {
            return;
        }
        self.settings_picker.invalidate();
        self.editor.draft = Some(draft);
        self.editor.dirty = true;
        self.editor.error.clear();
        self.editor.revision = self.editor.revision.saturating_add(1);
    }
    pub(crate) fn save_editor_draft(&mut self, send: &dyn Fn(Command) -> bool) {
        let draft = self.editor_draft();
        if !self.preferences.loaded || self.operations.closing {
            return;
        }
        if draft.original != self.preferences.values.editor {
            self.editor.error =
                "Editor settings changed while this draft was open. Discard changes to reload current settings."
                    .into();
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
}
