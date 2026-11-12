//! Metadata editor state contains only bounded preview data; native handles stay in the engine.
use super::*;

#[derive(Default)]
pub(crate) struct MetadataState {
    pub(crate) id: Option<OperationId>,
    pub(crate) revision: u64,
    pub(crate) start_case: bool,
    pub(crate) dirty: bool,
    pub(crate) busy: bool,
    pub(crate) error: String,
    pub(crate) preview: Option<Arc<MetadataPreview>>,
}

impl MetadataState {
    pub(crate) fn preview_current(&self) -> bool {
        !self.dirty
            && !self.busy
            && self.revision > 0
            && self.preview.as_ref().is_some_and(|preview| {
                Some(preview.id) == self.id && preview.revision == self.revision
            })
    }

    pub(crate) fn can_apply(&self) -> bool {
        self.id.is_some()
            && !self.dirty
            && !self.busy
            && self.error.is_empty()
            && self.preview.as_ref().is_some_and(|preview| {
                Some(preview.id) == self.id
                    && preview.revision == self.revision
                    && !preview.patch.is_empty()
                    && preview.unsupported == 0
                    && preview
                        .errors
                        .iter()
                        .all(|error| error.kind == std::io::ErrorKind::AlreadyExists)
            })
    }

    fn matches(&self, token: &str) -> bool {
        self.id.is_some_and(|id| token.parse::<u64>() == Ok(id.0))
    }
}

impl Presenter {
    pub(super) fn retire_metadata(&mut self) {
        self.retired
            .push(Box::new(std::mem::take(&mut self.metadata)));
    }

    pub(crate) fn open_metadata(
        &mut self,
        pane: PaneId,
        case: bool,
        send: &dyn Fn(Command) -> bool,
    ) {
        if self.panes[pane.index()].tree_view().is_some() {
            self.request_tree_metadata(pane, case, send);
            return;
        }
        self.request_selection_action(
            pane,
            selection_actions::SelectionIntent::Metadata { pane, case },
            send,
        );
    }
    pub(super) fn start_captured_metadata(
        &mut self,
        pane: PaneId,
        case: bool,
        files: &SelectedFiles,
        send: &dyn Fn(Command) -> bool,
    ) {
        let previous = self.operations.pending;
        self.start_captured_operation(pane, MutationKind::EditMetadata, files, send);
        if self.operations.pending != previous && self.operations.pending.is_some() {
            self.retire_metadata();
            self.metadata = MetadataState {
                id: self.operations.pending,
                start_case: case,
                busy: true,
                ..Default::default()
            };
        }
    }
    pub(crate) fn dirty_metadata(&mut self, token: &str) {
        if !self.metadata.matches(token) {
            return;
        }
        self.metadata.dirty = true;
        self.metadata.busy = false;
        self.metadata.error.clear();
        if let Some(next) = self.metadata.revision.checked_add(1) {
            self.metadata.revision = next;
        } else {
            self.metadata.error =
                "Draft revisions are exhausted. Close and reopen the editor.".into();
        }
    }
    pub(crate) fn preview_metadata(
        &mut self,
        token: &str,
        patch: Result<MetadataPatch, String>,
        send: &dyn Fn(Command) -> bool,
    ) {
        if !self.metadata.matches(token) || self.operations.closing {
            return;
        }
        self.dirty_metadata(token);
        let patch = match patch {
            Ok(patch) => patch,
            Err(error) => {
                self.metadata.error = error;
                return;
            }
        };
        if !self.metadata.error.is_empty() {
            return;
        }
        let id = self.metadata.id.expect("matched metadata operation");
        if send(Command::PreviewMetadata {
            id,
            revision: self.metadata.revision,
            patch,
        }) {
            self.metadata.dirty = false;
            self.metadata.busy = true;
        } else {
            self.metadata.error = "The command queue is busy. Try Preview again.".into();
        }
    }
    pub(crate) fn confirm_metadata(
        &mut self,
        token: &str,
        revision: &str,
        policy: i32,
        send: &dyn Fn(Command) -> bool,
    ) {
        if !self.metadata.matches(token)
            || revision.parse::<u64>() != Ok(self.metadata.revision)
            || !self.metadata.can_apply()
            || self.operations.closing
        {
            return;
        }
        let conflicts = match policy {
            0 => MetadataConflictPolicy::Skip,
            1 => MetadataConflictPolicy::SkipAll,
            2 => MetadataConflictPolicy::Cancel,
            _ => return,
        };
        let id = self.metadata.id.expect("matched metadata operation");
        if send(Command::ConfirmMetadata {
            id,
            revision: self.metadata.revision,
            conflicts,
        }) {
            self.operations.unconfirmed.remove(&id);
            self.operations.pending = None;
            self.operations.preview = None;
            self.retire_metadata();
        }
    }
    pub(crate) fn cancel_metadata(&mut self, token: &str, send: &dyn Fn(Command) -> bool) {
        if !self.metadata.matches(token) {
            return;
        }
        self.cancel_operation(token, send);
        if self.operations.pending != self.metadata.id {
            self.retire_metadata();
        }
    }

    pub(super) fn apply_metadata_preview(&mut self, preview: Arc<MetadataPreview>) {
        if self.operations.closing
            || self.metadata.id != Some(preview.id)
            || self.metadata.revision != preview.revision
            || self.metadata.dirty
        {
            self.retired.push(Box::new(preview));
            return;
        }
        self.metadata.busy = false;
        self.metadata.error = preview
            .errors
            .iter()
            .filter(|error| error.kind != std::io::ErrorKind::AlreadyExists)
            .map(crate::error_presentation::describe)
            .collect::<Vec<_>>()
            .join("\n");
        if let Some(previous) = self.metadata.preview.replace(preview) {
            self.retired.push(Box::new(previous));
        }
    }

    pub(super) fn apply_metadata_conflict(&mut self, conflict: Arc<MetadataConflict>) {
        if self.operations.closing
            || !self
                .operations
                .progress
                .iter()
                .any(|progress| progress.id == conflict.id && !progress.state.is_terminal())
        {
            self.retired.push(Box::new(conflict));
            return;
        }
        if let Some(current) = self
            .operations
            .metadata_conflicts
            .iter_mut()
            .find(|old| old.id == conflict.id)
        {
            if conflict.token > current.token {
                *current = conflict;
            }
        } else if self.operations.metadata_conflicts.len() < 32 {
            self.operations.metadata_conflicts.push(conflict);
        }
    }

    pub(crate) fn decide_metadata_conflict(
        &mut self,
        token: &str,
        choice: i32,
        send: &dyn Fn(Command) -> bool,
    ) {
        let Some(index) = self
            .operations
            .metadata_conflicts
            .iter()
            .position(|conflict| token == format!("metadata:{}:{}", conflict.id.0, conflict.token))
        else {
            return;
        };
        let choice = match choice {
            0 => MetadataConflictPolicy::Skip,
            1 => MetadataConflictPolicy::SkipAll,
            3 => MetadataConflictPolicy::Cancel,
            _ => return,
        };
        let conflict = &self.operations.metadata_conflicts[index];
        if send(Command::ResolveMetadataConflict {
            id: conflict.id,
            token: conflict.token,
            choice,
        }) {
            self.operations.metadata_conflicts.remove(index);
        } else {
            self.desktop.report(
                "Operation queue busy",
                "The conflict is still waiting. Try again.".into(),
            );
        }
    }
}
