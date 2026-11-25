//! Delete jobs pause on each failure until the user decides, then go on with every
//! remaining entry and target. Retry repeats the failed step. Skip leaves that entry
//! (the provider then keeps the folders that contain it, and the target says so once,
//! not as more errors). Skip all errors skips the job's later failures without asking.
//! Cancel ends the job and keeps what is left. The last target failing as a whole in a
//! way Retry cannot change is only reported: no decision is left to make.
use super::*;
use std::{cell::RefCell, sync::mpsc};

/// Decision routing only: the actor indexes it by job, like Rename's conflicts.
#[derive(Default)]
pub(crate) struct DecisionState {
    next: AtomicU64,
    pending: Mutex<Option<Pending>>,
}

struct Pending {
    version: u64,
    fallback: bool,
    sender: mpsc::SyncSender<Answer>,
}

enum Answer {
    Error(CopyChoice),
    Fallback(bool),
}

impl DecisionState {
    pub(crate) fn resolve(&self, decision: CopyDecision) {
        self.deliver(decision.version, Answer::Error(decision.choice));
    }

    pub(crate) fn answer_fallback(&self, version: u64, confirm: bool) {
        self.deliver(version, Answer::Fallback(confirm));
    }

    fn deliver(&self, version: u64, answer: Answer) {
        let pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(pending) = pending.as_ref()
            && pending.version == version
            && pending.fallback == matches!(answer, Answer::Fallback(_))
        {
            let _ = pending.sender.try_send(answer);
        }
    }

    /// Publishes `failure` and waits for its decision: Cancel once the job is cancelled
    /// or the failure cannot be published.
    fn ask(&self, mut failure: DeleteFailure, outbox: &Outbox, cancel: &Cancellation) -> Answer {
        let Ok(previous) = self
            .next
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_add(1))
        else {
            return Answer::Error(CopyChoice::Cancel);
        };
        failure.version = previous + 1;
        let (sender, receiver) = mpsc::sync_channel(1);
        *self.pending.lock().unwrap_or_else(|e| e.into_inner()) = Some(Pending {
            version: failure.version,
            fallback: failure.permanent_fallback.is_some(),
            sender,
        });
        let sent = outbox.blocking_send(
            Event::Operation(OperationEvent::DeleteFailure(Arc::new(failure))),
            cancel,
        );
        let choice = if sent {
            loop {
                if cancel.is_cancelled() {
                    break Answer::Error(CopyChoice::Cancel);
                }
                match receiver.recv_timeout(Duration::from_millis(25)) {
                    Ok(choice) => break choice,
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(_) => break Answer::Error(CopyChoice::Cancel),
                }
            }
        } else {
            Answer::Error(CopyChoice::Cancel)
        };
        *self.pending.lock().unwrap_or_else(|e| e.into_inner()) = None;
        choice
    }
}

/// Retrying cannot change these: the target or its request is no longer valid.
fn retryable(error: &FsError) -> bool {
    !matches!(
        error.kind,
        io::ErrorKind::InvalidInput
            | io::ErrorKind::InvalidData
            | io::ErrorKind::Unsupported
            | io::ErrorKind::AlreadyExists
    )
}

/// The job's decisions and the errors they leave.
struct Decider<'a> {
    job: &'a Job,
    outbox: &'a Outbox,
    mode: DeleteMode,
    skip_all: bool,
    /// Failures inside the current target that were skipped.
    skipped_inside: bool,
}

/// What one decision did.
enum Decided {
    Retry,
    Skip,
    Cancel,
}

impl Decider<'_> {
    /// Asks about `error` (or skips it after Skip all errors). A skipped or cancelling
    /// failure is one error of the job; a retried one is not.
    fn decide(
        &mut self,
        progress: &RefCell<&mut OperationProgress>,
        error: FsError,
        can_retry: bool,
    ) -> Decided {
        let id = progress.borrow().id;
        let choice = if self.skip_all {
            CopyChoice::Skip
        } else {
            {
                // While the job waits, its window shows the counts so far and why.
                let mut shown = progress.borrow_mut();
                shown.message = error.to_string();
                self.job.store_progress(&mut shown);
            }
            let choice = self.job.delete_decisions.ask(
                DeleteFailure {
                    id,
                    version: 0,
                    mode: self.mode,
                    permanent_fallback: None,
                    error: error.clone(),
                    can_retry,
                    can_skip: true,
                },
                self.outbox,
                &self.job.cancel,
            );
            progress.borrow_mut().message.clear();
            match choice {
                Answer::Error(choice) => choice,
                Answer::Fallback(_) => CopyChoice::Cancel,
            }
        };
        let decided = match choice {
            CopyChoice::Retry if can_retry => return Decided::Retry,
            CopyChoice::Cancel => {
                self.job.cancel.cancel();
                Decided::Cancel
            }
            CopyChoice::SkipAllErrors => {
                self.skip_all = true;
                Decided::Skip
            }
            // Skip, and any choice this failure did not offer.
            _ => Decided::Skip,
        };
        let mut progress = progress.borrow_mut();
        progress.failed += 1;
        if progress.errors.len() < 16 {
            progress.errors.push(error);
        }
        self.job.store_progress(&mut progress);
        decided
    }
}

pub(super) fn execute(job: Job, outbox: Outbox) -> OperationProgress {
    let mut progress = job.progress();
    let MutationKind::Delete(mode) = progress.kind else {
        unreachable!("only Delete jobs run here");
    };
    job.discover_deletion(&mut progress);
    // Completed targets that reported an issue (a residue); each is also one error.
    let mut residues = 0;
    // Targets kept only because they still contain skipped items.
    let mut kept = Vec::new();
    let mut decided = false;
    let mut decider = Decider {
        job: &job,
        outbox: &outbox,
        mode,
        skip_all: false,
        skipped_inside: false,
    };
    let mut original = 0..job.targets.len();
    let mut unavailable: Vec<usize> = Vec::new();
    let mut confirmed = Vec::new().into_iter();
    let mut first_refusal = None;
    let mut recycled_before_fallback = None;
    'targets: loop {
        if !job.cancel.wait_if_paused() {
            break;
        }
        let Some(index) = original.next().or_else(|| confirmed.next()) else {
            let Some(error) = first_refusal.take() else {
                break;
            };
            let mut preview = OperationPreview {
                id: progress.id,
                kind: MutationKind::Delete(DeleteMode::Permanent),
                files: 0,
                folders: 0,
                file_bytes: Some(0),
                sample: Vec::new(),
            };
            for &index in &unavailable {
                Job::include_item(&mut preview, job.targets[index].item());
            }
            let has_folders = preview.folders > 0;
            let choice = job.delete_decisions.ask(
                DeleteFailure {
                    id: progress.id,
                    version: 0,
                    mode: DeleteMode::Trash,
                    error,
                    can_retry: false,
                    can_skip: false,
                    permanent_fallback: Some(preview),
                },
                &outbox,
                &job.cancel,
            );
            if !matches!(choice, Answer::Fallback(true)) {
                job.cancel.cancel();
                break;
            }
            if has_folders {
                progress.discovery.unavailable("Trash discovery counted selected items only; permanent folder deletion visits their contents.");
            }
            decider.mode = DeleteMode::Permanent;
            decider.skip_all = false;
            recycled_before_fallback = Some(progress.completed);
            progress.kind = MutationKind::Delete(DeleteMode::Permanent);
            job.store_progress(&mut progress);
            confirmed = std::mem::take(&mut unavailable).into_iter();
            continue;
        };
        let last = original.is_empty() && confirmed.as_slice().is_empty();
        let mode = decider.mode;
        let target = &job.targets[index];
        let processed_before = progress.processed_entries;
        let path = target.item().path.clone();
        progress.current_path = Some(path.clone());
        job.store_progress(&mut progress);
        decider.skipped_inside = false;
        loop {
            let shared = RefCell::new(&mut progress);
            let result = target.delete_with_decisions(
                mode,
                &job.cancel,
                &mut |removed| {
                    let mut progress = shared.borrow_mut();
                    progress.processed_entries += 1;
                    if progress.discovery.state == DiscoveryState::Complete
                        && progress
                            .discovery
                            .entries()
                            .is_some_and(|total| progress.processed_entries > total)
                    {
                        progress.discovery.unavailable(
                            "The source changed after discovery; total progress is unavailable.",
                        );
                    }
                    progress.current_path = Some(removed.to_owned());
                    // Publish counters to the timer even if the next native call stalls.
                    // Only the timer builds and sends bounded display snapshots.
                    let mut status = job.status.lock().unwrap_or_else(|e| e.into_inner());
                    status.processed_entries = progress.processed_entries;
                    status.discovery = progress.discovery.clone();
                    status.current_path = progress.current_path.clone();
                },
                &mut |failure| {
                    decided = true;
                    let inside = failure.error.path != path;
                    match decider.decide(&shared, failure.error, failure.can_retry) {
                        Decided::Retry => DeleteFailureChoice::Retry,
                        Decided::Skip => {
                            decider.skipped_inside |= inside;
                            DeleteFailureChoice::Skip
                        }
                        Decided::Cancel => DeleteFailureChoice::Cancel,
                    }
                },
            );
            match result {
                Ok(done) => {
                    if done.skipped == 0 {
                        progress.completed += 1;
                    } else if decider.skipped_inside {
                        kept.push(format!(
                            "{} was not deleted because it still contains skipped items.",
                            path.display()
                        ));
                    }
                    // The change completed; its issue is counted without undoing it.
                    if let Some(residue) = done.residue {
                        let done = if mode == DeleteMode::Trash {
                            format!("Moved {} to Trash", path.display())
                        } else if done.skipped == 0 {
                            format!("Deleted {}", path.display())
                        } else {
                            format!("Deleted all but the skipped items of {}", path.display())
                        };
                        residues += 1;
                        progress.failed += 1;
                        if progress.errors.len() < 16 {
                            progress.errors.push(residue_issue(&done, &residue));
                        }
                    }
                    break;
                }
                // Native cancellation can follow already-reported deletions.
                Err(error)
                    if error.kind == io::ErrorKind::Interrupted && job.cancel.is_cancelled() =>
                {
                    break 'targets;
                }
                // Keep the prepared identity. A partial removal or a nested error
                // cannot enter the permanent-delete confirmation.
                Err(error)
                    if mode == DeleteMode::Trash
                        && error.is_trash_unavailable()
                        && error.path == path
                        && progress.processed_entries == processed_before
                        && !decider.skipped_inside =>
                {
                    unavailable.push(index);
                    first_refusal.get_or_insert(error);
                    break;
                }
                // Nothing to retry and nothing left after it: no decision to make.
                Err(error) if last && !retryable(&error) => {
                    progress.failed += 1;
                    if progress.errors.len() < 16 {
                        progress.errors.push(error);
                    }
                    break;
                }
                // A failure that ended this target: offered for the whole target.
                Err(error) => {
                    decided = true;
                    let can_retry = retryable(&error);
                    match decider.decide(&RefCell::new(&mut progress), error, can_retry) {
                        Decided::Retry => continue,
                        Decided::Skip => break,
                        Decided::Cancel => break 'targets,
                    }
                }
            }
        }
        job.store_progress(&mut progress);
    }
    if progress.completed == progress.total
        && progress.failed == residues
        && progress.discovery.state == DiscoveryState::Complete
        && progress.discovery.scope == DiscoveryScope::Recursive
        && progress.discovery.entries() != Some(progress.processed_entries)
    {
        progress.discovery.unavailable(
            "Completed work differs from the metadata discovery snapshot; the source changed.",
        );
    }
    progress.state = if progress.completed == progress.total {
        // Every target completed; residues alone are its errors.
        if progress.failed == 0 {
            OperationState::Completed
        } else {
            OperationState::CompletedWithErrors
        }
    } else if job.cancel.is_cancelled() {
        OperationState::Cancelled
    } else if progress.completed > 0 || progress.processed_entries > 0 || decided {
        OperationState::CompletedWithErrors
    } else {
        OperationState::Failed
    };
    progress.pause_requested = false;
    progress.paused = false;
    let mut lines = Vec::new();
    if let Some(recycled) = recycled_before_fallback {
        lines.push(format!("Moved to Trash: {recycled}"));
        lines.push(format!(
            "Permanently deleted: {}",
            progress.completed - recycled
        ));
        if progress.completed < progress.total {
            lines.push(format!(
                "Remaining selected items: {}",
                progress.total - progress.completed
            ));
        }
    }
    lines.extend(progress.errors.iter().map(ToString::to_string));
    lines.extend(kept);
    // A residue undoes nothing; an incomplete target can leave partial work.
    if progress.processed_entries > 0
        && (progress.state == OperationState::Cancelled || progress.failed > residues)
    {
        lines.push("Some changes were already completed and have not been undone.".into());
    }
    progress.message = lines.join("\n");
    progress
}
