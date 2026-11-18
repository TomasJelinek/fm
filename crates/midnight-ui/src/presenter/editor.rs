//! Editor choices and F4 capture are pure state; native discovery/launch run in the engine.
use super::*;

#[derive(Default)]
pub(crate) struct EditorState {
    pub(crate) default_label: String,
    pub(crate) error: String,
    pub(crate) revision: u64,
    pub(crate) draft: Option<super::editor_draft::Draft>,
    pub(crate) dirty: bool,
    pending: bool,
    pub(super) tree: Option<super::tree_editor::Pending>,
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

    pub(crate) fn edit_file(&mut self, pane: PaneId, send: &dyn Fn(Command) -> bool) {
        if !self.native_action_allowed(pane) {
            return;
        }
        if !self.editor_preferences_ready() {
            self.desktop.report("Cannot open editor", "Saved editor settings are not available. Wait for settings to load, or resolve the settings error before editing.".into());
            return;
        }
        if self.panes[pane.index()].tree_view().is_some() {
            self.request_tree_editor(pane, send);
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

#[cfg(all(test, windows))]
#[path = "editor_windows_tests.rs"]
mod native_tests;
#[cfg(all(test, unix))]
#[path = "editor_unix_tests.rs"]
mod native_tests;

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

    /// Saving an Editors draft whose displayed text replaced characters of a raw
    /// native parent is refused; an explicit Unicode path is then accepted.
    pub(super) fn assert_raw_parent_edit_is_refused(parent: PathBuf) {
        let mut presenter = presenter();
        presenter.preferences.loaded = true;
        presenter.preferences.values.editor.executable = Some(parent.join("original-tool"));
        let before = presenter.preferences.values.editor.clone();
        let focus = presenter.panes[0].cursor;
        let path = presenter.panes[0].display_path().to_owned();
        let edited = parent.join("other-tool").to_string_lossy().into_owned();
        presenter.edit_editor_draft(edited, false);
        presenter.save_editor_draft(&|_| panic!("Ambiguous editor settings were saved"));
        assert_eq!(presenter.preferences.values.editor, before);
        assert!(!presenter.editor.error.is_empty());
        assert!(presenter.editor.dirty, "the refused draft stays editable");
        assert_eq!(presenter.panes[0].cursor, focus);
        assert_eq!(presenter.panes[0].display_path(), path);
        let explicit = std::env::temp_dir().join("explicit Unicode tool");
        presenter.edit_editor_draft(explicit.to_str().unwrap().into(), false);
        presenter.save_editor_draft(&|_| true);
        assert_eq!(
            presenter.preferences.values.editor.executable,
            Some(explicit)
        );
        assert!(!presenter.editor.dirty);
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

    /// Saving waits for the saved settings: a draft saved before they load is kept,
    /// unsent, and its literal arguments are saved once the load completes.
    #[test]
    fn editor_draft_waits_for_saved_preferences_and_saves_after_they_load() {
        let mut presenter = presenter();
        let executable = std::env::current_exe().unwrap();
        presenter.edit_editor_draft(executable.to_string_lossy().into_owned(), false);
        presenter.edit_editor_argument(1, 0, "--label=文档 $HOME; &");
        presenter.save_editor_draft(&|_| panic!("the draft must wait for saved preferences"));
        assert!(presenter.editor.dirty, "the unsaved draft stays editable");
        assert!(presenter.editor.error.is_empty());
        assert_eq!(
            presenter.preferences.values.editor,
            EditorPreferences::default()
        );
        presenter.preferences_loaded(Ok(UserPreferences::default()));
        assert!(presenter.editor.dirty, "loading keeps the draft");
        let commands = RefCell::new(Vec::new());
        presenter.save_editor_draft(&|command| {
            commands.borrow_mut().push(command);
            true
        });
        assert!(!presenter.editor.dirty);
        let editor = &presenter.preferences.values.editor;
        assert_eq!(editor.executable, Some(executable));
        assert_eq!(editor.arguments, ["{file}", "--label=文档 $HOME; &"]);
        assert!(
            matches!(
                commands.borrow().as_slice(),
                [Command::SavePreferences { preferences, .. }] if preferences.editor == *editor
            ),
            "the loaded draft is saved once"
        );
    }

    #[test]
    fn invalid_editor_draft_preserves_saved_configuration_and_reports_error() {
        let mut presenter = presenter();
        presenter.preferences.loaded = true;
        presenter.edit_editor_draft("relative-editor".into(), false);
        presenter.save_editor_draft(&|_| panic!("invalid must not save"));
        assert!(!presenter.editor.error.is_empty());
        assert!(presenter.editor.dirty, "the refused draft stays editable");
        assert_eq!(
            presenter.preferences.values.editor,
            EditorPreferences::default()
        );
    }

    #[test]
    fn editor_launch_waits_for_saved_configuration_instead_of_using_startup_defaults() {
        let mut presenter = presenter();
        presenter.edit_file(PaneId::Left, &|_| panic!("must load saved editor first"));
        assert!(presenter.desktop.notice.is_some());
    }
}
