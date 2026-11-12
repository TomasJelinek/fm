//! Engine adapter: immutable captures, retained backend results and cancellable saves.
use crate::retirement::WorkerOwned;
use crate::{Outbox, checksum};
use checksum::ownership::{MAX_SAVES, Service};
pub(crate) use checksum::ownership::{RETIREMENT_CAPACITY, Retirement};
use midnight_core::*;
use std::{
    collections::HashMap,
    io,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    sync::{Semaphore, mpsc},
    task::{JoinHandle, JoinSet},
};

type Key = (ChecksumJobId, ChecksumGeneration);
#[derive(Default)]
struct State {
    jobs: HashMap<ChecksumJobId, Job>,
    saves: HashMap<ChecksumManifestId, SaveState>,
    highest_job: u64,
    highest_save: u64,
}
struct Job {
    generation: ChecksumGeneration,
    cancel: Cancellation,
    result: Option<Arc<ChecksumRun>>,
}
struct SaveState {
    key: Key,
    cancel: Cancellation,
}
enum Capture {
    Paths(ChecksumRequest),
    Selected(SelectedChecksumRequest),
    Captured(CapturedChecksumRequest),
    Verify(ChecksumVerifyRequest),
}
struct Start {
    capture: Option<Capture>,
    key: Key,
    cancel: Cancellation,
    retention: Option<Arc<ChecksumRetention>>,
    service: Service,
}
struct Save {
    id: ChecksumManifestId,
    key: Key,
    request: ChecksumManifestRequest,
    run: Arc<ChecksumRun>,
    cancel: Cancellation,
    worker: Option<tokio::sync::OwnedSemaphorePermit>,
    service: Service,
}
enum Input {
    Start(WorkerOwned<Start>),
    Save(WorkerOwned<Save>),
}

pub(crate) struct Controller {
    input: mpsc::Sender<Input>,
    state: Arc<Mutex<State>>,
    lifetime: Cancellation,
    available: bool,
    service: Service,
    retirement: crate::retirement::RetirementDomain,
    intake: tokio::sync::Mutex<crate::retirement::RetirementWaiter>,
    task: Option<JoinHandle<()>>,
}
impl Controller {
    pub(crate) fn new(
        retirement: Retirement,
        service: Option<Arc<dyn TransferService>>,
        outbox: Outbox,
        mutation: Arc<Semaphore>,
    ) -> Self {
        let service = Service::new(retirement.service, service);
        let available = service.available();
        let (input, receiver) = mpsc::channel(32);
        let state = Arc::new(Mutex::new(State::default()));
        let lifetime = Cancellation::default();
        let task = tokio::spawn(run(
            service.clone(),
            outbox,
            mutation,
            receiver,
            state.clone(),
            lifetime.clone(),
        ));
        Self {
            input,
            state,
            lifetime,
            available,
            service,
            retirement: retirement.domain,
            intake: tokio::sync::Mutex::new(retirement.intake),
            task: Some(task),
        }
    }
    pub(crate) fn start(&self, request: ChecksumRequest) -> Result<(), FsError> {
        let bytes = request.validate().map_err(invalid)?;
        let key = (request.job, request.generation);
        let retention = checksum::reserve(bytes, request.limits.max_retained_bytes)?;
        self.admit_plain(Capture::Paths(request), key, retention)
    }
    pub(crate) fn start_selected(&self, request: SelectedChecksumRequest) -> Result<(), FsError> {
        request.limits.validate().map_err(invalid)?;
        if request.job.0 == 0 || request.generation.0 == 0 {
            return Err(invalid("Zero checksum job/generation."));
        }
        let key = (request.job, request.generation);
        // Reserve the entire bounded expansion before retaining a queued selection.
        let retention = checksum::reserve(8 * 1024 * 1024, request.limits.max_retained_bytes)?;
        self.admit_plain(Capture::Selected(request), key, retention)
    }
    pub(crate) async fn start_captured(
        &self,
        request: CapturedChecksumRequest,
    ) -> Result<(), FsError> {
        // The enclosing guarded EngineRun owns this request while intake waits.
        // Keep the single intake locked until any rejected owner is actually retired.
        let mut intake = self.intake.lock().await;
        let lease = intake.reserve().await;
        let key = (request.job, request.generation);
        let mut start = lease.own(Start {
            capture: Some(Capture::Captured(request)),
            key,
            cancel: self.lifetime.child(),
            retention: None,
            service: self.service.clone(),
        });
        let validation = match start.capture.as_ref().expect("owned capture") {
            Capture::Captured(request) => {
                request.limits.validate().map_err(invalid).and_then(|()| {
                    if request.job.0 == 0 || request.generation.0 == 0 {
                        return Err(invalid("Zero checksum job/generation."));
                    }
                    checksum::reserve(8 * 1024 * 1024, request.limits.max_retained_bytes)
                })
            }
            _ => unreachable!("captured intake owns a captured request"),
        };
        match validation {
            Ok(retention) => start.retention = Some(retention),
            Err(error) => {
                let _ = start.retire().await;
                return Err(error);
            }
        }
        match self.admit(start) {
            Ok(()) => Ok(()),
            Err((error, start)) => {
                let _ = start.retire().await;
                Err(error)
            }
        }
    }
    pub(crate) fn verify(&self, request: ChecksumVerifyRequest) -> Result<(), FsError> {
        let key = (request.job, request.generation);
        let retention = checksum::reserve_verification(&request)?;
        self.admit_plain(Capture::Verify(request), key, retention)
    }
    fn admit_plain(
        &self,
        capture: Capture,
        key: Key,
        retention: Arc<ChecksumRetention>,
    ) -> Result<(), FsError> {
        // Only path/selection/manifest data reaches this synchronous helper.
        // Provider-bearing captured requests use the reserved async intake above.
        let lease = self.retirement.try_reserve().map_err(|_| {
            failure(
                io::ErrorKind::WouldBlock,
                "Checksum cleanup capacity is occupied.",
            )
        })?;
        let start = lease.own(Start {
            capture: Some(capture),
            key,
            cancel: self.lifetime.child(),
            retention: Some(retention),
            service: self.service.clone(),
        });
        self.admit(start).map_err(|(error, start)| {
            drop(start);
            error
        })
    }
    fn admit(&self, start: WorkerOwned<Start>) -> Result<(), (FsError, WorkerOwned<Start>)> {
        if !self.available {
            return Err((
                failure(
                    io::ErrorKind::Unsupported,
                    "Native checksum reads are unavailable.",
                ),
                start,
            ));
        }
        if self.lifetime.is_cancelled() {
            return Err((cancelled(), start));
        }
        let key = start.key;
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(job) = state.jobs.get(&key.0) {
            if job.generation >= key.1 {
                return Err((
                    invalid("Checksum generation is obsolete or already admitted."),
                    start,
                ));
            }
        } else if key.0.0 <= state.highest_job || state.jobs.len() >= MAX_CHECKSUM_JOBS {
            return Err((
                failure(
                    io::ErrorKind::WouldBlock,
                    "Checksum jobs are full or the released job ID was reused.",
                ),
                start,
            ));
        }
        let cancel = start.cancel.clone();
        if let Err(error) = self.input.try_send(Input::Start(start)) {
            let Input::Start(start) = error.into_inner() else {
                unreachable!("submitted Start")
            };
            return Err((
                failure(
                    io::ErrorKind::WouldBlock,
                    "Checksum admission queue is full or closed.",
                ),
                start,
            ));
        }
        // Queue publication and state replacement still occur under the same lock.
        if let Some(old) = state.jobs.insert(
            key.0,
            Job {
                generation: key.1,
                cancel,
                result: None,
            },
        ) {
            old.cancel.cancel();
        }
        for save in state.saves.values() {
            if save.key.0 == key.0 {
                save.cancel.cancel();
            }
        }
        state.highest_job = state.highest_job.max(key.0.0);
        Ok(())
    }
    pub(crate) fn cancel(&self, job: ChecksumJobId, generation: ChecksumGeneration) {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(current) = state
            .jobs
            .get(&job)
            .filter(|current| current.generation == generation)
        {
            current.cancel.cancel();
        }
    }
    pub(crate) fn release(&self, job: ChecksumJobId, generation: ChecksumGeneration) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if !is_current(&state, (job, generation)) {
            return;
        }
        if let Some(old) = state.jobs.remove(&job) {
            old.cancel.cancel();
        }
        for save in state.saves.values() {
            if save.key == (job, generation) {
                save.cancel.cancel();
            }
        }
    }
    pub(crate) fn save(
        &self,
        id: ChecksumManifestId,
        key: Key,
        request: ChecksumManifestRequest,
    ) -> Result<(), FsError> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if id.0 == 0 || id.0 <= state.highest_save {
            return Err(invalid(
                "Manifest save IDs must be nonzero, increasing and never reused.",
            ));
        }
        state.highest_save = id.0; // Failed admission also consumes the ID.
        if self.lifetime.is_cancelled() {
            return Err(cancelled());
        }
        if state.saves.len() >= MAX_SAVES {
            return Err(failure(
                io::ErrorKind::WouldBlock,
                "Two checksum manifests are pending or active.",
            ));
        }
        if request.destination.as_os_str().as_encoded_bytes().len() > 1024 * 1024 {
            return Err(invalid(
                "Manifest destination exceeds the 1 MiB input resource budget.",
            ));
        }
        let run = state
            .jobs
            .get(&key.0)
            .filter(|job| job.generation == key.1)
            .and_then(|job| job.result.clone())
            .ok_or_else(|| {
                invalid("No completed retained result exists for this checksum generation.")
            })?;
        let cancel = self.lifetime.child();
        // Reserve queue space before adopting a save payload. Rejected synchronous
        // requests cannot accumulate extra retiring save owners outside the two slots.
        let queue = self.input.try_reserve().map_err(|_| {
            failure(
                io::ErrorKind::WouldBlock,
                "Checksum manifest admission queue is full or closed.",
            )
        })?;
        let lease = self.retirement.try_reserve().map_err(|_| {
            failure(
                io::ErrorKind::WouldBlock,
                "Checksum cleanup capacity is occupied.",
            )
        })?;
        queue.send(Input::Save(lease.own(Save {
            id,
            key,
            request,
            run,
            cancel: cancel.clone(),
            worker: None,
            service: self.service.clone(),
        })));
        state.saves.insert(id, SaveState { key, cancel });
        Ok(())
    }
    pub(crate) fn cancel_save(&self, id: ChecksumManifestId) {
        if let Some(save) = self
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .saves
            .get(&id)
        {
            save.cancel.cancel();
        }
    }
    pub(crate) async fn shutdown(mut self) {
        self.lifetime.cancel();
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}
impl Drop for Controller {
    fn drop(&mut self) {
        self.lifetime.cancel();
    }
}
fn is_current(state: &State, key: Key) -> bool {
    state
        .jobs
        .get(&key.0)
        .is_some_and(|job| job.generation == key.1)
}
fn failure(kind: io::ErrorKind, message: &str) -> FsError {
    FsError::from_io("checksums", Path::new(""), io::Error::new(kind, message))
}
fn invalid(message: &str) -> FsError {
    failure(io::ErrorKind::InvalidInput, message)
}
fn cancelled() -> FsError {
    failure(io::ErrorKind::Interrupted, "Checksum operation cancelled.")
}

async fn send(outbox: &Outbox, event: Event, stop: &Cancellation) -> bool {
    send_guarded(outbox, event, stop, None).await
}
async fn send_guarded(
    outbox: &Outbox,
    event: Event,
    stop: &Cancellation,
    guard: Option<(&Arc<Mutex<State>>, Key)>,
) -> bool {
    // A full UI channel must not prevent cancellation/normal shutdown cleanup.
    let mut pending = event;
    loop {
        if stop.is_cancelled() {
            return false;
        }
        let attempt = {
            let locked =
                guard.map(|(state, _)| state.lock().unwrap_or_else(|error| error.into_inner()));
            if let Some((_, key)) = guard
                && !locked.as_ref().is_some_and(|state| is_current(state, key))
            {
                return false;
            }
            outbox.sender.try_send(pending)
        };
        match attempt {
            Ok(()) => {
                (outbox.wakeup)();
                return true;
            }
            Err(mpsc::error::TrySendError::Closed(_)) => return false,
            Err(mpsc::error::TrySendError::Full(event)) => pending = event,
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
async fn rejected(outbox: &Outbox, key: Key, error: FsError, stop: &Cancellation) {
    send(
        outbox,
        Event::ChecksumRejected(Box::new(ChecksumRejected {
            job: key.0,
            generation: key.1,
            error,
        })),
        stop,
    )
    .await;
}
async fn manifest_event(
    outbox: &Outbox,
    id: ChecksumManifestId,
    key: Key,
    update: ChecksumManifestUpdate,
    stop: &Cancellation,
    state: &Arc<Mutex<State>>,
) {
    send_guarded(
        outbox,
        Event::ChecksumManifest(Box::new(ChecksumManifestEvent {
            save_id: id,
            job: key.0,
            generation: key.1,
            update,
        })),
        stop,
        Some((state, key)),
    )
    .await;
}

async fn run(
    service: Service,
    outbox: Outbox,
    mutation: Arc<Semaphore>,
    mut input: mpsc::Receiver<Input>,
    state: Arc<Mutex<State>>,
    stop: Cancellation,
) {
    if !service.available() {
        return;
    }
    let mut backend = checksum::Controller::from_service(service);
    let mut saves = JoinSet::new();
    loop {
        if stop.is_cancelled() {
            break;
        }
        tokio::select! {
            received = input.recv() => match received {
                Some(Input::Start(start)) => {
                    if !is_current(&state.lock().unwrap_or_else(|error| error.into_inner()), start.key) {
                        let _ = start.retire().await;
                        continue;
                    }
                    send_guarded(&outbox, Event::Checksum(Box::new(ChecksumEvent { job: start.key.0,
                        generation: start.key.1, update: ChecksumUpdate::Queued })), &stop, Some((&state, start.key))).await;
                    let key = start.key;
                    let prepared = tokio::task::spawn_blocking(move || prepare(start)).await;
                    match prepared {
                        Ok(Ok(work)) => {
                            if let Err(error) = backend.admit_prepared(work).await {
                                rejected(&outbox, key, error, &stop).await;
                            }
                        }
                        Ok(Err(error)) => rejected(&outbox, key, error, &stop).await,
                        Err(_) => rejected(&outbox, key, failure(io::ErrorKind::Other,
                            "Checksum target preparation failed."), &stop).await,
                    }
                }
                Some(Input::Save(save)) => {
                    manifest_event(&outbox, save.id, save.key, ChecksumManifestUpdate::Queued, &stop, &state).await;
                    saves.spawn(export(save, outbox.clone(), mutation.clone(), state.clone(), stop.clone()));
                }
                None => break,
            },
            event = backend.next() => match event {
                Some(event) => {
                    let key = (event.job, event.generation);
                    let forward = {
                        let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
                        if is_current(&state, key) {
                            if let ChecksumUpdate::Finished(run) = &event.update {
                                state.jobs.get_mut(&key.0).unwrap().result = Some(run.clone());
                            }
                            true
                        } else { false }
                    };
                    if forward { send_guarded(&outbox, Event::Checksum(Box::new(event)), &stop, Some((&state, key))).await; }
                }
                None => { stop.cancel(); break; }
            },
            _ = saves.join_next(), if !saves.is_empty() => {},
            _ = tokio::time::sleep(Duration::from_millis(25)) => {},
        }
    }
    stop.cancel();
    input.close();
    // Await native work, then release results. Never abort a future holding native ownership.
    backend.shutdown().await;
    while saves.join_next().await.is_some() {}
    while let Ok(input) = input.try_recv() {
        match input {
            Input::Start(start) => {
                let _ = start.retire().await;
            }
            Input::Save(save) => {
                let _ = save.retire().await;
            }
        }
    }
    let state = state.clone();
    let _ = tokio::task::spawn_blocking(move || {
        let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
        state.jobs.clear();
        state.saves.clear();
    })
    .await;
}

fn prepare(owned: WorkerOwned<Start>) -> Result<WorkerOwned<checksum::Work>, FsError> {
    let (payload, lease) = owned.into_parts();
    let mut start = payload;
    let capture = start.capture.take().expect("pending checksum capture");
    let source = match &capture {
        Capture::Captured(request) => Some(request.sources.clone()),
        _ => None,
    };
    let work = match capture {
        Capture::Verify(request) => checksum::Work::verify(
            request,
            start.cancel.clone(),
            start.retention.take().expect("reserved quota"),
            start.service.clone(),
        ),
        capture => {
            let request = resolve(capture, &start.cancel)?;
            checksum::Work::calculate(
                request,
                start.cancel.clone(),
                start.retention.take().expect("reserved quota"),
                source,
                start.service.clone(),
            )
        }
    };
    Ok(lease.own(work))
}

fn resolve(capture: Capture, cancel: &Cancellation) -> Result<ChecksumRequest, FsError> {
    let request = match capture {
        Capture::Paths(request) => request,
        Capture::Captured(captured) => ChecksumRequest {
            job: captured.job,
            generation: captured.generation,
            targets: captured.sources.resolve(cancel)?,
            algorithms: captured.algorithms,
            recursive: captured.recursive,
            limits: captured.limits,
        },
        Capture::Verify(_) => {
            return Err(invalid(
                "Verification capture cannot be used for calculation.",
            ));
        }
        Capture::Selected(selected) => {
            let paths = crate::file_targets::resolve(
                &selected.files.selection,
                &selected.files.view,
                selected.files.fallback.as_deref(),
                cancel,
            )?;
            if !selected.files.selection.is_empty() && paths.len() != selected.files.selection.len()
            {
                return Err(invalid(
                    "The captured checksum selection no longer matches its visible directory.",
                ));
            }
            let mut request = ChecksumRequest::new(selected.job, selected.generation, paths);
            request.algorithms = selected.algorithms;
            request.recursive = selected.recursive;
            request.limits = selected.limits;
            for target in &mut request.targets {
                if let Some(position) = selected
                    .files
                    .view
                    .by_path
                    .get(&target.path)
                    .and_then(|id| selected.files.view.by_id.get(id))
                {
                    target.size = selected
                        .files
                        .view
                        .file_sizes
                        .get(*position)
                        .copied()
                        .flatten();
                    target.kind = target.size.map(|_| EntryKind::File);
                }
            }
            request
        }
    };
    request.validate().map_err(invalid)?;
    Ok(request)
}

async fn export(
    mut save: WorkerOwned<Save>,
    outbox: Outbox,
    mutation: Arc<Semaphore>,
    state: Arc<Mutex<State>>,
    stop: Cancellation,
) {
    let id = save.id;
    let key = save.key;
    let permit = loop {
        if save.cancel.is_cancelled() {
            break Err(cancelled());
        }
        match tokio::time::timeout(Duration::from_millis(25), mutation.clone().acquire_owned())
            .await
        {
            Ok(Ok(permit)) => break Ok(permit),
            Ok(Err(_)) => {
                break Err(failure(
                    io::ErrorKind::BrokenPipe,
                    "Mutation service is closing.",
                ));
            }
            Err(_) => {}
        }
    };
    let result = match permit {
        Ok(permit) => {
            // Saving is coalescible: do not hold the mutation gate waiting for the UI.
            if !stop.is_cancelled()
                && outbox
                    .sender
                    .try_send(Event::ChecksumManifest(Box::new(ChecksumManifestEvent {
                        save_id: save.id,
                        job: save.key.0,
                        generation: save.key.1,
                        update: ChecksumManifestUpdate::Saving,
                    })))
                    .is_ok()
            {
                (outbox.wakeup)();
            }
            save.worker = Some(permit);
            match tokio::task::spawn_blocking(move || {
                let (payload, lease) = save.into_parts();
                let save = payload;
                let result = checksum::save_manifest(
                    &save.run,
                    &save.request,
                    save.service.provider(),
                    &save.cancel,
                );
                drop(save); // Includes native cleanup before the mutation permit is released.
                drop(lease);
                result
            })
            .await
            {
                Ok(result) => result,
                Err(_) => Err(failure(
                    io::ErrorKind::Other,
                    "Checksum manifest worker failed.",
                )),
            }
        }
        Err(error) => {
            let _ = save.retire().await;
            Err(error)
        }
    };
    // Cleanup finished before releasing admission capacity or delivering terminal status.
    let current = {
        let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
        state.saves.remove(&id);
        is_current(&state, key)
    };
    if current {
        manifest_event(
            &outbox,
            id,
            key,
            ChecksumManifestUpdate::Finished(result),
            &stop,
            &state,
        )
        .await;
    }
}

#[cfg(test)]
#[path = "checksum_actor_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "checksum_actor_retirement_tests.rs"]
mod retirement_tests;
