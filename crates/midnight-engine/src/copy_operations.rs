//! A bounded FIFO of verified clipboard copies, independent of pane navigation.
//! One job and one native I/O step run at a time on every storage class. Any future
//! parallel scheduler must positively identify both endpoints as local SSDs and
//! share physical-device budgets; a fast endpoint cannot relax the other endpoint.
use crate::Outbox;
use midnight_core::*;
use std::{
    collections::HashMap,
    io,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore, mpsc},
    task::JoinSet,
};

#[path = "copy_admission.rs"]
mod admission;
#[path = "copy_input.rs"]
mod input;
#[path = "copy_worker.rs"]
mod worker;
pub(crate) use admission::{Admission, Reservation};
use input::CopyInput;
type Status = Arc<Mutex<CopyProgress>>;
type Registry = Arc<Mutex<HashMap<OperationId, Registered>>>;
const MAX_JOBS: usize = 32;
struct Registered {
    cancel: Cancellation,
    status: Status,
    decisions: mpsc::Sender<CopyDecision>,
    waiting: Arc<AtomicU64>,
}
struct Request {
    receipt: crate::copy_fence::Receipt,
    input: Input,
    // Rejected/aborted requests resolve and release payloads before returning capacity.
    _capacity: OwnedSemaphorePermit,
}
struct Input {
    command: CopyInput,
    cancel: Cancellation,
    status: Status,
    decisions: mpsc::Receiver<CopyDecision>,
    waiting: Arc<AtomicU64>,
}

pub(crate) struct Controller {
    admission: Admission,
    outbox: Outbox,
    registry: Registry,
    cancel: Cancellation,
    highest: AtomicU64,
}
impl Controller {
    pub(crate) fn new(
        service: Option<Arc<dyn TransferService>>,
        outbox: Outbox,
        mutation_gate: Arc<Semaphore>,
        io_budget: Arc<Semaphore>,
        tasks: &mut JoinSet<()>,
    ) -> Self {
        let admission = Admission::default();
        let registry = Registry::default();
        let cancel = Cancellation::default();
        tasks.spawn(run(
            admission.consumer(),
            service,
            outbox.clone(),
            registry.clone(),
            cancel.clone(),
            mutation_gate,
            io_budget,
        ));
        Self {
            admission,
            outbox,
            registry,
            cancel,
            highest: AtomicU64::new(0),
        }
    }
    #[cfg(test)]
    pub(crate) fn capacity(&self) -> Arc<Semaphore> {
        self.admission.capacity()
    }
    pub(crate) fn admission(&self) -> Admission {
        self.admission.clone()
    }
    pub(crate) fn reserve(&self) -> Result<Reservation, FsError> {
        self.admission.reserve()
    }
    pub(crate) async fn enqueue(&self, request: CopyRequest) {
        self.enqueue_input(request.into()).await;
    }
    pub(crate) async fn enqueue_selected(&self, request: SelectedCopyRequest) {
        self.enqueue_input(request.into()).await;
    }
    async fn enqueue_input(&self, request: CopyInput) {
        let permit = match self.reserve() {
            Ok(permit) => permit,
            Err(error) => {
                if request.id.0 <= self.highest.fetch_max(request.id.0, Ordering::Relaxed) {
                    return;
                }
                publish(&self.outbox, rejected_progress(initial(&request), error)).await;
                return;
            }
        };
        self.enqueue_input_reserved(request, permit).await;
    }
    pub(crate) async fn enqueue_reserved(&self, request: CopyRequest, capacity: Reservation) {
        self.enqueue_input_reserved(request.into(), capacity).await;
    }
    async fn enqueue_input_reserved(&self, request: CopyInput, capacity: Reservation) {
        if request.id.0 <= self.highest.fetch_max(request.id.0, Ordering::Relaxed) {
            return;
        }
        let progress = initial(&request);
        let rejected = request.validate().err().or_else(|| {
            if !request.destination.is_absolute()
                || request
                    .destination
                    .as_os_str()
                    .as_encoded_bytes()
                    .contains(&0)
            {
                Some(error(
                    &request.destination,
                    io::ErrorKind::InvalidInput,
                    "The destination must be an absolute native directory path.",
                ))
            } else if self.cancel.is_cancelled() {
                Some(error(
                    &request.destination,
                    io::ErrorKind::Interrupted,
                    "The application is closing.",
                ))
            } else {
                None
            }
        });
        if let Some(error) = rejected {
            publish(&self.outbox, rejected_progress(progress, error)).await;
            return;
        }
        let cancel = self.cancel.child();
        let status = Arc::new(Mutex::new(progress.clone()));
        let waiting = Arc::new(AtomicU64::new(0));
        let (decisions, decision_receiver) = mpsc::channel(8);
        let admitted = {
            let mut registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
            if registry.len() >= MAX_JOBS {
                false
            } else {
                registry.insert(
                    request.id,
                    Registered {
                        cancel: cancel.clone(),
                        status: status.clone(),
                        decisions,
                        waiting: waiting.clone(),
                    },
                );
                true
            }
        };
        if !admitted {
            let failure = error(
                &request.destination,
                io::ErrorKind::WouldBlock,
                "The copy queue is full (32 jobs).",
            );
            publish(&self.outbox, rejected_progress(progress, failure)).await;
            return;
        }
        publish(&self.outbox, progress.clone()).await;
        if capacity
            .send(Input {
                command: request,
                cancel,
                status,
                decisions: decision_receiver,
                waiting,
            })
            .is_err()
        {
            self.registry
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&progress.id);
            let failure = error(
                &progress.destination_directory,
                io::ErrorKind::BrokenPipe,
                "The copy queue is unavailable.",
            );
            publish(&self.outbox, rejected_progress(progress, failure)).await;
        }
    }
    pub(crate) async fn decide(&self, decision: CopyDecision) {
        let registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(job) = registry.get(&decision.id)
            && !job.cancel.is_cancelled()
            && decision.version != 0
            && job.waiting.load(Ordering::Acquire) == decision.version
        {
            let _ = job.decisions.try_send(decision);
        }
    }
    pub(crate) fn cancel(&self, id: OperationId) {
        let registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(job) = registry.get(&id) {
            job.cancel.cancel();
            job.waiting.store(0, Ordering::Release);
            let mut progress = job.status.lock().unwrap_or_else(|e| e.into_inner());
            if progress.state == CopyOperationState::CancelRequested || progress.state.is_terminal()
            {
                return;
            }
            progress.state = CopyOperationState::CancelRequested;
            if self
                .outbox
                .sender
                .try_send(Event::Copy(CopyEvent::Progress(Arc::new(progress.clone()))))
                .is_ok()
            {
                (self.outbox.wakeup)();
            }
        }
    }
    pub(crate) fn shutdown(&self) {
        self.cancel.cancel();
        self.admission.capacity().close();
    }
}
impl Drop for Controller {
    fn drop(&mut self) {
        self.shutdown();
    }
}

async fn run(
    consumer: admission::Consumer,
    service: Option<Arc<dyn TransferService>>,
    outbox: Outbox,
    registry: Registry,
    cancel: Cancellation,
    mutation_gate: Arc<Semaphore>,
    io_budget: Arc<Semaphore>,
) {
    let requests = consumer.admission();
    while let Some(request) = requests.next(&cancel).await {
        let id = request.input.command.id;
        let progress = run_job(
            request,
            service.clone(),
            &outbox,
            &mutation_gate,
            &io_budget,
        )
        .await;
        registry
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&id);
        // Resource cleanup and the logical mutation permit finish before terminal delivery.
        publish(&outbox, progress).await;
    }
}

async fn acquire(budget: &Arc<Semaphore>, cancel: &Cancellation) -> Option<OwnedSemaphorePermit> {
    if cancel.is_cancelled() {
        return None;
    }
    let pending = budget.clone().acquire_owned();
    tokio::pin!(pending);
    let mut timer = tokio::time::interval(Duration::from_millis(25));
    loop {
        tokio::select! {
            result = &mut pending => return result.ok(),
            _ = timer.tick() => if cancel.is_cancelled() { return None; },
        }
    }
}

async fn run_job(
    request: Request,
    service: Option<Arc<dyn TransferService>>,
    outbox: &Outbox,
    mutation_gate: &Arc<Semaphore>,
    io_budget: &Arc<Semaphore>,
) -> CopyProgress {
    let Request {
        _capacity,
        receipt,
        input,
    } = request;
    let Some(_mutation) = acquire(mutation_gate, &input.cancel).await else {
        let progress = cancelled(&input.status);
        receipt.finish(&progress);
        return progress;
    };
    let progress = run_input(input, service, outbox, io_budget).await;
    receipt.finish(&progress);
    progress
}

async fn run_input(
    mut request: Input,
    service: Option<Arc<dyn TransferService>>,
    outbox: &Outbox,
    io_budget: &Arc<Semaphore>,
) -> CopyProgress {
    let Some(service) = service else {
        let failure = error(
            &request.command.destination,
            io::ErrorKind::Unsupported,
            "Verified copying is unavailable for this filesystem.",
        );
        return failed(&request.status, failure, &request.cancel);
    };
    set_state(&request.status, CopyOperationState::Preparing);
    publish(outbox, snapshot(&request.status)).await;
    let Some(permit) = acquire(io_budget, &request.cancel).await else {
        return cancelled(&request.status);
    };
    let status = request.status.clone();
    let job_cancel = request.cancel.clone();
    let command = request.command;
    let prepared = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let command = command.resolve(&job_cancel)?;
        status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .requested_roots = command.clipboard.paths.len() as u64;
        worker::Job::prepare(command, &*service, job_cancel, status)
    })
    .await;
    let mut job = match prepared {
        Ok(Ok(job)) => job,
        Ok(Err(error)) => return failed(&request.status, error, &request.cancel),
        Err(error) => return panicked(&request.status, error.to_string(), &request.cancel),
    };
    set_state(
        &request.status,
        if request.cancel.is_cancelled() {
            CopyOperationState::CancelRequested
        } else {
            CopyOperationState::Running
        },
    );
    publish(outbox, snapshot(&request.status)).await;
    let mut last = Instant::now();
    loop {
        // Cancellation cannot abort an OS call or relinquish its permit early.
        let Some(permit) = acquire(io_budget, &request.cancel).await else {
            break;
        };
        let executed = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let step = job.step();
            (job, step)
        })
        .await;
        let step;
        (job, step) = match executed {
            Ok(result) => result,
            Err(error) => return panicked(&request.status, error.to_string(), &request.cancel),
        };
        match step {
            worker::Step::Done => break,
            worker::Step::Continue => {
                if last.elapsed() >= Duration::from_millis(250) {
                    publish(outbox, snapshot(&request.status)).await;
                    last = Instant::now();
                }
            }
            worker::Step::Conflict(conflict) => {
                set_state(&request.status, CopyOperationState::WaitingForDecision);
                request.waiting.store(conflict.version, Ordering::Release);
                publish(outbox, snapshot(&request.status)).await;
                outbox
                    .send(Event::Copy(CopyEvent::Conflict(conflict)))
                    .await;
                let mut timer = tokio::time::interval(Duration::from_millis(25));
                loop {
                    if request.cancel.is_cancelled() {
                        break;
                    }
                    tokio::select! {
                        decision = request.decisions.recv() => match decision {
                            Some(decision) => if job.accept_decision(decision) { break; },
                            None => { request.cancel.cancel(); break; },
                        },
                        _ = timer.tick() => {},
                    }
                }
                request.waiting.store(0, Ordering::Release);
                if !request.cancel.is_cancelled() {
                    set_state(&request.status, CopyOperationState::Running);
                }
            }
        }
    }
    request.waiting.store(0, Ordering::Release);
    // Cleanup must run despite cancellation and on a worker, holding I/O capacity.
    let permit = io_budget.clone().acquire_owned().await.ok();
    match tokio::task::spawn_blocking(move || {
        let _permit = permit;
        job.finish()
    })
    .await
    {
        Ok(progress) => progress,
        Err(error) => panicked(&request.status, error.to_string(), &request.cancel),
    }
}

fn snapshot(status: &Status) -> CopyProgress {
    status.lock().unwrap_or_else(|e| e.into_inner()).clone()
}
fn set_state(status: &Status, state: CopyOperationState) {
    status.lock().unwrap_or_else(|e| e.into_inner()).state = state;
}
async fn publish(outbox: &Outbox, progress: CopyProgress) {
    outbox
        .send(Event::Copy(CopyEvent::Progress(Arc::new(progress))))
        .await;
}
fn error(path: &Path, kind: io::ErrorKind, message: &str) -> FsError {
    FsError::from_io("copy files", path, io::Error::new(kind, message))
}
fn rejected_progress(mut progress: CopyProgress, error: FsError) -> CopyProgress {
    progress.state = CopyOperationState::Failed;
    progress.failed = 1;
    progress.message = error.to_string();
    progress.errors.push(error);
    progress
}
fn cancelled(status: &Status) -> CopyProgress {
    let mut progress = snapshot(status);
    progress.state = CopyOperationState::Cancelled;
    progress.message = "Copy cancelled; source data was retained.".into();
    progress
}
fn failed(status: &Status, error: FsError, cancel: &Cancellation) -> CopyProgress {
    if cancel.is_cancelled() {
        cancelled(status)
    } else {
        rejected_progress(snapshot(status), error)
    }
}
fn panicked(status: &Status, message: String, cancel: &Cancellation) -> CopyProgress {
    let path = snapshot(status).destination_directory;
    failed(
        status,
        error(
            &path,
            io::ErrorKind::Other,
            &format!("Copy worker failed: {message}"),
        ),
        cancel,
    )
}

fn initial(request: &CopyInput) -> CopyProgress {
    CopyProgress {
        id: request.id,
        state: CopyOperationState::Queued,
        phase: CopyOperationPhase::Planning,
        requested_roots: request.requested_roots(),
        files_completed: 0,
        directories_created: 0,
        skipped: 0,
        failed: 0,
        copied_bytes: 0,
        verified_bytes: 0,
        current_path: None,
        destination_directory: request.destination.clone(),
        message: String::new(),
        errors: vec![],
    }
}

#[cfg(test)]
#[path = "copy_tests.rs"]
mod tests;
