//! Fixed-slot modeless Find sessions. Native work and retained results stay bounded.
use crate::{Outbox, find_worker, task_lifetime::TaskLifetime};
#[path = "find_capture.rs"]
mod capture;
#[path = "find_order_task.rs"]
mod sorting;
use midnight_core::*;
use std::{
    io,
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime},
};
use tokio::{
    sync::{Semaphore, mpsc, watch},
    task::JoinSet,
};

pub(crate) const RETIREMENT_CAPACITY: usize = 16;
pub(crate) struct Retirement {
    pub(crate) domain: crate::retirement::RetirementDomain,
}
pub(crate) struct Controller {
    slots: Vec<Slot>,
    validations: mpsc::Sender<Validation>,
    lifetime: Cancellation,
    highest_session: Option<FindSessionId>,
    available: bool,
}
struct Slot {
    input: watch::Sender<Intent>,
    sorts: watch::Sender<u64>,
    state: Arc<Mutex<State>>,
}
#[derive(Clone, Default)]
struct Intent {
    request: Option<Arc<FindRequest>>,
    cancel: Cancellation,
    accepted_at: Option<SystemTime>,
}
#[derive(Default)]
struct State {
    owner: Option<(FindSessionId, FindGeneration)>,
    cancel: Cancellation,
    results: Vec<Arc<FindResult>>,
    sort: FindSortSpec,
    sort_revision: u64,
    order_cancel: Cancellation,
}
struct Validation {
    request: FindValidationRequest,
    capture: bool,
    result: Arc<FindResult>,
    state: Arc<Mutex<State>>,
}

#[derive(Clone)]
struct Services {
    native: Option<Arc<dyn FindService>>,
    captures: crate::retirement::RetirementDomain,
    capture_slots: Arc<Semaphore>,
    scope: TaskLifetime,
}

impl Controller {
    pub(crate) fn new(
        scope: TaskLifetime,
        retirement: Retirement,
        service: Option<Arc<dyn FindService>>,
        locale: Option<Arc<dyn LocaleService>>,
        outbox: Outbox,
        tasks: &mut JoinSet<()>,
    ) -> Self {
        let services = Services {
            native: service.clone(),
            captures: retirement.domain,
            capture_slots: Arc::new(Semaphore::new(RETIREMENT_CAPACITY)),
            scope: scope.clone(),
        };
        let lifetime = Cancellation::default();
        let budget = Arc::new(Semaphore::new(2));
        let slots = (0..MAX_FIND_SESSIONS)
            .map(|_| {
                let (input, receiver) = watch::channel(Intent::default());
                let (sorts, sort_input) = watch::channel(0);
                let state = Arc::new(Mutex::new(State::default()));
                tasks.spawn(scope.future(sorting::run(
                    scope.clone(),
                    state.clone(),
                    locale.clone(),
                    outbox.clone(),
                    sort_input,
                    lifetime.clone(),
                )));
                tasks.spawn(scope.future(run_slot(
                    services.clone(),
                    outbox.clone(),
                    state.clone(),
                    receiver,
                    budget.clone(),
                    lifetime.clone(),
                    sorts.clone(),
                )));
                Slot {
                    input,
                    sorts,
                    state,
                }
            })
            .collect();
        let (validations, receiver) = mpsc::channel(16);
        tasks.spawn(scope.future(run_validations(
            services,
            outbox,
            receiver,
            lifetime.clone(),
        )));
        Self {
            slots,
            validations,
            lifetime,
            highest_session: None,
            available: service.is_some(),
        }
    }

    pub(crate) fn start(&mut self, request: FindRequest) -> Result<bool, FsError> {
        request
            .validate()
            .map_err(|message| find_error(Path::new(""), io::ErrorKind::InvalidInput, message))?;
        if !self.available {
            return Err(find_error(
                Path::new(""),
                io::ErrorKind::Unsupported,
                "Recursive native Find is unavailable.",
            ));
        }
        let existing = self.slots.iter().position(|slot| {
            slot.state
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .owner
                .is_some_and(|(owner, _)| owner == request.session)
        });
        let index = if let Some(index) = existing {
            index
        } else {
            if self
                .highest_session
                .is_some_and(|highest| request.session.0 <= highest.0)
            {
                return Err(find_error(
                    Path::new(""),
                    io::ErrorKind::InvalidInput,
                    "Closed Find session IDs cannot be reused.",
                ));
            }
            let index = self
                .slots
                .iter()
                .position(|slot| {
                    slot.state
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .owner
                        .is_none()
                })
                .ok_or_else(|| {
                    find_error(
                        Path::new(""),
                        io::ErrorKind::WouldBlock,
                        "At most four modeless Find sessions may be active.",
                    )
                })?;
            self.highest_session = Some(request.session);
            index
        };
        let slot = &self.slots[index];
        let mut state = slot.state.lock().unwrap_or_else(|error| error.into_inner());
        if state
            .owner
            .is_some_and(|(_, generation)| request.generation <= generation)
        {
            return Ok(false);
        }
        state.cancel.cancel();
        state.order_cancel.cancel();
        state.sort = request.sort;
        state.sort_revision = 0;
        state.order_cancel = self.lifetime.child();
        let cancel = self.lifetime.child();
        state.owner = Some((request.session, request.generation));
        state.cancel = cancel.clone();
        state.results.clear();
        slot.input.send_replace(Intent {
            request: Some(Arc::new(request)),
            cancel,
            accepted_at: Some(SystemTime::now()),
        });
        slot.sorts
            .send_modify(|version| *version = version.wrapping_add(1));
        Ok(true)
    }

    pub(crate) fn sort(&self, request: FindSortRequest) {
        for slot in &self.slots {
            let mut state = slot.state.lock().unwrap_or_else(|error| error.into_inner());
            if state.owner == Some((request.session, request.generation))
                && request.revision > state.sort_revision
            {
                state.sort = request.sort;
                state.sort_revision = request.revision;
                state.order_cancel.cancel();
                state.order_cancel = self.lifetime.child();
                drop(state);
                slot.sorts
                    .send_modify(|version| *version = version.wrapping_add(1));
                return;
            }
        }
    }

    pub(crate) fn cancel(&self, session: FindSessionId, generation: FindGeneration) {
        for slot in &self.slots {
            let state = slot.state.lock().unwrap_or_else(|error| error.into_inner());
            if state.owner == Some((session, generation)) {
                state.cancel.cancel();
                state.order_cancel.cancel();
            }
        }
    }
    pub(crate) fn close(&self, session: FindSessionId) {
        for slot in &self.slots {
            let mut state = slot.state.lock().unwrap_or_else(|error| error.into_inner());
            if state.owner.is_some_and(|(owner, _)| owner == session) {
                state.cancel.cancel();
                state.order_cancel.cancel();
                state.owner = None;
                state.results.clear();
                slot.input.send_replace(Intent::default());
            }
        }
    }
    pub(crate) fn capture(&self, request: FindValidationRequest) -> Result<(), FsError> {
        self.enqueue_validation(request, true)
    }
    pub(crate) fn validate(&self, request: FindValidationRequest) -> Result<(), FsError> {
        self.enqueue_validation(request, false)
    }
    fn enqueue_validation(
        &self,
        request: FindValidationRequest,
        capture: bool,
    ) -> Result<(), FsError> {
        for slot in &self.slots {
            let state = slot.state.lock().unwrap_or_else(|error| error.into_inner());
            if state.owner != Some((request.session, request.generation)) {
                continue;
            }
            let result = request
                .result
                .0
                .checked_sub(1)
                .and_then(|row| usize::try_from(row).ok())
                .and_then(|row| state.results.get(row))
                .filter(|result| result.id == request.result)
                .cloned()
                .ok_or_else(|| {
                    find_error(
                        Path::new(""),
                        io::ErrorKind::NotFound,
                        "Find result is no longer available.",
                    )
                })?;
            return self
                .validations
                .try_send(Validation {
                    request,
                    capture,
                    result,
                    state: slot.state.clone(),
                })
                .map_err(|_| {
                    find_error(
                        Path::new(""),
                        io::ErrorKind::WouldBlock,
                        "Find result validation queue is full or closed.",
                    )
                });
        }
        Err(find_error(
            Path::new(""),
            io::ErrorKind::NotFound,
            "Find session or generation is no longer available.",
        ))
    }
    pub(crate) fn shutdown(&self) {
        self.lifetime.cancel();
        for slot in &self.slots {
            let mut state = slot.state.lock().unwrap_or_else(|error| error.into_inner());
            state.cancel.cancel();
            state.order_cancel.cancel();
            state.owner = None;
            state.results.clear();
            drop(state);
            // Wake an idle sorter once for shutdown; it needs no permanent poll.
            slot.sorts
                .send_modify(|version| *version = version.wrapping_add(1));
        }
    }
}
impl Drop for Controller {
    fn drop(&mut self) {
        self.shutdown();
    }
}

async fn run_slot(
    services: Services,
    outbox: Outbox,
    state: Arc<Mutex<State>>,
    mut input: watch::Receiver<Intent>,
    budget: Arc<Semaphore>,
    lifetime: Cancellation,
    sorts: watch::Sender<u64>,
) {
    while input.changed().await.is_ok() {
        let intent = input.borrow_and_update().clone();
        let Some(request) = intent.request else {
            continue;
        };
        if lifetime.is_cancelled() {
            return;
        }
        let Some(service) = services.native.clone() else {
            continue;
        };
        let worker_request = request.clone();
        let accepted_at = intent.accepted_at.unwrap_or_else(SystemTime::now);
        let worker_state = state.clone();
        let worker_outbox = outbox.clone();
        let cancel = intent.cancel.clone();
        let acquisition = budget.clone().acquire_owned();
        tokio::pin!(acquisition);
        let permit = loop {
            if cancel.is_cancelled() {
                break None;
            }
            tokio::select! {
                permit = &mut acquisition => { break permit.ok(); }
                _ = tokio::time::sleep(Duration::from_millis(10)) => {}
            }
        };
        let summary = if let Some(permit) = permit {
            let location = find_worker::CurrentDirectory::default();
            let worker_location = location.clone();
            let worker_sorts = sorts.clone();
            let mut worker = tokio::task::spawn_blocking(services.scope.blocking(move || {
                let _permit = permit;
                find_worker::search_with_location(
                    service.as_ref(),
                    &worker_request,
                    &cancel,
                    &mut |update| {
                        let batch = matches!(&update, FindUpdate::Batch(_));
                        let accepted = publish(
                            &worker_outbox,
                            &worker_state,
                            &worker_request,
                            update,
                            &cancel,
                        );
                        if accepted && batch {
                            worker_sorts.send_modify(|version| *version = version.wrapping_add(1));
                        }
                        accepted
                    },
                    &worker_location,
                    accepted_at,
                )
            }));
            let mut tick = tokio::time::interval(Duration::from_millis(250));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            tick.tick().await; // first sample is at250ms, never an initial burst
            let mut previous = None;
            let mut published_at: Option<Instant> = None;
            let joined = loop {
                tokio::select! {
                    result = &mut worker => break result,
                    _ = tick.tick() => {
                        if !intent.cancel.is_cancelled()
                            && published_at.is_none_or(|last| last.elapsed() >= Duration::from_millis(250))
                            && let Some(path) = location.get()
                            && previous.as_ref().is_none_or(|old| !Arc::ptr_eq(old, &path))
                            && publish_location(&outbox, &state, &request, path.clone())
                        { previous = Some(path); published_at = Some(Instant::now()); }
                    }
                }
            };
            joined.unwrap_or_else(|error| FindSummary {
                completion: FindCompletion::Failed,
                progress: FindProgress::default(),
                reached_budgets: Vec::new(),
                diagnostics: vec![find_error(
                    Path::new(""),
                    io::ErrorKind::Other,
                    &format!("Find worker failed: {error}"),
                )],
            })
        } else {
            FindSummary {
                completion: FindCompletion::Cancelled,
                progress: FindProgress::default(),
                reached_budgets: Vec::new(),
                diagnostics: Vec::new(),
            }
        };
        // A terminal Cancelled event remains useful while this same generation
        // exists. A superseded/closed generation is rejected again after capacity.
        let state = state.clone();
        let request = request.clone();
        let outbox = outbox.clone();
        let lifetime = lifetime.clone();
        let _ = tokio::task::spawn_blocking(services.scope.blocking(move || {
            publish(
                &outbox,
                &state,
                &request,
                FindUpdate::Finished(summary),
                &lifetime,
            )
        }))
        .await;
    }
}

fn publish_location(
    outbox: &Outbox,
    state: &Mutex<State>,
    request: &FindRequest,
    path: Arc<std::path::PathBuf>,
) -> bool {
    let Ok(permit) = outbox.sender.try_reserve() else {
        return false;
    };
    let state = state.lock().unwrap_or_else(|error| error.into_inner());
    if state.owner != Some((request.session, request.generation)) || state.cancel.is_cancelled() {
        return false;
    }
    permit.send(Event::Find(Box::new(FindEvent {
        session: request.session,
        generation: request.generation,
        update: FindUpdate::CurrentDirectory(path),
    })));
    drop(state);
    (outbox.wakeup)();
    true
}

fn publish(
    outbox: &Outbox,
    state: &Mutex<State>,
    request: &FindRequest,
    update: FindUpdate,
    cancel: &Cancellation,
) -> bool {
    loop {
        if cancel.is_cancelled()
            || state
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .owner
                != Some((request.session, request.generation))
        {
            return false;
        }
        match outbox.sender.try_reserve() {
            Ok(permit) => {
                let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
                if cancel.is_cancelled()
                    || state.owner != Some((request.session, request.generation))
                {
                    return false;
                }
                if let FindUpdate::Batch(batch) = &update {
                    state.results.extend(batch.iter().cloned());
                }
                permit.send(Event::Find(Box::new(FindEvent {
                    session: request.session,
                    generation: request.generation,
                    update,
                })));
                drop(state);
                (outbox.wakeup)();
                return true;
            }
            Err(mpsc::error::TrySendError::Closed(_)) => return false,
            Err(mpsc::error::TrySendError::Full(_)) => {
                std::thread::park_timeout(Duration::from_millis(10))
            }
        }
    }
}

async fn run_validations(
    services: Services,
    outbox: Outbox,
    mut input: mpsc::Receiver<Validation>,
    lifetime: Cancellation,
) {
    while let Some(validation) = input.recv().await {
        if lifetime.is_cancelled() {
            return;
        }
        let Some(service) = services.native.clone() else {
            continue;
        };
        let outbox = outbox.clone();
        let cancel = lifetime.clone();
        let capture_services = services.clone();
        let _ = tokio::task::spawn_blocking(services.scope.blocking(move || {
            let request = validation.request;
            let current = || {
                validation
                    .state
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .owner
                    == Some((request.session, request.generation))
            };
            if !current() || cancel.is_cancelled() {
                return;
            }
            loop {
                if !current() || cancel.is_cancelled() {
                    return;
                }
                match outbox.sender.try_reserve() {
                    Ok(permit) => {
                        // Revalidate after output backpressure, then check the
                        // current generation again before returning the target.
                        let result = find_worker::validate_result(
                            service.as_ref(),
                            &validation.result.entry,
                            &cancel,
                        )
                        .map(Box::new);
                        let event = if validation.capture {
                            let result = result.and_then(|_| {
                                capture::capture(&capture_services, &validation, &cancel)
                            });
                            Event::FindFileCaptured(Box::new(FindFileCaptureResult {
                                request,
                                result,
                            }))
                        } else {
                            Event::FindResultValidated(Box::new(FindValidationResult {
                                request,
                                result,
                            }))
                        };
                        let state = validation
                            .state
                            .lock()
                            .unwrap_or_else(|error| error.into_inner());
                        if state.owner != Some((request.session, request.generation))
                            || cancel.is_cancelled()
                        {
                            return;
                        }
                        permit.send(event);
                        drop(state);
                        (outbox.wakeup)();
                        return;
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => return,
                    Err(mpsc::error::TrySendError::Full(_)) => {
                        std::thread::park_timeout(Duration::from_millis(10))
                    }
                }
            }
        }))
        .await;
    }
}
