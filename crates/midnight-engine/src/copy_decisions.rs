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
        let mut timer = tokio::time::interval(Duration::from_millis(25));
        loop {
            if self.cancel.is_cancelled() {
                break;
            }
            tokio::select! {
                decision = self.decisions.recv() => match decision {
                    Some(decision) => {
                        let cancelling = matches!(decision.choice, CopyChoice::Cancel);
                        if !self.cancel.is_cancelled() && decision.id == id
                            && decision.version == version && accept(decision) {
                            // An accepted Cancel needs no further step, so teardown never
                            // waits for a disk slot to process it.
                            if cancelling {
                                self.cancel.cancel();
                            }
                            break;
                        }
                    }
                    None => { self.cancel.cancel(); break; }
                },
                _ = timer.tick() => {},
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
    !matches!(
        error.kind,
        io::ErrorKind::InvalidInput
            | io::ErrorKind::InvalidData
            | io::ErrorKind::Unsupported
            | io::ErrorKind::AlreadyExists
    )
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
        self.poll_grant(claim, endpoints, io, cancel)
            .map(|grant| grant == Grant::Granted)
    }
    fn poll_grant(
        &mut self,
        claim: &crate::copy_namespace::Claim,
        endpoints: Endpoints,
        io: Arc<midnight_work_budget::Permit>,
        cancel: &Cancellation,
    ) -> Result<Grant, FsError> {
        if !self.installed {
            // plan_union takes the immutable pins before refresh can fail.
            // Retry revalidates those same owners; it must not install again.
            self.installed = true;
            claim.plan_union(endpoints, io, cancel)?;
        } else {
            // Any revalidation clones close under the caller's sealed context.
            drop(endpoints);
            drop(io);
        }
        claim.admit(cancel)
    }
}
use crate::copy_namespace::Grant;

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
            let attempt = tokio::task::spawn_blocking(move || {
                let result = ownership::run(&mut preparation, |scope| {
                    let endpoints = if revalidate || !plan.installed {
                        scope.pin_endpoints(&cancel, &status)?
                    } else {
                        Vec::new()
                    };
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
                    let grant = plan.poll_grant(claim, endpoints, io, &cancel)?;
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
                    let changed =
                        snapshot(waiter.status).wait_reason != Some(TransferWaitReason::Resources);
                    if changed {
                        set_resource_wait(
                            waiter.status,
                            Some(waiter.throughput),
                            Some(TransferWaitReason::Resources),
                        );
                        publish(waiter.outbox, snapshot(waiter.status)).await;
                    }
                    // The grant wait keeps the namespace share and planned record, never
                    // the disk slot: granted peers and other disk users must progress.
                    waiter.io.release();
                    let watch = preparation
                        .resources_mut()
                        .namespace()
                        .map(crate::copy_namespace::Claim::watch);
                    let (since, structural) = match refused {
                        Grant::Busy(seen) => (Some(seen), false),
                        Grant::Conflict(seen) => (Some(seen), true),
                        Grant::Granted => (None, false),
                    };
                    match (watch, since) {
                        (Some(watch), Some(since)) => {
                            wait_for_namespace(&watch, since, structural, waiter.cancel).await;
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
        // Recorded preparation failures include owned-stage residuals. Retry
        // cannot erase those or accumulate further abandoned cleanup stages.
        // A quarantined claim never clears, so Retry could only repeat the refusal.
        if waiter.cancel.is_cancelled()
            || snapshot(waiter.status).failed != 0
            || !retryable_error(&error)
            || crate::copy_namespace::is_blocked_by_quarantine(&error)
            || version == u64::MAX
        {
            let progress = failed(waiter.status, error, waiter.cancel);
            return Err(Box::new(
                retire_input(preparation, progress, waiter.status).await,
            ));
        }
        version += 1;
        let progress = snapshot(waiter.status);
        let failure = CopyFailure {
            id: progress.id,
            version,
            source: (error.path != progress.destination_directory).then(|| error.path.clone()),
            destination: progress.destination_directory,
            error: error.clone(),
            stage: CopyErrorStage::Preparing,
            can_retry: true,
            can_skip: false,
        };
        let mut retry = false;
        let cancel = waiter.cancel.clone();
        waiter
            .wait(
                CopyEvent::Failure(Arc::new(failure)),
                |decision| match decision.choice {
                    CopyChoice::Retry => {
                        retry = true;
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
        if !retry || waiter.cancel.is_cancelled() {
            let mut progress = cancelled(waiter.status);
            progress.errors.push(error);
            return Err(Box::new(
                retire_input(preparation, progress, waiter.status).await,
            ));
        }
    }
}

#[cfg(test)]
#[path = "copy_namespace_preparation_tests.rs"]
mod namespace_preparation_tests;
