//! Bounded generation, confirmation ownership and cancellation for file lists.
#[cfg(test)]
#[path = "file_list_retirement_tests.rs"]
mod retirement_tests;
#[path = "file_list_sizes.rs"]
pub(super) mod sizes;
#[cfg(test)]
#[path = "file_list_actor_tests.rs"]
mod tests;
use crate::{
    Outbox,
    file_list_output::*,
    file_list_records::{check, error},
    file_list_worker::{self, Generated, OutputReservation, Text},
    retirement::{RetirementDomain, WorkerOwned},
    task_lifetime::TaskLifetime,
};
use midnight_core::*;
use std::{
    collections::HashMap,
    future::Future,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::{
    sync::{AcquireError, Notify, OwnedSemaphorePermit, Semaphore},
    task::JoinSet,
};

const MAX_WINDOWS: usize = 4;
const MAX_WORKERS: usize = 2;
pub(super) const MAX_RETAINED: usize = MAX_WINDOWS + MAX_WORKERS;
pub(crate) const RETIREMENT_CAPACITY: usize = 2 * MAX_RETAINED;
pub(crate) struct Retirement {
    pub(crate) domain: RetirementDomain,
}
pub(super) struct Retention {
    pub(super) _permit: OwnedSemaphorePermit,
}
struct Pending {
    pin: FileListDestinationPin,
    source: Arc<Text>,
}
struct Job {
    // Native pins, generated content and captured report storage retire before
    // their service anchor and shared logical admission token.
    pending: Mutex<Option<Pending>>,
    sizes: Arc<sizes::Captured>,
    request: FileListRequest,
    output: Mutex<Option<OutputReservation>>,
    cancel: Cancellation,
    // `cancel` is a polled flag; this wakes a save queued for the gate or a worker.
    stopped: Notify,
    active: AtomicBool,
    _scope: TaskLifetime,
    _retention: Arc<Retention>,
}
impl Job {
    /// The controller cancels jobs only here: native work polls `cancel`, and an
    /// async wait registered before its flag check is woken.
    fn stop(&self) {
        self.cancel.cancel();
        self.stopped.notify_waiters();
    }
}
pub(crate) struct Controller {
    jobs: HashMap<DesktopRequestId, Arc<WorkerOwned<Job>>>,
    budget: Arc<Semaphore>,
    mutation: Arc<Semaphore>,
    service: Option<Arc<dyn TransferService>>,
    scope: TaskLifetime,
    retirement: Retirement,
    retained: Arc<Semaphore>,
    outbox: Outbox,
    snapshots: crate::Snapshots,
}
impl Controller {
    pub(crate) fn new(
        retirement: Retirement,
        scope: TaskLifetime,
        service: Option<Arc<dyn TransferService>>,
        mutation: Arc<Semaphore>,
        outbox: Outbox,
        snapshots: crate::Snapshots,
    ) -> Self {
        Self {
            jobs: HashMap::new(),
            budget: Arc::new(Semaphore::new(MAX_WORKERS)),
            mutation,
            service,
            scope,
            retirement,
            retained: Arc::new(Semaphore::new(MAX_RETAINED)),
            outbox,
            snapshots,
        }
    }
    pub(crate) fn start(
        &mut self,
        request: FileListRequest,
        tasks: &mut JoinSet<()>,
    ) -> Result<(), FsError> {
        if request.id.0 == 0 || request.generation == 0 {
            return Err(error("Invalid file list request identity."));
        }
        if let Some(old) = self.jobs.get(&request.id) {
            if request.generation <= old.request.generation {
                return Err(error("Stale file list request."));
            }
            if old.active.load(Ordering::Acquire)
                && old.request.destination != FileListDestination::Preview
            {
                return Err(error("A file list output is already running."));
            }
        } else if self.jobs.len() >= MAX_WINDOWS {
            return Err(error("Close a file list window before opening another."));
        }
        let permit = self
            .budget
            .clone()
            .try_acquire_owned()
            .map_err(|_| error("File list workers are busy. Try again."))?;
        let retained = self.retained.clone().try_acquire_owned().map_err(|_| {
            error("File list outputs are still in use. Close an output and try again.")
        })?;
        let lease = self
            .retirement
            .domain
            .try_reserve()
            .map_err(|_| error("File-list cleanup is still busy. Try again."))?;
        let output_lease = self
            .retirement
            .domain
            .try_reserve()
            .map_err(|_| error("File-list cleanup is still busy. Try again."))?;
        let retention = Arc::new(Retention { _permit: retained });
        // Admission must succeed before superseding the last accepted preview.
        // Otherwise transient worker pressure cancels useful work without replacing it.
        if let Some(old) = self.jobs.get(&request.id) {
            old.stop();
        }
        let sizes = self
            .jobs
            .get(&request.id)
            .filter(|old| Arc::ptr_eq(&old.request.capture, &request.capture))
            .map_or_else(
                || Arc::new(sizes::Captured::bind(&request.capture, &self.snapshots)),
                |old| old.sizes.clone(),
            );
        let job = Arc::new(lease.own(Job {
            pending: Mutex::new(None),
            sizes,
            request,
            output: Mutex::new(Some(OutputReservation {
                lease: output_lease,
                retention: retention.clone(),
            })),
            cancel: Cancellation::default(),
            stopped: Notify::new(),
            active: AtomicBool::new(true),
            _scope: self.scope.clone(),
            _retention: retention,
        }));
        self.jobs.insert(job.request.id, job.clone());
        self.spawn(job, Some(permit), tasks);
        Ok(())
    }
    pub(crate) fn confirm(
        &mut self,
        id: DesktopRequestId,
        generation: u64,
        tasks: &mut JoinSet<()>,
    ) -> Result<(), FsError> {
        let job = self
            .jobs
            .get(&id)
            .filter(|job| job.request.generation == generation && !job.cancel.is_cancelled())
            .cloned()
            .ok_or_else(|| error("This file list confirmation has expired."))?;
        if job.active.load(Ordering::Acquire) {
            return Err(error("File list output is already running."));
        }
        if job
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_none()
        {
            return Err(error("This file list confirmation has already been used."));
        }
        // Keep the native pins in the guarded Job until a native worker actually
        // begins. Cancelling an unpolled confirmation never exposes a raw Pending.
        // A confirmed save takes its worker only after the mutation gate.
        job.active.store(true, Ordering::Release);
        self.spawn(job, None, tasks);
        Ok(())
    }
    /// `generation` is the worker admitted by `start`; a confirmed save has none.
    fn spawn(
        &self,
        job: Arc<WorkerOwned<Job>>,
        generation: Option<OwnedSemaphorePermit>,
        tasks: &mut JoinSet<()>,
    ) {
        let output = Output {
            service: self.service.clone(),
            workers: self.budget.clone(),
            mutation: self.mutation.clone(),
            scope: self.scope.clone(),
        };
        let outbox = self.outbox.clone();
        tasks.spawn(self.scope.future(async move {
            // Delivery keeps the worker of the last native step until its
            // bounded send finishes.
            let (result, _delivery_permit) = output.run(&job, generation).await;
            job.active.store(false, Ordering::Release);
            if !job.cancel.is_cancelled() {
                outbox
                    .send(Event::FileList(Box::new(FileListEvent {
                        id: job.request.id,
                        generation: job.request.generation,
                        result,
                    })))
                    .await;
            }
        }));
    }
    pub(crate) fn cancel(&mut self, id: DesktopRequestId, generation: u64) {
        if self
            .jobs
            .get(&id)
            .is_some_and(|job| job.request.generation <= generation)
            && let Some(job) = self.jobs.remove(&id)
        {
            job.stop();
        }
    }
    pub(crate) fn shutdown(&mut self) {
        for (_, job) in self.jobs.drain() {
            job.stop();
        }
    }
}

impl Drop for Controller {
    fn drop(&mut self) {
        self.shutdown();
    }
}

type Worker = Arc<OwnedSemaphorePermit>;

/// What one admitted output needs: native generation and saving run on blocking
/// workers, while the waits between them stay on the async task.
struct Output {
    service: Option<Arc<dyn TransferService>>,
    workers: Arc<Semaphore>,
    mutation: Arc<Semaphore>,
    scope: TaskLifetime,
}
impl Output {
    /// Returns the result and the worker that delivery keeps. A save waiting for
    /// the mutation gate holds neither a worker nor a blocking thread, so Tokio
    /// drops (never polls) it at runtime shutdown and other outputs keep both
    /// workers; it takes a worker once the gate is granted.
    async fn run(
        self,
        job: &Arc<WorkerOwned<Job>>,
        generation: Option<OwnedSemaphorePermit>,
    ) -> (Result<FileListUpdate, FsError>, Option<Worker>) {
        let unavailable = || error("Native file output is unavailable.");
        let (source, service) = match generation {
            Some(worker) => {
                let worker = Arc::new(worker);
                let source = match self.native(job, worker.clone(), generate).await {
                    Ok(Step::Save(source)) => source,
                    Ok(Step::Done(update)) => return (Ok(update), Some(worker)),
                    Err(error) => return (Err(error), Some(worker)),
                };
                let Some(service) = self.service.clone() else {
                    return (Err(unavailable()), Some(worker));
                };
                (Some(source), service)
            }
            // The confirmed source and destination pin stay in the guarded job.
            None => match self.service.clone() {
                Some(service) => (None, service),
                None => return (Err(unavailable()), None),
            },
        };
        // Generation does not hold the write gate; only the save waits for it.
        let gate = match queued(
            job,
            self.mutation
                .clone()
                .acquire_many_owned(crate::transfer_leases::NAMESPACE_WEIGHT),
        )
        .await
        {
            Ok(gate) => gate,
            Err(error) => return (Err(error), None),
        };
        // Workers are held only by generation, delivery or this gate holder's own
        // save; none of them waits for the gate, so this wait cannot deadlock.
        let worker = match queued(job, self.workers.clone().acquire_owned()).await {
            Ok(worker) => Arc::new(worker),
            Err(error) => return (Err(error), None),
        };
        let saved = self
            .native(job, worker.clone(), move |job| {
                // The gate stays held until native publication returns, even if
                // the outer task is aborted meanwhile.
                let _gate = gate;
                save(job, source, service.as_ref())
            })
            .await;
        (saved, Some(worker))
    }

    /// Native work keeps its worker even if the outer task is cancelled.
    async fn native<R: Send + 'static>(
        &self,
        job: &Arc<WorkerOwned<Job>>,
        worker: Worker,
        work: impl FnOnce(&Job) -> Result<R, FsError> + Send + 'static,
    ) -> Result<R, FsError> {
        let job = job.clone();
        tokio::task::spawn_blocking(self.scope.blocking(move || {
            let _worker = worker;
            work(&job)
        }))
        .await
        .unwrap_or_else(|e| Err(error(format!("File list worker failed: {e}"))))
    }
}

/// Queue once (FIFO), like Delete/Rename, until granted or the job is stopped. A
/// nonqueuing `try_acquire` succeeds only when nobody is queued (released permits
/// go to waiters first), so polling it could wait until a refilled Copy queue
/// drained. No timer is involved: `Job::stop` wakes the wait.
async fn queued<T>(
    job: &Job,
    acquire: impl Future<Output = Result<T, AcquireError>>,
) -> Result<T, FsError> {
    wait_unless_stopped(&job.cancel, &job.stopped, acquire).await
}

async fn wait_unless_stopped<T>(
    cancel: &Cancellation,
    stopped: &Notify,
    acquire: impl Future<Output = Result<T, AcquireError>>,
) -> Result<T, FsError> {
    // Created before the flag check, so a later `notify_waiters` reaches it.
    let stopped = stopped.notified();
    check(cancel)?;
    tokio::select! {
        biased;
        // `Job::stop` cancels before it notifies, so this reports the cancellation.
        () = stopped => check(cancel).and(Err(error("File output was stopped."))),
        permit = acquire => permit.map_err(|_| error("File output is shutting down.")),
    }
}

enum Step {
    Done(FileListUpdate),
    Save(Arc<Text>),
}

fn generate(job: &Job) -> Result<Step, FsError> {
    let output = job
        .output
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
        .ok_or_else(|| error("This file-list generation has already been used."))?;
    Ok(
        match file_list_worker::generate_with_sizes(&job.request, &job.cancel, &job.sizes, output)?
        {
            Generated::Preview {
                text,
                items,
                truncated,
            } => Step::Done(FileListUpdate::Preview {
                text,
                items,
                truncated,
            }),
            Generated::Content { source, .. }
                if matches!(job.request.destination, FileListDestination::File { .. }) =>
            {
                Step::Save(source)
            }
            Generated::Content { source, items } => Step::Done(FileListUpdate::Generated {
                content: source,
                items,
            }),
        },
    )
}

/// Runs on a native worker while the mutation gate is held. A confirmed save
/// (`generated` is None) takes the source and destination pin it consented to.
fn save(
    job: &Job,
    generated: Option<Arc<Text>>,
    service: &dyn TransferService,
) -> Result<FileListUpdate, FsError> {
    let (source, pin) = match generated {
        Some(source) => (source, None),
        None => {
            let pending = job
                .pending
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take()
                .ok_or_else(|| error("This file list confirmation has already been used."))?;
            (pending.source, Some(pending.pin))
        }
    };
    let FileListDestination::File { path, append } = &job.request.destination else {
        return Err(error("No file destination was captured."));
    };
    match save_file_list(
        file_list_worker::transfer(source.clone()),
        path,
        *append,
        pin,
        service,
        &job.cancel,
    )? {
        FileListSaveOutcome::Conflict(pin) => {
            let entry = pin.entry().clone();
            *job.pending.lock().unwrap_or_else(|e| e.into_inner()) = Some(Pending { source, pin });
            Ok(FileListUpdate::Conflict {
                entry,
                append: *append,
            })
        }
        FileListSaveOutcome::Saved(receipt) => Ok(FileListUpdate::Saved {
            path: receipt.path,
            bytes: receipt.verified_bytes,
            recovery: receipt.recovery,
            warning: receipt.warning,
        }),
    }
}

#[cfg(test)]
impl Controller {
    fn testing(
        service: Option<Arc<dyn TransferService>>,
        mutation: Arc<Semaphore>,
        outbox: Outbox,
        snapshots: crate::Snapshots,
    ) -> Self {
        Self::new(
            Retirement {
                domain: RetirementDomain::try_start(RETIREMENT_CAPACITY, 0).unwrap(),
            },
            TaskLifetime::testing(service.clone()),
            service,
            mutation,
            outbox,
            snapshots,
        )
    }
}
