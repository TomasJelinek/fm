//! Fixed-slot modeless Find sessions. Native work and retained results stay bounded.
use crate::{Outbox, find_worker, task_lifetime::TaskLifetime};
#[path = "find_capture.rs"]
mod capture;
#[path = "find_installed_archives.rs"]
mod installed_archives;
#[path = "find_order_task.rs"]
mod sorting;
pub(crate) use installed_archives::InstalledArchives;
use midnight_core::*;
use std::{
    io,
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime},
};
use tokio::{
    sync::{Notify, Semaphore, mpsc, watch},
    task::JoinSet,
};

pub(crate) const RETIREMENT_CAPACITY: usize = 16;
pub(crate) struct Retirement {
    pub(crate) domain: crate::retirement::RetirementDomain,
}
pub(crate) struct Controller {
    slots: Vec<Slot>,
    validations: mpsc::Sender<Validation>,
    /// Replaced result authorities are dropped on a backend worker.
    retired: mpsc::Sender<Vec<Arc<FindResult>>>,
    lifetime: Cancellation,
    highest_session: Option<FindSessionId>,
    available: bool,
}
struct Slot {
    input: watch::Sender<Intent>,
    sorts: watch::Sender<u64>,
    /// Wakes a session waiting for a worker when its generation is cancelled.
    wake: Arc<Notify>,
    state: Arc<Mutex<State>>,
}
/// Concurrent result validations/captures; a hung network open keeps the others usable.
const VALIDATION_WORKERS: usize = 2;
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
    /// Cancelled by restart and Close but not by Stop, which keeps file actions.
    generation: Cancellation,
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
    cancel: Cancellation,
}

#[derive(Clone)]
struct Services {
    native: Option<Arc<dyn FindService>>,
    /// Search inside archives; absent without the archive provider's capture.
    archives: Option<Arc<dyn find_worker::ArchiveSearch>>,
    /// Formats published and validated results' modification times on the workers.
    locale: Option<Arc<dyn LocaleService>>,
    captures: crate::retirement::RetirementDomain,
    capture_slots: Arc<Semaphore>,
    /// At most two searches traverse at once; other sessions wait for a permit.
    searches: Arc<Semaphore>,
    scope: TaskLifetime,
}

impl Controller {
    pub(crate) fn with_archives(
        scope: TaskLifetime,
        retirement: Retirement,
        service: Option<Arc<dyn FindService>>,
        archives: Option<Arc<dyn find_worker::ArchiveSearch>>,
        locale: Option<Arc<dyn LocaleService>>,
        outbox: Outbox,
        tasks: &mut JoinSet<()>,
    ) -> Self {
        let services = Services {
            native: service.clone(),
            archives,
            locale: locale.clone(),
            captures: retirement.domain,
            capture_slots: Arc::new(Semaphore::new(RETIREMENT_CAPACITY)),
            searches: Arc::new(Semaphore::new(2)),
            scope: scope.clone(),
        };
        let lifetime = Cancellation::default();
        let slots = (0..MAX_FIND_SESSIONS)
            .map(|_| {
                let (input, receiver) = watch::channel(Intent::default());
                let (sorts, sort_input) = watch::channel(0);
                let wake = Arc::new(Notify::new());
                let state = Arc::new(Mutex::new(State {
                    generation: lifetime.child(),
                    ..State::default()
                }));
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
                    lifetime.clone(),
                    sorts.clone(),
                    wake.clone(),
                )));
                Slot {
                    input,
                    sorts,
                    wake,
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
        let (retired, mut retiring) = mpsc::channel::<Vec<Arc<FindResult>>>(MAX_FIND_SESSIONS);
        let retire_scope = scope.clone();
        tasks.spawn(scope.future(async move {
            while let Some(results) = retiring.recv().await {
                let _ =
                    tokio::task::spawn_blocking(retire_scope.blocking(move || drop(results))).await;
            }
        }));
        Self {
            slots,
            validations,
            retired,
            lifetime,
            highest_session: None,
            available: service.is_some(),
        }
    }

    /// Up to the Find result cap (paths and metadata) are freed on a backend worker.
    /// A full queue keeps the bounded fallback of dropping them here.
    fn retire(&self, results: Vec<Arc<FindResult>>) {
        if !results.is_empty() {
            let _ = self.retired.try_send(results);
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
        state.generation.cancel();
        state.generation = self.lifetime.child();
        state.sort = request.sort;
        state.sort_revision = 0;
        state.order_cancel = self.lifetime.child();
        let cancel = self.lifetime.child();
        state.owner = Some((request.session, request.generation));
        state.cancel = cancel.clone();
        let replaced = std::mem::take(&mut state.results);
        slot.input.send_replace(Intent {
            request: Some(Arc::new(request)),
            cancel,
            accepted_at: Some(SystemTime::now()),
        });
        drop(state);
        slot.wake.notify_one();
        slot.sorts
            .send_modify(|version| *version = version.wrapping_add(1));
        self.retire(replaced);
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
            let mut state = slot.state.lock().unwrap_or_else(|error| error.into_inner());
            if state.owner == Some((session, generation)) {
                state.cancel.cancel();
                // Retained results keep an automatic order: replace the sort token
                // and request a final sort (a final batch requests another).
                state.order_cancel.cancel();
                state.order_cancel = self.lifetime.child();
                drop(state);
                slot.wake.notify_one();
                slot.sorts
                    .send_modify(|version| *version = version.wrapping_add(1));
            }
        }
    }
    pub(crate) fn close(&self, session: FindSessionId) {
        for slot in &self.slots {
            let mut state = slot.state.lock().unwrap_or_else(|error| error.into_inner());
            if state.owner.is_some_and(|(owner, _)| owner == session) {
                state.cancel.cancel();
                state.order_cancel.cancel();
                state.generation.cancel();
                state.owner = None;
                let replaced = std::mem::take(&mut state.results);
                slot.input.send_replace(Intent::default());
                drop(state);
                slot.wake.notify_one();
                self.retire(replaced);
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
                    cancel: state.generation.clone(),
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
            state.generation.cancel();
            state.owner = None;
            let replaced = std::mem::take(&mut state.results);
            drop(state);
            slot.wake.notify_one();
            self.retire(replaced);
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
    lifetime: Cancellation,
    sorts: watch::Sender<u64>,
    wake: Arc<Notify>,
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
        let locale = services.locale.clone();
        let archives = services.archives.clone();
        let worker_request = request.clone();
        let accepted_at = intent.accepted_at.unwrap_or_else(SystemTime::now);
        let worker_state = state.clone();
        let worker_outbox = outbox.clone();
        let cancel = intent.cancel.clone();
        let acquisition = services.searches.clone().acquire_owned();
        tokio::pin!(acquisition);
        // A waiting session sleeps until a worker frees or its generation is
        // stopped, restarted, closed or shut down (each of those wakes it).
        let permit = loop {
            if cancel.is_cancelled() {
                break None;
            }
            tokio::select! {
                permit = &mut acquisition => { break permit.ok(); }
                () = wake.notified() => {}
            }
        };
        let summary = if let Some(permit) = permit {
            let location = find_worker::CurrentDirectory::default();
            let worker_location = location.clone();
            let worker_sorts = sorts.clone();
            let worker_lifetime = lifetime.clone();
            let mut worker = tokio::task::spawn_blocking(services.scope.blocking(move || {
                let _permit = permit;
                find_worker::search_with_archives(
                    service.as_ref(),
                    archives.as_deref(),
                    locale.as_deref(),
                    &worker_request,
                    &cancel,
                    &mut |update| {
                        let batch = matches!(&update, FindUpdate::Batch(_));
                        // Stop retains results: matches found before the worker
                        // observed Stop are still accepted for the same owner.
                        let accepted = publish(
                            &worker_outbox,
                            &worker_state,
                            &worker_request,
                            update,
                            if batch { &worker_lifetime } else { &cancel },
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
    let mut running = JoinSet::new();
    loop {
        // At most VALIDATION_WORKERS native validations/captures run at once.
        let validation = if running.len() < VALIDATION_WORKERS {
            tokio::select! {
                validation = input.recv() => validation,
                Some(_) = running.join_next(), if !running.is_empty() => continue,
            }
        } else {
            let _ = running.join_next().await;
            continue;
        };
        let Some(validation) = validation else {
            break;
        };
        if lifetime.is_cancelled() {
            break;
        }
        let Some(service) = services.native.clone() else {
            continue;
        };
        let locale = services.locale.clone();
        let outbox = outbox.clone();
        // Restart, Close and shutdown abandon cooperative provider checks of
        // obsolete requests; Stop keeps them, like the retained file actions.
        let cancel = validation.cancel.clone();
        let capture_lifetime = lifetime.clone();
        let capture_services = services.clone();
        running.spawn_blocking(services.scope.blocking(move || {
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
                        // An archive member is reached through its archive: the
                        // native archive file is what must be unchanged.
                        let result = match &validation.result.archive {
                            None => find_worker::validate_result(
                                service.as_ref(),
                                &validation.result.entry,
                                &cancel,
                            ),
                            Some(member) => find_worker::validate_result(
                                service.as_ref(),
                                &member.container,
                                &cancel,
                            )
                            .map(|_| validation.result.entry.clone()),
                        };
                        let result = if validation.capture && validation.result.archive.is_some()
                        {
                            Err(find_error(
                                &validation.result.entry.path,
                                io::ErrorKind::Unsupported,
                                "An archive member opens from its archive: use Go to file, then view or copy it there.",
                            ))
                        } else {
                            result
                        };
                        let event = if validation.capture {
                            let result = result.and_then(|_| {
                                capture::capture(&capture_services, &validation, &capture_lifetime)
                            });
                            Event::FindFileCaptured(Box::new(FindFileCaptureResult {
                                request,
                                result,
                            }))
                        } else {
                            // The reported current entry is formatted like a published result.
                            let result = result.map(|mut entry| {
                                find_worker::localize(&mut entry, locale.as_deref());
                                Box::new(entry)
                            });
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
        }));
    }
    // Native calls keep their borrowed state until they return.
    while running.join_next().await.is_some() {}
}
