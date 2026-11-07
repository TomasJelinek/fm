//! Prepared native identities remain in the backend; this state is bounded UI data.
use super::*;

#[derive(Default)]
pub(crate) struct OperationsState {
    pub(super) next_id: u64,
    pub(crate) pending: Option<OperationId>,
    pub(crate) preview: Option<Arc<OperationPreview>>,
    pub(crate) progress: Vec<Arc<OperationProgress>>,
    pub(crate) metadata_conflicts: Vec<Arc<MetadataConflict>>,
    pub(crate) metadata_results: Vec<Arc<MetadataResult>>,
    pub(crate) closing: bool,
    pub(super) refresh: Vec<(PaneId, PathBuf, Option<PathBuf>)>,
}

impl Presenter {
    /// Availability is based on native entry identities, never the rendered row label.
    pub(crate) fn can_target_cursor(&self, pane: PaneId) -> bool {
        let p = &self.panes[pane.index()];
        !p.parent_cursor
            && !p.search_view_waiting()
            && p.cursor.and_then(|id| p.entry_by_id(id)).is_some()
    }

    pub(crate) fn can_target_selection(&self, pane: PaneId) -> bool {
        let p = &self.panes[pane.index()];
        !p.search_view_waiting()
            && !p.selection_pending_bulk
            && !p.selection_deferred
            && !p.selection_rebase_pending
            && p.selection_revision <= p.selection_resolved
            && p.archive
                .as_ref()
                .is_none_or(|archive| !archive.selection_pending)
            && (p.selected_count() > 0 || self.can_target_cursor(pane))
    }

    pub(crate) fn start_operation(
        &mut self,
        pane: PaneId,
        kind: MutationKind,
        send: &dyn Fn(Command) -> bool,
    ) {
        if !self.native_action_allowed(pane) {
            return;
        }
        if self.operations.closing {
            return;
        }
        if self.visible_directory(pane).is_none() {
            return;
        }
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
        let files = SelectedFiles {
            view: p.display_index().clone(),
            selection: p.selected.clone(),
            fallback: (!p.parent_cursor)
                .then(|| {
                    p.cursor
                        .and_then(|id| p.entry_by_id(id))
                        .map(|entry| entry.path.clone())
                })
                .flatten(),
        };
        self.start_captured_operation(pane, kind, &files, send);
    }

    pub(super) fn start_captured_operation(
        &mut self,
        pane: PaneId,
        kind: MutationKind,
        files: &SelectedFiles,
        send: &dyn Fn(Command) -> bool,
    ) {
        if self.operations.closing {
            return;
        }
        let Some(source) = self.visible_directory(pane) else {
            return;
        };
        let focused = files
            .fallback
            .as_ref()
            .and_then(|path| files.view.by_path.get(path))
            .copied();
        if kind == MutationKind::Rename && focused.is_none() {
            return;
        }
        if matches!(kind, MutationKind::Delete(_) | MutationKind::EditMetadata)
            && files.selection.is_empty()
            && focused.is_none()
        {
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
            view: files.view.clone(),
            selection: files.selection.clone(),
            focused,
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
                source_directory: self.panes[pane.index()].path.clone(),
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
            MutationKind::EditMetadata => return,
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
        self.copies.progress.iter().any(|p| !p.state.is_terminal())
            || self
                .operations
                .progress
                .iter()
                .any(|p| !p.state.is_terminal())
    }
    pub fn request_operation_shutdown(&mut self, send: &dyn Fn(Command) -> bool) {
        self.retire_copy_draft();
        self.shutdown_drag_drop(send);
        self.retire_metadata();
        self.operations.closing = true;
        self.operations.pending = None;
        self.operations.preview = None;
        self.copies.conflicts.clear();
        self.operations.metadata_conflicts.clear();
        for progress in &self.copies.progress {
            if !progress.state.is_terminal() {
                let _ = send(Command::CancelCopy(progress.id));
            }
        }
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
            OperationEvent::MetadataPreview(preview) => self.apply_metadata_preview(preview),
            OperationEvent::MetadataResult(result) => {
                self.operations
                    .metadata_results
                    .retain(|old| old.id != result.id);
                self.operations.metadata_results.push(result);
                if self.operations.metadata_results.len() > 16 {
                    self.operations.metadata_results.remove(0);
                }
            }
            OperationEvent::MetadataConflict(conflict) => self.apply_metadata_conflict(conflict),
            OperationEvent::Preview(preview) => {
                if preview.kind != MutationKind::EditMetadata
                    && self.operations.pending == Some(preview.id)
                    && !self.operations.closing
                {
                    self.operations.preview = Some(preview);
                }
            }
            OperationEvent::Progress(progress) => {
                if progress.state.is_terminal() {
                    self.operations
                        .metadata_conflicts
                        .retain(|conflict| conflict.id != progress.id);
                    if self.metadata.id == Some(progress.id) {
                        self.retire_metadata();
                    }
                    if self.operations.pending == Some(progress.id) {
                        self.operations.pending = None;
                        self.operations.preview = None;
                    }
                    for pane in [PaneId::Left, PaneId::Right] {
                        if self.panes[pane.index()].display_path() != progress.source_directory {
                            continue;
                        }
                        let reveal = progress
                            .renamed_to
                            .as_ref()
                            .filter(|destination| {
                                destination.parent() == Some(progress.source_directory.as_path())
                            })
                            .cloned();
                        if let Some(change) = self
                            .operations
                            .refresh
                            .iter_mut()
                            .find(|(target, _, _)| *target == pane)
                        {
                            change.1 = progress.source_directory.clone();
                            change.2 = reveal;
                        } else {
                            self.operations.refresh.push((
                                pane,
                                progress.source_directory.clone(),
                                reveal,
                            ));
                        }
                    }
                    if !progress.errors.is_empty() {
                        self.desktop.report(
                            "File operation failed",
                            crate::error_presentation::issues(
                                &progress.errors,
                                progress.completed > 0 || progress.processed_entries > 0,
                            ),
                        );
                    } else if !progress.message.is_empty() {
                        if matches!(
                            progress.state,
                            OperationState::Failed | OperationState::CompletedWithErrors
                        ) {
                            self.desktop
                                .report("File operation failed", progress.message.clone());
                        } else {
                            self.desktop
                                .inform("File operation", progress.message.clone());
                        }
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
