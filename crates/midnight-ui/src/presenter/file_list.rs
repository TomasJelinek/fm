//! Modal file-list drafts keep their original selection and never perform I/O.
use super::*;

#[cfg(test)]
#[path = "file_list_tests.rs"]
mod tests;

pub(crate) const PRESETS: [&str; 3] = [
    "$(FileName)$(CRLF)",
    "$(Drive)$(Path)$(FileName)$(CRLF)",
    "$(FileName:max)$(TAB)$(FileSize)$(TAB)$(FileDate) $(FileTime)$(TAB)$(FileAttributes)$(CRLF)",
];
pub(crate) const VARIABLES: [(&str, &str); 13] = [
    ("File name", "$(FileName)"),
    ("Name without extension", "$(FileNamePart)"),
    ("Extension", "$(FileExtension)"),
    ("Size in bytes", "$(FileSize)"),
    ("Date", "$(FileDate)"),
    ("Time", "$(FileTime)"),
    ("Attributes", "$(FileAttributes)"),
    ("Drive", "$(Drive)"),
    ("Directory path", "$(Path)"),
    ("Line ending (CRLF)", "$(CRLF)"),
    ("Line feed", "$(LF)"),
    ("Carriage return", "$(CR)"),
    ("Tab", "$(TAB)"),
];

#[derive(Default)]
pub(crate) struct FileListState {
    pub session: Option<FileListSession>,
    viewer: Option<ViewerLaunch>,
}

struct Scope {
    pane: PaneId,
    navigation: u64,
    revision: SelectionRevision,
    view: Arc<DirectoryIndex>,
    tree: Option<Arc<TreeProjection>>,
    archive: bool,
    fallback: Option<EntryId>,
}

pub(crate) struct FileListSession {
    pub id: DesktopRequestId,
    pub generation: u64,
    pub submitted_generation: u64,
    pub revision: u64,
    pub template: String,
    pub history: Vec<String>,
    pub destination: i32,
    pub path: PathBuf,
    pub path_revision: u64,
    pub append: bool,
    pub preview: String,
    pub summary: String,
    pub error: String,
    pub conflict: Option<TransferEntry>,
    pub running: bool,
    pub waiting: bool,
    pub closing: bool,
    pub picker: Option<DesktopRequestId>,
    scope: Scope,
    directory: String,
    native_directory: PathBuf,
    capture: Option<Arc<FileListCapture>>,
    preview_due: bool,
    clipboard_content: Option<Arc<dyn GeneratedTextSource>>,
    clipboard_request: Option<DesktopRequestId>,
    clipboard_cancel: Cancellation,
    viewer_pending: bool,
    completed: bool,
    preferences_saved: bool,
    released: bool,
}

impl FileListSession {
    pub fn busy(&self) -> bool {
        self.running || self.waiting || self.closing || self.picker.is_some()
    }

    pub fn can_confirm(&self) -> bool {
        !self.closing && self.conflict.is_some()
    }
}

impl Scope {
    fn matches(&self, pane: &Pane) -> bool {
        self.navigation == pane.generation
            && self.archive == pane.archive.is_some()
            && Arc::ptr_eq(&self.view, pane.display_index())
            && match (&self.tree, pane.tree_view()) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b) && self.revision == pane.tree.revision,
                (None, None) => {
                    self.revision
                        == pane
                            .archive
                            .as_ref()
                            .map_or(pane.selection_revision, |archive| {
                                archive.selection_revision
                            })
                }
                _ => false,
            }
    }

    fn ready(&self, pane: &Pane) -> bool {
        if self.tree.is_some() {
            pane.tree_input_ready() && pane.tree.applied == self.revision
        } else if let Some(archive) = &pane.archive {
            !archive.selection_pending
        } else {
            pane.selection_resolved == self.revision
                && !pane.selection_rebase_pending
                && !pane.selection_deferred.any()
                && !pane.selection_pending_bulk
        }
    }

    fn capture(&self, pane: &Pane, directory: String) -> Option<FileListCapture> {
        if !self.matches(pane) || !self.ready(pane) {
            return None;
        }
        let source = if let Some(view) = &self.tree {
            let publication = pane.tree.publication.as_ref()?;
            let fallback = self
                .fallback
                .and_then(|id| view.index.directory.by_id.get(&id))
                .and_then(|&position| view.rows.get(position))
                .filter(|row| row.selectable && !row.context_only)
                .map(|row| row.id);
            FileListSource::Tree {
                view: view.clone(),
                selection: publication.selection.clone(),
                fallback,
            }
        } else if let Some(archive) = &pane.archive {
            FileListSource::Archive {
                entries: pane.entries.clone(),
                view: self.view.clone(),
                selected: archive.selected.clone(),
                fallback: self.fallback,
            }
        } else {
            FileListSource::Native {
                entries: pane.entries.clone(),
                view: self.view.clone(),
                selection: pane.selected.clone(),
                fallback: self.fallback,
            }
        };
        Some(FileListCapture { source, directory })
    }
}

impl Presenter {
    pub(super) fn file_list_selection_failed(
        &mut self,
        pane: PaneId,
        navigation: NavigationId,
        revision: SelectionRevision,
    ) {
        let Some(s) = self.file_list.session.as_mut().filter(|s| {
            s.waiting
                && s.scope.pane == pane
                && s.scope.navigation == navigation.0
                && s.scope.revision == revision
        }) else {
            return;
        };
        s.waiting = false;
        s.preview_due = false;
        s.summary = "Selection unavailable".into();
        s.error =
            "The requested selection failed. Close this window and make a new file list.".into();
        s.revision = s.revision.wrapping_add(1);
    }

    pub(crate) fn request_make_file_list(&mut self, pane: PaneId, send: &dyn Fn(Command) -> bool) {
        if self.operations.closing || self.file_list.session.is_some() {
            return;
        }
        let p = &self.panes[pane.index()];
        if p.loading
            || p.showing_stream()
            || p.search_view_waiting()
            || p.network.is_some()
            || p.network_request
        {
            self.desktop.report(
                "Make File List",
                "Wait for the directory listing to finish.".into(),
            );
            return;
        }
        let scope = Scope {
            pane,
            navigation: p.generation,
            revision: if p.tree_view().is_some() {
                p.tree.revision
            } else {
                p.archive
                    .as_ref()
                    .map_or(p.selection_revision, |a| a.selection_revision)
            },
            view: p.display_index().clone(),
            tree: p.tree_view().cloned(),
            archive: p.archive.is_some(),
            fallback: (!p.parent_cursor).then_some(p.cursor).flatten(),
        };
        let directory = p.display_location_text();
        let native_directory = p
            .archive
            .as_ref()
            .and_then(|a| a.location.address.container.parent())
            .unwrap_or(p.display_path())
            .to_owned();
        let capture = scope.capture(p, directory.clone()).map(Arc::new);
        let preferences = &self.preferences.values.file_list;
        let template = preferences
            .template_history
            .first()
            .cloned()
            .unwrap_or_else(|| DEFAULT_FILE_LIST_TEMPLATE.into());
        let history = preferences.template_history.clone();
        let destination = i32::from(preferences.destination);
        let path = preferences
            .path
            .clone()
            .unwrap_or_else(|| native_directory.join("file-list.txt"));
        let append = preferences.append;
        let id = self.desktop.request();
        self.file_list.session = Some(FileListSession {
            id,
            generation: 1,
            submitted_generation: 0,
            revision: 1,
            template,
            history,
            destination,
            path,
            path_revision: 0,
            append,
            preview: String::new(),
            summary: if capture.is_some() {
                directory.clone()
            } else {
                "Waiting for selection…".into()
            },
            error: String::new(),
            conflict: None,
            running: false,
            waiting: capture.is_none(),
            closing: false,
            picker: None,
            scope,
            directory,
            native_directory,
            capture,
            preview_due: true,
            clipboard_content: None,
            clipboard_request: None,
            clipboard_cancel: Cancellation::default(),
            viewer_pending: false,
            completed: false,
            preferences_saved: false,
            released: false,
        });
        self.flush_file_list(send);
    }

    /// Regular UI pump retries only accepted work; it never changes the captured target.
    pub fn flush_file_list(&mut self, send: &dyn Fn(Command) -> bool) {
        let Some(s) = &mut self.file_list.session else {
            return;
        };
        if s.closing {
            let id = s.id;
            self.close_file_list(id, send);
            return;
        }
        if s.waiting {
            let pane = &self.panes[s.scope.pane.index()];
            if !s.scope.matches(pane) {
                s.waiting = false;
                s.preview_due = false;
                s.error =
                    "The panel selection changed. Close this window and make a new file list."
                        .into();
                s.revision = s.revision.wrapping_add(1);
            } else if let Some(capture) = s.scope.capture(pane, s.directory.clone()) {
                s.capture = Some(Arc::new(capture));
                s.waiting = false;
                s.summary = s.directory.clone();
                s.revision = s.revision.wrapping_add(1);
            }
        }
        if s.preview_due && !s.waiting && s.capture.is_some() {
            s.preview_due = false;
            let id = s.id;
            self.preview_file_list(id, send);
        }
        let Some(s) = &mut self.file_list.session else {
            return;
        };
        if let Some(content) = &s.clipboard_content {
            let request = self.desktop.request();
            if send(Command::CopyGeneratedText {
                request,
                content: content.clone(),
                cancel: s.clipboard_cancel.clone(),
            }) {
                s.clipboard_content = None;
                s.clipboard_request = Some(request);
            }
        }
        if !s.completed {
            return;
        }
        if !s.released {
            if !send(Command::CancelFileList {
                id: s.id,
                generation: s.submitted_generation,
            }) {
                return;
            }
            s.released = true;
        }
        let scope_matches = s.scope.matches(&self.panes[s.scope.pane.index()]);
        let pane = s.scope.pane;
        let mut values = self.preferences.values.clone();
        if let Err(error) = values.file_list.remember(&s.template) {
            s.error = error;
            s.completed = false;
            s.running = false;
            return;
        }
        values.file_list.destination = s.destination as u8;
        values.file_list.append = s.append;
        if s.destination == 2 {
            values.file_list.path = Some(s.path.clone());
        }
        let save = !s.preferences_saved;
        s.preferences_saved = true;
        if save {
            self.save_file_list_preferences(values.file_list, send);
        }
        if scope_matches && !self.bulk_selection(pane, SelectionOperation::Clear, send) {
            return;
        }
        if let Some(s) = self.file_list.session.take() {
            self.retired.push(Box::new(s));
        }
    }

    pub(crate) fn edit_file_list_template(&mut self, id: DesktopRequestId, template: String) {
        let Some(s) = self
            .file_list
            .session
            .as_mut()
            .filter(|s| s.id == id && !s.busy())
        else {
            return;
        };
        s.template = template;
        s.generation = s.generation.saturating_add(1);
        s.preview.clear();
        s.error.clear();
        s.summary = "Updating preview…".into();
        s.revision = s.revision.wrapping_add(1);
    }

    pub(crate) fn preview_file_list(
        &mut self,
        id: DesktopRequestId,
        send: &dyn Fn(Command) -> bool,
    ) {
        let Some(s) = self
            .file_list
            .session
            .as_mut()
            .filter(|s| s.id == id && !s.busy())
        else {
            return;
        };
        let Some(capture) = &s.capture else { return };
        if s.submitted_generation == s.generation {
            return;
        }
        if send(Command::RunFileList(Box::new(FileListRequest {
            id,
            generation: s.generation,
            capture: capture.clone(),
            template: s.template.clone(),
            destination: FileListDestination::Preview,
        }))) {
            s.submitted_generation = s.generation;
            s.summary = "Updating preview…".into();
        } else {
            s.error = "Preview queue is busy. Edit the template to retry.".into();
        }
        s.revision = s.revision.wrapping_add(1);
    }

    pub(crate) fn generate_file_list(
        &mut self,
        id: DesktopRequestId,
        destination: i32,
        path: String,
        append: bool,
        send: &dyn Fn(Command) -> bool,
    ) {
        let Some(s) = self
            .file_list
            .session
            .as_mut()
            .filter(|s| s.id == id && !s.busy())
        else {
            return;
        };
        let Some(capture) = &s.capture else { return };
        let target = match destination {
            0 => FileListDestination::Clipboard,
            1 => FileListDestination::Viewer,
            2 => {
                if path.is_empty()
                    || path.contains('\0')
                    || (path != s.path.to_string_lossy()
                        && (crate::native_path_edit::ambiguous_edit(&s.native_directory, &path)
                            || crate::native_path_edit::ambiguous_edit(&s.path, &path)))
                {
                    s.error = "Choose a valid output file using Browse.".into();
                    s.revision = s.revision.wrapping_add(1);
                    return;
                }
                let raw = if path == s.path.to_string_lossy() {
                    s.path.clone()
                } else {
                    PathBuf::from(path)
                };
                s.path = if raw.is_absolute() {
                    raw
                } else {
                    s.native_directory.join(raw)
                };
                FileListDestination::File {
                    path: s.path.clone(),
                    append,
                }
            }
            _ => return,
        };
        let generation = s.generation.saturating_add(1);
        if send(Command::RunFileList(Box::new(FileListRequest {
            id,
            generation,
            capture: capture.clone(),
            template: s.template.clone(),
            destination: target,
        }))) {
            s.generation = generation;
            s.submitted_generation = generation;
            s.destination = destination;
            s.append = append;
            s.running = true;
            s.clipboard_cancel = Cancellation::default();
            s.summary = "Creating file list…".into();
            s.preview_due = false;
            s.error.clear();
            s.conflict = None;
        } else {
            s.error = "File list queue is busy. Try Create list again.".into();
        }
        s.revision = s.revision.wrapping_add(1);
    }

    pub(crate) fn confirm_file_list(
        &mut self,
        id: DesktopRequestId,
        send: &dyn Fn(Command) -> bool,
    ) {
        let Some(s) = self
            .file_list
            .session
            .as_mut()
            .filter(|s| s.id == id && s.can_confirm())
        else {
            return;
        };
        if send(Command::ConfirmFileList {
            id,
            generation: s.generation,
        }) {
            s.conflict = None;
            s.error.clear();
        } else {
            s.error = "File list queue is busy. Try confirmation again.".into();
        }
        s.revision = s.revision.wrapping_add(1);
    }

    pub(crate) fn apply_file_list(&mut self, event: FileListEvent) {
        let Some(s) = self
            .file_list
            .session
            .as_mut()
            .filter(|s| s.id == event.id && s.generation == event.generation && !s.closing)
        else {
            self.retired.push(Box::new(event));
            return;
        };
        s.revision = s.revision.wrapping_add(1);
        match event.result {
            Err(error) => {
                s.running = false;
                s.conflict = None;
                s.error = crate::error_presentation::describe(&error);
            }
            Ok(FileListUpdate::Preview {
                text,
                items,
                truncated,
            }) => {
                s.preview = text;
                s.summary = format!(
                    "{} items{}",
                    self.number_locale.format_u64(items),
                    if truncated {
                        " · preview shortened"
                    } else {
                        ""
                    }
                );
            }
            Ok(FileListUpdate::Generated { content, items }) => {
                s.summary = format!("{} items", self.number_locale.format_u64(items));
                if s.destination == 0 {
                    s.clipboard_content = Some(content);
                } else if s.destination == 1 {
                    let Some(disabled_plugins) = self.viewer_plugins.startup_disabled.clone()
                    else {
                        s.running = false;
                        s.error =
                            "Viewer settings are still loading. Try Create list again.".into();
                        self.retired.push(Box::new(content));
                        return;
                    };
                    self.file_list.viewer = Some(ViewerLaunch {
                        find: None,
                        associations: self.viewer_plugins.startup_associations.clone(),
                        tree: None,
                        generated: Some(content),
                        preferences: Arc::new(self.preferences.values.viewer.clone()),
                        archive: None,
                        entries: Arc::from([]),
                        view: Arc::new(DirectoryIndex::default()),
                        focused: PathBuf::from("File list.txt"),
                        basic: true,
                        disabled_plugins,
                    });
                    s.viewer_pending = true;
                }
            }
            Ok(FileListUpdate::Conflict { entry, append }) => {
                s.append = append;
                s.conflict = Some(entry);
            }
            Ok(FileListUpdate::Saved {
                path,
                bytes,
                recovery,
                warning,
            }) => {
                s.path = path;
                s.summary = format!("Saved {} bytes", self.number_locale.format_u64(bytes));
                if warning.is_some() || !recovery.is_empty() {
                    let mut message = warning
                        .as_ref()
                        .map(crate::error_presentation::describe)
                        .unwrap_or_default();
                    for path in recovery {
                        message.push_str(&format!("\nRecovery file\n{}", path.display()));
                    }
                    self.desktop.report("Make File List", message);
                }
                s.completed = true;
            }
        }
    }

    pub(crate) fn close_file_list(
        &mut self,
        id: DesktopRequestId,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let Some(s) = self.file_list.session.as_mut().filter(|s| s.id == id) else {
            return true;
        };
        s.closing = true;
        s.clipboard_cancel.cancel();
        s.revision = s.revision.wrapping_add(1);
        if let Some(request) = s.picker {
            if !send(Command::CancelPathPicker { request }) {
                return false;
            }
            s.picker = None;
        }
        if s.submitted_generation != 0
            && !send(Command::CancelFileList {
                id,
                generation: s.submitted_generation,
            })
        {
            s.error = "Cancelling file list…".into();
            return false;
        }
        if let Some(s) = self.file_list.session.take() {
            self.retired.push(Box::new(s));
        }
        if let Some(viewer) = self.file_list.viewer.take() {
            self.retired.push(Box::new(viewer));
        }
        true
    }

    pub(crate) fn owns_file_list_clipboard(&self, request: DesktopRequestId) -> bool {
        self.file_list
            .session
            .as_ref()
            .is_some_and(|s| s.clipboard_request == Some(request))
    }

    pub(crate) fn file_list_clipboard_finished(
        &mut self,
        request: DesktopRequestId,
        result: Result<DesktopActionResult, FsError>,
    ) {
        let Some(s) = self
            .file_list
            .session
            .as_mut()
            .filter(|s| s.clipboard_request == Some(request))
        else {
            return;
        };
        s.clipboard_request = None;
        s.revision = s.revision.wrapping_add(1);
        match result {
            Ok(DesktopActionResult::ClipboardCopied) => s.completed = true,
            Ok(_) => {
                s.running = false;
                s.error = "Clipboard did not confirm the generated file list.".into();
            }
            Err(error) => {
                s.running = false;
                s.error = crate::error_presentation::describe(&error);
            }
        }
    }

    /// The app launches this immutable content through its normal viewer service.
    pub fn take_generated_file_list_viewer(
        &mut self,
    ) -> Option<(DesktopRequestId, u64, ViewerLaunch)> {
        let session = self
            .file_list
            .session
            .as_ref()
            .filter(|s| s.viewer_pending && !s.closing)?;
        self.file_list
            .viewer
            .take()
            .map(|launch| (session.id, session.generation, launch))
    }

    /// Only the matching viewer's initial content acknowledgement completes the output.
    pub fn generated_file_list_viewer_finished(
        &mut self,
        id: DesktopRequestId,
        generation: u64,
        result: Result<(), String>,
    ) {
        if self.operations.closing {
            return;
        }
        let Some(s) =
            self.file_list.session.as_mut().filter(|s| {
                s.id == id && s.generation == generation && s.viewer_pending && !s.closing
            })
        else {
            return;
        };
        s.viewer_pending = false;
        s.revision = s.revision.wrapping_add(1);
        match result {
            Ok(()) => s.completed = true,
            Err(error) => {
                s.running = false;
                s.error = error;
            }
        }
    }

    pub(crate) fn request_file_list_picker(
        &mut self,
        id: DesktopRequestId,
        owner: PathPickerOwner,
        send: &dyn Fn(Command) -> bool,
    ) {
        let request = self.desktop.request();
        let Some(s) = self
            .file_list
            .session
            .as_mut()
            .filter(|s| s.id == id && !s.busy())
        else {
            return;
        };
        if send(Command::PickPath {
            request,
            input: PathPickerRequest {
                kind: PathPickerKind::TextSave,
                title: "Save file list".into(),
                initial_directory: s.path.parent().map(Path::to_owned),
                owner,
            },
        }) {
            s.picker = Some(request);
            s.error.clear();
        } else {
            s.error = "File chooser queue is busy. Try Browse again.".into();
        }
        s.revision = s.revision.wrapping_add(1);
    }

    pub(crate) fn owns_file_list_picker(&self, request: DesktopRequestId) -> bool {
        self.file_list
            .session
            .as_ref()
            .is_some_and(|s| s.picker == Some(request))
    }

    pub(crate) fn file_list_path_picked(
        &mut self,
        request: DesktopRequestId,
        result: Result<Option<PathBuf>, FsError>,
    ) {
        let Some(s) = self
            .file_list
            .session
            .as_mut()
            .filter(|s| s.picker == Some(request) && !s.closing)
        else {
            self.retired.push(Box::new(result));
            return;
        };
        s.picker = None;
        s.revision = s.revision.wrapping_add(1);
        match result {
            Ok(Some(path)) => match validate_picked_path(&path) {
                Ok(()) => {
                    s.path = path;
                    s.path_revision = s.path_revision.wrapping_add(1);
                    s.error.clear();
                }
                Err(error) => s.error = crate::error_presentation::describe(&error),
            },
            Ok(None) => (),
            Err(error) => s.error = crate::error_presentation::describe(&error),
        }
    }
}
