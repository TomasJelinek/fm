//! Bounded generation, confirmation ownership and cancellation for file lists.
#[path = "file_list_sizes.rs"]
pub(super) mod sizes;
#[cfg(test)]
#[path = "file_list_actor_tests.rs"]
mod tests;
use crate::{
    Outbox,
    file_list_output::*,
    file_list_records::error,
    file_list_worker::{self, Generated, Text},
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

struct Pending {
    source: Arc<Text>,
    pin: FileListDestinationPin,
}
struct Job {
    request: FileListRequest,
    sizes: Arc<sizes::Captured>,
    cancel: Cancellation,
    active: AtomicBool,
    pending: Mutex<Option<Pending>>,
}
pub(crate) struct Controller {
    jobs: HashMap<DesktopRequestId, Arc<Job>>,
    budget: Arc<Semaphore>,
    mutation: Arc<Semaphore>,
    service: Option<Arc<dyn TransferService>>,
    outbox: Outbox,
    snapshots: crate::Snapshots,
}
impl Controller {
    pub(crate) fn new(
        service: Option<Arc<dyn TransferService>>,
        mutation: Arc<Semaphore>,
        outbox: Outbox,
        snapshots: crate::Snapshots,
    ) -> Self {
        Self {
            jobs: HashMap::new(),
            budget: Arc::new(Semaphore::new(2)),
            mutation,
            service,
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
        } else if self.jobs.len() >= 4 {
            return Err(error("Close a file list window before opening another."));
        }
        let permit = self
            .budget
            .clone()
            .try_acquire_owned()
            .map_err(|_| error("File list workers are busy. Try again."))?;
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
        let job = Arc::new(Job {
            request,
            sizes,
            cancel: Cancellation::default(),
            active: AtomicBool::new(true),
            pending: Mutex::new(None),
        });
        self.jobs.insert(job.request.id, job.clone());
        self.spawn(job, None, permit, tasks);
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
        let pending = job
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
            .ok_or_else(|| error("This file list confirmation has already been used."))?;
        job.active.store(true, Ordering::Release);
        self.spawn(job, Some(pending), permit, tasks);
        Ok(())
    }
    fn spawn(
        &self,
        job: Arc<Job>,
        pending: Option<Pending>,
        permit: OwnedSemaphorePermit,
        tasks: &mut JoinSet<()>,
    ) {
        let service = self.service.clone();
        let mutation = self.mutation.clone();
        let outbox = self.outbox.clone();
        tasks.spawn(async move {
            let worker = job.clone();
            let result = tokio::task::spawn_blocking(move || {
                let _permit = permit;
                let output = run(&worker, pending, service.as_deref(), &mutation);
                worker.active.store(false, Ordering::Release);
                output
            })
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
        });
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

fn run(
    job: &Job,
    pending: Option<Pending>,
    service: Option<&dyn TransferService>,
    mutation: &Arc<Semaphore>,
) -> Result<FileListUpdate, FsError> {
    let (source, pin) = if let Some(pending) = pending {
        (pending.source, Some(pending.pin))
    } else {
        match file_list_worker::generate_with_sizes(&job.request, &job.cancel, &job.sizes)? {
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
