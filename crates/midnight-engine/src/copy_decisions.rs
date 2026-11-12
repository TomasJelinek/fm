//! Async decision waiting never holds a blocking worker or an I/O permit.
use super::*;

pub(super) struct DecisionWaiter<'a> {
    pub status: &'a Status,
    pub cancel: &'a Cancellation,
    pub waiting: &'a AtomicU64,
    pub decisions: &'a mut mpsc::Receiver<CopyDecision>,
    pub outbox: &'a Outbox,
    pub throughput: &'a Mutex<Tracker>,
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
                        if !self.cancel.is_cancelled() && decision.id == id
                            && decision.version == version && accept(decision) { break; }
                    }
                    None => { self.cancel.cancel(); break; }
                },
                _ = timer.tick() => {},
            }
        }
        self.waiting.store(0, Ordering::Release);
        if !self.cancel.is_cancelled() {
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

pub(super) async fn prepare_job(
    input: Owned<(CopyInput, Arc<dyn TransferService>)>,
    io_budget: &Arc<Semaphore>,
    waiter: &mut DecisionWaiter<'_>,
) -> Result<Owned<worker::Job>, Box<CopyProgress>> {
    let mut preparation = ownership::map(input, |(input, service)| {
        preparation::Preparation::new(input, service)
    });
    let mut version = 0u64;
    loop {
        set_state(waiter.status, CopyOperationState::Preparing);
        publish(waiter.outbox, snapshot(waiter.status)).await;
        if !wait_for_pause(
            waiter.cancel,
            waiter.status,
            waiter.outbox,
            waiter.throughput,
        )
        .await
        {
            drop_preparation(preparation).await;
            return Err(Box::new(cancelled(waiter.status)));
        }
        let Some(permit) = acquire(io_budget, waiter.cancel).await else {
            drop_preparation(preparation).await;
            return Err(Box::new(cancelled(waiter.status)));
        };
        let cancel = waiter.cancel.clone();
        let status = waiter.status.clone();
        let attempt = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            match preparation.attempt(cancel, status) {
                Ok(mut job) => {
                    job.set_decision_version(version);
                    // Close retained preparation owners on the worker, before Move can remove sources.
                    let (scope, lease) = preparation.into_parts();
                    let Scope {
                        payload: preparation,
                        resources,
                    } = scope;
                    let job = lease.own(Scope {
                        payload: job,
                        resources,
                    });
                    drop(preparation);
                    Ok(job)
                }
                Err(error) => Err(Box::new((preparation, error))),
            }
        })
        .await;
        let error;
        (preparation, error) = match attempt {
            Ok(Ok(job)) => return Ok(job),
            Ok(Err(failure)) => *failure,
            Err(error) => {
                return Err(Box::new(panicked(
                    waiter.status,
                    error.to_string(),
                    waiter.cancel,
                )));
            }
        };
        // Before admission, recorded errors are owned-stage cleanup residuals.
        // A fresh attempt cannot erase them or accumulate more abandoned stages.
        if waiter.cancel.is_cancelled()
            || snapshot(waiter.status).failed != 0
            || !retryable_error(&error)
            || version == u64::MAX
        {
            drop_preparation(preparation).await;
            return Err(Box::new(failed(waiter.status, error, waiter.cancel)));
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
            drop_preparation(preparation).await;
            let mut progress = cancelled(waiter.status);
            progress.errors.push(error);
            return Err(Box::new(progress));
        }
    }
}

async fn drop_preparation(preparation: Owned<preparation::Preparation>) {
    let _ = preparation.retire().await;
}
