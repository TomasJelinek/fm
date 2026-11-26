//! Delete failures share the copy decision prompt (Retry, Skip, Skip all errors, Cancel
//! job, Decide later) while keeping their own job and version authority, as Rename
//! conflicts do. Tokens: `delete-error:{job}:{version}` for the prompt, and the job
//! row's own identity (`{job}` or `delete-job:{job}`) for Resolve.
//!
//! The Trash refusal that offers permanent deletion instead is one of these prompts too
//! (Permanently delete, Cancel job, Decide later). It used to be the main window's
//! embedded confirmation, which the Delete's own progress window covered: that window
//! is owned by the main window, so always above it, and centred over it. The job then
//! waited for an answer nobody could see, and Resolve had no window to bring up.
use super::*;
use std::collections::{HashMap, HashSet};

/// Delete failures that wait for the user.
#[derive(Default)]
pub(crate) struct DeleteDecisions {
    pub(crate) failures: Vec<Arc<DeleteFailure>>,
    versions: HashMap<OperationId, u64>,
    deferred: HashSet<(OperationId, u64)>,
    selected: Option<(OperationId, u64)>,
}

fn prompt_token(failure: &DeleteFailure) -> String {
    format!("delete-error:{}:{}", failure.id.0, failure.version)
}

fn job_token(token: &str, id: OperationId) -> bool {
    token == id.0.to_string() || token == format!("delete-job:{}", id.0)
}

/// Failures that wait for an answer and were not put off with Decide later.
fn pending(decisions: &DeleteDecisions) -> impl Iterator<Item = (OperationId, u64)> + '_ {
    decisions
        .failures
        .iter()
        .map(|f| (f.id, f.version))
        .filter(|key| !decisions.deferred.contains(key))
}

impl Presenter {
    pub(super) fn apply_delete_failure(&mut self, failure: Arc<DeleteFailure>) {
        let decisions = &mut self.operations.delete_decisions;
        if self.operations.closing
            || !self.operations.progress.iter().any(|p| {
                p.id == failure.id
                    && matches!(p.kind, MutationKind::Delete(_))
                    && !p.state.is_terminal()
                    && p.state != OperationState::CancelRequested
            })
            || decisions
                .versions
                .get(&failure.id)
                .is_some_and(|v| *v >= failure.version)
        {
            return;
        }
        decisions.versions.insert(failure.id, failure.version);
        decisions.failures.retain(|f| f.id != failure.id);
        decisions.deferred.retain(|(id, _)| *id != failure.id);
        if decisions.selected.is_some_and(|(id, _)| id == failure.id) {
            decisions.selected = Some((failure.id, failure.version));
        }
        decisions.failures.push(failure);
    }

    /// The Delete failure the shared prompt shows: one chosen with Resolve, otherwise
    /// the first waiting one while no copy or Rename decision is shown.
    pub(crate) fn active_delete_decision(&self) -> Option<(OperationId, u64)> {
        self.chosen_delete_decision().or_else(|| {
            (self.active_copy_decision().is_none() && self.active_rename_decision().is_none())
                .then(|| pending(&self.operations.delete_decisions).next())
                .flatten()
        })
    }

    /// A waiting Delete failure chosen with Resolve: it goes before a Rename decision
    /// that only waits for its turn (`active_rename_decision`).
    pub(super) fn chosen_delete_decision(&self) -> Option<(OperationId, u64)> {
        let decisions = &self.operations.delete_decisions;
        decisions
            .selected
            .filter(|key| pending(decisions).any(|item| item == *key))
    }

    /// Resolve chose another job's decision, which is then shown instead.
    pub(super) fn unchoose_delete_decision(&mut self) {
        self.operations.delete_decisions.selected = None;
    }

    pub(crate) fn active_delete_failure(&self) -> Option<&Arc<DeleteFailure>> {
        let (id, version) = self.active_delete_decision()?;
        self.operations
            .delete_decisions
            .failures
            .iter()
            .find(|f| f.id == id && f.version == version)
    }

    /// The pending confirmation of a prepared operation, shown in the main window.
    pub(crate) fn operation_form(&self) -> Option<(&OperationPreview, String)> {
        let preview: &OperationPreview = self.operations.preview.as_ref()?;
        (self.operations.pending == Some(preview.id)).then(|| (preview, preview.id.0.to_string()))
    }

    pub(super) fn retire_delete_decision(&mut self, id: OperationId) {
        let decisions = &mut self.operations.delete_decisions;
        decisions.failures.retain(|f| f.id != id);
        decisions.versions.remove(&id);
        decisions.deferred.retain(|(job, _)| *job != id);
        if decisions.selected.is_some_and(|(job, _)| job == id) {
            decisions.selected = None;
        }
    }

    pub(super) fn retire_delete_decisions(&mut self) {
        self.operations.delete_decisions = DeleteDecisions::default();
    }

    /// Decide later (the prompt's X or Escape).
    pub(super) fn defer_delete_decision(&mut self, token: &str) -> bool {
        let decisions = &mut self.operations.delete_decisions;
        let Some(key) = decisions
            .failures
            .iter()
            .find(|f| token == prompt_token(f))
            .map(|f| (f.id, f.version))
        else {
            return false;
        };
        decisions.deferred.insert(key);
        decisions.selected = None;
        true
    }

    /// Resolve on the job's row shows its waiting failure again.
    pub(super) fn resolve_delete_decision(&mut self, token: &str) -> bool {
        let decisions = &mut self.operations.delete_decisions;
        let Some(key) = decisions
            .failures
            .iter()
            .find(|f| job_token(token, f.id))
            .map(|f| (f.id, f.version))
        else {
            return false;
        };
        decisions.deferred.remove(&key);
        decisions.selected = Some(key);
        // Shown next, also before a Rename decision chosen earlier.
        self.operations.rename_selected = None;
        true
    }

    /// The shared prompt's buttons: 8 Retry, 0 Skip, 9 Skip all errors, 3 Cancel job; for
    /// a Trash refusal, 10 Permanently delete and 3 Cancel job.
    pub(super) fn decide_delete(
        &mut self,
        token: &str,
        choice: i32,
        send: &dyn Fn(Command) -> bool,
    ) {
        let decisions = &self.operations.delete_decisions;
        let Some(index) = decisions
            .failures
            .iter()
            .position(|f| token == prompt_token(f))
        else {
            return;
        };
        let failure = &decisions.failures[index];
        if failure.permanent_fallback.is_some() {
            // The items stay where they are unless Permanently delete is chosen.
            let confirm = match choice {
                10 => true,
                3 => false,
                _ => return,
            };
            let (id, version) = (failure.id, failure.version);
            if send(Command::AnswerTrashFallback {
                id,
                version,
                confirm,
            }) {
                let decisions = &mut self.operations.delete_decisions;
                decisions.failures.remove(index);
                decisions.selected = None;
                if !confirm
                    && let Some(progress) = self.operations.progress.iter_mut().find(|p| p.id == id)
                {
                    Arc::make_mut(progress).state = OperationState::CancelRequested;
                }
            } else {
                self.report_control_busy();
            }
            return;
        }
        let choice = match choice {
            8 if failure.can_retry => CopyChoice::Retry,
            0 if failure.can_skip => CopyChoice::Skip,
            9 if failure.can_skip => CopyChoice::SkipAllErrors,
            3 => CopyChoice::Cancel,
            _ => return,
        };
        let id = failure.id;
        let cancelling = matches!(choice, CopyChoice::Cancel);
        if send(Command::DecideDelete(CopyDecision {
            id,
            version: failure.version,
            choice,
        })) {
            let decisions = &mut self.operations.delete_decisions;
            decisions.failures.remove(index);
            decisions.selected = None;
            if cancelling
                && let Some(progress) = self.operations.progress.iter_mut().find(|p| p.id == id)
            {
                Arc::make_mut(progress).state = OperationState::CancelRequested;
            }
        } else {
            self.report_control_busy();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn presenter() -> Presenter {
        let mut p = Presenter::new("left".into(), "right".into());
        p.operations.progress.push(Arc::new(OperationProgress {
            pause_requested: false,
            paused: false,
            discovery: Default::default(),
            id: OperationId(5),
            kind: MutationKind::Delete(DeleteMode::Permanent),
            state: OperationState::Running,
            total: 2,
            completed: 0,
            failed: 0,
            processed_entries: 108,
            throughput: Arc::default(),
            current_path: None,
            message: String::new(),
            errors: vec![],
            source_directory: "left".into(),
            renamed_to: None,
        }));
        p
    }

    fn failure(version: u64, can_retry: bool) -> Arc<DeleteFailure> {
        Arc::new(DeleteFailure {
            permanent_fallback: None,
            id: OperationId(5),
            version,
            mode: DeleteMode::Permanent,
            error: FsError::from_io(
                "mutate entry",
                std::path::Path::new("left/tree/locked-folder"),
                std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "This folder is owned by another user, who does not allow deleting \
                     anything in it",
                ),
            ),
            can_retry,
            can_skip: true,
        })
    }

    fn sent(p: &mut Presenter, token: &str, choice: i32) -> Vec<Command> {
        let sent = RefCell::new(Vec::new());
        p.decide_copy(token, choice, String::new(), &|command| {
            sent.borrow_mut().push(command);
            true
        });
        sent.into_inner()
    }

    #[test]
    fn delete_failures_use_the_shared_prompt_buttons_with_their_own_command() {
        let mut p = presenter();
        p.apply(Event::Operation(OperationEvent::DeleteFailure(failure(
            1, true,
        ))));
        assert_eq!(p.active_delete_decision(), Some((OperationId(5), 1)));
        // A copy or Rename token never answers a Delete failure.
        assert!(sent(&mut p, "copy-error:5:1", 8).is_empty());
        assert!(sent(&mut p, "rename:5:1", 0).is_empty());
        // Unknown buttons and stale versions are ignored.
        assert!(sent(&mut p, "delete-error:5:1", 4).is_empty());
        assert!(sent(&mut p, "delete-error:5:0", 8).is_empty());
        let retry = sent(&mut p, "delete-error:5:1", 8);
        assert!(matches!(
            retry.as_slice(),
            [Command::DecideDelete(CopyDecision {
                id: OperationId(5),
                version: 1,
                choice: CopyChoice::Retry
            })]
        ));
        assert_eq!(p.active_delete_decision(), None);
        // A failure that cannot be retried offers Skip and Skip all, never Retry.
        p.apply(Event::Operation(OperationEvent::DeleteFailure(failure(
            2, false,
        ))));
        assert!(sent(&mut p, "delete-error:5:2", 8).is_empty());
        assert!(matches!(
            sent(&mut p, "delete-error:5:2", 9).as_slice(),
            [Command::DecideDelete(CopyDecision {
                version: 2,
                choice: CopyChoice::SkipAllErrors,
                ..
            })]
        ));
        p.apply(Event::Operation(OperationEvent::DeleteFailure(failure(
            3, true,
        ))));
        assert!(matches!(
            sent(&mut p, "delete-error:5:3", 0).as_slice(),
            [Command::DecideDelete(CopyDecision {
                choice: CopyChoice::Skip,
                ..
            })]
        ));
        // A replayed or older version is dropped.
        p.apply(Event::Operation(OperationEvent::DeleteFailure(failure(
            3, true,
        ))));
        assert_eq!(p.active_delete_decision(), None);
    }

    #[test]
    fn a_busy_queue_keeps_the_delete_failure_waiting() {
        let mut p = presenter();
        p.apply(Event::Operation(OperationEvent::DeleteFailure(failure(
            1, true,
        ))));
        p.decide_copy("delete-error:5:1", 0, String::new(), &|_| false);
        assert_eq!(p.active_delete_decision(), Some((OperationId(5), 1)));
    }

    #[test]
    fn a_delete_failure_can_be_deferred_resolved_cancelled_and_retired() {
        let mut p = presenter();
        p.apply(Event::Operation(OperationEvent::DeleteFailure(failure(
            1, true,
        ))));
        p.defer_copy_decision("delete-error:5:1");
        assert_eq!(p.active_delete_decision(), None);
        // The job row's Resolve shows it again.
        p.resolve_copy_decision("5");
        assert_eq!(p.active_delete_decision(), Some((OperationId(5), 1)));
        let cancel = sent(&mut p, "delete-error:5:1", 3);
        assert!(matches!(
            cancel.as_slice(),
            [Command::DecideDelete(CopyDecision {
                choice: CopyChoice::Cancel,
                ..
            })]
        ));
        assert_eq!(
            p.operations.progress[0].state,
            OperationState::CancelRequested
        );
        // A cancelling job asks nothing more.
        p.apply(Event::Operation(OperationEvent::DeleteFailure(failure(
            2, true,
        ))));
        assert_eq!(p.active_delete_decision(), None);
        // A terminal job retires what it still had.
        let mut done = (*p.operations.progress[0]).clone();
        done.state = OperationState::Cancelled;
        p.apply(Event::Operation(OperationEvent::Progress(Arc::new(done))));
        assert!(p.operations.delete_decisions.failures.is_empty());
        assert!(p.operations.delete_decisions.versions.is_empty());
    }

    #[test]
    fn a_delete_failure_waits_while_a_copy_decision_is_shown() {
        let mut p = presenter();
        p.copies.progress.push(Arc::new(CopyProgress::queued(
            OperationId(9),
            "right".into(),
            1,
            TransferIntent::Copy,
        )));
        p.copies.failures.push(Arc::new(CopyFailure {
            id: OperationId(9),
            version: 1,
            error: FsError::from_io(
                "copy",
                std::path::Path::new("right/file"),
                std::io::ErrorKind::PermissionDenied.into(),
            ),
            source: None,
            destination: "right/file".into(),
            stage: CopyErrorStage::Writing,
            can_retry: true,
            can_skip: true,
        }));
        p.apply(Event::Operation(OperationEvent::DeleteFailure(failure(
            1, true,
        ))));
        assert_eq!(p.active_delete_decision(), None);
        // Resolve on the Delete job shows its failure first.
        p.resolve_copy_decision("delete-job:5");
        assert_eq!(p.active_delete_decision(), Some((OperationId(5), 1)));
        p.request_operation_shutdown(&|_| true);
        assert_eq!(p.active_delete_decision(), None);
        assert!(p.operations.delete_decisions.failures.is_empty());
    }

    fn trash_refusal(version: u64) -> Arc<DeleteFailure> {
        let path = std::path::Path::new("left/kept.png");
        Arc::new(DeleteFailure {
            id: OperationId(5),
            version,
            mode: DeleteMode::Trash,
            error: FsError::trash_unavailable(path, "Windows Recycle Bin is disabled"),
            can_retry: false,
            can_skip: false,
            permanent_fallback: Some(OperationPreview {
                id: OperationId(5),
                kind: MutationKind::Delete(DeleteMode::Permanent),
                files: 1,
                folders: 0,
                file_bytes: Some(1),
                sample: vec![MutationItem {
                    path: path.into(),
                    kind: EntryKind::File,
                    metadata: Default::default(),
                }],
            }),
        })
    }

    /// The Trash refusal is a decision of the shared prompt, never the main window's
    /// confirmation form: only Permanently delete (10) and Cancel job (3) answer it.
    #[test]
    fn a_trash_refusal_is_answered_in_the_shared_prompt_and_can_wait() {
        let mut p = presenter();
        p.apply(Event::Operation(OperationEvent::DeleteFailure(
            trash_refusal(1),
        )));
        assert_eq!(p.active_delete_decision(), Some((OperationId(5), 1)));
        assert!(p.operation_form().is_none(), "not the main window's form");
        for choice in [8, 0, 9, 4] {
            assert!(sent(&mut p, "delete-error:5:1", choice).is_empty());
        }
        // Decide later keeps it waiting; Resolve shows it again.
        p.defer_copy_decision("delete-error:5:1");
        assert_eq!(p.active_delete_decision(), None);
        p.resolve_copy_decision("5");
        assert!(matches!(
            sent(&mut p, "delete-error:5:1", 10).as_slice(),
            [Command::AnswerTrashFallback {
                id: OperationId(5),
                version: 1,
                confirm: true
            }]
        ));
        assert_eq!(p.active_delete_decision(), None);
        p.apply(Event::Operation(OperationEvent::DeleteFailure(
            trash_refusal(2),
        )));
        assert!(matches!(
            sent(&mut p, "delete-error:5:2", 3).as_slice(),
            [Command::AnswerTrashFallback {
                version: 2,
                confirm: false,
                ..
            }]
        ));
        assert_eq!(
            p.operations.progress[0].state,
            OperationState::CancelRequested
        );
    }
}
