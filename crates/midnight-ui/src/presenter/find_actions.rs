//! Explicit result actions use a separate backend capability, never a reveal path.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FileAction {
    View(bool),
    Edit,
    Open,
}
pub(super) struct PendingFile {
    request: FindValidationRequest,
    action: FileAction,
    editor: EditorPreferences,
}
impl Presenter {
    pub(crate) fn find_file_action(
        &mut self,
        id: FindSessionId,
        row_token: &str,
        action: FileAction,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        if self.operations.closing {
            return false;
        }
        let Some(s) = self.finds.sessions.get(&id) else {
            return false;
        };
        let Some((generation, result)) = row_token
            .split_once(':')
            .and_then(|(g, r)| Some((g.parse::<u64>().ok()?, r.parse::<u64>().ok()?)))
        else {
            return false;
        };
        let Some(found) = s
            .results
            .iter()
            .find(|r| r.id.0 == result)
            .filter(|_| generation == s.generation.0)
        else {
            return false;
        };
        if found.entry.kind == EntryKind::Directory {
            return self.reveal_find(id, row_token, send);
        }
        if found.entry.kind != EntryKind::File {
            return false;
        }
        if s.pending_file.is_some() {
            if let Some(s) = self.finds.session_mut(&id) {
                s.error =
                    "A file action is still opening. Wait for it to finish, then try again.".into();
            }
            return false;
        }
        let Some(token) = self.finds.next_validation.checked_add(1) else {
            return false;
        };
        self.finds.next_validation = token;
        let request = FindValidationRequest {
            session: id,
            generation: s.generation,
            result: found.id,
            token,
        };
        let s = self.finds.session_mut(&id).expect("existing Find session");
        if !send(Command::CaptureFindFile(request)) {
            s.error = "Find file action queue is busy. Try again.".into();
            return false;
        }
        s.pending_file = Some(PendingFile {
            request,
            action,
            editor: self.preferences.values.editor.clone(),
        });
        s.error.clear();
        true
    }

    pub(crate) fn find_action_key(&self, chord: &ShortcutChord) -> Option<FileAction> {
        let bindings = self.keyboard.bindings.borrow();
        match bindings
            .map
            .resolve(chord, ShortcutContext::Files)?
            .id
            .as_str()
        {
            "files.view" => Some(FileAction::View(false)),
            "files.view-basic" => Some(FileAction::View(true)),
            "files.edit" => Some(FileAction::Edit),
            _ => None,
        }
    }

    pub(in crate::presenter) fn find_file_captured(
        &mut self,
        result: FindFileCaptureResult,
        send: &dyn Fn(Command) -> bool,
    ) {
        let request = result.request;
        let Some(s) = self.finds.session_mut(&request.session).filter(|s| {
            s.generation == request.generation
                && s.pending_file.as_ref().is_some_and(|pending| {
                    pending.request.token == request.token
                        && pending.request.result == request.result
                })
        }) else {
            self.retired.push(Box::new(result));
            return;
        };
        let pending = s.pending_file.take().expect("matched pending file action");
        let source = match result.result {
            Ok(source) => source,
            Err(error) => {
                s.error = crate::error_presentation::describe(&error);
                return;
            }
        };
        let captured = source.request();
        if captured.session != request.session
            || captured.generation != request.generation
            || captured.result != request.result
            || captured.token != request.token
            || source.entry().kind != EntryKind::File
            || !s
                .results
                .iter()
                .any(|r| r.id == request.result && source.entry().matches_capture(&r.entry))
        {
            s.error = "The captured Find file changed. Search again.".into();
            self.retired.push(Box::new(source));
            return;
        }
        s.error.clear();
        match pending.action {
            FileAction::View(basic) => {
                let Some(disabled_plugins) = self.viewer_plugins.startup_disabled.clone() else {
                    s.error = "Viewer settings are still loading. Try again.".into();
                    self.retired.push(Box::new(source));
                    return;
                };
                let launch = ViewerLaunch {
                    associations: self.viewer_plugins.startup_associations.clone(),
                    preferences: Arc::new(self.preferences.values.viewer.clone()),
                    find: Some(source),
                    generated: None,
                    archive: None,
                    tree: None,
                    // The capability supplies the only authority; these fields are unused.
                    entries: Arc::from([]),
                    view: Arc::new(DirectoryIndex::default()),
                    focused: PathBuf::new(),
                    basic,
                    disabled_plugins,
                };
                self.retired.push(Box::new(s.viewer.replace(launch)));
            }
            action => {
                let operation = match action {
                    FileAction::Open => FindFileOperation::Open,
                    FileAction::Edit => FindFileOperation::Edit(pending.editor),
                    FileAction::View(_) => unreachable!(),
                };
                let desktop_request = self.desktop.request();
                let accepted = send(Command::ExecuteFindFile {
                    request: desktop_request,
                    source: source.clone(),
                    action: operation,
                    cancel: s.file_cancel.clone(),
                });
                self.retired.push(Box::new(source));
                if accepted {
                    self.desktop.requests.insert(desktop_request);
                    self.desktop.status = "Opening Find result…".into();
                } else {
                    s.error = "Native action queue is busy. Try again.".into();
                }
            }
        }
    }

    /// The application opens this captured request through its ordinary viewer manager.
    pub fn take_find_viewer(&mut self) -> Option<ViewerLaunch> {
        self.finds
            .sessions
            .values_mut()
            .find_map(|s| s.viewer.take())
    }
}

#[cfg(test)]
#[path = "find_actions_tests.rs"]
mod tests;
