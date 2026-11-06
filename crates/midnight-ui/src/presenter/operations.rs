//! Prepared native identities remain in the backend; this state is bounded UI data.
use super::*;

#[derive(Default)]
pub(crate) struct OperationsState {
    next_id: u64,
    pub(crate) pending: Option<OperationId>,
    pub(crate) preview: Option<Arc<OperationPreview>>,
    pub(crate) progress: Vec<Arc<OperationProgress>>,
    pub(crate) closing: bool,
    refresh: Vec<(PaneId, PathBuf, Option<PathBuf>)>,
}

impl Presenter {
    pub(crate) fn start_operation(
        &mut self,
        pane: PaneId,
        kind: MutationKind,
        send: &dyn Fn(Command) -> bool,
    ) {
        if self.operations.closing {
            return;
        }
        let Some(source) = self.visible_directory(pane) else {
            return;
        };
        let p = &self.panes[pane.index()];
        if p.search_view_waiting() {
            return;
        }
        if matches!(kind, MutationKind::Delete(_)) && p.selection_revision > p.selection_resolved {
            self.desktop.report(
                "Selection is updating",
                "Wait for the selection to finish, then try again.".into(),
            );
            return;
        }
        if kind == MutationKind::Rename && (p.parent_cursor || p.cursor.is_none()) {
            return;
        }
        if let Some(id) = self.operations.pending
            && !send(Command::CancelOperation(id))
        {
            return;
        }
        let Some(next) = self.operations.next_id.checked_add(1) else {
            return;
        };
        let id = OperationId(next);
        let request = PrepareOperation {
            id,
            kind,
            source,
            view: p.display_index().clone(),
            selection: p.selected.clone(),
            focused: if p.parent_cursor { None } else { p.cursor },
        };
        if send(Command::PrepareOperation(request)) {
            self.operations.next_id = next;
            self.operations.pending = Some(id);
            self.operations.preview = None;
            // Register immediately: closing before Preparing arrives must not abandon a job.
            self.operations.progress.push(Arc::new(OperationProgress {
                id,
                kind,
                state: OperationState::Preparing,
                total: 0,
                completed: 0,
                failed: 0,
                processed_entries: 0,
                current_path: None,
                message: String::new(),
                errors: vec![],
                source_directory: p.path.clone(),
                renamed_to: None,
            }));
        } else {
            self.desktop
                .report("Operation queue busy", "Try again.".into());
        }
    }

    pub(crate) fn confirm_operation(
        &mut self,
        token: &str,
        name: String,
        send: &dyn Fn(Command) -> bool,
    ) {
        let Some(preview) = self.operations.preview.as_ref() else {
            return;
        };
        if token != preview.id.0.to_string() {
            return;
        }
        let id = preview.id;
        let name = match preview.kind {
            MutationKind::Delete(_) => None,
            MutationKind::Rename => {
                if preview.sample.first().is_some_and(|item| {
                    item.path
                        .file_name()
                        .is_some_and(|old| old.to_string_lossy() == name)
                }) {
                    self.cancel_operation(token, send);
                    return;
                }
                Some(name)
            }
            MutationKind::CreateDirectory | MutationKind::CreateFile => Some(name),
        };
        if send(Command::ConfirmOperation { id, new_name: name }) {
            self.operations.pending = None;
            self.operations.preview = None;
        }
    }
    pub(crate) fn cancel_operation(&mut self, token: &str, send: &dyn Fn(Command) -> bool) {
        let Ok(id) = token.parse::<u64>().map(OperationId) else {
            return;
        };
        if send(Command::CancelOperation(id)) && self.operations.pending == Some(id) {
            self.operations.pending = None;
            self.operations.preview = None;
        }
    }
    pub fn has_live_operations(&self) -> bool {
        self.operations
            .progress
            .iter()
            .any(|p| !p.state.is_terminal())
    }
    pub fn request_operation_shutdown(&mut self, send: &dyn Fn(Command) -> bool) {
        self.operations.closing = true;
        self.operations.pending = None;
        self.operations.preview = None;
        for progress in &self.operations.progress {
            if !progress.state.is_terminal() {
                let _ = send(Command::CancelOperation(progress.id));
            }
        }
    }
    pub fn continue_operation_shutdown(&mut self, send: &dyn Fn(Command) -> bool) -> bool {
        if self.operations.closing {
            self.request_operation_shutdown(send);
            return !self.has_live_operations();
        }
        false
    }
    pub(crate) fn apply_operation(&mut self, event: Event) -> Option<Event> {
        let Event::Operation(event) = event else {
            return Some(event);
        };
        match event {
            OperationEvent::Preview(preview) => {
                if self.operations.pending == Some(preview.id) && !self.operations.closing {
                    self.operations.preview = Some(preview);
                }
            }
            OperationEvent::Progress(progress) => {
                if progress.state.is_terminal() {
                    if self.operations.pending == Some(progress.id) {
                        self.operations.pending = None;
                        self.operations.preview = None;
                    }
                    for pane in [PaneId::Left, PaneId::Right] {
                        if self.panes[pane.index()].display_path() != progress.source_directory {
                            continue;
                        }
                        if let Some(change) = self
                            .operations
                            .refresh
                            .iter_mut()
                            .find(|(target, _, _)| *target == pane)
                        {
                            change.1 = progress.source_directory.clone();
                            change.2 = progress.renamed_to.clone();
                        } else {
                            self.operations.refresh.push((
                                pane,
                                progress.source_directory.clone(),
                                progress.renamed_to.clone(),
                            ));
                        }
                    }
                    if !progress.message.is_empty() {
                        self.desktop
                            .report("File operation", progress.message.clone());
                    }
                }
                if let Some(old) = self
                    .operations
                    .progress
                    .iter_mut()
                    .find(|old| old.id == progress.id)
                {
                    if !old.state.is_terminal() {
                        *old = progress;
                    }
                } else {
                    self.operations.progress.push(progress);
                }
                while self
                    .operations
                    .progress
                    .iter()
                    .filter(|p| p.state.is_terminal())
                    .count()
                    > 16
                {
                    let index = self
                        .operations
                        .progress
                        .iter()
                        .position(|p| p.state.is_terminal())
                        .unwrap();
                    self.operations.progress.remove(index);
                }
            }
        }
        None
    }
    pub(super) fn refresh_operation_paths(&mut self, send: &dyn Fn(Command) -> bool) {
        let changes = mem::take(&mut self.operations.refresh);
        for (pane, directory, destination) in changes {
            let p = &self.panes[pane.index()];
            if p.display_path() != directory {
                continue;
            }
            if p.loading || !self.navigate(pane, directory.clone(), None, send) {
                self.operations.refresh.push((pane, directory, destination));
            } else {
                if destination.is_some() {
                    self.change_search_query(pane, "", send);
                }
                let p = &mut self.panes[pane.index()];
                p.preserve_viewport = destination.is_none();
                if let Some(destination) = &destination {
                    // A same-directory refresh retains old rows. Release their old
                    // caret so wanted_cursor can reveal the created/renamed entry.
                    // Subsequent user input still wins through cursor_touched.
                    p.cursor = p.index.by_path.get(destination).copied();
                    p.parent_cursor = false;
                }
                if let Some(request) = &mut p.pending {
                    request.wanted_cursor = destination;
                }
            }
        }
    }
}
