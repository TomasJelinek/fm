//! Bounded independent transfers with admission-ordered opt-in waiting.
//! Each active job retains its real I/O slot and complete native namespace claim
//! through checked teardown. Within-job file parallelism remains separately gated.
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

pub(crate) struct Controller {
    admission: Admission,
    outbox: Outbox,
    registry: Registry,
    cancel: Cancellation,
    highest: AtomicU64,
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
            #[cfg(test)]
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
        let events = CopyOutbox::pending_with_shutdown(self.outbox.clone(), self.cancel.clone());
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
        if let Err(rejected) = sender.send(request) {
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
            job.cancel.set_pause_requested(paused);
            progress.pause_requested = job.cancel.pause_requested();
            progress.paused = job.cancel.is_paused();
            (progress.clone(), job.events.clone())
        };
        publish(&events, progress).await;
    }
    pub(crate) fn cancel(&self, id: OperationId) {
        self.admission.cancel_pending(id);
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
            progress.wait_reason = None;
            crate::operation_throughput::stop(&mut progress.throughput);
            crate::operation_throughput::stop(&mut progress.verification_throughput);
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
    }
}
impl Drop for Controller {
    fn drop(&mut self) {
        self.shutdown();
    }
}

async fn run(
    consumer: admission::Consumer,
    service: WorkerOwned<CopyServices>,
    outbox: Outbox,
    registry: Registry,
    cancel: Cancellation,
    mutation_gate: Arc<Semaphore>,
    io_budget: Arc<IoBudget>,
) {
    let service = Arc::new(service);
    let namespace = crate::copy_namespace::Registry::new_with_io(io_budget.gate().clone());
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

#[cfg(test)]
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

#[cfg(test)]
async fn acquire_resources(
    budget: &Arc<Semaphore>,
    cancel: &Cancellation,
    status: &Status,
    outbox: &CopyOutbox,
    tracker: Option<&Mutex<Tracker>>,
) -> Option<OwnedSemaphorePermit> {
    if cancel.is_cancelled() {
        return None;
    }
    match budget.clone().try_acquire_owned() {
        Ok(permit) => return Some(permit),
        Err(tokio::sync::TryAcquireError::Closed) => return None,
        Err(tokio::sync::TryAcquireError::NoPermits) => {}
    }
    set_resource_wait(status, tracker, Some(TransferWaitReason::Resources));
    let permit = if tracker.is_some() {
        // run_input owns this job's heartbeat and successfully delivered markers.
        // A second publisher can lose a full-outbox clear or double the cadence.
        acquire(budget, cancel).await
    } else {
        // Mutation admission precedes run_input, so it needs its own bounded retry.
        publish_wait_snapshot(outbox, status);
        let pending = acquire(budget, cancel);
        tokio::pin!(pending);
        let mut timer = tokio::time::interval_at(tokio::time::Instant::now() + CADENCE, CADENCE);
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                permit = &mut pending => break permit,
                _ = timer.tick() => publish_wait_snapshot(outbox, status),
            }
        }
    };
    set_resource_wait(status, tracker, None);
    if tracker.is_none() {
        publish_wait_snapshot(outbox, status);
    }
    permit
}

#[cfg(test)]
fn publish_wait_snapshot(outbox: &CopyOutbox, status: &Status) {
    // A full result queue must not prevent cancelled native ownership retiring.
    if outbox
        .sender
        .try_send(Event::Copy(CopyEvent::Progress(Arc::new(snapshot(status)))))
        .is_ok()
    {
        (outbox.wakeup)();
    }
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
                refresh_pause(&status, &cancel);
                let progress = sample_throughput(&status, &throughput);
                publish_progress_tick(outbox, &status, progress);
            },
        }
    }
}

fn publish_progress_tick(outbox: &CopyOutbox, status: &Status, sampled: Option<CopyProgress>) {
    // A worker can already have sampled this interval. The timer must still
    // attempt to publish the current bytes, elapsed time and activity snapshot.
    let progress = Arc::new(sampled.unwrap_or_else(|| snapshot(status)));
    let event = Event::Copy(CopyEvent::Progress(progress));
    if outbox.sender.try_send(event).is_ok() {
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
        let finished = predecessors.wait(&cancel).await;
        set_resource_wait(&status, Some(throughput), None);
        if !finished {
            return retire_input(request, cancelled(&status), &status).await;
        }
    }
    drop(predecessors);
    if !wait_for_pause(&cancel, &status, outbox, throughput).await {
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
    // Pair failure immediately rolls I/O back; legacy namespace owners can
    // obtain the same configured disk budget they need to finish.
    loop {
        if cancel.is_cancelled() {
            set_resource_wait(&status, Some(throughput), None);
            return retire_input(request, cancelled(&status), &status).await;
        }
        match crate::transfer_leases::try_pair(io_budget.gate(), mutation_gate) {
            Ok(Some(leases)) => {
                let resources = request.resources_mut();
                resources.mutation = Some(leases.namespace);
                if let Err(error) = resources.attach_io(io_budget.gate().clone(), leases.io) {
                    return retire_input(request, failed(&status, error, &cancel), &status).await;
                }
                break;
            }
            Ok(None) => {
                set_resource_wait(
                    &status,
                    Some(throughput),
                    Some(TransferWaitReason::Resources),
                );
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            Err(error) => {
                return retire_input(request, failed(&status, error, &cancel), &status).await;
            }
        }
    }
    set_resource_wait(&status, Some(throughput), None);
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
    };
    if !wait_for_pause(&cancel, &status, outbox, throughput).await {
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
    loop {
        if !wait_for_pause(&cancel, &status, outbox, throughput).await {
            break;
        }
        // Cancellation cannot abort an OS call or relinquish its permit early.
        let executed = tokio::task::spawn_blocking(move || {
            let step = ownership::run(&mut job, |job| {
                let step = job.step();
                let completed = job.take_completed_root();
                (step, completed)
            });
            (job, step)
        })
        .await;
        let (step, completed);
        match executed {
            Ok((returned, Ok(result))) => {
                job = returned;
                (step, completed) = result;
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
    // The original physical slot, namespace share and complete claim are still
    // inside this job. Cleanup never reacquires a resource or honors cancellation
    // by releasing native ownership before the outstanding call returns.
    match tokio::task::spawn_blocking(move || ownership::finish(job)).await {
        Ok(Ok(progress)) => progress,
        Ok(Err(_)) => retirement_unconfirmed(&status),
        Err(error) => {
            std::mem::forget(error);
            retirement_unconfirmed(&status)
        }
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
    outbox: &CopyOutbox,
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
async fn publish(outbox: &CopyOutbox, progress: CopyProgress) {
    if outbox
        .sender
        .try_send(Event::Copy(CopyEvent::Progress(Arc::new(progress))))
        .is_ok()
    {
        (outbox.wakeup)();
    }
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
#[path = "copy_resource_wait_publication_tests.rs"]
mod resource_wait_publication_tests;

#[cfg(test)]
#[path = "copy_link_tests.rs"]
mod link_tests;

#[cfg(test)]
#[path = "copy_progress_tick_tests.rs"]
mod progress_tick_tests;
