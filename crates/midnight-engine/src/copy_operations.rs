//! Bounded independent transfers with admission-ordered opt-in waiting.
//! Each active job retains its I/O slot identity and complete native namespace claim
//! through checked teardown; the slot consumes capacity only while the job executes
//! native steps. Within-job file parallelism remains separately gated.
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
#[path = "copy_submission.rs"]
mod submission;
use crate::copy_services::CopyServices;
use crate::transfer_composition::QualifiedTransferInstallation;
use crate::transfer_leases::IoBudget;
use crate::worker_retirement::{ActorRetirement, WorkerOwned};
#[path = "copy_delivery.rs"]
mod delivery;
use delivery::CopyOutbox;
#[path = "copy_ownership.rs"]
mod ownership;
use decisions::{DecisionWaiter, prepare_job, retryable_error};
use ownership::{Owned, Scope};
#[path = "copy_worker.rs"]
mod worker;
pub(crate) use admission::{Admission, Reservation};
use input::CopyInput;
/// Copy and Move compare every pinned native revision (sources, conflict pins and Move
/// captures) under this policy: a new change time alone (Windows ChangeTime, Linux ctime),
/// which endpoint protection gives freshly written files, is no change (VIEW-05). User
/// commands, checksum and file-list outputs and Rename keep exact revisions.
pub(crate) const TRANSFER_REVISIONS: TransferRevisionPolicy =
    TransferRevisionPolicy::AcceptChangeTimeOnly;

/// `TransferService::prepare_move_source` under `TRANSFER_REVISIONS`.
pub(crate) fn prepare_ordinary_move_source(
    service: &dyn TransferService,
    path: &Path,
    cancel: &Cancellation,
) -> Result<PreparedTransferMove, FsError> {
    match service.prepare_move_source_with_policy(
        path,
        CopyLinkPolicy::Reject,
        TRANSFER_REVISIONS,
        cancel,
    )? {
        PreparedTransferMoveEntry::Ordinary(prepared) => Ok(prepared),
        PreparedTransferMoveEntry::DirectoryLink { .. } => Err(FsError::from_io(
            "prepare Move source",
            path,
            io::Error::new(
                io::ErrorKind::InvalidData,
                "The provider prepared a directory link for an ordinary Move.",
            ),
        )),
    }
}
/// Whether `command` is a Copy-family command, from the same list guarded admission
/// uses for its operation ID (`input::copy_command_id`).
pub(crate) fn is_copy_command(command: &Command) -> bool {
    input::copy_command_id(command).is_some()
}
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
    events: CopyOutbox,
}
#[derive(Default)]
struct PauseRequests {
    manual: bool,
    latest_token: u64,
    confirmation: Option<u64>,
}

/// Combines the controls for one admitted job under the same short lock.
/// The job's own token remains the sole authority for checkpoints and cancellation.
struct CopyPause {
    cancel: Cancellation,
    requests: Mutex<PauseRequests>,
}

impl CopyPause {
    fn new(cancel: Cancellation) -> Self {
        Self {
            cancel,
            requests: Mutex::default(),
        }
    }

    fn set_manual(&self, paused: bool) {
        let mut requests = self
            .requests
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        requests.manual = paused;
        self.apply_requests(&requests);
    }

    fn confirm(&self, token: u64, action: CopyCancelConfirmationAction) -> bool {
        use CopyCancelConfirmationAction::{Begin, Confirm, Dismiss};

        let mut requests = self
            .requests
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if self.cancel.is_cancelled() {
            return false;
        }
        match action {
            Begin if token > requests.latest_token => {
                requests.latest_token = token;
                requests.confirmation = Some(token);
            }
            Dismiss if requests.confirmation == Some(token) => {
                requests.confirmation = None;
            }
            Confirm if requests.confirmation == Some(token) && self.cancel.is_paused() => {
                self.cancel.cancel();
                requests.confirmation = None;
            }
            _ => return false,
        }
        self.apply_requests(&requests);
        true
    }

    fn cancel(&self) {
        let mut requests = self
            .requests
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        self.cancel.cancel();
        requests.confirmation = None;
        self.apply_requests(&requests);
    }

    fn ready_confirmation(&self) -> Option<u64> {
        let requests = self
            .requests
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        requests.confirmation.filter(|_| self.cancel.is_paused())
    }

    fn apply_requests(&self, requests: &PauseRequests) {
        self.cancel.set_pause_requested(
            !self.cancel.is_cancelled() && (requests.manual || requests.confirmation.is_some()),
        );
    }
}

type Request = Owned<Input>;
pub(crate) struct Admitted {
    request: Owned<submission::Submission>,
    sender: tokio::sync::oneshot::Sender<Request>,
}
struct Input {
    command: CopyInput,
    ordered_id: bool,
    cancel: Cancellation,
    status: Status,
    decisions: mpsc::Receiver<CopyDecision>,
    waiting: Arc<AtomicU64>,
    initial: CopyProgress,
    events: CopyOutbox,
}

struct InputRegistration {
    ordered_id: bool,
    cancel: Cancellation,
    status: Status,
    decisions: mpsc::Receiver<CopyDecision>,
    waiting: Arc<AtomicU64>,
    initial: CopyProgress,
    events: CopyOutbox,
}
impl InputRegistration {
    fn attach(self, command: CopyInput) -> Input {
        Input {
            command,
            ordered_id: self.ordered_id,
            cancel: self.cancel,
            status: self.status,
            decisions: self.decisions,
            waiting: self.waiting,
            initial: self.initial,
            events: self.events,
        }
    }
}

/// Job controls (cancel, pause, resume) and engine shutdown as an awaitable signal.
/// Cancellation and pause stay plain flags; the controller notifies after every change
/// and each wait re-checks its own flags, so no job polls while it waits. A slow
/// re-check guards against a flag changed without a notification.
#[derive(Clone, Default)]
pub(crate) struct Controls(Arc<tokio::sync::Notify>);
const CONTROL_RECHECK: Duration = Duration::from_secs(1);
impl Controls {
    pub(crate) fn changed(&self) {
        self.0.notify_waiters();
    }
    /// Completes once `stop()` holds; it is re-checked after every control change.
    pub(crate) async fn until(&self, stop: impl Fn() -> bool) {
        loop {
            let notified = self.0.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if stop() {
                return;
            }
            let _ = tokio::time::timeout(CONTROL_RECHECK, notified).await;
        }
    }
}

pub(crate) struct Controller {
    admission: Admission,
    outbox: Outbox,
    registry: Registry,
    cancel: Cancellation,
    controls: Controls,
    highest: AtomicU64,
    #[cfg(test)]
    namespace: crate::copy_namespace::Registry,
    #[cfg(test)]
    retirement: tokio::sync::Mutex<crate::retirement::RetirementWaiter>,
}
impl Controller {
    #[cfg(test)]
    pub(crate) fn new(
        retirement: ActorRetirement,
        service: Option<QualifiedTransferInstallation>,
        outbox: Outbox,
        mutation_gate: Arc<Semaphore>,
        io_budget: Arc<IoBudget>,
        tasks: &mut JoinSet<()>,
    ) -> Self {
        let admission = Admission::new(retirement.domain.clone());
        Self::with_admission(
            retirement,
            admission,
            service,
            outbox,
            mutation_gate,
            io_budget,
            tasks,
        )
    }
    pub(crate) fn with_admission(
        retirement: ActorRetirement,
        admission: Admission,
        service: Option<QualifiedTransferInstallation>,
        outbox: Outbox,
        mutation_gate: Arc<Semaphore>,
        io_budget: Arc<IoBudget>,
        tasks: &mut JoinSet<()>,
    ) -> Self {
        let ActorRetirement {
            domain: _,
            intake,
            service: service_lease,
        } = retirement;
        let service = service_lease.own(CopyServices::new(service));
        #[cfg(not(test))]
        drop(intake);
        let registry = Registry::default();
        let cancel = admission.cancellation();
        let namespace = crate::copy_namespace::Registry::new_with_io(io_budget.gate().clone());
        #[cfg(test)]
        let retained_namespace = namespace.clone();
        tasks.spawn(run(
            admission.consumer(),
            service,
            outbox.clone(),
            registry.clone(),
            cancel.clone(),
            mutation_gate,
            io_budget,
            namespace,
        ));
        Self {
            admission,
            outbox,
            registry,
            cancel,
            controls: Controls::default(),
            highest: AtomicU64::new(0),
            #[cfg(test)]
            namespace: retained_namespace,
            #[cfg(test)]
            retirement: tokio::sync::Mutex::new(intake),
        }
    }
    /// The actor's namespace registry, for tests that steer granted claims.
    #[cfg(test)]
    pub(crate) fn namespace(&self) -> &crate::copy_namespace::Registry {
        &self.namespace
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
    #[cfg(test)]
    pub(crate) fn pending_slots(&self) -> usize {
        self.admission.pending_slots()
    }
    #[cfg(test)]
    pub(crate) async fn enqueue(&self, request: CopyRequest) {
        self.enqueue_input(request.into()).await;
    }
    #[cfg(test)]
    pub(crate) async fn enqueue_selected(&self, request: SelectedCopyRequest) {
        self.enqueue_input(request.into()).await;
    }
    #[cfg(test)]
    pub(crate) async fn enqueue_move(&self, request: CopyRequest) {
        self.enqueue_input(CopyInput::move_paths(request)).await;
    }
    #[cfg(test)]
    pub(crate) async fn enqueue_clipboard_move(&self, request: ClipboardMoveRequest) {
        self.enqueue_input(CopyInput::move_clipboard(request)).await;
    }
    #[cfg(test)]
    pub(crate) async fn enqueue_metadata(&self, request: MetadataTransferRequest) {
        self.enqueue_input(CopyInput::metadata(request)).await;
    }
    #[cfg(test)]
    pub(crate) async fn enqueue_options(&self, request: CopyOptionsRequest) {
        self.enqueue_input(CopyInput::options(request)).await;
    }
    #[cfg(test)]
    pub(crate) async fn enqueue_captured(&self, request: CapturedTransferRequest) {
        self.enqueue_input(request.into()).await;
    }
    #[cfg(test)]
    pub(crate) async fn enqueue_archive(&self, request: ArchiveCopyRequest) {
        self.enqueue_input(request.into()).await;
    }
    #[cfg(test)]
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
                publish_rejected(&self.outbox, progress).await;
                return;
            }
        };
        let (request, sender) = slot.attach(request);
        self.enqueue_input_reserved(request, sender).await;
    }
    #[cfg(test)]
    pub(crate) async fn enqueue_reserved(&self, request: CopyRequest, capacity: Reservation) {
        if let Ok((request, sender)) = capacity.attach(request.into()) {
            self.enqueue_input_reserved(request, sender).await;
        }
    }
    pub(crate) async fn enqueue_drop_reserved(
        &self,
        request: CopyRequest,
        intent: TransferIntent,
        capacity: Reservation,
    ) {
        let mut input = CopyInput::from(request);
        input.intent = intent;
        input._completions = capacity.completions();
        let (input, sender) = capacity.attach_input(input);
        self.enqueue_input_reserved(input, sender).await;
    }
    pub(crate) fn enqueue_admitted(&self, admitted: Admitted) {
        self.register_input(
            ownership::convert_submission(admitted.request),
            admitted.sender,
        );
    }
    async fn enqueue_input_reserved(
        &self,
        request: Owned<CopyInput>,
        sender: tokio::sync::oneshot::Sender<Request>,
    ) {
        self.register_input(request, sender);
    }
    fn register_input(
        &self,
        mut request: Owned<CopyInput>,
        sender: tokio::sync::oneshot::Sender<Request>,
    ) {
        // Cache the accepted-order relation without taking over a live ID's
        // controls. Its diagnostic and teardown belong to the bounded runner.
        let ordered_id = request.id.0 > self.highest.fetch_max(request.id.0, Ordering::Relaxed);
        // No foreign intent/root-count getter is called until the bounded worker
        // owns this registration. Its placeholder has no publication permission.
        let mut progress =
            CopyProgress::queued(request.id, request.destination.clone(), 0, request.intent);
        progress.verification = request.verification;
        let cancel = request
            .resources_mut()
            .pending_cancel()
            .unwrap_or_else(|| self.cancel.pausable_child());
        let status = Arc::new(Mutex::new(progress.clone()));
        request.resources_mut().attach_status(status.clone());
        let events = CopyOutbox::pending_with_shutdown(
            self.outbox.clone(),
            self.cancel.clone(),
            self.controls.clone(),
            cancel.clone(),
        );
        let waiting = Arc::new(AtomicU64::new(0));
        let (decisions, decision_receiver) = mpsc::channel(8);
        if ordered_id {
            let mut registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
            // Every entry already owns one of the same actual32 admissions.
            registry.insert(
                request.id,
                Registered {
                    cancel: cancel.clone(),
                    status: status.clone(),
                    decisions,
                    waiting: waiting.clone(),
                    events: events.clone(),
                },
            );
        }
        let request = ownership::register_input(
            request,
            InputRegistration {
                ordered_id,
                cancel,
                status,
                decisions: decision_receiver,
                waiting,
                initial: progress.clone(),
                events,
            },
        );
        match sender.send(request) {
            // The consumer rescans now instead of on a timer.
            Ok(()) => self.admission.wake(),
            Err(rejected) => {
                drop(rejected);
                if ordered_id {
                    self.registry
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .remove(&progress.id);
                }
                // A closed consumer is Engine shutdown, so no output publication may
                // delay controls. The sealed input still retires independently.
            }
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
        let (progress, events) = {
            let registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
            let Some(job) = registry.get(&id) else { return };
            let mut progress = job.status.lock().unwrap_or_else(|e| e.into_inner());
            if job.cancel.is_cancelled() || progress.state.is_terminal() {
                return;
            }
            job.events.pause.set_manual(paused);
            progress.pause_requested = job.cancel.pause_requested();
            progress.paused = job.cancel.is_paused();
            (progress.clone(), job.events.clone())
        };
        self.controls.changed();
        publish(&events, progress).await;
    }
    pub(crate) async fn cancel_confirmation(
        &self,
        id: OperationId,
        token: u64,
        action: CopyCancelConfirmationAction,
    ) {
        let (progress, events) = {
            let registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
            let Some(job) = registry.get(&id) else { return };
            let mut progress = job.status.lock().unwrap_or_else(|e| e.into_inner());
            if progress.state.is_terminal() || !job.events.pause.confirm(token, action) {
                return;
            }
            if job.cancel.is_cancelled() {
                job.waiting.store(0, Ordering::Release);
                mark_cancel_requested(&mut progress);
            } else {
                progress.pause_requested = job.cancel.pause_requested();
                progress.paused = job.cancel.is_paused();
            }
            (progress.clone(), job.events.clone())
        };
        self.controls.changed();
        publish(&events, progress).await;
    }
    pub(crate) fn cancel(&self, id: OperationId) {
        self.admission.cancel_pending(id);
        self.cancel_registered(id);
        // Waiting jobs re-check their flags now instead of polling for them.
        self.controls.changed();
    }
    fn cancel_registered(&self, id: OperationId) {
        let registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(job) = registry.get(&id) {
            job.events.pause.cancel();
            job.waiting.store(0, Ordering::Release);
            let mut progress = job.status.lock().unwrap_or_else(|e| e.into_inner());
            if progress.state == CopyOperationState::CancelRequested || progress.state.is_terminal()
            {
                return;
            }
            mark_cancel_requested(&mut progress);
            if job
                .events
                .sender
                .try_send(Event::Copy(CopyEvent::Progress(Arc::new(progress.clone()))))
                .is_ok()
            {
                (self.outbox.wakeup)();
            }
        }
    }
    pub(crate) fn shutdown(&self) {
        self.admission.shutdown();
        self.controls.changed();
    }
}
fn mark_cancel_requested(progress: &mut CopyProgress) {
    progress.pause_requested = false;
    progress.paused = false;
    progress.cancel_confirmation_ready = None;
    progress.state = CopyOperationState::CancelRequested;
    progress.wait_reason = None;
    crate::operation_throughput::stop(&mut progress.throughput);
    crate::operation_throughput::stop(&mut progress.verification_throughput);
}

impl Drop for Controller {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "The actor owns each shared lease source separately; a bundle would only rename them."
)]
async fn run(
    consumer: admission::Consumer,
    service: WorkerOwned<CopyServices>,
    outbox: Outbox,
    registry: Registry,
    cancel: Cancellation,
    mutation_gate: Arc<Semaphore>,
    io_budget: Arc<IoBudget>,
    namespace: crate::copy_namespace::Registry,
) {
    let service = Arc::new(service);
    let requests = consumer.admission();
    let mut jobs = JoinSet::new();
    loop {
        tokio::select! {
            request = requests.next(&cancel), if jobs.len() < MAX_JOBS => {
                let Some(request) = request else { break; };
                jobs.spawn(run_accepted(request, service.clone(), registry.clone(),
                    mutation_gate.clone(), io_budget.clone(), namespace.clone()));
            },
            _ = jobs.join_next(), if !jobs.is_empty() => {},
        }
    }
    // Closing intake retires filled and empty reservations independently of
    // output capacity. Never abort a native call when cancelling the actor.
    drop(consumer);
    while jobs.join_next().await.is_some() {}
    drop(outbox);
}

async fn run_accepted(
    mut request: Request,
    service: Arc<WorkerOwned<CopyServices>>,
    registry: Registry,
    mutation_gate: Arc<Semaphore>,
    io_budget: Arc<IoBudget>,
    namespace: crate::copy_namespace::Registry,
) {
    let id = request.command.id;
    let registered = request.ordered_id;
    let fallback = request.initial.clone();
    let events = request.events.clone();
    let status = request.status.clone();
    // This is the SAME original admission permit, containing no provider. An
    // undelivered terminal cannot create unbounded completed delivery tasks.
    let _delivery_capacity = request.resources_mut().delivery_tail();
    if !registered && id.0 != 0 {
        // Replayed IDs are presentation aliases of an existing operation. Keep
        // the original replay protection: retire on the bounded worker without
        // publishing any initial/terminal event against that original row.
        if let Err(error) =
            tokio::task::spawn_blocking(move || ownership::retire_confirmed(request)).await
        {
            std::mem::forget(error);
        }
        return;
    }
    // Captured accessors and all fallible validation are protected by the same
    // original seal on a bounded worker, before first publication or I/O leases.
    let initialized = tokio::task::spawn_blocking(move || {
        let result = ownership::run(&mut request, |request| {
            let mut progress = if request.ordered_id {
                request.command.initialize_captured_intent();
                initial(&request.command)
            } else {
                request.initial.clone()
            };
            if request.cancel.is_cancelled() {
                progress.state = CopyOperationState::CancelRequested;
            }
            let failure = if !request.ordered_id {
                Some(error(
                    &request.command.destination,
                    io::ErrorKind::InvalidInput,
                    "The operation ID must be positive and greater than earlier accepted IDs.",
                ))
            } else {
                request.command.validate().err().or_else(|| {
                    (!request.command.destination.is_absolute()
                        || request
                            .command
                            .destination
                            .as_os_str()
                            .as_encoded_bytes()
                            .contains(&0))
                    .then(|| {
                        error(
                            &request.command.destination,
                            io::ErrorKind::InvalidInput,
                            "The destination must be an absolute native directory path.",
                        )
                    })
                })
            };
            if failure.is_none()
                && let Some(roots) = request.command.normalize_copy_roots()
            {
                progress.requested_roots = roots;
            }
            *request
                .status
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = progress.clone();
            request.initial = progress.clone();
            (progress, failure)
        });
        (request, result)
    })
    .await;
    let (request, initial, failure) = match initialized {
        Ok((request, Ok((initial, failure)))) => (Some(request), initial, failure),
        Ok((request, Err(_))) => {
            drop(request);
            (None, fallback, None)
        }
        Err(error) => {
            std::mem::forget(error);
            (None, fallback, None)
        }
    };
    let (terminal, finished) = tokio::sync::oneshot::channel();
    let failure_initial = initial.clone();
    let work = async {
        let progress = match (request, failure) {
            (Some(request), Some(failure)) => {
                let progress = rejected_progress(failure_initial, failure);
                retire_input(request, progress, &status).await
            }
            (Some(request), None) => {
                run_input(
                    request,
                    &service,
                    &events,
                    &io_budget,
                    &mutation_gate,
                    &namespace,
                )
                .await
            }
            (None, _) => retirement_unconfirmed(&status),
        };
        if registered {
            registry
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .remove(&id);
        }
        let _ = terminal.send(progress);
    };
    tokio::join!(work, events.clone().deliver(initial, finished));
}

fn set_resource_wait(
    status: &Status,
    tracker: Option<&Mutex<Tracker>>,
    reason: Option<TransferWaitReason>,
) {
    let mut progress = status.lock().unwrap_or_else(|e| e.into_inner());
    progress.wait_reason = reason;
    if let Some(tracker) = tracker {
        tracker
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .copy(&mut progress);
    }
}

async fn run_input(
    request: Owned<Input>,
    service: &CopyServices,
    outbox: &CopyOutbox,
    io_budget: &Arc<IoBudget>,
    mutation_gate: &Arc<Semaphore>,
    namespace: &crate::copy_namespace::Registry,
) -> CopyProgress {
    let status = request.status.clone();
    let cancel = request.cancel.clone();
    let throughput = Arc::new(Mutex::new(Tracker::default()));
    let pending = execute_input(
        request,
        service,
        outbox,
        io_budget,
        mutation_gate,
        namespace,
        &throughput,
    );
    with_progress_cadence(pending, &status, &cancel, &throughput, outbox).await
}

/// Await the job's terminal state, publishing its live status every `CADENCE` until
/// then. Checked teardown stores that terminal state in `status` before `pending`
/// returns it; only delivery publishes it (`publish_intermediate`).
async fn with_progress_cadence(
    pending: impl std::future::Future<Output = CopyProgress>,
    status: &Status,
    cancel: &Cancellation,
    throughput: &Mutex<Tracker>,
    outbox: &CopyOutbox,
) -> CopyProgress {
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
                refresh_pause(status, cancel);
                let progress = sample_throughput(status, throughput);
                publish_progress_tick(outbox, status, progress);
            },
        }
    }
}

fn publish_progress_tick(outbox: &CopyOutbox, status: &Status, sampled: Option<CopyProgress>) {
    // A worker can already have sampled this interval. The timer must still
    // attempt to publish the current bytes, elapsed time and activity snapshot.
    publish_intermediate(outbox, sampled.unwrap_or_else(|| snapshot(status)));
}

/// Publish nonterminal progress if the output has room. A job's terminal state is
/// published once, by delivery after the job returns it (`CopyOutbox::deliver`).
/// Checked teardown stores it in the shared status earlier (`Completion::finish`), so
/// late controls see a finished job; republishing that status here would deliver the
/// terminal state twice.
fn publish_intermediate(outbox: &CopyOutbox, mut progress: CopyProgress) {
    if progress.state.is_terminal() {
        return;
    }
    // The regular cadence retries a ready token if the presentation queue is full.
    progress.cancel_confirmation_ready = progress
        .paused
        .then(|| outbox.pause.ready_confirmation())
        .flatten();
    if outbox
        .sender
        .try_send(Event::Copy(CopyEvent::Progress(Arc::new(progress))))
        .is_ok()
    {
        (outbox.wakeup)();
    }
}

fn sample_throughput(status: &Status, throughput: &Mutex<Tracker>) -> Option<CopyProgress> {
    let mut progress = status.lock().unwrap_or_else(|e| e.into_inner());
    let previous = progress.throughput.clone();
    let previous_verification = progress.verification_throughput.clone();
    throughput
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .copy(&mut progress);
    (!Arc::ptr_eq(&previous, &progress.throughput)
        || !Arc::ptr_eq(&previous_verification, &progress.verification_throughput))
    .then(|| progress.clone())
}

async fn execute_input(
    mut request: Owned<Input>,
    service: &CopyServices,
    outbox: &CopyOutbox,
    io_budget: &Arc<IoBudget>,
    mutation_gate: &Arc<Semaphore>,
    namespace: &crate::copy_namespace::Registry,
    throughput: &Arc<Mutex<Tracker>>,
) -> CopyProgress {
    let status = request.status.clone();
    let cancel = request.cancel.clone();
    let waiting = request.waiting.clone();
    let predecessors = request.resources_mut().take_predecessors();
    if request.command.start_policy == TransferStartPolicy::WaitForEarlier
        && !predecessors.is_finished()
    {
        set_resource_wait(
            &status,
            Some(throughput),
            Some(TransferWaitReason::EarlierTransfers),
        );
        let finished = loop {
            if !wait_for_pause(&cancel, &status, outbox, throughput, None).await {
                break false;
            }
            if let Some(finished) = interruptible(
                predecessors.wait(&cancel, &outbox.controls),
                &cancel,
                outbox,
            )
            .await
            {
                break finished;
            }
        };
        set_resource_wait(&status, Some(throughput), None);
        if !finished {
            return retire_input(request, cancelled(&status), &status).await;
        }
    }
    drop(predecessors);
    if !wait_for_pause(&cancel, &status, outbox, throughput, None).await {
        return retire_input(request, cancelled(&status), &status).await;
    }
    let Some(installation) = service.transfer() else {
        let failure = error(
            &request.command.destination,
            io::ErrorKind::Unsupported,
            "Verified copying is unavailable for this filesystem.",
        );
        return retire_input(request, failed(&status, failure, &cancel), &status).await;
    };
    let Some(witness) = installation.witness() else {
        let failure = error(
            &request.command.destination,
            io::ErrorKind::Unsupported,
            "The transfer service lacks an audited native retirement composition.",
        );
        return retire_input(request, failed(&status, failure, &cancel), &status).await;
    };
    let checked = tokio::task::spawn_blocking(move || {
        let checked = ownership::run(&mut request, |request| {
            request.command.native_children_are_audited()
        });
        (request, checked)
    })
    .await;
    request = match checked {
        Ok((request, Ok(true))) => request,
        Ok((request, Ok(false))) => {
            let failure = error(
                &request.command.destination,
                io::ErrorKind::Unsupported,
                "Captured sources do not supply the audited native-child composition.",
            );
            return retire_input(request, failed(&status, failure, &cancel), &status).await;
        }
        Ok((request, Err(_))) => {
            drop(request);
            return retirement_unconfirmed(&status);
        }
        Err(error) => {
            std::mem::forget(error);
            return retirement_unconfirmed(&status);
        }
    };
    // Wait-for-earlier is complete before either lifetime lease is attempted.
    let leases = match acquire_leases(
        io_budget.gate(),
        mutation_gate,
        &cancel,
        &status,
        outbox,
        throughput,
    )
    .await
    {
        Ok(Some(leases)) => leases,
        Ok(None) => return retire_input(request, cancelled(&status), &status).await,
        Err(error) => {
            return retire_input(request, failed(&status, error, &cancel), &status).await;
        }
    };
    let io = leases.io.clone();
    let resources = request.resources_mut();
    resources.mutation = Some(leases.namespace);
    if let Err(error) = resources.attach_io(io_budget.gate().clone(), leases.io) {
        return retire_input(request, failed(&status, error, &cancel), &status).await;
    }
    let sequence = request.resources_mut().admission_sequence();
    let (claim, mut native) = match namespace.new_job(sequence) {
        Ok(owners) => owners,
        Err(error) => return retire_input(request, failed(&status, error, &cancel), &status).await,
    };
    native.qualify(witness);
    if let Err(error) = request.resources_mut().install_namespace(claim, native) {
        return retire_input(request, failed(&status, error, &cancel), &status).await;
    }
    let mut decisions = None;
    let command = ownership::map(request, |input| {
        decisions = Some(input.decisions);
        input.command
    });
    let command = match command {
        Ok(command) => command,
        Err(_) => return retirement_unconfirmed(&status),
    };
    let mut decisions = decisions.expect("admitted Input owns its decision channel");
    let mut waiter = DecisionWaiter {
        status: &status,
        cancel: &cancel,
        waiting: &waiting,
        decisions: &mut decisions,
        outbox,
        throughput,
        io: &io,
    };
    if !ready_for_io(&io, &cancel, &status, outbox, throughput).await {
        return retire_input(command, cancelled(&status), &status).await;
    }
    let mut job = match prepare_job(command, installation.clone(), &mut waiter).await {
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
    let watch = job
        .resources_mut()
        .namespace()
        .map(crate::copy_namespace::Claim::watch);
    loop {
        if !ready_for_io(&io, &cancel, &status, outbox, throughput).await {
            break;
        }
        // Cancellation cannot abort an OS call or relinquish its permit early.
        let executed = tokio::task::spawn_blocking(move || {
            let step = ownership::run(&mut job, |job| {
                let step = job.step();
                let completed = job.take_completed_root();
                (step, completed, job.take_namespace_busy())
            });
            (job, step)
        })
        .await;
        let (step, completed, busy);
        match executed {
            Ok((returned, Ok(result))) => {
                job = returned;
                (step, completed, busy) = result;
            }
            Ok((returned, Err(_))) => {
                drop(returned);
                return retirement_unconfirmed(&status);
            }
            Err(error) => {
                std::mem::forget(error);
                return retirement_unconfirmed(&status);
            }
        };
        if let Some(source) = completed {
            let id = snapshot(&status).id;
            announce_root(outbox, &io, CopyEvent::RootCompleted { id, source }).await;
        }
        if let Some(blockers) = busy {
            // Granted peers refused this change: only this item waits (holding no disk
            // slot) until one of them changes, then the step repeats instead of asking.
            // This job's own turns never end the wait.
            io.release();
            set_resource_wait(
                &status,
                Some(throughput),
                Some(TransferWaitReason::SameFolders),
            );
            match &watch {
                Some(watch) => wait_for_peers(watch, &blockers, &cancel, outbox).await,
                None => tokio::time::sleep(NAMESPACE_RETRY_SPACING).await,
            }
            set_resource_wait(&status, Some(throughput), None);
            continue;
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
    // The namespace share and complete claim are still inside this job. Checked
    // teardown reclaims a released disk slot without waiting (cancellation cannot
    // queue behind other jobs) and never releases native ownership before the
    // outstanding call returns.
    match tokio::task::spawn_blocking(move || ownership::finish(job)).await {
        Ok(Ok(progress)) => progress,
        Ok(Err(_)) => retirement_unconfirmed(&status),
        Err(error) => {
            std::mem::forget(error);
            retirement_unconfirmed(&status)
        }
    }
}

/// Announce a root the job completed. An archive copy can complete more roots than the
/// notice queue holds (`MAX_NATIVE_TRANSFER_ROOTS` and two more), and once the UI takes
/// no notices the announcement waits for room. No native call is outstanding here, so
/// the disk slot goes back first, as for a decision: other transfers and call-scoped
/// reads never wait for this job's UI, and the next step readmits it (`ready_for_io`).
async fn announce_root(outbox: &CopyOutbox, io: &midnight_work_budget::Permit, root: CopyEvent) {
    io.release();
    outbox.send(Event::Copy(root)).await;
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
#[cfg(test)]
thread_local! {
    /// Runs once between the pause check and the checkpoint (pause-race tests).
    static BEFORE_PAUSE_CHECKPOINT: std::cell::Cell<Option<fn(&Cancellation)>> =
        const { std::cell::Cell::new(None) };
}
/// Acknowledge a requested pause only without the disk slot: "paused" implies "no slot".
/// The slot is returned before the checkpoint, and again if a pause requested between the
/// two checks was acknowledged, so it is never published as paused while holding it.
fn pause_checkpoint(cancel: &Cancellation, io: Option<&midnight_work_budget::Permit>) -> bool {
    if let Some(io) = io
        && cancel.pause_requested()
    {
        io.release();
    }
    #[cfg(test)]
    if let Some(hook) = BEFORE_PAUSE_CHECKPOINT.with(std::cell::Cell::take) {
        hook(cancel);
    }
    let ready = cancel.pause_checkpoint();
    if !ready && let Some(io) = io {
        io.release();
    }
    ready
}
/// A safe checkpoint between native steps. A paused job returns its disk slot (`io`)
/// while it waits for resume or cancellation; `ready_for_io` readmits it before the next
/// step. False: cancelled.
async fn wait_for_pause(
    cancel: &Cancellation,
    status: &Status,
    outbox: &CopyOutbox,
    throughput: &Mutex<Tracker>,
    io: Option<&midnight_work_budget::Permit>,
) -> bool {
    loop {
        let ready = pause_checkpoint(cancel, io);
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
        outbox
            .controls
            .until(|| cancel.is_cancelled() || !cancel.pause_requested())
            .await;
    }
}

/// A wait for the disk slot shorter than this is another job's step, not shown as waiting.
const DISK_WAIT_SHOWN: Duration = Duration::from_millis(250);

/// Every native step runs under the job's own held disk slot. Before a step: wait while
/// paused (holding no slot), yield the slot while anything else is queued for it (another
/// transfer, or a checksum, thumbnail or folder-size read), and readmit it through the
/// FIFO gate. Jobs sharing the configured limit thus interleave step by step; none waits
/// for a whole job. Returns false when cancelled; checked teardown then reclaims the slot
/// without queuing.
async fn ready_for_io(
    io: &midnight_work_budget::Permit,
    cancel: &Cancellation,
    status: &Status,
    outbox: &CopyOutbox,
    throughput: &Mutex<Tracker>,
) -> bool {
    loop {
        if !wait_for_pause(cancel, status, outbox, throughput, Some(io)).await {
            return false;
        }
        if io.is_held() && io.is_contended() {
            io.release();
        }
        if io.try_reacquire() {
            return true;
        }
        let previous = snapshot(status).wait_reason;
        let reacquire = io.reacquire();
        tokio::pin!(reacquire);
        let queued = async {
            if tokio::time::timeout(DISK_WAIT_SHOWN, &mut reacquire)
                .await
                .is_err()
            {
                set_resource_wait(
                    status,
                    Some(throughput),
                    Some(TransferWaitReason::DiskCapacity),
                );
                (&mut reacquire).await;
            }
        };
        let admitted = interruptible(queued, cancel, outbox).await;
        set_resource_wait(status, Some(throughput), previous);
        if admitted.is_some() {
            return true;
        }
        if cancel.is_cancelled() {
            return false;
        }
        // A pause requested while queued withdrew the request; wait at the checkpoint.
    }
}

/// Await `future` unless the job is cancelled or paused first. Both are plain flags that
/// the controller announces through `Controls`; dropping the future withdraws it.
async fn interruptible<F: std::future::Future>(
    future: F,
    cancel: &Cancellation,
    outbox: &CopyOutbox,
) -> Option<F::Output> {
    let interrupted = outbox
        .controls
        .until(|| cancel.is_cancelled() || cancel.pause_requested());
    tokio::select! {
        biased;
        () = interrupted => None,
        output = future => Some(output),
    }
}

/// Refused namespace work never retries faster than this, whatever peers change.
const NAMESPACE_RETRY_SPACING: Duration = Duration::from_millis(25);
/// Fact changes and external renames, which do not signal the registry, are revalidated
/// at least this often while a job waits for a namespace grant or change.
const NAMESPACE_REVALIDATE: Duration = Duration::from_secs(1);

/// A job refused as a whole waits for the registry instead of polling: until a claim is
/// granted, removed or quarantined after `since`. The caller holds no disk slot here;
/// cancellation and pause end the wait at once.
async fn wait_for_namespace(
    watch: &crate::copy_namespace::Watch,
    since: crate::copy_namespace::Seen,
    cancel: &Cancellation,
    outbox: &CopyOutbox,
) {
    let started = tokio::time::Instant::now();
    let changed = interruptible(watch.changed(since, true), cancel, outbox);
    let _ = tokio::time::timeout(NAMESPACE_REVALIDATE, changed).await;
    if !cancel.is_cancelled() && !cancel.pause_requested() {
        tokio::time::sleep_until(started + NAMESPACE_RETRY_SPACING).await;
    }
}

/// A namespace change refused by granted peers waits until one of them changes (or is
/// removed); the refused job's own turns never end it. External changes that the
/// registry cannot see are revalidated once per second, except against peers latched
/// opaque, which clear only by their removal. No disk slot is held here.
async fn wait_for_peers(
    watch: &crate::copy_namespace::Watch,
    blockers: &crate::copy_namespace::Blockers,
    cancel: &Cancellation,
    outbox: &CopyOutbox,
) {
    let started = tokio::time::Instant::now();
    let changed = interruptible(watch.blockers_changed(blockers), cancel, outbox);
    if blockers.can_clear_in_place() {
        let _ = tokio::time::timeout(NAMESPACE_REVALIDATE, changed).await;
    } else {
        changed.await;
    }
    if !cancel.is_cancelled() && !cancel.pause_requested() {
        tokio::time::sleep_until(started + NAMESPACE_RETRY_SPACING).await;
    }
}

/// Lifetime leases: the namespace share first, then the job's retained FIFO disk slot.
/// Legacy namespace owners take their complete namespace before any disk slot and no
/// disk-slot holder waits for a namespace share, so waiting for disk capacity while
/// holding a share cannot deadlock. A paused job takes neither lease. None: cancelled.
async fn acquire_leases(
    gate: &Arc<midnight_work_budget::Gate>,
    namespace: &Arc<Semaphore>,
    cancel: &Cancellation,
    status: &Status,
    outbox: &CopyOutbox,
    throughput: &Mutex<Tracker>,
) -> Result<Option<crate::transfer_leases::Leases>, FsError> {
    let closing = || {
        FsError::from_io(
            "acquire transfer namespace",
            Path::new(""),
            io::Error::new(io::ErrorKind::BrokenPipe, "Transfer namespace is closing."),
        )
    };
    let result = loop {
        if !wait_for_pause(cancel, status, outbox, throughput, None).await {
            break Ok(None);
        }
        let share = match namespace.clone().try_acquire_owned() {
            Ok(share) => share,
            Err(tokio::sync::TryAcquireError::Closed) => break Err(closing()),
            Err(tokio::sync::TryAcquireError::NoPermits) => {
                // Copies take one share each; only a whole-namespace operation (Delete,
                // Rename, a new folder, a saved list) holds or queues for the rest, and
                // the fair semaphore keeps a later copy behind it.
                set_resource_wait(
                    status,
                    Some(throughput),
                    Some(TransferWaitReason::OtherOperation),
                );
                match interruptible(namespace.clone().acquire_owned(), cancel, outbox).await {
                    Some(Ok(share)) => share,
                    Some(Err(_)) => break Err(closing()),
                    None if cancel.is_cancelled() => break Ok(None),
                    None => continue,
                }
            }
        };
        let admission = midnight_work_budget::Admission::Shared;
        let disk = match gate.try_acquire_retained(admission) {
            Some(disk) => disk,
            None => {
                // Running jobs yield their slots at their next step boundary.
                set_resource_wait(
                    status,
                    Some(throughput),
                    Some(TransferWaitReason::DiskCapacity),
                );
                match interruptible(gate.acquire_retained(admission), cancel, outbox).await {
                    Some(disk) => disk,
                    None if cancel.is_cancelled() => break Ok(None),
                    // Paused while queued: hold neither lease during the pause.
                    None => continue,
                }
            }
        };
        if cancel.is_cancelled() {
            // Admitted in the same instant as the cancellation: a cancelled job never
            // starts native work, so it returns both leases before retiring.
            drop((disk, share));
            break Ok(None);
        }
        break Ok(Some(crate::transfer_leases::Leases {
            io: Arc::new(disk),
            namespace: share,
        }));
    };
    set_resource_wait(status, Some(throughput), None);
    result
}

fn snapshot(status: &Status) -> CopyProgress {
    status.lock().unwrap_or_else(|e| e.into_inner()).clone()
}
fn set_state(status: &Status, state: CopyOperationState) {
    status.lock().unwrap_or_else(|e| e.into_inner()).state = state;
}
async fn publish(outbox: &CopyOutbox, progress: CopyProgress) {
    publish_intermediate(outbox, progress);
}
#[cfg(test)]
async fn publish_rejected(outbox: &Outbox, mut progress: CopyProgress) {
    if progress.state.is_terminal() {
        progress.pause_requested = false;
        progress.paused = false;
        progress.wait_reason = None;
        progress.verification_pass_id = 0;
        progress.verification_throughput = Arc::default();
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
    progress.wait_reason = None;
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
async fn retire_input<T: crate::copy_nested_retirement::CheckedRetirement + 'static>(
    value: Owned<T>,
    mut progress: CopyProgress,
    status: &Status,
) -> CopyProgress {
    let before = snapshot(status);
    match tokio::task::spawn_blocking(move || ownership::retire_confirmed(value)).await {
        Ok(Ok(())) => {
            let after = snapshot(status);
            progress.failed = progress
                .failed
                .saturating_add(after.failed.saturating_sub(before.failed));
            let mut added = false;
            for error in after.errors {
                if !progress.errors.iter().any(|existing| {
                    existing.operation == error.operation
                        && existing.path == error.path
                        && existing.kind == error.kind
                        && existing.message == error.message
                }) {
                    if progress.errors.len() == MAX_ERROR_DETAILS {
                        progress.errors.remove(0);
                    }
                    progress.errors.push(error);
                    added = true;
                }
            }
            if added {
                let errors = progress
                    .errors
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("\n");
                if progress.message.is_empty() {
                    progress.message = errors;
                } else {
                    progress.message.push_str(&format!("\n{errors}"));
                }
            }
            progress
        }
        Ok(Err(_)) => retirement_unconfirmed(status),
        Err(error) => {
            std::mem::forget(error);
            retirement_unconfirmed(status)
        }
    }
}

fn retirement_unconfirmed(status: &Status) -> CopyProgress {
    let mut progress = snapshot(status);
    let failure = error(
        &progress.destination_directory,
        io::ErrorKind::Other,
        "Native cleanup could not be confirmed. Its transfer resources remain held.",
    );
    retain_error(&mut progress, failure.clone(), true);
    progress.state = CopyOperationState::Failed;
    progress.message = failure.to_string();
    progress.wait_reason = None;
    progress.pause_requested = false;
    progress.paused = false;
    progress.verification_pass_id = 0;
    progress.verification_throughput = Arc::default();
    crate::operation_throughput::stop(&mut progress.throughput);
    progress
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
#[path = "copy_retirement_diagnostics_tests.rs"]
mod retirement_diagnostics_tests;

#[cfg(test)]
#[path = "copy_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "copy_link_tests.rs"]
mod link_tests;

#[cfg(test)]
#[path = "copy_progress_tick_tests.rs"]
mod progress_tick_tests;
