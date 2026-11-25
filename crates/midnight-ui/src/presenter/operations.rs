//! Prepared native identities remain in the backend; this state is bounded UI data.
use super::*;

/// A change of files as its refusals name it.
pub(super) fn mutation_action(kind: MutationKind) -> &'static str {
    match kind {
        MutationKind::Rename => "Rename",
        MutationKind::CreateDirectory => "Create folder",
        MutationKind::CreateFile => "New file",
        MutationKind::EditMetadata => "Attributes, times and case",
        MutationKind::Delete(_) => "Delete",
    }
}

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
    /// Delete failures waiting for the shared decision prompt.
    pub(crate) delete_decisions: super::delete_decisions::DeleteDecisions,
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
    /// Per pane, an action requested while a background re-read kept its rows.
    pub(super) deferred: [Option<DeferredAction>; 2],
    pub(crate) progress_windows: ProgressWindowJobs,
}

/// The jobs whose own progress window is open. That window shows how its job ended
/// (cancelled, with errors or completed), so the main window adds no notice for it.
/// The progress windows (`job_progress_windows.rs`) keep this current; the
/// presenter only reads it.
#[derive(Default)]
pub(crate) struct ProgressWindowJobs(std::cell::RefCell<HashSet<crate::job_row_cache::Key>>);

impl ProgressWindowJobs {
    /// Replaces the set with the jobs whose progress window is open now.
    pub(crate) fn set(&self, jobs: HashSet<crate::job_row_cache::Key>) {
        *self.0.borrow_mut() = jobs;
    }

    pub(crate) fn contains(&self, job: crate::job_row_cache::Key) -> bool {
        self.0.borrow().contains(&job)
    }
}

/// A pane action requested while a background re-read keeps the pane's rows on
/// screen without a loading state (`Pane::keeps_rows_in_background`). The
/// Engine prepares operations and tree roots only against a committed listing,
/// so the action runs once the re-read ends (`run_deferred_actions`), on the
/// targets captured when it was requested, unless the pane left the folder.
pub(super) struct DeferredAction {
    folder: PathBuf,
    kind: DeferredKind,
}

enum DeferredKind {
    Operation(MutationKind, SelectedFiles),
    Disclose { identity: String, recursive: bool },
}

/// How long after its confirmation a job may end and still hold panel input:
/// a job that runs longer went on in the background while the user did other
/// work, and its panels then update like any other background re-read.
const WAITED_JOB: std::time::Duration = std::time::Duration::from_secs(5);

/// How a pane's input waits for a file job's change (`Presenter::mutation_holds`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PaneHold {
    None,
    /// A confirmed Rename, New file or New folder in the pane's folder still
    /// runs: input waits for as long as it takes, and the pane describes the
    /// job once it takes a while.
    Job,
    /// A completed job's re-read or tree refresh that shows the change is in
    /// flight: the window bounds this wait and then says the pane is still
    /// updating until the change appears.
    Reread,
}

impl PaneHold {
    /// The window's `mutation-hold` value.
    pub(crate) fn code(self) -> i32 {
        match self {
            Self::None => 0,
            Self::Job => 1,
            Self::Reread => 2,
        }
    }
}

/// Panel input waits while a confirmed single-entry job (Rename, New file, New
/// folder) runs, and after a completed file job the user waited for while the
/// panel re-reads that show its change are in flight. Only the panels showing
/// the change wait (`mutation_holds`): stale rows cannot be acted on, and a
/// completed change does not look as if it failed. The window bounds the
/// re-read's wait, lets Escape end any wait, and replays the keys typed
/// meanwhile (`shortcut_bridge`).
#[derive(Debug)]
pub(crate) struct Settle {
    /// The confirmed single-entry job input waits for.
    job: Option<OperationId>,
    /// How the held pane describes that job, such as `Renaming “a” to “b”…`.
    job_text: String,
    /// Panes whose queued refresh shows a completed change.
    hold: [bool; 2],
    /// Per pane, the in-flight re-read that shows it.
    reads: [Option<NavigationId>; 2],
    /// Per native tree pane, the refresh epoch that shows it.
    pub(super) tree: [Option<u64>; 2],
    /// Per native tree pane, the renamed or created entry to focus once published.
    pub(super) tree_reveal: [Option<PathBuf>; 2],
    /// When each confirmed job was first reported running, by whether it is a
    /// Copy or Move job (those are numbered apart from other file jobs).
    started: HashMap<(bool, OperationId), std::time::Instant>,
    /// See [`WAITED_JOB`].
    pub(super) waited: std::time::Duration,
}

impl Default for Settle {
    fn default() -> Self {
        Self {
            job: None,
            job_text: String::new(),
            hold: [false; 2],
            reads: [None; 2],
            tree: [None; 2],
            tree_reveal: [None, None],
            started: HashMap::new(),
            waited: WAITED_JOB,
        }
    }
}

impl Settle {
    /// A confirmed job (`copy`: a Copy or Move job) reported that it runs.
    pub(super) fn job_running(&mut self, copy: bool, id: OperationId) {
        self.started
            .entry((copy, id))
            .or_insert_with(std::time::Instant::now);
    }

    /// Whether the user waited for a job that just ended: it ended within
    /// [`WAITED_JOB`] of running, or too soon to report that it runs.
    pub(super) fn job_ended(&mut self, copy: bool, id: OperationId) -> bool {
        self.started
            .remove(&(copy, id))
            .is_none_or(|started| started.elapsed() <= self.waited)
    }
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
        if let Some(feed) = self.feed(pane) {
            return feed.focused().is_some();
        }
        let p = &self.panes[pane.index()];
        !p.parent_cursor
            && !p.search_view_waiting()
            && p.cursor.and_then(|id| p.entry_by_id(id)).is_some()
    }

    pub(crate) fn can_target_selection(&self, pane: PaneId) -> bool {
        if let Some(feed) = self.feed(pane) {
            return feed.selected_count > 0 || feed.focused().is_some();
        }
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
        // Search results fed to the panel cover its folder (the toolbar's New file).
        if self.feed(pane).is_some() {
            self.feed_mutation(pane, kind, send);
            return;
        }
        if self.comparison_refuses(mutation_action(kind)) {
            return;
        }
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
        // A background re-read shows no loading state: wait for it rather than
        // refuse silently (`run_deferred_actions`).
        let deferred = p.keeps_rows_in_background() && p.native_capabilities();
        if (!deferred && self.visible_directory(pane).is_none()) || p.search_view_waiting() {
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
        if deferred {
            self.defer_action(pane, DeferredKind::Operation(kind, files));
            return;
        }
        self.start_captured_operation(pane, kind, &files, send);
    }

    /// Run `kind` once the pane's background re-read ends; a later request
    /// replaces an earlier one, as a second request would cancel its preparation.
    fn defer_action(&mut self, pane: PaneId, kind: DeferredKind) {
        let folder = self.panes[pane.index()].display_path().to_owned();
        let old = self.operations.deferred[pane.index()].replace(DeferredAction { folder, kind });
        self.retired.push(Box::new(old));
    }

    /// Defer a disclosure clicked while a background re-read keeps the rows.
    pub(super) fn defer_disclosure(&mut self, pane: PaneId, identity: &str, recursive: bool) {
        self.defer_action(
            pane,
            DeferredKind::Disclose {
                identity: identity.to_owned(),
                recursive,
            },
        );
    }

    /// Run each pane's deferred action once its background re-read committed
    /// or failed, before another re-read of the pane can start. One whose pane
    /// left the folder or started a visible load is dropped.
    pub(super) fn run_deferred_actions(&mut self, send: &dyn Fn(Command) -> bool) {
        for pane in [PaneId::Left, PaneId::Right] {
            let p = &self.panes[pane.index()];
            if self.operations.deferred[pane.index()].is_none() || p.keeps_rows_in_background() {
                continue;
            }
            let Some(action) = self.operations.deferred[pane.index()].take() else {
                continue;
            };
            if p.loading || !p.native_capabilities() || p.display_path() != action.folder {
                self.retired.push(Box::new(action));
                continue;
            }
            match action.kind {
                DeferredKind::Operation(kind, files) => {
                    // The Engine prepares against the listing the UI acknowledged;
                    // the application acknowledges a commit only after this event.
                    if let Some(visible) = self.visible_directory(pane) {
                        let _ = send(Command::PaneCommitted(visible));
                    }
                    self.start_captured_operation(pane, kind, &files, send);
                    self.retired.push(Box::new(files));
                }
                DeferredKind::Disclose {
                    identity,
                    recursive,
                } => {
                    if recursive {
                        self.disclose_tree_recursive(pane, &identity, send);
                    } else {
                        self.disclose_tree(pane, &identity, send);
                    }
                }
            }
        }
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
        if self.answer_trash_fallback(token, true, send) {
            return;
        }
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
        let shown = |name: &str| midnight_core::visible_name(name).into_owned();
        let job_text = match (preview.kind, name.as_deref()) {
            (MutationKind::Rename, Some(new)) => Some(format!(
                "Renaming “{}” to “{}”…",
                shown(
                    &preview
                        .sample
                        .first()
                        .and_then(|item| item.path.file_name())
                        .map_or_else(String::new, |old| old.to_string_lossy().into_owned())
                ),
                shown(new)
            )),
            (MutationKind::CreateFile, Some(new)) => Some(format!("Creating “{}”…", shown(new))),
            (MutationKind::CreateDirectory, Some(new)) => {
                Some(format!("Creating folder “{}”…", shown(new)))
            }
            _ => None,
        };
        if send(Command::ConfirmOperation {
            id,
            new_name: name,
            replace_existing,
        }) {
            // A job queued behind another one may wait for it; never hold input
            // for that.
            if let Some(text) = job_text
                && !self.other_mutation_live(id)
            {
                self.operations.settle.job = Some(id);
                self.operations.settle.job_text = text;
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
        self.panes.iter().any(|pane| {
            pane.display_path() == directory
                && path_target::lists_file(&pane.entries, &pane.index, &directory, name)
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

    /// Whether some panel's input waits for a file job's change to become visible.
    #[cfg(test)]
    pub(crate) fn mutation_settling(&self) -> bool {
        self.mutation_settling_panes().contains(&true)
    }

    /// Per pane, whether its input waits for a file job's change to become
    /// visible (see `mutation_holds`).
    #[cfg(test)]
    pub(crate) fn mutation_settling_panes(&self) -> [bool; 2] {
        self.mutation_holds().map(|hold| hold != PaneHold::None)
    }

    /// Per pane, how its input waits for a file job's change to become
    /// visible: a confirmed Rename or creation in the folder it shows that
    /// still runs, or the re-read or tree refresh that shows a completed
    /// change. The other pane, the command line and the toolbars never wait.
    pub(crate) fn mutation_holds(&self) -> [PaneHold; 2] {
        let settle = &self.operations.settle;
        let job = settle
            .job
            .and_then(|id| {
                self.operations
                    .progress
                    .iter()
                    .find(|progress| progress.id == id)
            })
            .map(|progress| progress.source_directory.as_path());
        [PaneId::Left, PaneId::Right].map(|pane| {
            let p = &self.panes[pane.index()];
            if job.is_some_and(|folder| {
                p.native_capabilities()
                    && (p.display_path() == folder
                        || (p.tree_view().is_some() && folder.starts_with(p.display_path())))
            }) {
                PaneHold::Job
            } else if settle.reads[pane.index()].is_some() || settle.tree[pane.index()].is_some() {
                PaneHold::Reread
            } else {
                PaneHold::None
            }
        })
    }

    /// How a pane held by a running job describes it, such as
    /// `Renaming “a” to “b”…`; empty without one.
    pub(crate) fn mutation_job_text(&self) -> &str {
        let settle = &self.operations.settle;
        if settle.job.is_some() {
            &settle.job_text
        } else {
            ""
        }
    }

    /// Stop waiting for `id`'s job (every wait without one); its re-reads still
    /// hold input once it completes.
    pub(super) fn release_mutation_settle(&mut self, id: Option<OperationId>) {
        let settle = &mut self.operations.settle;
        match id {
            Some(id) if settle.job == Some(id) => {
                settle.job = None;
                settle.job_text.clear();
            }
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
        if self.answer_trash_fallback(token, false, send) {
            return;
        }
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
        self.retired
            .push(Box::new(mem::take(&mut self.operations.deferred)));
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
        self.retire_delete_decisions();
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
            OperationEvent::DeleteFailure(failure) => {
                // The failure prompt needs the user; the panel must not wait for it.
                self.release_mutation_settle(Some(failure.id));
                self.apply_delete_failure(failure);
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
                // A Delete frees space on its folder's volume while it runs and as it ends.
                if matches!(progress.kind, MutationKind::Delete(_))
                    && !matches!(
                        progress.state,
                        OperationState::Preparing
                            | OperationState::AwaitingConfirmation
                            | OperationState::Queued
                    )
                {
                    free_space::job_changed(
                        &mut self.space_refresh,
                        &self.volumes,
                        &self.panes,
                        [progress.source_directory.as_path()],
                    );
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
                if matches!(
                    progress.state,
                    OperationState::Queued
                        | OperationState::Running
                        | OperationState::CancelRequested
                ) {
                    self.operations.settle.job_running(false, progress.id);
                }
                if progress.state.is_terminal() {
                    self.retire_rename_decision(progress.id);
                    self.retire_delete_decision(progress.id);
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
                    // The entry the shown folder gained: the new name, or for a nested
                    // new folder path (F7 `alpha/beta`) its first new folder, `alpha`.
                    let reveal = progress.renamed_to.as_ref().and_then(|destination| {
                        if destination.parent() == Some(progress.source_directory.as_path()) {
                            return Some(destination.clone());
                        }
                        if progress.kind != MutationKind::CreateDirectory {
                            return None;
                        }
                        let below = destination.strip_prefix(&progress.source_directory).ok()?;
                        match below.components().next()? {
                            std::path::Component::Normal(name) => {
                                Some(progress.source_directory.join(name))
                            }
                            _ => None,
                        }
                    });
                    // A panel held since a Rename's or creation's confirmation
                    // waited for it, however long the job took.
                    let held = self.operations.settle.job == Some(progress.id);
                    self.release_mutation_settle(Some(progress.id));
                    // Only a completed change the user waited for keeps input
                    // waiting, for its re-reads; a failure is reported without
                    // holding the panel, and a long job's panels update in place.
                    let ended_soon = self.operations.settle.job_ended(false, progress.id);
                    let waited = ended_soon || held;
                    let hold = waited
                        && matches!(
                            progress.state,
                            OperationState::Completed | OperationState::CompletedWithErrors
                        );
                    for pane in [PaneId::Left, PaneId::Right] {
                        if self.panes[pane.index()].queue_native_tree_refresh(&mut self.retired) {
                            if hold {
                                // A tree shows entries of loaded subfolders too.
                                self.hold_tree_refresh(
                                    pane,
                                    std::slice::from_ref(&progress.source_directory),
                                    progress.renamed_to.clone(),
                                );
                            }
                            continue;
                        }
                        self.queue_operation_refresh(
                            pane,
                            &progress.source_directory,
                            reveal.clone(),
                            hold,
                        );
                    }
                    // The job's own open window shows how it ended, errors included;
                    // a notice would repeat it. Only a job without one gets a notice.
                    if !self
                        .operations
                        .progress_windows
                        .contains(crate::job_row_cache::Key::Mutation(progress.id))
                    {
                        self.report_operation_outcome(&progress);
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
    /// The notice for a finished file operation that no window of its own shows.
    fn report_operation_outcome(&mut self, progress: &OperationProgress) {
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
        // An action that waited for a re-read runs before the next one starts.
        self.run_deferred_actions(send);
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
            // The superseded re-read may reveal an earlier job's renamed or
            // created entry; that still applies unless this job reveals its own
            // or the user moved the cursor since.
            let superseded_target = p
                .pending
                .as_ref()
                .filter(|request| supersede && !request.cursor_touched)
                .and_then(|request| request.wanted_cursor.clone());
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
            let carried = superseded_target.filter(|target| {
                destination.is_none()
                    && p.pending
                        .as_ref()
                        .is_none_or(|request| request.wanted_cursor.as_ref() != Some(target))
            });
            p.preserve_viewport = destination.is_none() && carried.is_none();
            if let Some(destination) = &destination {
                // A same-directory refresh retains old rows. Release their old
                // caret so wanted_cursor can reveal the created/renamed entry.
                // Subsequent user input still wins through cursor_touched.
                p.cursor = p.index.by_path.get(destination).copied();
                p.parent_cursor = false;
                revealed.push(pane);
            }
            if let Some(request) = &mut p.pending {
                request.wanted_cursor = destination.or(carried);
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
