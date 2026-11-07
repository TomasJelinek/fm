//! Editor choices and F4 capture are pure state; native discovery/launch run in the engine.
use super::*;

#[derive(Default)]
pub(crate) struct EditorState {
    pub(crate) default_label: String,
    pub(crate) error: String,
    pub(crate) revision: u64,
    pending: bool,
}

impl Presenter {
    pub(crate) fn request_default_editor(&mut self, send: &dyn Fn(Command) -> bool) {
        if !self.editor.pending && send(Command::ResolveDefaultEditor) {
            self.editor.pending = true;
            self.editor.default_label = "Finding the default editor…".into();
        }
    }

    pub(super) fn editor_default_resolved(&mut self, result: Result<EditorCommand, FsError>) {
        self.editor.pending = false;
        self.editor.default_label = match result {
            Ok(command) => command.executable.to_string_lossy().into_owned(),
            Err(error) => crate::error_presentation::describe(&error),
        };
    }

    pub(crate) fn change_editor(
        &mut self,
        executable: &str,
        arguments: &str,
        send: &dyn Fn(Command) -> bool,
    ) {
        if arguments.len() > 256 * 1024 {
            self.editor.error = "Editor argument text exceeds 256 KiB.".into();
            return;
        }
        let arguments = match serde_json::from_str::<Vec<String>>(arguments) {
            Ok(arguments) => arguments,
            Err(error) => {
                self.editor.error = format!("Enter arguments as a JSON list of strings: {error}");
                return;
            }
        };
        let previous = &self.preferences.values.editor.executable;
        let executable = if executable.is_empty() {
            None
        } else if previous
            .as_ref()
            .is_some_and(|path| path.to_string_lossy() == executable)
        {
            // Editing only arguments must not replace a non-Unicode native path with its label.
            previous.clone()
        } else {
            Some(PathBuf::from(executable))
        };
        let editor = EditorPreferences {
            executable,
            arguments,
        };
        if let Err(error) = editor.validate() {
            self.editor.error = error.into();
            return;
        }
        self.editor.error.clear();
        self.preferences.edited.editor = true;
        let mut preferences = self.preferences.values.clone();
        preferences.editor = editor;
        self.update_preferences(preferences, send);
    }

    pub(crate) fn restore_editor(&mut self, send: &dyn Fn(Command) -> bool) {
        self.editor.error.clear();
        self.editor.revision = self.editor.revision.wrapping_add(1);
        self.preferences.edited.editor = true;
        let mut preferences = self.preferences.values.clone();
        preferences.editor = EditorPreferences::default();
        self.update_preferences(preferences, send);
    }

    pub(crate) fn edit_file(&mut self, pane: PaneId, send: &dyn Fn(Command) -> bool) {
        if !self.native_action_allowed(pane) {
            return;
        }
        if !self.editor_preferences_ready() {
            self.desktop.report("Cannot open editor", "Saved editor settings are not available. Wait for settings to load, or resolve the settings error before editing.".into());
            return;
        }
        let Some(launch) = self.viewer_launch(pane, false) else {
            self.desktop.report(
                "Cannot open editor",
                "Focus a regular local file before pressing F4.".into(),
            );
            return;
        };
        let request = self.desktop.request();
        if send(Command::EditFile {
            request,
            path: launch.focused,
            editor: self.preferences.values.editor.clone(),
        }) {
            self.desktop.requests.insert(request);
            self.desktop.status = "Opening external editor…".into();
        } else {
            self.desktop.report(
                "Editor queue busy",
                "Try F4 again when the backend is ready.".into(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn presenter() -> Presenter {
        let root = std::env::current_dir().unwrap();
        let mut presenter = Presenter::new(root.clone(), root.clone());
        let entries: Arc<[Entry]> = vec![
            Entry::new(EntryId(1), root.join("selected.txt"), EntryKind::File),
            Entry::new(
                EntryId(2),
                root.join("focused 文档 & $.txt"),
                EntryKind::File,
            ),
            Entry::new(EntryId(3), root.join("folder"), EntryKind::Directory),
        ]
        .into();
        let panel = &mut presenter.panes[0];
        panel.index = Arc::new(DirectoryIndex::from_entries(&entries));
        panel.entries = entries;
        panel.cursor = Some(EntryId(2));
        panel.parent_cursor = false;
        presenter
    }

    #[test]
    fn f4_dispatches_only_the_focused_file_and_never_a_directory() {
        let mut presenter = presenter();
        presenter.preferences_loaded(Ok(UserPreferences::default()));
        presenter.panes[0].selected = Arc::new(
            SelectionSnapshot::from_bits(presenter.panes[0].index.clone(), vec![1]).unwrap(),
        );
        let commands = RefCell::new(Vec::new());
        let send = |command| {
            commands.borrow_mut().push(command);
            true
        };
        presenter.edit_file(PaneId::Left, &send);
        assert_eq!(commands.borrow().len(), 1);
        let commands = commands.borrow();
        let Command::EditFile { path, .. } = &commands[0] else {
            panic!()
        };
        assert_eq!(path, &presenter.panes[0].entries[1].path);
        drop(commands);
        presenter.panes[0].cursor = Some(EntryId(3));
        presenter.edit_file(PaneId::Left, &|_| panic!("directory must not dispatch"));
    }

    #[test]
    fn editor_settings_parse_literal_json_args_and_survive_late_preferences() {
        let mut presenter = presenter();
        let executable = std::env::current_exe().unwrap();
        presenter.change_editor(
            executable.to_str().unwrap(),
            r#"["{file}","--label=文档 $HOME; &","", "line\nnext"]"#,
            &|_| panic!("await preferences"),
        );
        assert_eq!(
            presenter.preferences.values.editor.arguments,
            ["{file}", "--label=文档 $HOME; &", "", "line\nnext"]
        );
        presenter.preferences_loaded(Ok(UserPreferences::default()));
        assert_eq!(
            presenter.preferences.values.editor.executable,
            Some(executable)
        );
    }

    #[test]
    fn invalid_editor_text_preserves_saved_configuration_and_reports_error() {
        let mut presenter = presenter();
        presenter.change_editor("relative-editor", "[\"{file}\"]", &|_| {
            panic!("invalid must not save")
        });
        assert!(!presenter.editor.error.is_empty());
        assert_eq!(
            presenter.preferences.values.editor,
            EditorPreferences::default()
        );
        presenter.change_editor("relative-editor", "not json", &|_| {
            panic!("invalid must not save")
        });
        assert!(!presenter.editor.error.is_empty());
    }

    #[test]
    fn restore_default_refreshes_the_editor_draft_even_if_preferences_are_unchanged() {
        let mut presenter = presenter();
        let previous = presenter.editor.revision;
        presenter.restore_editor(&|_| true);
        assert_ne!(presenter.editor.revision, previous);
    }

    #[test]
    fn editor_launch_waits_for_saved_configuration_instead_of_using_startup_defaults() {
        let mut presenter = presenter();
        presenter.edit_file(PaneId::Left, &|_| panic!("must load saved editor first"));
        assert!(presenter.desktop.notice.is_some());
    }
}
