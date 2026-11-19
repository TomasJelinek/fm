//! Prepared native identities remain in the backend; this state is bounded UI data.
use super::*;

#[derive(Default)]
pub(crate) struct OperationsState {
    pub(super) next_id: u64,
    pub(crate) pending: Option<OperationId>,
    pub(super) tree_delete: Option<tree_delete::Pending>,
    pub(super) tree_rename: Option<tree_rename::Pending>,
    pub(super) tree_metadata: Option<tree_metadata::Pending>,
    pub(crate) preview: Option<Arc<OperationPreview>>,
    pub(crate) progress: Vec<Arc<OperationProgress>>,
    pub(crate) rename_conflicts: Vec<Arc<CopyConflict>>,
    pub(super) rename_versions: std::collections::HashMap<OperationId, u64>,
    pub(super) rename_deferred: std::collections::HashSet<(OperationId, u64)>,
    pub(super) rename_selected: Option<(OperationId, u64)>,
    pub(crate) unconfirmed: std::collections::HashSet<OperationId>,
    /// A new empty file whose name the displayed folder already has: confirming the
    /// same name again replaces that file.
    pub(crate) replace_name: Option<(OperationId, String)>,
    pub(super) forgotten_through: u64,
    pub(crate) metadata_conflicts: Vec<Arc<MetadataConflict>>,
    pub(crate) metadata_results: HashMap<OperationId, Arc<MetadataResult>>,
    pub(crate) closing: bool,
    pub(super) refresh: Vec<(PaneId, PathBuf, Option<PathBuf>)>,
    pub(crate) settle: Settle,
}

/// Panel input waits while a confirmed single-entry job (Rename, New file, New
/// folder) runs, and after any completed file job while the panel re-reads that
/// show its change are in flight. Stale rows cannot be acted on, and a completed
/// change does not look as if it failed. The window bounds the wait.
#[derive(Debug, Default)]
pub(crate) struct Settle {
    /// The confirmed single-entry job input waits for.
    job: Option<OperationId>,
    /// Panes whose queued refresh shows a completed change.
    hold: [bool; 2],
    /// Per pane, the in-flight re-read that shows it.
    reads: [Option<NavigationId>; 2],
}

impl OperationsState {
    /// See `Presenter::queue_operation_refresh`; borrows only what it changes so a
    /// caller can hold another part of the presenter.
    pub(super) fn queue_refresh(
        &mut self,
        pane: PaneId,
        shown: &Pane,
        directory: &Path,
        reveal: Option<PathBuf>,
        hold: bool,
        retired: &mut Vec<DeferredDrop>,
    ) {
        if !shown.native_capabilities() || shown.display_path() != directory {
            return;
        }
        self.settle.hold[pane.index()] |= hold;
        if let Some(change) = self
            .refresh
            .iter_mut()
            .find(|(target, _, _)| *target == pane)
        {
            if change.1 != directory {
                change.1 = directory.to_owned();
                retired.push(Box::new(mem::replace(&mut change.2, reveal)));
            } else if reveal.is_some() {
                retired.push(Box::new(mem::replace(&mut change.2, reveal)));
            }
        } else {
            self.refresh.push((pane, directory.to_owned(), reveal));
        }
    }
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
            && !p.selection_deferred.any()
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
        if kind == MutationKind::Rename && self.panes[pane.index()].tree_view().is_some() {
            self.request_tree_rename(pane, send);
            return;
        }
        if let MutationKind::Delete(mode) = kind
            && self.panes[pane.index()].tree_view().is_some()
        {
            self.request_tree_delete(pane, mode, send);
            return;
        }
        self.cancel_tree_metadata(None);
        self.cancel_tree_rename(None);
        self.cancel_tree_delete(None);
        if !matches!(
            kind,
            MutationKind::CreateDirectory | MutationKind::CreateFile
        ) && !self.tree_action_allowed(pane)
        {
            return;
        }
        if !self.native_action_allowed(pane) {
            return;
        }
        if self.operations.closing {
            return;
        }
        let p = &self.panes[pane.index()];
        let deleting = matches!(kind, MutationKind::Delete(_));
        if self.visible_directory(pane).is_none() || p.search_view_waiting() {
            if deleting {
                self.desktop.report(
                    "Delete unavailable",
                    "Wait for the file list to finish updating, then try again.".into(),
                );
            }
            return;
        }
        if deleting && p.selection_revision > p.selection_resolved {
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
            view: if p.tree_view().is_some() {
                p.index.clone()
            } else {
                p.display_index().clone()
            },
            selection: p.selected.clone(),
            fallback: (!p.parent_cursor && p.tree_view().is_none())
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
        self.cancel_tree_metadata(None);
        self.cancel_tree_rename(None);
        self.cancel_tree_delete(None);
        let Some(id) = self.next_operation_id(send) else {
            return;
        };
        let request = PrepareOperation {
            id,
            kind,
            source,
            view: files.view.clone(),
            selection: files.selection.clone(),
            focused,
        };
        if send(Command::PrepareOperation(request)) {
            self.register_preparing_operation(id, kind, self.panes[pane.index()].path.clone());
        } else {
            self.desktop
                .report("Operation queue busy", "Try again.".into());
        }
    }

    pub(super) fn next_operation_id(
        &mut self,
        send: &dyn Fn(Command) -> bool,
    ) -> Option<OperationId> {
        if self.operations.closing {
            return None;
        }
        if let Some(id) = self.operations.pending
            && !send(Command::CancelOperation(id))
        {
            return None;
        }
        self.operations.next_id.checked_add(1).map(OperationId)
    }

    pub(super) fn register_preparing_operation(
        &mut self,
        id: OperationId,
        kind: MutationKind,
        source_directory: PathBuf,
    ) {
        self.operations.next_id = id.0;
        self.operations.pending = Some(id);
        self.operations.preview = None;
        self.operations.unconfirmed.insert(id);
        // Register immediately: closing before Preparing arrives must not abandon a job.
        self.operations.progress.push(Arc::new(OperationProgress {
            pause_requested: false,
            paused: false,
            discovery: Default::default(),
            id,
            throughput: Arc::default(),
            kind,
            state: OperationState::Preparing,
            total: 0,
            completed: 0,
            failed: 0,
            processed_entries: 0,
            current_path: None,
            message: String::new(),
            errors: vec![],
            source_directory,
            renamed_to: None,
        }));
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
        let mut replace_existing = false;
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
            MutationKind::CreateDirectory => Some(name),
            MutationKind::CreateFile => {
                if self.displayed_file_exists(id, &name) {
                    if self.operations.replace_name.as_ref() != Some(&(id, name.clone())) {
                        // Ask first: the dialog now offers Replace for this name.
                        self.operations.replace_name = Some((id, name));
                        return;
                    }
                    replace_existing = true;
                }
                Some(name)
            }
        };
        let single_entry = matches!(
            preview.kind,
            MutationKind::Rename | MutationKind::CreateFile | MutationKind::CreateDirectory
        );
        if send(Command::ConfirmOperation {
            id,
            new_name: name,
            replace_existing,
        }) {
            // A job queued behind another one may wait for it; never hold input
            // for that.
            if single_entry && !self.other_mutation_live(id) {
                self.operations.settle.job = Some(id);
            }
            self.operations.replace_name = None;
            self.operations.unconfirmed.remove(&id);
            self.operations.pending = None;
            self.operations.preview = None;
        }
    }
    /// Whether the panel showing the operation's folder lists a file with this name. The
    /// engine still creates exclusively, so a file that appeared since is reported, not replaced.
    fn displayed_file_exists(&self, id: OperationId, name: &str) -> bool {
        let Some(directory) = self
            .operations
            .progress
            .iter()
            .find(|progress| progress.id == id)
            .map(|progress| progress.source_directory.clone())
        else {
            return false;
        };
        let same = |entry: &std::ffi::OsStr| {
            if cfg!(windows) {
                entry.to_string_lossy().to_lowercase() == name.to_lowercase()
            } else {
                entry == std::ffi::OsStr::new(name)
            }
        };
        self.panes.iter().any(|pane| {
            pane.display_path() == directory
                && pane.entries.iter().any(|entry| {
                    !matches!(entry.kind, EntryKind::Directory | EntryKind::DirectoryLink)
                        && entry.path.file_name().is_some_and(same)
                })
        })
    }

    /// Whether a copy or another file operation can delay the engine's next job.
    fn other_mutation_live(&self, id: OperationId) -> bool {
        self.copies.progress.iter().any(|p| !p.state.is_terminal())
            || self
                .operations
                .progress
                .iter()
                .any(|p| p.id != id && !p.state.is_terminal())
    }

    /// Whether panel input waits for a file job's change to become visible.
    pub(crate) fn mutation_settling(&self) -> bool {
        let settle = &self.operations.settle;
        settle.job.is_some() || settle.reads.iter().any(Option::is_some)
    }

    /// Stop waiting for `id`'s job (every wait without one); its re-reads still
    /// hold input once it completes.
    pub(super) fn release_mutation_settle(&mut self, id: Option<OperationId>) {
        let settle = &mut self.operations.settle;
        match id {
            Some(id) if settle.job == Some(id) => settle.job = None,
            Some(_) => {}
            None => *settle = Settle::default(),
        }
    }

    /// Release each re-read once it committed, failed or was superseded.
    fn settle_mutation_reads(&mut self) {
        let settle = &mut self.operations.settle;
        for pane in [PaneId::Left, PaneId::Right] {
            let p = &self.panes[pane.index()];
            if settle.reads[pane.index()]
                .is_some_and(|navigation| p.generation != navigation.0 || !p.loading)
            {
                settle.reads[pane.index()] = None;
            }
        }
    }

    pub(crate) fn cancel_operation(&mut self, token: &str, send: &dyn Fn(Command) -> bool) {
        let Ok(id) = token
            .strip_prefix("rename-job:")
            .unwrap_or(token)
            .parse::<u64>()
            .map(OperationId)
        else {
            return;
        };
        if send(Command::CancelOperation(id)) {
            self.release_mutation_settle(Some(id));
            if let Some(progress) = self
                .operations
                .progress
                .iter_mut()
                .find(|progress| progress.id == id && !progress.state.is_terminal())
            {
                Arc::make_mut(progress).state = OperationState::CancelRequested;
            }
            if self.operations.pending == Some(id) {
                self.operations.pending = None;
                self.operations.preview = None;
            }
        }
    }
    pub fn has_live_operations(&self) -> bool {
        self.command_launches.jobs.iter().any(|job| !job.finished)
            || self.copies.progress.iter().any(|p| !p.state.is_terminal())
            || self
                .operations
                .progress
                .iter()
                .any(|p| !p.state.is_terminal())
    }
    pub fn request_operation_shutdown(&mut self, send: &dyn Fn(Command) -> bool) {
        self.command_toolbar.invalidate_preview();
        self.retire_copy_draft();
        self.shutdown_drag_drop(send);
        self.retire_metadata();
        self.operations.closing = true;
        self.cancel_tree_editor(None);
        self.cancel_tree_checksums(None);
        self.cancel_tree_user_commands(None);
        self.cancel_tree_metadata(None);
        self.cancel_tree_rename(None);
        self.cancel_tree_delete(None);
        self.retire_warning_viewer();
        self.operations.pending = None;
        self.operations.preview = None;
        self.copies.conflicts.clear();
        self.copies.failures.clear();
        self.copies.deferred_decisions.clear();
        self.operations.metadata_conflicts.clear();
        self.operations.rename_conflicts.clear();
        self.operations.rename_versions.clear();
        self.operations.rename_deferred.clear();
        self.operations.rename_selected = None;
        self.release_mutation_settle(None);
        for job in &self.command_launches.jobs {
            if !job.finished {
                let _ = send(Command::CancelUserCommand(job.id));
            }
        }
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
                if self.forgotten_operation(result.id) {
                    self.retired.push(Box::new(result));
                    return None;
                }
                if let Some(old) = self.operations.metadata_results.insert(result.id, result) {
                    self.retired.push(Box::new(old));
                }
            }
            OperationEvent::MetadataConflict(conflict) => self.apply_metadata_conflict(conflict),
            OperationEvent::RenameConflict(conflict) => {
                // The conflict prompt needs the user; the panel must not wait for it.
                self.release_mutation_settle(Some(conflict.id));
                self.apply_rename_conflict(conflict);
            }
            OperationEvent::Preview(preview) => {
                if preview.kind != MutationKind::EditMetadata
                    && self.operations.pending == Some(preview.id)
                    && !self.operations.closing
                {
                    self.operations.preview = Some(preview);
                }
            }
            OperationEvent::Progress(progress) => {
                if self.forgotten_operation(progress.id) {
                    return None;
                }
                let unconfirmed = self.operations.unconfirmed.contains(&progress.id);
                if unconfirmed
                    && progress.state == OperationState::Cancelled
                    && progress.failed == 0
                    && progress.completed == 0
                    && progress.processed_entries == 0
                    && progress
                        .errors
                        .iter()
                        .all(|error| error.kind == std::io::ErrorKind::Interrupted)
                {
                    self.forget_unconfirmed_operation(progress.id);
                    return None;
                }
                if progress.state.is_terminal()
                    || matches!(
                        progress.state,
                        OperationState::Queued | OperationState::Running
                    )
                {
                    self.operations.unconfirmed.remove(&progress.id);
                }
                if progress.state.is_terminal() {
                    self.retire_rename_decision(progress.id);
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
                    let reveal = progress
                        .renamed_to
                        .as_ref()
                        .filter(|destination| {
                            destination.parent() == Some(progress.source_directory.as_path())
                        })
                        .cloned();
                    self.release_mutation_settle(Some(progress.id));
                    // Only a completed change keeps input waiting, for its re-reads;
                    // a failure is reported without holding the panel.
                    let hold = matches!(
                        progress.state,
                        OperationState::Completed | OperationState::CompletedWithErrors
                    );
                    for pane in [PaneId::Left, PaneId::Right] {
                        if self.panes[pane.index()].queue_native_tree_refresh(&mut self.retired) {
                            continue;
                        }
                        self.queue_operation_refresh(
                            pane,
                            &progress.source_directory,
                            reveal.clone(),
                            hold,
                        );
                    }
                    let title = if progress.state == OperationState::CompletedWithErrors {
                        "File operation completed with errors"
                    } else {
                        "File operation failed"
                    };
                    if !progress.errors.is_empty() {
                        // Only targets that did not complete can leave partial changes.
                        let partial = (progress.completed > 0 || progress.processed_entries > 0)
                            && progress.completed < progress.total;
                        self.desktop.report(
                            title,
                            crate::error_presentation::issues(&progress.errors, partial),
                        );
                    } else if !progress.message.is_empty()
                        // A clean metadata edit just closes; the refreshed panel shows it.
                        && !(progress.kind == MutationKind::EditMetadata
                            && progress.state == OperationState::Completed)
                    {
                        if matches!(
                            progress.state,
                            OperationState::Failed | OperationState::CompletedWithErrors
                        ) {
                            self.desktop.report(title, progress.message.clone());
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
                    // A buffered Delete snapshot cannot undo accepted cancellation.
                    // Workers can still refresh the details while cancellation is pending.
                    let cancelling_delete = matches!(old.kind, MutationKind::Delete(_))
                        && old.state == OperationState::CancelRequested;
                    if !old.state.is_terminal()
                        && (!cancelling_delete
                            || progress.state == OperationState::CancelRequested
                            || progress.state.is_terminal())
                    {
                        *old = progress;
                    }
                } else {
                    self.operations.progress.push(progress);
                }
            }
        }
        None
    }
    /// Queue one coalesced post-operation re-read of a native pane showing
    /// `directory`. A newer completion for the pane replaces an older target, so a
    /// pane that moved on while its previous re-read waited is still refreshed.
    pub(super) fn queue_operation_refresh(
        &mut self,
        pane: PaneId,
        directory: &Path,
        reveal: Option<PathBuf>,
        hold: bool,
    ) {
        self.operations.queue_refresh(
            pane,
            &self.panes[pane.index()],
            directory,
            reveal,
            hold,
            &mut self.retired,
        );
    }

    pub(super) fn refresh_operation_paths(&mut self, send: &dyn Fn(Command) -> bool) {
        let changes = mem::take(&mut self.operations.refresh);
        // A completion's re-read holds input only when it starts now; one left
        // waiting (comparison, drag, full queue) runs later without a hold.
        let hold = mem::take(&mut self.operations.settle.hold);
        let mut revealed = Vec::new();
        for (pane, directory, destination) in changes {
            let p = &self.panes[pane.index()];
            if !p.native_capabilities() || p.display_path() != directory {
                continue;
            }
            // A completed change must not wait for a same-folder re-read (a watch
            // refresh, often caused by this very change) to finish first: that
            // read may predate the change, and a busy folder keeps one in flight
            // nearly all the time. Supersede it; any other load still wins.
            let supersede = (destination.is_some() || hold[pane.index()])
                && p.pending
                    .as_ref()
                    .is_some_and(|request| request.keep_rows && request.path == directory);
            if (p.loading && !supersede)
                || self.background_refresh_blocked(pane)
                || !self.navigate_impl(pane, directory.clone(), None, Reload::Background, send)
            {
                self.operations.refresh.push((pane, directory, destination));
                continue;
            }
            if hold[pane.index()] {
                self.operations.settle.reads[pane.index()] =
                    Some(NavigationId(self.panes[pane.index()].generation));
            }
            let p = &mut self.panes[pane.index()];
            p.preserve_viewport = destination.is_none();
            if let Some(destination) = &destination {
                // A same-directory refresh retains old rows. Release their old
                // caret so wanted_cursor can reveal the created/renamed entry.
                // Subsequent user input still wins through cursor_touched.
                p.cursor = p.index.by_path.get(destination).copied();
                p.parent_cursor = false;
                revealed.push(pane);
            }
            if let Some(request) = &mut p.pending {
                request.wanted_cursor = destination;
            }
        }
        // Clearing a quick search flushes selection work, which can re-enter this
        // queue; do it only after every queued re-read was settled above.
        for pane in revealed {
            self.change_search_query(pane, "", send);
        }
        self.settle_mutation_reads();
    }
}
