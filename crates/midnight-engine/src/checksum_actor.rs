//! Engine adapter: immutable captures, retained backend results and cancellable saves.
use crate::{Outbox, checksum};
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
}
struct Start {
    capture: Capture,
    key: Key,
    cancel: Cancellation,
    retention: Arc<ChecksumRetention>,
}
struct Save {
    id: ChecksumManifestId,
    key: Key,
    request: ChecksumManifestRequest,
    run: Arc<ChecksumRun>,
    cancel: Cancellation,
}
enum Input {
    Start(Start),
    Save(Save),
}

pub(crate) struct Controller {
    input: mpsc::Sender<Input>,
    state: Arc<Mutex<State>>,
    lifetime: Cancellation,
    available: bool,
    task: Option<JoinHandle<()>>,
}
impl Controller {
    pub(crate) fn new(
        service: Option<Arc<dyn TransferService>>,
        outbox: Outbox,
        mutation: Arc<Semaphore>,
    ) -> Self {
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
            available: service.is_some(),
            task: Some(task),
        }
    }
    pub(crate) fn start(&self, request: ChecksumRequest) -> Result<(), FsError> {
        let bytes = request.validate().map_err(invalid)?;
        let key = (request.job, request.generation);
        let retention = checksum::reserve(bytes, request.limits.max_retained_bytes)?;
        self.admit(Capture::Paths(request), key, retention)
    }
    pub(crate) fn start_selected(&self, request: SelectedChecksumRequest) -> Result<(), FsError> {
        request.limits.validate().map_err(invalid)?;
        if request.job.0 == 0 || request.generation.0 == 0 {
            return Err(invalid("Zero checksum job/generation."));
        }
        let key = (request.job, request.generation);
        // Reserve the entire bounded expansion before retaining a queued selection.
        let retention = checksum::reserve(8 * 1024 * 1024, request.limits.max_retained_bytes)?;
        self.admit(Capture::Selected(request), key, retention)
    }
    fn admit(
        &self,
        capture: Capture,
        key: Key,
        retention: Arc<ChecksumRetention>,
    ) -> Result<(), FsError> {
        if !self.available {
            return Err(failure(
                io::ErrorKind::Unsupported,
                "Native checksum reads are unavailable.",
            ));
        }
        if self.lifetime.is_cancelled() {
            return Err(cancelled());
        }
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(job) = state.jobs.get(&key.0) {
            if job.generation >= key.1 {
                return Err(invalid(
                    "Checksum generation is obsolete or already admitted.",
                ));
            }
        } else if key.0.0 <= state.highest_job || state.jobs.len() >= MAX_CHECKSUM_JOBS {
            return Err(failure(
                io::ErrorKind::WouldBlock,
                "Checksum jobs are full or the released job ID was reused.",
            ));
        }
        let cancel = self.lifetime.child();
        self.input
            .try_send(Input::Start(Start {
                capture,
                key,
                cancel: cancel.clone(),
                retention,
            }))
            .map_err(|_| {
                failure(
                    io::ErrorKind::WouldBlock,
                    "Checksum admission queue is full or closed.",
                )
            })?;
        // Queue publication and state replacement occur under the same lock.
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
        if state.saves.len() >= 2 {
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
        self.input
            .try_send(Input::Save(Save {
                id,
                key,
                request,
                run,
                cancel: cancel.clone(),
            }))
            .map_err(|_| {
                failure(
                    io::ErrorKind::WouldBlock,
                    "Checksum manifest admission queue is full or closed.",
                )
            })?;
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
    save: &Save,
    update: ChecksumManifestUpdate,
    stop: &Cancellation,
    state: &Arc<Mutex<State>>,
) {
    send_guarded(
        outbox,
        Event::ChecksumManifest(Box::new(ChecksumManifestEvent {
            save_id: save.id,
            job: save.key.0,
            generation: save.key.1,
            update,
        })),
        stop,
        Some((state, save.key)),
    )
    .await;
}

async fn run(
    service: Option<Arc<dyn TransferService>>,
    outbox: Outbox,
    mutation: Arc<Semaphore>,
    mut input: mpsc::Receiver<Input>,
    state: Arc<Mutex<State>>,
    stop: Cancellation,
) {
    let Some(service) = service else {
        return;
    };
    let mut backend = checksum::Controller::new(service.clone());
    let mut saves = JoinSet::new();
    loop {
        if stop.is_cancelled() {
            break;
        }
        tokio::select! {
            received = input.recv() => match received {
                Some(Input::Start(start)) => {
                    if !is_current(&state.lock().unwrap_or_else(|error| error.into_inner()), start.key) { continue; }
                    send_guarded(&outbox, Event::Checksum(Box::new(ChecksumEvent { job: start.key.0,
                        generation: start.key.1, update: ChecksumUpdate::Queued })), &stop, Some((&state, start.key))).await;
                    let key = start.key;
                    let cancel = start.cancel.clone();
                    let prepared = tokio::task::spawn_blocking(move || {
                        resolve(start.capture, &start.cancel).map(|request| (request, start.retention))
                    }).await;
                    match prepared {
                        Ok(Ok((request, retention))) => {
                            if let Err(error) = backend.start_reserved(request, cancel, retention) {
                                rejected(&outbox, key, error, &stop).await;
                            }
                        }
                        Ok(Err(error)) => rejected(&outbox, key, error, &stop).await,
                        Err(_) => rejected(&outbox, key, failure(io::ErrorKind::Other,
                            "Checksum target preparation failed."), &stop).await,
                    }
                }
                Some(Input::Save(save)) => {
                    manifest_event(&outbox, &save, ChecksumManifestUpdate::Queued, &stop, &state).await;
                    saves.spawn(export(save, service.clone(), outbox.clone(), mutation.clone(), state.clone(), stop.clone()));
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
    while input.try_recv().is_ok() {}
    let state = state.clone();
    let _ = tokio::task::spawn_blocking(move || {
        let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
        state.jobs.clear();
        state.saves.clear();
    })
    .await;
}

fn resolve(capture: Capture, cancel: &Cancellation) -> Result<ChecksumRequest, FsError> {
    let request = match capture {
        Capture::Paths(request) => request,
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
    save: Save,
    service: Arc<dyn TransferService>,
    outbox: Outbox,
    mutation: Arc<Semaphore>,
    state: Arc<Mutex<State>>,
    stop: Cancellation,
) {
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
            let request = save.request.clone();
            let run = save.run.clone();
            let cancel = save.cancel.clone();
            match tokio::task::spawn_blocking(move || {
                let _permit = permit; // Includes stage cleanup on every return path.
                checksum::save_manifest(&run, &request, service.as_ref(), &cancel)
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
        Err(error) => Err(error),
    };
    // Cleanup finished before releasing admission capacity or delivering terminal status.
    let current = {
        let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
        state.saves.remove(&save.id);
        is_current(&state, save.key)
    };
    if current {
        manifest_event(
            &outbox,
            &save,
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
