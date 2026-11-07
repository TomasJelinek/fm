//! Copy progress is bounded presentation state; all paths and bytes are processed by workers.
use super::*;

#[derive(Default)]
pub(crate) struct CopiesState {
    pub(crate) draft: Option<copy_dialog::CopyDraft>,
    pub(crate) progress: Vec<Arc<CopyProgress>>,
    pub(crate) conflicts: Vec<Arc<CopyConflict>>,
}

impl Presenter {
    pub(crate) fn start_copy(
        &mut self,
        clipboard: FileClipboard,
        destination: PathBuf,
        send: &dyn Fn(Command) -> bool,
    ) {
        if clipboard.intent == FileClipboardIntent::Move {
            self.retired.push(Box::new(clipboard));
            self.desktop.report(
                "Move unavailable",
                "Cut items remain at their source. Use Copy to duplicate them safely.".into(),
            );
            return;
        }
        let requested_roots = clipboard.paths.len() as u64;
        let target = destination.clone();
        self.submit_copy(
            destination,
            requested_roots,
            move |id| {
                Command::EnqueueCopy(CopyRequest {
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
                .report("Copy queue busy", "Try copying again.".into());
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
                "Copy queue full",
                "Wait for a copy to finish or cancel a queued job.".into(),
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
        let Some(id) = self.next_copy_id(send) else {
            // The builder owns captured clipboard paths or selection snapshots.
            // Destroy potentially large allocations on the reclamation worker.
            self.retired.push(Box::new(command));
            return false;
        };
        if !send(command(id)) {
            self.desktop
                .report("Copy queue busy", "Try copying again.".into());
            return false;
        }
        self.operations.next_id = id.0;
        self.copies.progress.push(Arc::new(CopyProgress {
            id,
            state: CopyOperationState::Queued,
            phase: CopyOperationPhase::Planning,
            requested_roots,
            files_completed: 0,
            directories_created: 0,
            skipped: 0,
            failed: 0,
            copied_bytes: 0,
            verified_bytes: 0,
            current_path: None,
            destination_directory: destination,
            message: String::new(),
            errors: vec![],
        }));
        true
    }

    pub(crate) fn decide_copy(
        &mut self,
        token: &str,
        choice: i32,
        name: String,
        send: &dyn Fn(Command) -> bool,
    ) {
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
            _ => return,
        };
        if send(Command::DecideCopy(CopyDecision {
            id: conflict.id,
            version: conflict.version,
            choice,
        })) {
            self.copies.conflicts.remove(index);
        }
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
        if self
            .copies
            .progress
            .iter()
            .any(|p| p.id == id && !p.state.is_terminal())
            && send(Command::CancelCopy(id))
        {
            self.copies.conflicts.retain(|conflict| conflict.id != id);
        }
    }

    pub(super) fn apply_copy_event(&mut self, event: Event) -> Option<Event> {
        let Event::Copy(event) = event else {
            return Some(event);
        };
        match event {
            CopyEvent::Conflict(conflict) => {
                if !self.operations.closing
                    && self
                        .copies
                        .progress
                        .iter()
                        .any(|p| p.id == conflict.id && !p.state.is_terminal())
                {
                    if let Some(old) = self
                        .copies
                        .conflicts
                        .iter_mut()
                        .find(|c| c.id == conflict.id)
                    {
                        if old.version < conflict.version {
                            *old = conflict;
                        }
                    } else {
                        self.copies.conflicts.push(conflict);
                    }
                }
            }
            CopyEvent::Progress(progress) => {
                let old = self
                    .copies
                    .progress
                    .iter_mut()
                    .find(|p| p.id == progress.id)?;
                if old.state.is_terminal() {
                    return None;
                }
                if progress.state.is_terminal() {
                    self.copies.conflicts.retain(|c| c.id != progress.id);
                    for pane in [PaneId::Left, PaneId::Right] {
                        if self.panes[pane.index()].display_path() == progress.destination_directory
                            && !self
                                .operations
                                .refresh
                                .iter()
                                .any(|(target, _, _)| *target == pane)
                        {
                            self.operations.refresh.push((
                                pane,
                                progress.destination_directory.clone(),
                                None,
                            ));
                        }
                    }
                    if !progress.errors.is_empty() {
                        self.desktop.report(
                            "Copy failed",
                            crate::error_presentation::issues(
                                &progress.errors,
                                progress.files_completed > 0,
                            ),
                        );
                    } else if !progress.message.is_empty() {
                        if matches!(
                            progress.state,
                            CopyOperationState::Failed | CopyOperationState::CompletedWithErrors
                        ) {
                            self.desktop.report("Copy failed", progress.message.clone());
                        } else {
                            self.desktop.inform("Copy", progress.message.clone());
                        }
                    }
                }
                *old = progress;
                self.trim_copy_history();
            }
        }
        None
    }

    pub(super) fn trim_copy_history(&mut self) {
        while self
            .copies
            .progress
            .iter()
            .filter(|p| p.state.is_terminal())
            .count()
            > 16
        {
            let index = self
                .copies
                .progress
                .iter()
                .position(|p| p.state.is_terminal())
                .unwrap();
            self.copies.progress.remove(index);
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
        p.start_copy(files, std::env::temp_dir(), &|_| {
            panic!("Move is unsupported")
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
