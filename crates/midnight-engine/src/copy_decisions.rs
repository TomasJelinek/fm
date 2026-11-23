//! Async decisions keep native authority and the namespace share without occupying a
//! blocking worker or the disk slot.
use super::*;

pub(super) struct DecisionWaiter<'a> {
    pub status: &'a Status,
    pub cancel: &'a Cancellation,
    pub waiting: &'a AtomicU64,
    pub decisions: &'a mut mpsc::Receiver<CopyDecision>,
    pub outbox: &'a CopyOutbox,
    pub throughput: &'a Arc<Mutex<Tracker>>,
    /// The job's disk slot: released for the user's decision and readmitted through the
    /// FIFO gate before the next native step (`ready_for_io`).
    pub io: &'a midnight_work_budget::Permit,
}

impl DecisionWaiter<'_> {
    pub async fn wait(&mut self, event: CopyEvent, mut accept: impl FnMut(CopyDecision) -> bool) {
        let (id, version) = match &event {
            CopyEvent::Conflict(conflict) => (conflict.id, conflict.version),
            CopyEvent::Failure(failure) => (failure.id, failure.version),
            _ => return,
        };
        if self.cancel.is_cancelled() {
            return;
        }
        // No native call is outstanding here; queued disk users must not wait for a user.
        self.io.release();
        set_state(self.status, CopyOperationState::WaitingForDecision);
        sample_throughput(self.status, self.throughput);
        self.waiting.store(version, Ordering::Release);
        publish(self.outbox, snapshot(self.status)).await;
        self.outbox.send(Event::Copy(event)).await;
        let cancel = self.cancel;
        loop {
            if !wait_for_pause(cancel, self.status, self.outbox, self.throughput, None).await {
                break;
            }
            // Decisions and newly requested controls both wake this wait.
            tokio::select! {
                decision = self.decisions.recv() => match decision {
                    Some(decision) => {
                        let cancelling = matches!(decision.choice, CopyChoice::Cancel);
                        if !cancel.is_cancelled() && decision.id == id
                            && decision.version == version && accept(decision) {
                            // An accepted Cancel needs no further step, so teardown never
                            // waits for a disk slot to process it.
                            if cancelling {
                                cancel.cancel();
                            }
                            break;
                        }
                    }
                    None => { cancel.cancel(); break; }
                },
                () = self.outbox.controls.until(|| cancel.is_cancelled() || cancel.pause_requested()) => {},
            }
        }
        self.waiting.store(0, Ordering::Release);
        if self.cancel.is_cancelled() {
            set_state(self.status, CopyOperationState::CancelRequested);
        } else {
            set_state(self.status, CopyOperationState::Running);
            sample_throughput(self.status, self.throughput);
        }
    }
}

pub(super) fn retryable_error(error: &FsError) -> bool {
    // A quarantined claim never clears, so Retry could only repeat the refusal.
    !matches!(
        error.kind,
        io::ErrorKind::InvalidInput
            | io::ErrorKind::InvalidData
            | io::ErrorKind::Unsupported
            | io::ErrorKind::AlreadyExists
    ) && !crate::copy_namespace::is_blocked_by_quarantine(error)
}

#[derive(Default)]
struct NamespacePlan {
    installed: bool,
}
type Endpoints = Vec<(
    TransferNamespaceAccess,
    Option<Arc<dyn TransferNamespacePin>>,
)>;
impl NamespacePlan {
    #[cfg(test)]
    fn poll(
        &mut self,
        claim: &crate::copy_namespace::Claim,
        endpoints: Endpoints,
        io: Arc<midnight_work_budget::Permit>,
        cancel: &Cancellation,
    ) -> Result<bool, FsError> {
        self.poll_grant(claim, endpoints, None, io, cancel)
            .map(|grant| grant == Grant::Granted)
    }
    /// `folder_reads`: the endpoints are folders, which the job reads where true, rather
    /// than a (source, destination) pair per selected item.
    fn poll_grant(
        &mut self,
        claim: &crate::copy_namespace::Claim,
        endpoints: Endpoints,
        folder_reads: Option<Vec<bool>>,
        io: Arc<midnight_work_budget::Permit>,
        cancel: &Cancellation,
    ) -> Result<Grant, FsError> {
        if !self.installed {
            // Planning takes the immutable pins before refresh can fail. Retry
            // revalidates those same owners; it must not install again. The endpoints
            // are (source, destination) pairs (`Preparation::pin_endpoints`).
            self.installed = true;
            match folder_reads {
                Some(reads) => claim.plan_folders(endpoints, reads, io, cancel)?,
                None => claim.plan_transfer(endpoints, io, cancel)?,
            }
        } else {
            // Any revalidation clones close under the caller's sealed context.
            drop(endpoints);
            drop(io);
        }
        claim.admit(cancel)
    }
}
use crate::copy_namespace::{Grant, Hold};

enum PreparationStep {
    Waiting(Owned<preparation::Preparation>, NamespacePlan, Grant),
    Failed(Owned<preparation::Preparation>, NamespacePlan, FsError),
    Ready(Owned<worker::Job>),
    Unconfirmed,
}

pub(super) async fn prepare_job(
    input: Owned<CopyInput>,
    service: QualifiedTransferInstallation,
    waiter: &mut DecisionWaiter<'_>,
) -> Result<Owned<worker::Job>, Box<CopyProgress>> {
    let tracker = waiter.throughput.clone();
    let created = tokio::task::spawn_blocking(move || {
        ownership::map(input, |input| {
            let mut preparation = preparation::Preparation::qualified(input, service);
            preparation.attach_tracker(tracker);
            preparation
        })
    })
    .await;
    let mut preparation = match created {
        Ok(Ok(preparation)) => preparation,
        Ok(Err(_)) => return Err(Box::new(retirement_unconfirmed(waiter.status))),
        Err(error) => {
            std::mem::forget(error);
            return Err(Box::new(retirement_unconfirmed(waiter.status)));
        }
    };
    let mut plan = NamespacePlan::default();
    let mut version = 0u64;
    // Selected items left out by Skip, each recorded as skipped and as one error.
    let mut skipped_roots = 0u64;
    let mut skipped_root: Option<std::path::PathBuf> = None;
    let mut skip_all = std::collections::HashSet::new();
    loop {
        set_state(waiter.status, CopyOperationState::Preparing);
        publish(waiter.outbox, snapshot(waiter.status)).await;
        // Revalidate preparation endpoints once for each explicit attempt. A
        // blocked grant subsequently refreshes through its retained registry.
        let mut revalidate = true;
        let error = loop {
            if !ready_for_io(
                waiter.io,
                waiter.cancel,
                waiter.status,
                waiter.outbox,
                waiter.throughput,
            )
            .await
            {
                set_resource_wait(waiter.status, Some(waiter.throughput), None);
                let progress =
                    retire_input(preparation, cancelled(waiter.status), waiter.status).await;
                return Err(Box::new(progress));
            }
            let cancel = waiter.cancel.clone();
            let status = waiter.status.clone();
            let tracker = waiter.throughput.clone();
            let skipped = skipped_root.take();
            let attempt = tokio::task::spawn_blocking(move || {
                let result = ownership::run(&mut preparation, |scope| {
                    if let Some(root) = &skipped {
                        scope.skip_root(root);
                    }
                    let endpoints = if revalidate || !plan.installed {
                        scope.pin_endpoints(&cancel, &status)?
                    } else {
                        Vec::new()
                    };
                    let reads = scope.folder_reads();
                    let resources = scope.resources_mut();
                    let io = resources.real_io.as_ref().cloned().ok_or_else(|| {
                        error(
                            Path::new(""),
                            io::ErrorKind::InvalidInput,
                            "Preparation has no admitted I/O owner.",
                        )
                    })?;
                    let claim = resources.namespace().ok_or_else(|| {
                        error(
                            Path::new(""),
                            io::ErrorKind::InvalidInput,
                            "Preparation has no namespace claim.",
                        )
                    })?;
                    let grant = plan.poll_grant(claim, endpoints, reads, io, &cancel)?;
                    if grant != Grant::Granted {
                        return Ok(Err(grant));
                    }
                    set_resource_wait(&status, Some(&tracker), None);
                    let mut job = ownership::Armed::new(scope.attempt(cancel, status)?);
                    job.set_decision_version(version);
                    Ok(Ok(job))
                });
                match result {
                    Ok(Ok(Ok(job))) => {
                        match ownership::handoff_job(preparation, job.into_inner()) {
                            Ok(job) => PreparationStep::Ready(job),
                            Err(_) => PreparationStep::Unconfirmed,
                        }
                    }
                    Ok(Ok(Err(refused))) => PreparationStep::Waiting(preparation, plan, refused),
                    Ok(Err(error)) => PreparationStep::Failed(preparation, plan, error),
                    Err(_) => {
                        // run has already armed/quarantined every native owner.
                        drop(preparation);
                        PreparationStep::Unconfirmed
                    }
                }
            })
            .await;
            match attempt {
                Ok(PreparationStep::Ready(job)) => return Ok(job),
                Ok(PreparationStep::Waiting(returned, retained_plan, refused)) => {
                    preparation = returned;
                    plan = retained_plan;
                    revalidate = false;
                    // Refused as a whole: a footprint is unknown (so one job runs alone),
                    // it would read what another job changes, or an older waiting job
                    // shares an item.
                    let (since, hold) = match refused {
                        Grant::Conflict(seen, hold) => (Some(seen), hold),
                        Grant::Granted => (None, Hold::Overlap),
                    };
                    let reason = Some(hold.wait_reason());
                    if snapshot(waiter.status).wait_reason != reason {
                        set_resource_wait(waiter.status, Some(waiter.throughput), reason);
                        publish(waiter.outbox, snapshot(waiter.status)).await;
                    }
                    // The grant wait keeps the namespace share and planned record, never
                    // the disk slot: granted peers and other disk users must progress.
                    waiter.io.release();
                    let watch = preparation
                        .resources_mut()
                        .namespace()
                        .map(crate::copy_namespace::Claim::watch);
                    match (watch, since) {
                        (Some(watch), Some(since)) => {
                            wait_for_namespace(&watch, since, waiter.cancel, waiter.outbox).await;
                        }
                        _ => tokio::time::sleep(NAMESPACE_RETRY_SPACING).await,
                    }
                }
                Ok(PreparationStep::Failed(returned, retained_plan, error)) => {
                    preparation = returned;
                    plan = retained_plan;
                    set_resource_wait(waiter.status, Some(waiter.throughput), None);
                    break error;
                }
                Ok(PreparationStep::Unconfirmed) => {
                    return Err(Box::new(retirement_unconfirmed(waiter.status)));
                }
                Err(error) => {
                    std::mem::forget(error);
                    return Err(Box::new(retirement_unconfirmed(waiter.status)));
                }
            }
        };
        // Recorded preparation failures other than skipped items are owned-stage
        // residuals. Retry cannot erase those or accumulate further abandoned
        // cleanup stages, and Skip would leave them behind.
        let residuals = snapshot(waiter.status).failed != skipped_roots;
        // A selected item that is gone was removed from the source before the job reached it.
        let error = if preparation.skippable_root(&error.path) {
            vanished_source(
                error.clone(),
                Some(&error.path),
                snapshot(waiter.status).intent,
            )
        } else {
            plain_namespace_error(error)
        };
        let can_retry = retryable_error(&error);
        // One of several selected items that failed alone can be left out; the
        // others are still copied.
        let can_skip = !residuals && preparation.skippable_root(&error.path);
        if waiter.cancel.is_cancelled()
            || residuals
            || !(can_retry || can_skip)
            || version == u64::MAX
        {
            let progress = failed(waiter.status, error, waiter.cancel);
            return Err(Box::new(
                retire_input(preparation, progress, waiter.status).await,
            ));
        }
        let scope = (CopyErrorStage::Preparing, error.kind, error.operation);
        let choice = if can_skip && skip_all.contains(&scope) {
            CopyChoice::Skip
        } else {
            version += 1;
            let progress = snapshot(waiter.status);
            let failure = CopyFailure {
                id: progress.id,
                version,
                source: (error.path != progress.destination_directory).then(|| error.path.clone()),
                destination: progress.destination_directory,
                error: error.clone(),
                stage: CopyErrorStage::Preparing,
                can_retry,
                can_skip,
            };
            let mut choice = CopyChoice::Cancel;
            let cancel = waiter.cancel.clone();
            waiter
                .wait(
                    CopyEvent::Failure(Arc::new(failure)),
                    |decision| match decision.choice {
                        CopyChoice::Retry if can_retry => {
                            choice = decision.choice;
                            true
                        }
                        CopyChoice::Skip | CopyChoice::SkipAllErrors if can_skip => {
                            choice = decision.choice;
                            true
                        }
                        CopyChoice::Cancel => {
                            cancel.cancel();
                            true
                        }
                        _ => false,
                    },
                )
                .await;
            choice
        };
        if waiter.cancel.is_cancelled() || matches!(choice, CopyChoice::Cancel) {
            let mut progress = cancelled(waiter.status);
            progress.errors.push(error);
            return Err(Box::new(
                retire_input(preparation, progress, waiter.status).await,
            ));
        }
        if matches!(choice, CopyChoice::Skip | CopyChoice::SkipAllErrors) {
            if matches!(choice, CopyChoice::SkipAllErrors) {
                skip_all.insert(scope);
            }
            let last = preparation.is_last_root(&error.path);
            skipped_root = Some(error.path.clone());
            skipped_roots += 1;
            {
                let mut status = waiter.status.lock().unwrap_or_else(|e| e.into_inner());
                status.skipped += 1;
                retain_error(&mut status, error, false);
            }
            if last {
                // Every selected item was left out: the job has nothing left to do.
                let progress = all_roots_skipped(waiter.status);
                return Err(Box::new(
                    retire_input(preparation, progress, waiter.status).await,
                ));
            }
        }
    }
}

/// The end of a job whose every selected item Skip left out before anything was copied:
/// each skip is one of its errors, so it ends Failed, as a job that completed nothing
/// does, with every item retained where it was.
fn all_roots_skipped(status: &Status) -> CopyProgress {
    let mut progress = snapshot(status);
    progress.wait_reason = None;
    progress.state = if progress.failed == 0 {
        CopyOperationState::Completed
    } else {
        CopyOperationState::Failed
    };
    let mut lines: Vec<String> = progress.errors.iter().map(ToString::to_string).collect();
    lines.push(skipped_line(progress.skipped));
    progress.message = lines.join("\n");
    progress
}

/// The line that counts the entries and subtrees Skip left out, each where it was.
pub(super) fn skipped_line(skipped: u64) -> String {
    match skipped {
        1 => "1 entry or subtree skipped; source data retained.".into(),
        count => format!("{count} entries or subtrees skipped; source data retained."),
    }
}

/// An item that disappeared from the source while the job copied or moved it (or before
/// the job reached it) is said so in words, not with the system's text for a missing
/// file ("The system cannot find the file specified (os error 2)"). Only a missing
/// `source`, or something below it, is meant; a missing destination keeps its error.
pub(super) fn vanished_source(
    mut error: FsError,
    source: Option<&Path>,
    intent: TransferIntent,
) -> FsError {
    if error.kind == io::ErrorKind::NotFound
        && source.is_some_and(|source| error.path.starts_with(source))
    {
        error.message = match intent {
            TransferIntent::Move => "removed from the source while moving",
            TransferIntent::Copy => "removed from the source while copying",
        }
        .into();
    }
    error
}

/// The namespace host's own errors name its internals (announcements, lanes, aliases),
/// and an adapter that wraps one keeps them in its message. The user reads what happened
/// and what to do instead; the original text follows as technical detail.
pub(super) fn plain_namespace_error(mut error: FsError) -> FsError {
    let prefix = format!("{}: ", crate::copy_namespace::OPERATION);
    if error.kind == io::ErrorKind::Interrupted
        || !(error.operation == crate::copy_namespace::OPERATION
            || error.message.starts_with(&prefix))
    {
        return error;
    }
    let plain = if error.kind == io::ErrorKind::Unsupported {
        "This change needs the job to run alone: the names in this folder can no longer be \
         checked exactly, because its name rules or a folder above it changed while the job \
         ran. Skip the item and copy or move it again after this job has finished."
    } else {
        "The job could not confirm that the folders it uses are still as it found them, so \
         it changed nothing more here. Skip the item, or cancel the job."
    };
    error.message = format!("{plain}\n\nTechnical details: {}", error.message);
    error
}

#[cfg(test)]
#[path = "copy_namespace_preparation_tests.rs"]
mod namespace_preparation_tests;

#[cfg(test)]
mod plain_namespace_error_tests {
    use super::*;

    /// The owner's report showed the host's jargon as wrapped by the Windows adapter.
    #[test]
    fn namespace_jargon_becomes_a_sentence_with_the_original_as_detail() {
        let internal = "transfer namespace: An unknown announcement requires the already \
                        exclusive opaque lane. ()";
        let wrapped = FsError::from_io(
            "copy file",
            Path::new("target/folder"),
            io::Error::new(io::ErrorKind::Unsupported, internal),
        );
        let plain = plain_namespace_error(wrapped.clone());
        let (said, details) = plain.message.split_once("\n\nTechnical details: ").unwrap();
        assert!(
            said.starts_with("This change needs the job to run alone"),
            "{said}"
        );
        for jargon in ["announcement", "lane", "namespace", "opaque"] {
            assert!(!said.contains(jargon), "{said}");
        }
        assert_eq!(details, internal);
        assert_eq!(
            (plain.operation, plain.kind, plain.path),
            (wrapped.operation, wrapped.kind, wrapped.path)
        );
        let own = plain_namespace_error(FsError::from_io(
            crate::copy_namespace::OPERATION,
            Path::new(""),
            io::Error::other("Native namespace teardown is unconfirmed."),
        ));
        assert!(
            own.message.starts_with("The job could not confirm"),
            "{}",
            own.message
        );
        // Other errors and a cancellation keep their own words.
        for error in [
            FsError::from_io("copy file", Path::new("a"), io::Error::other("Disk full")),
            FsError::from_io(
                crate::copy_namespace::OPERATION,
                Path::new(""),
                io::Error::new(io::ErrorKind::Interrupted, "Namespace admission cancelled."),
            ),
        ] {
            assert_eq!(plain_namespace_error(error.clone()).message, error.message);
        }
    }
}

#[cfg(test)]
mod skipped_line_tests {
    #[test]
    fn one_skipped_item_is_counted_in_the_singular() {
        let one = "1 entry or subtree skipped; source data retained.";
        assert_eq!(super::skipped_line(1), one);
        let three = "3 entries or subtrees skipped; source data retained.";
        assert_eq!(super::skipped_line(3), three);
    }
}
