//! Backend-only checksum controller. One process-wide worker, bounded queues and retained quota.
//! The host must await shutdown during normal close; UI/protocol integration is separate.
use midnight_core::*;
use std::{
    collections::HashMap,
    io,
    path::Path,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore, mpsc},
    task::JoinHandle,
};

#[path = "checksum_manifest.rs"]
mod manifest;
#[path = "checksum_worker.rs"]
mod worker;
pub use manifest::save_manifest;

type Key = (ChecksumJobId, ChecksumGeneration);
type Registry = Arc<Mutex<HashMap<Key, Cancellation>>>;
struct Work {
    request: ChecksumRequest,
    cancel: Cancellation,
    retention: Arc<ChecksumRetention>,
}
struct Quota {
    _job: OwnedSemaphorePermit,
    _memory: OwnedSemaphorePermit,
}
fn jobs() -> Arc<Semaphore> {
    static JOBS: OnceLock<Arc<Semaphore>> = OnceLock::new();
    JOBS.get_or_init(|| Arc::new(Semaphore::new(MAX_CHECKSUM_JOBS)))
        .clone()
}
fn memory() -> Arc<Semaphore> {
    static MEMORY: OnceLock<Arc<Semaphore>> = OnceLock::new();
    MEMORY
        .get_or_init(|| Arc::new(Semaphore::new(64 * 1024 * 1024)))
        .clone()
}
fn workers() -> Arc<Semaphore> {
    static WORKERS: OnceLock<Arc<Semaphore>> = OnceLock::new();
    WORKERS.get_or_init(|| Arc::new(Semaphore::new(1))).clone()
}

pub(super) fn reserve(bytes: usize, retained: usize) -> Result<Arc<ChecksumRetention>, FsError> {
    let fail = |kind, message| worker::failure(Path::new(""), kind, message);
    let job = jobs().try_acquire_owned().map_err(|_| {
        fail(
            io::ErrorKind::WouldBlock,
            "Checksum job budget is full (32 including retained results).",
        )
    })?;
    let bytes = bytes
        .checked_add(retained)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| {
            fail(
                io::ErrorKind::InvalidInput,
                "Checksum memory reservation overflow.",
            )
        })?;
    let memory = memory().try_acquire_many_owned(bytes).map_err(|_| {
        fail(
            io::ErrorKind::WouldBlock,
            "Checksum target/result memory budget is full (64 MiB).",
        )
    })?;
    Ok(ChecksumRetention::new(Quota {
        _job: job,
        _memory: memory,
    }))
}

pub struct Controller {
    requests: mpsc::Sender<Work>,
    events: mpsc::Receiver<ChecksumEvent>,
    registry: Registry,
    stop: Cancellation,
    task: Option<JoinHandle<()>>,
}
impl Controller {
    /// Construct and own this service on the engine/backend runtime.
    pub fn new(service: Arc<dyn TransferService>) -> Self {
        let (requests, incoming) = mpsc::channel(MAX_CHECKSUM_JOBS);
        let (outgoing, events) = mpsc::channel(64);
        let registry = Registry::default();
        let stop = Cancellation::default();
        let task = tokio::spawn(run(
            incoming,
            outgoing,
            service,
            registry.clone(),
            stop.clone(),
        ));
        Self {
            requests,
            events,
            registry,
            stop,
            task: Some(task),
        }
    }
    /// Nonblocking backend admission. Resource exhaustion rejects the complete request.
    #[cfg(test)]
    pub fn start(&self, request: ChecksumRequest) -> Result<(), FsError> {
        let fail = |kind, text| worker::failure(Path::new(""), kind, text);
        let request_bytes = request
            .validate()
            .map_err(|text| fail(io::ErrorKind::InvalidInput, text))?;
        if self.stop.is_cancelled() {
            return Err(fail(
                io::ErrorKind::Interrupted,
                "Checksum service is closing.",
            ));
        }
        let retention = reserve(request_bytes, request.limits.max_retained_bytes)?;
        self.start_reserved(request, self.stop.child(), retention)
    }

    pub(super) fn start_reserved(
        &self,
        request: ChecksumRequest,
        cancel: Cancellation,
        retention: Arc<ChecksumRetention>,
    ) -> Result<(), FsError> {
        let fail = |kind, text| worker::failure(Path::new(""), kind, text);
        let key = (request.job, request.generation);
        let mut registry = self
            .registry
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if registry
            .keys()
            .any(|(job, generation)| *job == request.job && *generation >= request.generation)
        {
            return Err(fail(
                io::ErrorKind::AlreadyExists,
                "Checksum generation is already active or obsolete.",
            ));
        }
        // A recalculate starts only after the serial worker has cleaned up its predecessor.
        for ((job, _), old) in registry.iter() {
            if *job == request.job {
                old.cancel();
            }
        }
        registry.insert(key, cancel.clone());
        let work = Work {
            request,
            cancel,
            retention,
        };
        if self.requests.try_send(work).is_err() {
            registry.remove(&key);
            return Err(fail(
                io::ErrorKind::WouldBlock,
                "Checksum admission queue is unavailable or full.",
            ));
        }
        Ok(())
    }
    #[cfg(test)]
    pub fn cancel(&self, job: ChecksumJobId, generation: ChecksumGeneration) {
        if let Some(cancel) = self
            .registry
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(&(job, generation))
        {
            cancel.cancel();
        }
    }
    pub async fn next(&mut self) -> Option<ChecksumEvent> {
        self.events.recv().await
    }
    #[cfg(test)]
    pub fn try_next(&mut self) -> Option<ChecksumEvent> {
        self.events.try_recv().ok()
    }
    pub async fn shutdown(mut self) {
        self.stop.cancel();
        for cancel in self
            .registry
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .values()
        {
            cancel.cancel();
        }
        self.events.close();
        while self.events.try_recv().is_ok() {}
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}
impl Drop for Controller {
    fn drop(&mut self) {
        self.stop.cancel();
        for cancel in self
            .registry
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .values()
        {
            cancel.cancel();
        }
    }
}

async fn run(
    mut incoming: mpsc::Receiver<Work>,
    outgoing: mpsc::Sender<ChecksumEvent>,
    service: Arc<dyn TransferService>,
    registry: Registry,
    stop: Cancellation,
) {
    loop {
        if stop.is_cancelled() {
            break;
        }
        let work = match tokio::time::timeout(Duration::from_millis(25), incoming.recv()).await {
            Ok(Some(work)) => work,
            Ok(None) => break,
            Err(_) => continue,
        };
        let key = (work.request.job, work.request.generation);
        let permit = loop {
            if stop.is_cancelled() {
                return;
            }
            if let Ok(permit) = workers().try_acquire_owned() {
                break permit;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        };
        let events = outgoing.clone();
        let stopping = stop.clone();
        let provider = service.clone();
        let cancellation = work.cancel.clone();
        let completed = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let sending_job = work.cancel.clone();
            let send = |update| send_update(&events, key, update, &stopping, &sending_job);
            if !send(ChecksumUpdate::Started) {
                work.cancel.cancel();
            }
            worker::calculate(
                work.request,
                provider.as_ref(),
                &work.cancel,
                work.retention,
                send,
            )
        })
        .await;
        registry
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&key);
        match completed {
            Ok(run) if !stop.is_cancelled() => {
                if outgoing
                    .send(ChecksumEvent {
                        job: key.0,
                        generation: key.1,
                        update: ChecksumUpdate::Finished(run),
                    })
                    .await
                    .is_err()
                {
                    break;
                }
            }
            Ok(_) => {}
            Err(_) => {
                cancellation.cancel();
                // A panic is not a completed checksum. Close transport so the host can
                // report service failure rather than invent digest rows or keep waiting.
                stop.cancel();
                break;
            }
        }
    }
    for cancel in registry
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .values()
    {
        cancel.cancel();
    }
}
fn send_update(
    sender: &mpsc::Sender<ChecksumEvent>,
    key: Key,
    update: ChecksumUpdate,
    stop: &Cancellation,
    job_cancel: &Cancellation,
) -> bool {
    let mut event = ChecksumEvent {
        job: key.0,
        generation: key.1,
        update,
    };
    loop {
        if stop.is_cancelled() || job_cancel.is_cancelled() {
            return false;
        }
        match sender.try_send(event) {
            Ok(()) => return true,
            Err(mpsc::error::TrySendError::Closed(_)) => return false,
            Err(mpsc::error::TrySendError::Full(value)) => event = value,
        }
        std::thread::park_timeout(Duration::from_millis(10));
    }
}

#[cfg(test)]
#[path = "checksum_controller_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "checksum_boundaries_tests.rs"]
mod boundary_tests;
