//! Backend-only checksum controller. One process-wide worker, bounded queues and retained quota.
//! The host must await shutdown during normal close; UI/protocol integration is separate.
use crate::retirement::WorkerOwned;
use midnight_core::*;
#[path = "checksum_retirement.rs"]
pub(crate) mod ownership;
#[cfg(test)]
pub(crate) use ownership::Retirement;
use ownership::Service;
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

#[path = "checksum_format.rs"]
mod format;
#[path = "checksum_manifest.rs"]
mod manifest;
#[path = "checksum_verify.rs"]
mod verify;
#[path = "checksum_worker.rs"]
mod worker;
pub use manifest::save_manifest;

type Key = (ChecksumJobId, ChecksumGeneration);
type Registry = Arc<Mutex<HashMap<Key, Cancellation>>>;
pub(super) struct Work {
    request: Option<Request>,
    cancel: Cancellation,
    // Captured native/provider ownership closes before quota, worker permit and service.
    source: Option<Arc<dyn CapturedChecksumFiles>>,
    retention: Arc<ChecksumRetention>,
    worker: Option<OwnedSemaphorePermit>,
    service: Service,
}
impl Work {
    pub(super) fn calculate(
        request: ChecksumRequest,
        cancel: Cancellation,
        retention: Arc<ChecksumRetention>,
        source: Option<Arc<dyn CapturedChecksumFiles>>,
        service: Service,
    ) -> Self {
        Self {
            request: Some(Request::Calculate(request)),
            cancel,
            source,
            retention,
            worker: None,
            service,
        }
    }
    pub(super) fn verify(
        request: ChecksumVerifyRequest,
        cancel: Cancellation,
        retention: Arc<ChecksumRetention>,
        service: Service,
    ) -> Self {
        Self {
            request: Some(Request::Verify(request)),
            cancel,
            source: None,
            retention,
            worker: None,
            service,
        }
    }
    fn key(&self) -> Key {
        self.request
            .as_ref()
            .expect("pending checksum request")
            .key()
    }
}
enum Request {
    Calculate(ChecksumRequest),
    Verify(ChecksumVerifyRequest),
}
impl Request {
    fn key(&self) -> Key {
        match self {
            Self::Calculate(request) => (request.job, request.generation),
            Self::Verify(request) => (request.job, request.generation),
        }
    }
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

pub(super) fn reserve_verification(
    request: &ChecksumVerifyRequest,
) -> Result<Arc<ChecksumRetention>, FsError> {
    let bytes = request.validate().map_err(|message| {
        worker::failure(&request.manifest, io::ErrorKind::InvalidInput, message)
    })?;
    // Parsing bounds encoded text and decoded/entry data independently. During
    // calculation the original rows, comparison index and transformed rows share
    // the existing conservative per-row charge. Three budgets cover overlap,
    // while admitting a replacement beside a retained verification generation.
    reserve(bytes, request.limits.max_retained_bytes * 3)
}

pub struct Controller {
    requests: mpsc::Sender<WorkerOwned<Work>>,
    _service: Service,
    #[cfg(test)]
    retirement: Option<crate::retirement::RetirementDomain>,
    events: mpsc::Receiver<ChecksumEvent>,
    registry: Registry,
    stop: Cancellation,
    task: Option<JoinHandle<()>>,
}
impl Controller {
    /// The adapter passes the already-guarded service owner into this child actor.
    pub(super) fn from_service(service: Service) -> Self {
        let (requests, incoming) = mpsc::channel(MAX_CHECKSUM_JOBS);
        let (outgoing, events) = mpsc::channel(64);
        let registry = Registry::default();
        let stop = Cancellation::default();
        let task = tokio::spawn(run(
            incoming,
            outgoing,
            service.clone(),
            registry.clone(),
            stop.clone(),
        ));
        Self {
            requests,
            events,
            _service: service,
            registry,
            stop,
            task: Some(task),
            #[cfg(test)]
            retirement: None,
        }
    }
    #[cfg(test)]
    pub fn new(retirement: Retirement, service: Arc<dyn TransferService>) -> Self {
        let service = Service::new(retirement.service, Some(service));
        let mut controller = Self::from_service(service);
        controller.retirement = Some(retirement.domain);
        controller
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
        self.admit_plain(Request::Calculate(request), self.stop.child(), retention)
    }

    pub(super) async fn admit_prepared(&self, work: WorkerOwned<Work>) -> Result<(), FsError> {
        match self.admit(work) {
            Ok(()) => Ok(()),
            Err((error, work)) => {
                let _ = work.retire().await;
                Err(error)
            }
        }
    }
    #[cfg(test)]
    #[allow(dead_code)] // Used by path-included native integration tests.
    pub fn verify(&self, request: ChecksumVerifyRequest) -> Result<(), FsError> {
        let retention = reserve_verification(&request)?;
        self.admit_plain(Request::Verify(request), self.stop.child(), retention)
    }
    #[cfg(test)]
    fn admit_plain(
        &self,
        request: Request,
        cancel: Cancellation,
        retention: Arc<ChecksumRetention>,
    ) -> Result<(), FsError> {
        let lease = self
            .retirement
            .as_ref()
            .expect("standalone fixture retirement")
            .try_reserve()
            .map_err(|_| {
                worker::failure(
                    Path::new(""),
                    io::ErrorKind::WouldBlock,
                    "Checksum cleanup capacity is occupied.",
                )
            })?;
        let work = lease.own(Work {
            request: Some(request),
            cancel,
            source: None,
            retention,
            worker: None,
            service: self._service.clone(),
        });
        self.admit(work).map_err(|(error, work)| {
            drop(work);
            error
        })
    }
    fn admit(&self, work: WorkerOwned<Work>) -> Result<(), (FsError, WorkerOwned<Work>)> {
        let fail = |kind, text| worker::failure(Path::new(""), kind, text);
        let key = work.key();
        let mut registry = self
            .registry
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if registry
            .keys()
            .any(|(job, generation)| *job == key.0 && *generation >= key.1)
        {
            return Err((
                fail(
                    io::ErrorKind::AlreadyExists,
                    "Checksum generation is already active or obsolete.",
                ),
                work,
            ));
        }
        // Recalculation still waits for the serial worker to clean its predecessor.
        for ((job, _), old) in registry.iter() {
            if *job == key.0 {
                old.cancel();
            }
        }
        registry.insert(key, work.cancel.clone());
        if let Err(error) = self.requests.try_send(work) {
            registry.remove(&key);
            return Err((
                fail(
                    io::ErrorKind::WouldBlock,
                    "Checksum admission queue is unavailable or full.",
                ),
                error.into_inner(),
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
    mut incoming: mpsc::Receiver<WorkerOwned<Work>>,
    outgoing: mpsc::Sender<ChecksumEvent>,
    _service: Service,
    registry: Registry,
    stop: Cancellation,
) {
    'jobs: loop {
        if stop.is_cancelled() {
            break;
        }
        let mut work = match tokio::time::timeout(Duration::from_millis(25), incoming.recv()).await
        {
            Ok(Some(work)) => work,
            Ok(None) => break,
            Err(_) => continue,
        };
        let key = work.key();
        let permit = loop {
            if stop.is_cancelled() {
                let _ = work.retire().await;
                break 'jobs;
            }
            if let Ok(permit) = workers().try_acquire_owned() {
                break permit;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        };
        work.worker = Some(permit);
        let events = outgoing.clone();
        let stopping = stop.clone();
        let cancellation = work.cancel.clone();
        let completed =
            tokio::task::spawn_blocking(move || execute(work, &events, &stopping)).await;
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
    incoming.close();
    while let Ok(work) = incoming.try_recv() {
        let _ = work.retire().await;
    }
}
fn execute(
    owned: WorkerOwned<Work>,
    events: &mpsc::Sender<ChecksumEvent>,
    stopping: &Cancellation,
) -> Arc<ChecksumRun> {
    let (payload, lease) = owned.into_parts();
    // Bind after the lease so error/panic destruction keeps the slot occupied.
    let mut work = payload;
    let key = work.key();
    let request = work.request.take().expect("pending checksum request");
    let send = |update| send_update(events, key, update, stopping, &work.cancel);
    if !send(ChecksumUpdate::Started) {
        work.cancel.cancel();
    }
    let run = match request {
        Request::Calculate(request) => worker::calculate_captured(
            request,
            work.source.as_deref(),
            work.service.provider(),
            &work.cancel,
            work.retention.clone(),
            send,
        ),
        Request::Verify(request) => verify::verify(
            request,
            work.service.provider(),
            &work.cancel,
            work.retention.clone(),
            send,
        ),
    };
    // Actual native reads are gone before the captured provider, permits and lease.
    drop(work);
    drop(lease);
    run
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
pub(crate) async fn test_quota_fixture() -> tokio::sync::MutexGuard<'static, ()> {
    // This module is also path-included by checksums_backend. Every test using
    // real reservation admission must hold this guard until its controllers,
    // retained runs and individual row Arcs are released. Production admission
    // and its process-wide 32-job/64-MiB quotas remain unchanged.
    static FIXTURE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    FIXTURE.lock().await
}

#[cfg(test)]
pub(super) async fn test_wait_for_quota_idle() {
    // A capture Drop-entry signal precedes the following quota field's destruction.
    // Keep the shared fixture until automatic retirement returns the actual permit.
    tokio::time::timeout(Duration::from_secs(5), async {
        while jobs().available_permits() != MAX_CHECKSUM_JOBS {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("checksum quota retained after retirement");
}

#[cfg(test)]
#[path = "checksum_controller_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "checksum_boundaries_tests.rs"]
mod boundary_tests;

#[cfg(test)]
#[path = "checksum_retirement_fixture.rs"]
pub(super) mod retirement_fixture;
#[cfg(test)]
#[path = "checksum_backend_retirement_tests.rs"]
mod retirement_tests;
