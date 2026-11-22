//! Copy progress is bounded presentation state; all paths and bytes are processed by workers.
use super::*;

#[derive(Default)]
pub(crate) struct CopiesState {
    pub(super) selection: copy_selection::CopySelections,
    pub(crate) draft: Option<copy_dialog::CopyDraft>,
    /// The draft's typed destination while the backend checks that it is a folder.
    pub(crate) destination_check: Option<copy_dialog::DestinationCheck>,
    /// What the draft's destination does with links, and the user's link choice.
    pub(crate) links: Option<copy_dialog::DraftLinks>,
    pub(super) tree_pending: Option<tree_copy::Pending>,
    pub(crate) progress: Vec<Arc<CopyProgress>>,
    // Only live worker snapshots, excluding optimistic submission/drop rows.
    backend_observed: HashSet<OperationId>,
    pub(crate) conflicts: Vec<Arc<CopyConflict>>,
    pub(crate) failures: Vec<Arc<CopyFailure>>,
    pub(super) decision_versions: HashMap<OperationId, u64>,
    pub(super) deferred_decisions: HashSet<(OperationId, u64)>,
    pub(super) selected_decision: Option<(OperationId, u64)>,
}

impl Presenter {
    pub(crate) fn copy_control_available(&self, id: OperationId) -> bool {
        self.copies.backend_observed.contains(&id)
    }

    pub(crate) fn start_copy(
        &mut self,
        clipboard: FileClipboard,
        destination: PathBuf,
        send: &dyn Fn(Command) -> bool,
    ) {
        if clipboard.intent == FileClipboardIntent::Move {
            // The folder of the first item stands for the selection's volume.
            let source = clipboard
                .paths
                .first()
                .and_then(|path| path.parent())
                .map(Path::to_owned);
            self.open_clipboard_move(clipboard, destination);
            self.inspect_copy_draft_links(source, send);
            return;
        }
        let requested_roots = clipboard.paths.len() as u64;
        let target = destination.clone();
        let verification = self.transfer_verification();
        self.submit_copy(
            destination,
            requested_roots,
            move |id| {
                Command::EnqueueCopy(CopyRequest {
                    start_policy: TransferStartPolicy::Immediate,
                    verification,
                    id,
                    clipboard,
                    destination: target,
                })
            },
            send,
        );
    }

    fn next_copy_id(&mut self, send: &dyn Fn(Command) -> bool) -> Option<OperationId> {
        if self.operations.closing {
            return None;
        }
        if !self.flush_admitted_drops_before_copy(send) {
            self.desktop
                .report("Copy/Move queue busy", "Try the transfer again.".into());
            return None;
        }
        if self
            .copies
            .progress
            .iter()
            .filter(|p| !p.state.is_terminal())
            .count()
            >= 32
        {
            self.desktop.report(
                "Copy/Move queue full",
                "Wait for a transfer to finish or cancel a queued job.".into(),
            );
            return None;
        }
        self.operations.next_id.checked_add(1).map(OperationId)
    }

    pub(super) fn submit_copy(
        &mut self,
        destination: PathBuf,
        requested_roots: u64,
        command: impl FnOnce(OperationId) -> Command + Send + 'static,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        self.submit_transfer(
            destination,
            requested_roots,
            TransferIntent::Copy,
            command,
            send,
        )
    }
    pub(super) fn submit_transfer(
        &mut self,
        destination: PathBuf,
        requested_roots: u64,
        intent: TransferIntent,
        command: impl FnOnce(OperationId) -> Command + Send + 'static,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let Some(id) = self.next_copy_id(send) else {
            // The builder owns captured clipboard paths or selection snapshots.
            // Destroy potentially large allocations on the reclamation worker.
            self.retired.push(Box::new(command));
            return false;
        };
        if !send(command(id)) {
            self.desktop
                .report("Copy/Move queue busy", "Try the transfer again.".into());
            return false;
        }
        self.operations.next_id = id.0;
        self.track_copy_selection(id);
        let mut progress = CopyProgress::queued(id, destination, requested_roots, intent);
        progress.verification = self.transfer_verification();
        self.copies.progress.push(Arc::new(progress));
        true
    }

    pub(crate) fn decide_copy(
        &mut self,
        token: &str,
        choice: i32,
        name: String,
        send: &dyn Fn(Command) -> bool,
    ) {
        if token.starts_with("rename:") {
            self.decide_rename(token, choice, send);
            return;
        }
        if token.starts_with("copy-error:") {
            self.decide_copy_failure(token, choice, send);
            return;
        }
        if token.starts_with("delete-error:") {
            self.decide_delete(token, choice, send);
            return;
        }
        let Some(index) = self
            .copies
            .conflicts
            .iter()
            .position(|c| token == format!("copy:{}:{}", c.id.0, c.version))
        else {
            return;
        };
        let conflict = &self.copies.conflicts[index];
        let choice = match choice {
            0 => CopyChoice::Skip,
            1 => CopyChoice::SkipAll,
            2 if !name.is_empty() => CopyChoice::Rename(
                conflict
                    .destination
                    .file_name()
                    .filter(|native| native.to_string_lossy() == name)
                    .map_or_else(|| name.into(), |native| native.to_owned()),
            ),
            3 => CopyChoice::Cancel,
            4 if conflict.can_replace => CopyChoice::ReplaceRecoverable { all: false },
            5 if conflict.can_replace => CopyChoice::ReplaceRecoverable { all: true },
            6 if conflict.can_merge => CopyChoice::Merge { all: false },
            7 if conflict.can_merge => CopyChoice::Merge { all: true },
            _ => return,
        };
        let id = conflict.id;
        let cancelling = matches!(choice, CopyChoice::Cancel);
        if send(Command::DecideCopy(CopyDecision {
            id: conflict.id,
            version: conflict.version,
            choice,
        })) {
            self.copies.conflicts.remove(index);
            self.copies.selected_decision = None;
            if cancelling {
                self.mark_copy_cancel_requested(id);
            }
        } else {
            self.report_control_busy();
        }
    }

    /// A refused job control leaves the decision or job unchanged and its job
    /// window keeps the retryable choice; say why on the status line without a
    /// popup over the modeless job windows.
    pub(super) fn report_control_busy(&mut self) {
        self.desktop.status = "Job queue is busy. Try the action again.".into();
    }

    pub(crate) fn cancel_copy(&mut self, token: &str, send: &dyn Fn(Command) -> bool) {
        let Some(id) = token
            .strip_prefix("copy-job:")
            .and_then(|value| value.parse().ok())
            .map(OperationId)
        else {
            return;
        };
        if self.cancel_pending_drop(id, send) {
            return;
        }
        if !self
            .copies
            .progress
            .iter()
            .any(|p| p.id == id && !p.state.is_terminal())
        {
            return;
        }
        if send(Command::CancelCopy(id)) {
            self.mark_copy_cancel_requested(id);
            self.copies.conflicts.retain(|conflict| conflict.id != id);
            self.copies.failures.retain(|failure| failure.id != id);
            self.copies.deferred_decisions.retain(|(job, _)| *job != id);
        } else {
            self.report_control_busy();
        }
    }

    pub(super) fn mark_copy_cancel_requested(&mut self, id: OperationId) {
        if let Some(progress) = self
            .copies
            .progress
            .iter_mut()
            .find(|progress| progress.id == id && !progress.state.is_terminal())
        {
            let progress = Arc::make_mut(progress);
            progress.state = CopyOperationState::CancelRequested;
            // Cancellation owns presentation until the terminal cleanup event.
            progress.paused = false;
            progress.pause_requested = false;
            progress.cancel_confirmation_ready = None;
            progress.wait_reason = None;
            self.copies.backend_observed.remove(&id);
        }
    }

    pub(super) fn apply_copy_event(&mut self, event: Event) -> Option<Event> {
        let Event::Copy(event) = event else {
            return Some(event);
        };
        match event {
            CopyEvent::RootsCompleted { id, sources } => self.complete_copy_selection(id, sources),
            CopyEvent::Conflict(conflict) => {
                if self.accept_copy_decision(conflict.id, conflict.version) {
                    self.copies.conflicts.push(conflict);
                }
            }
            CopyEvent::Failure(failure) => {
                if self.accept_copy_decision(failure.id, failure.version) {
                    self.copies.failures.push(failure);
                }
            }
            CopyEvent::Progress(progress) => {
                if progress.state.is_terminal() {
                    self.forget_copy_selection(progress.id);
                }
                let old = self
                    .copies
                    .progress
                    .iter_mut()
                    .find(|p| p.id == progress.id)?;
                if old.state.is_terminal()
                    || (old.state == CopyOperationState::CancelRequested
                        && !progress.state.is_terminal())
                {
                    return None;
                }
                if progress.state.is_terminal()
                    || progress.state == CopyOperationState::CancelRequested
                {
                    self.copies.backend_observed.remove(&progress.id);
                    self.copies.conflicts.retain(|c| c.id != progress.id);
                    self.copies
                        .failures
                        .retain(|failure| failure.id != progress.id);
                    self.copies
                        .deferred_decisions
                        .retain(|(id, _)| *id != progress.id);
                } else {
                    self.copies.backend_observed.insert(progress.id);
                }
                if !progress.state.is_terminal() {
                    self.operations.settle.job_running(true, progress.id);
                }
                let mut tree_holds = Vec::new();
                let mut affected = Vec::new();
                // A Copy leaves its sources as they were; only its destination
                // shows a change, while a Move changes both.
                let copy = progress.intent == TransferIntent::Copy;
                if progress.state.is_terminal() {
                    // A completed transfer the user waited for holds input until
                    // its panels show it; a long one's panels update in place.
                    let waited = self.operations.settle.job_ended(true, progress.id);
                    let hold = waited
                        && matches!(
                            progress.state,
                            CopyOperationState::Completed | CopyOperationState::CompletedWithErrors
                        );
                    for pane in [PaneId::Left, PaneId::Right] {
                        if self.panes[pane.index()].queue_native_tree_refresh(&mut self.retired) {
                            if hold {
                                tree_holds.push(pane);
                            }
                            continue;
                        }
                        let shown = &self.panes[pane.index()];
                        let directory = shown.display_path();
                        let destination = directory == progress.destination_directory;
                        if destination
                            || progress
                                .source_directories
                                .iter()
                                .any(|path| path == directory)
                        {
                            self.operations.queue_refresh(
                                pane,
                                shown,
                                directory,
                                None,
                                hold && (destination || !copy),
                                &mut self.retired,
                            );
                        }
                    }
                    // The job's own open window shows how it ended, errors included;
                    // a notice would repeat it. Only a job without one gets a notice.
                    if !self
                        .operations
                        .progress_windows
                        .contains(crate::job_row_cache::Key::Copy(progress.id))
                    {
                        report_copy_outcome(&mut self.desktop, &progress);
                    }
                }
                if !tree_holds.is_empty() {
                    if !copy {
                        affected.extend(progress.source_directories.iter().cloned());
                    }
                    affected.push(progress.destination_directory.clone());
                }
                *old = progress;
                for pane in tree_holds {
                    self.hold_tree_refresh(pane, &affected, None);
                }
                self.trim_copy_history();
            }
        }
        None
    }

    pub(super) fn trim_copy_history(&mut self) {
        self.copies.selection.retain_live(&self.copies.progress);
        self.copies.decision_versions.retain(|id, _| {
            self.copies
                .progress
                .iter()
                .any(|p| p.id == *id && !p.state.is_terminal())
        });
    }
}

/// The notice for a finished Copy or Move that no window of its own shows: its
/// errors, or else its cancellation. A completion needs none.
fn report_copy_outcome(desktop: &mut super::desktop::DesktopState, progress: &CopyProgress) {
    let copy = progress.intent == TransferIntent::Copy;
    let failed = if copy {
        "Copy failed"
    } else {
        "Move needs attention"
    };
    if !progress.errors.is_empty() {
        desktop.report(
            failed,
            crate::error_presentation::issues(&progress.errors, progress.files_completed > 0),
        );
    } else if !progress.message.is_empty() {
        if matches!(
            progress.state,
            CopyOperationState::Failed | CopyOperationState::CompletedWithErrors
        ) {
            desktop.report(failed, progress.message.clone());
        } else if progress.state != CopyOperationState::Completed {
            desktop.inform(if copy { "Copy" } else { "Move" }, progress.message.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn clipboard() -> FileClipboard {
        FileClipboard {
            paths: vec![std::env::temp_dir().join("文档.txt")],
            intent: FileClipboardIntent::Copy,
        }
    }
    #[test]
    fn refused_cancel_and_pause_report_a_busy_queue_instead_of_doing_nothing() {
        let mut p = Presenter::new("left".into(), "right".into());
        p.start_copy(clipboard(), std::env::temp_dir(), &|_| true);
        let token = format!("copy-job:{}", p.copies.progress[0].id.0);
        p.desktop.status.clear();
        p.cancel_copy(&token, &|_| false);
        assert!(
            p.desktop.status.contains("busy"),
            "refused Cancel is reported"
        );
        assert!(p.desktop.notice.is_none(), "no popup over the job windows");
        assert_ne!(
            p.copies.progress[0].state,
            CopyOperationState::CancelRequested
        );
        p.desktop.status.clear();
        p.set_job_paused(&token, true, &|_| false);
        assert!(
            p.desktop.status.contains("busy"),
            "refused Pause is reported"
        );
        assert!(!p.copies.progress[0].pause_requested);
    }

    #[test]
    fn replacement_and_merge_choices_require_advertised_capability() {
        for (choice, can_replace, can_merge, accepted) in [
            (4, true, false, true),
            (5, true, false, true),
            (6, false, true, true),
            (7, false, true, true),
            (4, false, true, false),
            (5, false, false, false),
            (6, true, false, false),
            (7, false, false, false),
        ] {
            let mut p = Presenter::new("left".into(), "right".into());
            p.start_copy(clipboard(), std::env::temp_dir(), &|_| true);
            let id = p.copies.progress[0].id;
            let item = MutationItem {
                path: std::env::temp_dir().join("existing"),
                kind: EntryKind::File,
                metadata: EntryMetadata::default(),
            };
            p.apply(Event::Copy(CopyEvent::Conflict(Arc::new(CopyConflict {
                id,
                version: 2,
                source: item.clone(),
                target: item.clone(),
                destination: item.path,
                can_replace,
                can_merge,
            }))));
            let commands = RefCell::new(Vec::new());
            p.decide_copy(
                &format!("copy:{}:2", id.0),
                choice,
                String::new(),
                &|command| {
                    commands.borrow_mut().push(command);
                    true
                },
            );
            assert_eq!(commands.borrow().len(), usize::from(accepted));
            if accepted {
                let commands = commands.borrow();
                let Command::DecideCopy(decision) = &commands[0] else {
                    panic!("expected decision");
                };
                assert_eq!(decision.version, 2);
                assert_eq!(decision.id, id);
                assert!(match (&decision.choice, choice) {
                    (CopyChoice::ReplaceRecoverable { all }, 4 | 5) => *all == (choice == 5),
                    (CopyChoice::Merge { all }, 6 | 7) => *all == (choice == 7),
                    _ => false,
                });
            }
        }
    }
    pub(super) fn assert_unchanged_display_preserves_native_name(name: std::ffi::OsString) {
        let mut p = Presenter::new("left".into(), "right".into());
        p.start_copy(clipboard(), std::env::temp_dir(), &|_| true);
        let id = p.copies.progress[0].id;
        let destination = std::env::temp_dir().join(&name);
        let item = MutationItem {
            path: destination.clone(),
            kind: EntryKind::File,
            metadata: EntryMetadata::default(),
        };
        p.apply(Event::Copy(CopyEvent::Conflict(Arc::new(CopyConflict {
            id,
            version: 1,
            source: item.clone(),
            target: item,
            destination,
            can_replace: false,
            can_merge: false,
        }))));
        let called = std::cell::Cell::new(false);
        p.decide_copy(&format!("copy:{}:1", id.0), 2, name.to_string_lossy().into_owned(), &|command| {
            assert!(matches!(command, Command::DecideCopy(CopyDecision {choice: CopyChoice::Rename(actual), ..}) if actual == name));
            called.set(true);
            true
        });
        assert!(called.get());
    }
    #[test]
    fn paste_queues_captured_paths_and_destination_and_close_cancels_the_job() {
        let mut p = Presenter::new("left".into(), "right".into());
        let commands = RefCell::new(Vec::new());
        let send = |command| {
            commands.borrow_mut().push(command);
            true
        };
        let destination = std::env::temp_dir().join("captured destination");
        p.start_copy(clipboard(), destination.clone(), &send);
        let id = match &commands.borrow()[0] {
            Command::EnqueueCopy(request) => {
                assert_eq!(request.destination, destination);
                assert_eq!(request.clipboard.paths, clipboard().paths);
                request.id
            }
            command => panic!("unexpected {command:?}"),
        };
        assert!(p.has_live_operations());
        p.request_operation_shutdown(&send);
        assert!(
            commands
                .borrow()
                .iter()
                .any(|command| matches!(command, Command::CancelCopy(actual) if *actual == id))
        );
        assert!(p.copies.conflicts.is_empty());
    }
    #[test]
    fn move_intent_and_rejected_queue_do_not_create_phantom_jobs() {
        let mut p = Presenter::new("left".into(), "right".into());
        let mut files = clipboard();
        files.intent = FileClipboardIntent::Move;
        // Opening the confirmation asks only what its destination does with links.
        p.start_copy(files, std::env::temp_dir(), &|command| {
            assert!(
                matches!(command, Command::InspectTransferLinks(_)),
                "Move requires confirmation: {command:?}"
            );
            true
        });
        assert!(!p.has_live_operations());
        p.start_copy(clipboard(), std::env::temp_dir(), &|_| false);
        assert!(!p.has_live_operations());
    }
    #[test]
    fn rejected_copy_payload_is_reclaimed_by_the_backend_not_the_ui_callback() {
        use std::sync::atomic::{AtomicBool, Ordering};
        struct Payload(Arc<AtomicBool>);
        impl Drop for Payload {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }
        let dropped = Arc::new(AtomicBool::new(false));
        let payload = Payload(dropped.clone());
        let mut p = Presenter::new("left".into(), "right".into());
        p.operations.closing = true;
        assert!(!p.submit_copy(
            std::env::temp_dir(),
            1,
            move |_| {
                drop(payload);
                panic!("a rejected copy must not build a command")
            },
            &|_| panic!("no admission")
        ));
        assert!(
            !dropped.load(Ordering::Acquire),
            "payload destruction must leave the UI thread"
        );
        let retired = p.take_retired();
        std::thread::spawn(move || drop(retired)).join().unwrap();
        assert!(dropped.load(Ordering::Acquire));
    }
    #[test]
    fn conflict_decisions_require_the_exact_job_version_and_cannot_replay() {
        let mut p = Presenter::new("left".into(), "right".into());
        p.start_copy(clipboard(), std::env::temp_dir(), &|_| true);
        let id = p.copies.progress[0].id;
        let item = MutationItem {
            path: std::env::temp_dir().join("existing.txt"),
            kind: EntryKind::File,
            metadata: EntryMetadata::default(),
        };
        p.apply(Event::Copy(CopyEvent::Conflict(Arc::new(CopyConflict {
            id,
            version: 7,
            source: item.clone(),
            target: item,
            destination: std::env::temp_dir().join("existing.txt"),
            can_replace: false,
            can_merge: false,
        }))));
        let commands = RefCell::new(Vec::new());
        let send = |command| {
            commands.borrow_mut().push(command);
            true
        };
        p.decide_copy(&format!("copy:{}:6", id.0), 0, "".into(), &send);
        assert!(commands.borrow().is_empty());
        let token = format!("copy:{}:7", id.0);
        p.decide_copy(&token, 2, "different.txt".into(), &send);
        p.decide_copy(&token, 0, "".into(), &send);
        assert_eq!(commands.borrow().len(), 1);
        assert!(
            matches!(&commands.borrow()[0], Command::DecideCopy(CopyDecision {id: actual, version:7,
            choice: CopyChoice::Rename(name)}) if *actual == id && name == "different.txt")
        );
        assert!(p.copies.conflicts.is_empty());
    }
}

#[cfg(all(test, unix))]
#[path = "copies_unix_tests.rs"]
mod native_name_tests;
#[cfg(all(test, windows))]
#[path = "copies_windows_tests.rs"]
mod native_name_tests;
