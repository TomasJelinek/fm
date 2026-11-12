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
    file_list_records::error,
    file_list_worker::{self, Generated, OutputReservation, Text},
    retirement::{RetirementDomain, WorkerOwned},
    task_lifetime::TaskLifetime,
};
use midnight_core::*;
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore},
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
    active: AtomicBool,
    _scope: TaskLifetime,
    _retention: Arc<Retention>,
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
            old.cancel.cancel();
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
            active: AtomicBool::new(true),
            _scope: self.scope.clone(),
            _retention: retention,
        }));
        self.jobs.insert(job.request.id, job.clone());
        self.spawn(job, false, permit, tasks);
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
        let permit = self
            .budget
            .clone()
            .try_acquire_owned()
            .map_err(|_| error("File list workers are busy. Try again."))?;
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
        job.active.store(true, Ordering::Release);
        self.spawn(job, true, permit, tasks);
        Ok(())
    }
    fn spawn(
        &self,
        job: Arc<WorkerOwned<Job>>,
        confirmed: bool,
        permit: OwnedSemaphorePermit,
        tasks: &mut JoinSet<()>,
    ) {
        let service = self.service.clone();
        let mutation = self.mutation.clone();
        let outbox = self.outbox.clone();
        let scope = self.scope.clone();
        tasks.spawn(self.scope.future(async move {
            // Native work keeps admission after outer cancellation; delivery
            // keeps it after native completion until its bounded send finishes.
            let _delivery_permit = Arc::new(permit);
            let native_permit = _delivery_permit.clone();
            let worker = job.clone();
            let result = tokio::task::spawn_blocking(scope.blocking(move || {
                let _permit = native_permit;
                let output = run(&worker, confirmed, service.as_deref(), &mutation);
                worker.active.store(false, Ordering::Release);
                output
            }))
            .await
            .unwrap_or_else(|e| Err(error(format!("File list worker failed: {e}"))));
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
            job.cancel.cancel();
        }
    }
    pub(crate) fn shutdown(&mut self) {
        for (_, job) in self.jobs.drain() {
            job.cancel.cancel();
        }
    }
}

impl Drop for Controller {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn run(
    job: &Job,
    confirmed: bool,
    service: Option<&dyn TransferService>,
    mutation: &Arc<Semaphore>,
) -> Result<FileListUpdate, FsError> {
    let pending = if confirmed {
        Some(
            job.pending
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take()
                .ok_or_else(|| error("This file list confirmation has already been used."))?,
        )
    } else {
        None
    };
    let (source, pin) = if let Some(pending) = pending {
        (pending.source, Some(pending.pin))
    } else {
        let output = job
            .output
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
            .ok_or_else(|| error("This file-list generation has already been used."))?;
        match file_list_worker::generate_with_sizes(&job.request, &job.cancel, &job.sizes, output)?
        {
            Generated::Preview {
                text,
                items,
                truncated,
            } => {
                return Ok(FileListUpdate::Preview {
                    text,
                    items,
                    truncated,
                });
            }
            Generated::Content { source, items } => {
                if !matches!(job.request.destination, FileListDestination::File { .. }) {
                    return Ok(FileListUpdate::Generated {
                        content: source,
                        items,
                    });
                }
                (source, None)
            }
        }
    };
    let FileListDestination::File { path, append } = &job.request.destination else {
        return Err(error("No file destination was captured."));
    };
    let service = service.ok_or_else(|| error("Native file output is unavailable."))?;
    // Generation does not hold the write gate. Waiting for existing operations
    // remains bounded and cancellation releases this worker within one poll.
    let _guard = loop {
        super::file_list_records::check(&job.cancel)?;
        match mutation.clone().try_acquire_owned() {
            Ok(permit) => break permit,
            Err(tokio::sync::TryAcquireError::Closed) => {
                return Err(error("File output is shutting down."));
            }
            Err(tokio::sync::TryAcquireError::NoPermits) => {
                std::thread::park_timeout(std::time::Duration::from_millis(20))
            }
        }
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
