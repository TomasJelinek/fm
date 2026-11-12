//! A bounded FIFO of verified clipboard copies, independent of pane navigation.
//! One job and one native I/O step run at a time on every storage class. Any future
//! parallel scheduler must positively identify both endpoints as local SSDs and
//! share physical-device budgets; a fast endpoint cannot relax the other endpoint.
use crate::Outbox;
use crate::operation_throughput::{CADENCE, Tracker};
use midnight_core::*;
use std::{
    collections::HashMap,
    io,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore, mpsc},
    task::JoinSet,
};

#[path = "copy_admission.rs"]
mod admission;
#[path = "archive_transfer.rs"]
pub(crate) mod archive_source;
#[path = "copy_decisions.rs"]
mod decisions;
#[path = "copy_input.rs"]
mod input;
#[path = "copy_preparation.rs"]
mod preparation;
use crate::worker_retirement::{ActorRetirement, WorkerOwned};
#[path = "copy_ownership.rs"]
mod ownership;
use decisions::{DecisionWaiter, prepare_job, retryable_error};
use ownership::{Owned, Scope};
#[path = "copy_worker.rs"]
mod worker;
pub(crate) use admission::{Admission, Reservation};
use input::CopyInput;
pub(crate) fn check_move_bounds<'a>(
    destination: &Path,
    paths: impl IntoIterator<Item = &'a Path>,
) -> Result<(), FsError> {
    worker::check_move_bounds(destination, paths)
}
type Status = Arc<Mutex<CopyProgress>>;
type Registry = Arc<Mutex<HashMap<OperationId, Registered>>>;
pub(crate) const MAX_JOBS: usize = 32;
const MAX_ERROR_DETAILS: usize = 16;
struct Registered {
    cancel: Cancellation,
    status: Status,
    decisions: mpsc::Sender<CopyDecision>,
    waiting: Arc<AtomicU64>,
}
type Request = Owned<Input>;
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
    retirement: tokio::sync::Mutex<crate::retirement::RetirementWaiter>,
}
impl Controller {
    pub(crate) fn new(
        retirement: ActorRetirement,
        service: Option<Arc<dyn TransferService>>,
        outbox: Outbox,
        mutation_gate: Arc<Semaphore>,
        io_budget: Arc<Semaphore>,
        tasks: &mut JoinSet<()>,
    ) -> Self {
        let ActorRetirement {
            domain,
            intake,
            service: service_lease,
        } = retirement;
        let service = service_lease.own(service);
        let admission = Admission::new(domain);
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
            retirement: tokio::sync::Mutex::new(intake),
        }
    }
    #[cfg(test)]
    pub(crate) fn capacity(&self) -> Arc<Semaphore> {
        self.admission.capacity()
    }
    pub(crate) fn admission(&self) -> Admission {
        self.admission.clone()
    }
    #[cfg(test)]
    pub(crate) fn reserve(&self) -> Result<Reservation, FsError> {
        self.admission.reserve()
    }
    pub(crate) async fn enqueue(&self, request: CopyRequest) {
        self.enqueue_input(request.into()).await;
    }
    pub(crate) async fn enqueue_selected(&self, request: SelectedCopyRequest) {
        self.enqueue_input(request.into()).await;
    }
    pub(crate) async fn enqueue_move(&self, request: CopyRequest) {
        self.enqueue_input(CopyInput::move_paths(request)).await;
    }
    pub(crate) async fn enqueue_selected_move(&self, request: SelectedCopyRequest) {
        self.enqueue_input(CopyInput::move_selection(request)).await;
    }
    pub(crate) async fn enqueue_clipboard_move(&self, request: ClipboardMoveRequest) {
        self.enqueue_input(CopyInput::move_clipboard(request)).await;
    }
    pub(crate) async fn enqueue_metadata(&self, request: MetadataTransferRequest) {
        self.enqueue_input(CopyInput::metadata(request)).await;
    }
    pub(crate) async fn enqueue_captured(&self, request: CapturedTransferRequest) {
        self.enqueue_input(request.into()).await;
    }
    pub(crate) async fn enqueue_archive(&self, request: ArchiveCopyRequest) {
        self.enqueue_input(request.into()).await;
    }
    async fn enqueue_input(&self, request: CopyInput) {
        let mut intake = self.retirement.lock().await;
        let lease = intake.reserve().await;
        let request = lease.own(request);
        let slot = match self.admission.reserve_slot() {
            Ok(slot) => slot,
            Err(error) => {
                if request.id.0 <= self.highest.fetch_max(request.id.0, Ordering::Relaxed) {
                    let _ = request.retire().await;
                    return;
                }
                let progress = rejected_progress(initial(&request), error);
                let _ = request.retire().await;
                publish(&self.outbox, progress).await;
                return;
            }
        };
        let (request, sender) = slot.attach(request);
        self.enqueue_input_reserved(request, sender).await;
    }
    #[cfg(test)]
    pub(crate) async fn enqueue_reserved(&self, request: CopyRequest, capacity: Reservation) {
        let (request, sender) = capacity.attach(request.into());
        self.enqueue_input_reserved(request, sender).await;
    }
    pub(crate) async fn enqueue_drop_reserved(
        &self,
        request: CopyRequest,
        intent: TransferIntent,
        retention: Option<Arc<crate::drag_drop::Retention>>,
        capacity: Reservation,
    ) {
        let mut input = CopyInput::from(request);
        input.intent = intent;
        input._retention = retention;
        let (input, sender) = capacity.attach(input);
        self.enqueue_input_reserved(input, sender).await;
    }
    async fn enqueue_input_reserved(
        &self,
        request: Owned<CopyInput>,
        sender: tokio::sync::oneshot::Sender<Request>,
    ) {
        if request.id.0 <= self.highest.fetch_max(request.id.0, Ordering::Relaxed) {
            let _ = request.retire().await;
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
            let _ = request.retire().await;
            publish(&self.outbox, rejected_progress(progress, error)).await;
            return;
        }
        let cancel = self.cancel.pausable_child();
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
            let _ = request.retire().await;
            publish(&self.outbox, rejected_progress(progress, failure)).await;
            return;
        }
        publish(&self.outbox, progress.clone()).await;
        let request = ownership::map(request, |command| Input {
            command,
            cancel,
            status,
            decisions: decision_receiver,
            waiting,
        });
        if let Err(rejected) = sender.send(request) {
            let _ = rejected.retire().await;
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
    pub(crate) async fn set_paused(&self, id: OperationId, paused: bool) {
        let progress = {
            let registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
            let Some(job) = registry.get(&id) else { return };
            let mut progress = job.status.lock().unwrap_or_else(|e| e.into_inner());
            if job.cancel.is_cancelled() || progress.state.is_terminal() {
                return;
            }
            job.cancel.set_pause_requested(paused);
            progress.pause_requested = job.cancel.pause_requested();
            progress.paused = job.cancel.is_paused();
            progress.clone()
        };
        publish(&self.outbox, progress).await;
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
            progress.pause_requested = false;
            progress.paused = false;
            progress.state = CopyOperationState::CancelRequested;
            crate::operation_throughput::stop(&mut progress.throughput);
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
    service: WorkerOwned<Option<Arc<dyn TransferService>>>,
    outbox: Outbox,
    registry: Registry,
    cancel: Cancellation,
    mutation_gate: Arc<Semaphore>,
    io_budget: Arc<Semaphore>,
) {
    let requests = consumer.admission();
    while let Some(request) = requests.next(&cancel).await {
        let id = request.command.id;
        let progress = run_job(request, &service, &outbox, &mutation_gate, &io_budget).await;
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
    service: &Option<Arc<dyn TransferService>>,
    outbox: &Outbox,
    mutation_gate: &Arc<Semaphore>,
    io_budget: &Arc<Semaphore>,
) -> CopyProgress {
    let mut input = request;
    let Some(mutation) = acquire(mutation_gate, &input.cancel).await else {
        let progress = cancelled(&input.status);
        let _ = input.retire().await;
        return progress;
    };
    input.resources.mutation = Some(mutation);
    run_input(input, service, outbox, io_budget).await
}

async fn run_input(
    request: Owned<Input>,
    service: &Option<Arc<dyn TransferService>>,
    outbox: &Outbox,
    io_budget: &Arc<Semaphore>,
) -> CopyProgress {
    let status = request.status.clone();
    let cancel = request.cancel.clone();
    let throughput = Mutex::new(Tracker::default());
    let pending = execute_input(request, service, outbox, io_budget, &throughput);
    tokio::pin!(pending);
    let mut timer = tokio::time::interval(CADENCE);
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            progress = &mut pending => {
                let mut progress = progress;
                progress.pause_requested = false;
                progress.paused = false;
                throughput.lock().unwrap_or_else(|e| e.into_inner()).copy(&mut progress);
                return progress;
            },
            _ = timer.tick() => {
                let changed = refresh_pause(&status, &cancel);
                let progress = sample_throughput(&status, &throughput)
                    .or_else(|| changed.then(|| snapshot(&status)));
                if let Some(progress) = progress
                    && outbox.sender.try_send(Event::Copy(CopyEvent::Progress(Arc::new(progress)))).is_ok()
                {
                    (outbox.wakeup)();
                }
            },
        }
    }
}

fn sample_throughput(status: &Status, throughput: &Mutex<Tracker>) -> Option<CopyProgress> {
    let mut progress = status.lock().unwrap_or_else(|e| e.into_inner());
    let previous = progress.throughput.clone();
    throughput
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .copy(&mut progress);
    (!Arc::ptr_eq(&previous, &progress.throughput)).then(|| progress.clone())
}

async fn execute_input(
    request: Owned<Input>,
    service: &Option<Arc<dyn TransferService>>,
    outbox: &Outbox,
    io_budget: &Arc<Semaphore>,
    throughput: &Mutex<Tracker>,
) -> CopyProgress {
    let (scope, lease) = request.into_parts();
    let Scope { payload, resources } = scope;
    let Input {
        command,
        cancel,
        status,
        mut decisions,
        waiting,
    } = payload;
    let command = lease.own(Scope {
        payload: command,
        resources,
    });
    let Some(service) = service else {
        let failure = error(
            &command.destination,
            io::ErrorKind::Unsupported,
            "Verified copying is unavailable for this filesystem.",
        );
        let progress = failed(&status, failure, &cancel);
        let _ = command.retire().await;
        return progress;
    };
    let mut waiter = DecisionWaiter {
        status: &status,
        cancel: &cancel,
        waiting: &waiting,
        decisions: &mut decisions,
        outbox,
        throughput,
    };
    if !wait_for_pause(&cancel, &status, outbox, throughput).await {
        let _ = command.retire().await;
        return cancelled(&status);
    }
    let command = ownership::map(command, |command| (command, service.clone()));
    let mut job = match prepare_job(command, io_budget, &mut waiter).await {
        Ok(job) => job,
        Err(progress) => return *progress,
    };
    set_state(
        &status,
        if cancel.is_cancelled() {
            CopyOperationState::CancelRequested
        } else {
            CopyOperationState::Running
        },
    );
    sample_throughput(&status, throughput);
    publish(outbox, snapshot(&status)).await;
    loop {
        if !wait_for_pause(&cancel, &status, outbox, throughput).await {
            break;
        }
        // Cancellation cannot abort an OS call or relinquish its permit early.
        let Some(permit) = acquire(io_budget, &cancel).await else {
            break;
        };
        // Pause may have arrived while waiting for the shared I/O permit.
        if !wait_for_pause(&cancel, &status, outbox, throughput).await {
            break;
        }
        let executed = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let step = job.step();
            let completed = job.take_completed_root();
            (job, step, completed)
        })
        .await;
        let (step, completed);
        (job, step, completed) = match executed {
            Ok(result) => result,
            Err(error) => return panicked(&status, error.to_string(), &cancel),
        };
        if let Some(source) = completed {
            outbox
                .send(Event::Copy(CopyEvent::RootCompleted {
                    id: snapshot(&status).id,
                    source,
                }))
                .await;
        }
        match step {
            worker::Step::Done => break,
            worker::Step::Continue => {}
            worker::Step::Conflict(conflict) => {
                waiter
                    .wait(CopyEvent::Conflict(conflict), |decision| {
                        job.accept_decision(decision)
                    })
                    .await;
            }
            worker::Step::Failure(failure) => {
                waiter
                    .wait(CopyEvent::Failure(failure), |decision| {
                        job.accept_decision(decision)
                    })
                    .await;
            }
        }
    }
    waiting.store(0, Ordering::Release);
    // Cleanup must run despite cancellation and on a worker, holding I/O capacity.
    let permit = io_budget.clone().acquire_owned().await.ok();
    match tokio::task::spawn_blocking(move || {
        let _permit = permit;
        ownership::finish(job)
    })
    .await
    {
        Ok(progress) => progress,
        Err(error) => panicked(&status, error.to_string(), &cancel),
    }
}

fn refresh_pause(status: &Status, cancel: &Cancellation) -> bool {
    let mut progress = status.lock().unwrap_or_else(|e| e.into_inner());
    let requested = !progress.state.is_terminal() && cancel.pause_requested();
    let paused = requested && cancel.is_paused();
    let changed = (progress.pause_requested, progress.paused) != (requested, paused);
    progress.pause_requested = requested;
    progress.paused = paused;
    changed
}
async fn wait_for_pause(
    cancel: &Cancellation,
    status: &Status,
    outbox: &Outbox,
    throughput: &Mutex<Tracker>,
) -> bool {
    loop {
        let ready = cancel.pause_checkpoint();
        let changed = refresh_pause(status, cancel);
        if changed || ready {
            sample_throughput(status, throughput);
        }
        if changed {
            publish(outbox, snapshot(status)).await;
        }
        if ready || cancel.is_cancelled() {
            return ready;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn snapshot(status: &Status) -> CopyProgress {
    status.lock().unwrap_or_else(|e| e.into_inner()).clone()
}
fn set_state(status: &Status, state: CopyOperationState) {
    status.lock().unwrap_or_else(|e| e.into_inner()).state = state;
}
async fn publish(outbox: &Outbox, mut progress: CopyProgress) {
    if progress.state.is_terminal() {
        progress.pause_requested = false;
        progress.paused = false;
    }
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
    let mut progress = snapshot(status);
    let cancelled = cancel.is_cancelled();
    progress.state = if cancelled {
        CopyOperationState::Cancelled
    } else {
        CopyOperationState::Failed
    };
    if (!cancelled || error.kind != io::ErrorKind::Interrupted)
        && !progress.errors.iter().any(|retained| {
            retained.operation == error.operation
                && retained.path == error.path
                && retained.kind == error.kind
                && retained.message == error.message
        })
    {
        retain_error(&mut progress, error, true);
    }
    // Metadata preflight can already have retained the exact cleanup diagnostic.
    // Preserve it once, including when cancellation arrives during retirement.
    progress.message = progress
        .errors
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    if cancelled {
        progress
            .message
            .insert_str(0, "Copy cancelled; source data was retained.\n");
    }
    progress
}

fn retain_error(progress: &mut CopyProgress, error: FsError, critical: bool) {
    progress.failed = progress.failed.saturating_add(1);
    if progress.errors.len() >= MAX_ERROR_DETAILS && critical {
        // The actionable current cleanup receipt takes priority over older
        // diagnostics. The terminal message discloses omitted details.
        progress.errors.remove(0);
    }
    if progress.errors.len() < MAX_ERROR_DETAILS {
        progress.errors.push(error);
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
    let mut progress = CopyProgress::queued(
        request.id,
        request.destination.clone(),
        request.requested_roots(),
        request.intent,
    );
    progress.verification = request.verification;
    progress
}

#[cfg(test)]
#[path = "copy_tests.rs"]
mod tests;
