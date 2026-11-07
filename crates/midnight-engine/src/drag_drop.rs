//! One native actor, one coalescing source resolver, and reserved copy admission.
use crate::{Outbox, Snapshots, copy_operations, file_targets};
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
use tokio::{sync::mpsc, task::JoinSet};

type Pending = Arc<Mutex<HashMap<DropToken, Accepted>>>;
type Hover = Arc<Mutex<Option<(u64, Option<PaneId>)>>>;
struct Accepted {
    request: FileDropRequest,
    permit: copy_operations::Reservation,
}

struct Sink {
    host: DragHost,
    targets: SharedDropTargets,
    snapshots: Snapshots,
    pending: Pending,
    capacity: copy_operations::Admission,
    events: mpsc::Sender<DragDropEvent>,
    next: Arc<AtomicU64>,
    hover: Hover,
    cancel: Cancellation,
}
impl FileDropSink for Sink {
    fn try_accept(&self, request: FileDropRequest) -> Result<(), FsError> {
        request.validate()?;
        if self.cancel.is_cancelled() || self.host.lifetime.is_cancelled() {
            return Err(error(
                io::ErrorKind::Interrupted,
                "The drop window has closed.",
            ));
        }
        // Keep the target snapshot locked through admission: UI invalidation either precedes
        // this drop or follows an already accepted immutable destination.
        let targets = self.targets.lock().unwrap_or_else(|e| e.into_inner());
        targets.validate()?;
        if targets.host_generation != self.host.generation
            || targets.revision != request.targets_revision
            || !targets.targets.contains(&request.target)
        {
            return Err(error(
                io::ErrorKind::InvalidInput,
                "The drop destination changed.",
            ));
        }
        {
            let snapshots = self.snapshots.lock().unwrap_or_else(|e| e.into_inner());
            let state = &snapshots[request.target.pane.index()];
            if state.pending.is_some()
                || !state
                    .visible
                    .as_ref()
                    .or(state.committed.as_ref())
                    .is_some_and(|snapshot| {
                        snapshot.navigation == request.target.navigation
                            && snapshot.path == *request.target.directory
                    })
            {
                return Err(error(
                    io::ErrorKind::InvalidInput,
                    "The destination listing changed before the drop.",
                ));
            }
        }
        let permit = self.capacity.reserve()?;
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        // Shutdown clears this same registry. Recheck under its lock so a callback that
        // waited for admission cannot insert a new acknowledged request after cleanup.
        if self.cancel.is_cancelled() || self.host.lifetime.is_cancelled() {
            return Err(error(
                io::ErrorKind::Interrupted,
                "The drop window has closed.",
            ));
        }
        let token = DropToken(
            self.next
                .try_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                    value.checked_add(1)
                })
                .map_err(|_| {
                    error(
                        io::ErrorKind::Other,
                        "The drop token sequence is exhausted.",
                    )
                })?
                + 1,
        );
        let event = DragDropEvent::DropAccepted {
            token,
            pane: request.target.pane,
            item_count: request.files.paths.len(),
            destination: request.target.directory.clone(),
        };
        pending.insert(token, Accepted { request, permit });
        if self.events.try_send(event).is_err() {
            pending.remove(&token);
            return Err(error(
                io::ErrorKind::WouldBlock,
                "The application cannot receive a drop right now.",
            ));
        }
        Ok(())
    }
    fn hover(&self, pane: Option<PaneId>) {
        *self.hover.lock().unwrap_or_else(|e| e.into_inner()) = Some((self.host.generation, pane));
    }
}

fn error(kind: io::ErrorKind, message: impl Into<String>) -> FsError {
    FsError::from_io(
        "file drag/drop",
        Path::new(""),
        io::Error::new(kind, message.into()),
    )
}

fn resolve_source(
    snapshots: &Snapshots,
    pane: PaneId,
    navigation: NavigationId,
    files: &SelectedFiles,
    cancel: &Cancellation,
) -> Result<FileClipboard, FsError> {
    let entries = {
        let snapshots = snapshots.lock().unwrap_or_else(|e| e.into_inner());
        let state = &snapshots[pane.index()];
        current_source(state, navigation)?.entries.clone()
    };
    if files.selection.is_empty()
        && files
            .fallback
            .as_ref()
            .is_some_and(|path| !files.view.by_path.contains_key(path))
    {
        return Err(error(
            io::ErrorKind::InvalidInput,
            "The dragged item is no longer visible.",
        ));
    }
    let paths = file_targets::resolve(
        &files.selection,
        &files.view,
        files.fallback.as_deref(),
        cancel,
    )?;
    let mut remaining: std::collections::HashSet<_> = paths.iter().collect();
    for entry in entries.iter() {
        if cancel.is_cancelled() {
            return Err(error(
                io::ErrorKind::Interrupted,
                "Drag preparation cancelled.",
            ));
        }
        remaining.remove(&entry.path);
        if remaining.is_empty() {
            break;
        }
    }
    if !remaining.is_empty() {
        return Err(error(
            io::ErrorKind::InvalidInput,
            "The dragged entries changed before preparation.",
        ));
    }
    let result = FileClipboard {
        paths,
        intent: FileClipboardIntent::Copy,
    };
    result.validate()?;
    current_source(
        &snapshots.lock().unwrap_or_else(|e| e.into_inner())[pane.index()],
        navigation,
    )?;
    Ok(result)
}

fn current_source(
    state: &crate::SnapshotState,
    navigation: NavigationId,
) -> Result<&crate::CommittedSnapshot, FsError> {
    state
        .visible
        .as_ref()
        .or(state.committed.as_ref())
        .filter(|source| state.pending.is_none() && source.navigation == navigation)
        .ok_or_else(|| {
            error(
                io::ErrorKind::InvalidInput,
                "The drag source navigation changed.",
            )
        })
}

#[derive(Clone)]
struct Host {
    host: DragHost,
    targets: SharedDropTargets,
    sink: Arc<Sink>,
}
struct Source {
    id: DragId,
    pane: PaneId,
    navigation: NavigationId,
    cancel: Cancellation,
    files: Option<Arc<FileClipboard>>,
    running: bool,
}
struct Prepare {
    id: DragId,
    pane: PaneId,
    navigation: NavigationId,
    files: SelectedFiles,
    cancel: Cancellation,
}
#[derive(Default)]
struct State {
    host: Option<Host>,
    source: Option<Source>,
    prepare: Option<Prepare>,
    highest_drag: u64,
    highest_host: u64,
}
type SharedState = Arc<Mutex<State>>;
struct Start {
    id: DragId,
    host: u64,
    files: Arc<FileClipboard>,
    cancel: Cancellation,
}

pub(crate) struct Controller {
    state: SharedState,
    snapshots: Snapshots,
    pending: Pending,
    capacity: copy_operations::Admission,
    next: Arc<AtomicU64>,
    hover: Hover,
    cancel: Cancellation,
    service: Option<Arc<dyn FileDragDropService>>,
    events: mpsc::Sender<DragDropEvent>,
    native: Option<mpsc::Sender<Start>>,
    prepare: Option<std::sync::mpsc::SyncSender<()>>,
}

impl Controller {
    pub(crate) fn new(
        service: Option<Arc<dyn FileDragDropService>>,
        snapshots: Snapshots,
        capacity: copy_operations::Admission,
        outbox: Outbox,
        tasks: &mut JoinSet<()>,
    ) -> Self {
        let (events, receiver) = mpsc::channel(64);
        let cancel = Cancellation::default();
        let hover = Hover::default();
        tasks.spawn(forward(receiver, outbox, hover.clone(), cancel.clone()));
        Self {
            state: SharedState::default(),
            snapshots,
            pending: Pending::default(),
            capacity,
            next: Arc::new(AtomicU64::new(0)),
            hover,
            cancel,
            service,
            events,
            native: None,
            prepare: None,
        }
    }

    pub(crate) async fn attach(
        &mut self,
        host: DragHost,
        targets: SharedDropTargets,
        tasks: &mut JoinSet<()>,
    ) {
        {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if host.generation == 0 || host.generation <= state.highest_host {
                return;
            }
            if let Some(old) = state.host.take() {
                old.host.lifetime.cancel();
            }
            if let Some(source) = state.source.take() {
                source.cancel.cancel();
            }
            state.prepare = None;
            state.highest_host = host.generation;
            state.host = Some(Host {
                host: host.clone(),
                targets: targets.clone(),
                sink: Arc::new(Sink {
                    host,
                    targets,
                    snapshots: self.snapshots.clone(),
                    pending: self.pending.clone(),
                    capacity: self.capacity.clone(),
                    events: self.events.clone(),
                    next: self.next.clone(),
                    hover: self.hover.clone(),
                    cancel: self.cancel.clone(),
                }),
            });
        }
        if self.native.is_none() {
            let (sender, receiver) = mpsc::channel(1);
            self.native = Some(sender);
            let state = self.state.clone();
            let events = self.events.clone();
            let cancel = self.cancel.clone();
            let service = self.service.clone();
            tasks.spawn_blocking(move || actor::run(state, service, receiver, events, cancel));
            let (sender, receiver) = std::sync::mpsc::sync_channel(1);
            self.prepare = Some(sender);
            let state = self.state.clone();
            let events = self.events.clone();
            let snapshots = self.snapshots.clone();
            let cancel = self.cancel.clone();
            tasks
                .spawn_blocking(move || prepare_worker(state, snapshots, receiver, events, cancel));
        }
    }

    pub(crate) fn detach(&self, generation: u64) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state
            .host
            .as_ref()
            .is_some_and(|host| host.host.generation == generation)
        {
            if let Some(host) = state.host.take() {
                host.host.lifetime.cancel();
                host.sink.hover(None);
            }
            if let Some(source) = state.source.take() {
                source.cancel.cancel();
            }
            state.prepare = None;
        }
    }

    pub(crate) async fn prepare(
        &self,
        id: DragId,
        pane: PaneId,
        navigation: NavigationId,
        files: SelectedFiles,
    ) {
        let accepted = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if id.0 == 0 || id.0 <= state.highest_drag {
                return;
            }
            state.highest_drag = id.0;
            if let Some(source) = state.source.take() {
                source.cancel.cancel();
            }
            state.prepare = None;
            if state
                .host
                .as_ref()
                .is_none_or(|host| host.host.lifetime.is_cancelled())
            {
                false
            } else {
                let cancel = self.cancel.child();
                state.source = Some(Source {
                    id,
                    pane,
                    navigation,
                    cancel: cancel.clone(),
                    files: None,
                    running: false,
                });
                state.prepare = Some(Prepare {
                    id,
                    pane,
                    navigation,
                    files,
                    cancel,
                });
                true
            }
        };
        if accepted {
            if let Some(sender) = &self.prepare {
                let _ = sender.try_send(());
            }
        } else {
            self.failed(
                Some(id),
                error(
                    io::ErrorKind::Interrupted,
                    "The drag window is not available.",
                ),
            )
            .await;
        }
    }

    pub(crate) async fn start(&self, id: DragId) {
        let command = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let host = state
                .host
                .as_ref()
                .filter(|host| !host.host.lifetime.is_cancelled())
                .map(|host| host.host.generation);
            state
                .source
                .as_mut()
                .filter(|source| {
                    source.id == id && !source.running && !source.cancel.is_cancelled()
                })
                .and_then(|source| {
                    let host = host?;
                    let snapshots = self.snapshots.lock().unwrap_or_else(|e| e.into_inner());
                    current_source(&snapshots[source.pane.index()], source.navigation).ok()?;
                    let files = source.files.clone()?;
                    source.running = true;
                    Some(Start {
                        id,
                        host,
                        files,
                        cancel: source.cancel.clone(),
                    })
                })
        };
        let Some(command) = command else {
            self.failed(
                Some(id),
                error(
                    io::ErrorKind::InvalidInput,
                    "The prepared drag is no longer current.",
                ),
            )
            .await;
            return;
        };
        if self
            .native
            .as_ref()
            .is_none_or(|sender| sender.try_send(command).is_err())
        {
            self.cancel(id).await;
            self.failed(
                Some(id),
                error(io::ErrorKind::WouldBlock, "The native drag worker is busy."),
            )
            .await;
        }
    }

    pub(crate) async fn internal(&self, id: DragId, target: FileDropTarget, revision: u64) {
        let prepared = {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state
                .source
                .as_ref()
                .filter(|source| {
                    source.id == id && !source.running && !source.cancel.is_cancelled()
                })
                .and_then(|source| {
                    Some((
                        source.files.clone()?,
                        state.host.as_ref()?.sink.clone(),
                        source.pane,
                        source.navigation,
                    ))
                })
        };
        let result = match prepared {
            Some((files, sink, pane, navigation)) => {
                let valid = {
                    let snapshots = self.snapshots.lock().unwrap_or_else(|e| e.into_inner());
                    current_source(&snapshots[pane.index()], navigation).map(|_| ())
                };
                valid.and_then(|()| {
                    sink.try_accept(FileDropRequest {
                        files: (*files).clone(),
                        target,
                        targets_revision: revision,
                    })
                })
            }
            None => Err(error(
                io::ErrorKind::InvalidInput,
                "The internal drag is no longer current.",
            )),
        };
        match result {
            Ok(()) => {
                self.clear_source(id);
                let _ = self
                    .events
                    .send(DragDropEvent::Finished {
                        id,
                        outcome: DragFinish::Copied,
                    })
                    .await;
            }
            Err(error) => self.failed(Some(id), error).await,
        }
    }

    pub(crate) async fn accept(
        &self,
        token: DropToken,
        id: OperationId,
        copies: &copy_operations::Controller,
    ) {
        let accepted = self
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&token);
        if let Some(accepted) = accepted {
            copies
                .enqueue_reserved(
                    CopyRequest {
                        id,
                        clipboard: accepted.request.files,
                        destination: (*accepted.request.target.directory).clone(),
                    },
                    accepted.permit,
                )
                .await;
        }
    }
    pub(crate) fn reject(&self, token: DropToken) {
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&token);
    }
    pub(crate) async fn cancel(&self, id: DragId) {
        let running = self.clear_source(id);
        if running == Some(false) {
            let _ = self
                .events
                .send(DragDropEvent::Finished {
                    id,
                    outcome: DragFinish::Cancelled,
                })
                .await;
        }
    }
    fn clear_source(&self, id: DragId) -> Option<bool> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.source.as_ref().is_some_and(|source| source.id == id) {
            let source = state.source.take().unwrap();
            source.cancel.cancel();
            state.prepare = None;
            Some(source.running)
        } else {
            None
        }
    }
    async fn failed(&self, id: Option<DragId>, error: FsError) {
        let _ = self.events.send(DragDropEvent::Failed { id, error }).await;
    }
    pub(crate) fn shutdown(&self) {
        self.cancel.cancel();
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(host) = state.host.take() {
            host.host.lifetime.cancel();
        }
        if let Some(source) = state.source.take() {
            source.cancel.cancel();
        }
        state.prepare = None;
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }
}
impl Drop for Controller {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn prepare_worker(
    state: SharedState,
    snapshots: Snapshots,
    signal: std::sync::mpsc::Receiver<()>,
    events: mpsc::Sender<DragDropEvent>,
    cancel: Cancellation,
) {
    while !cancel.is_cancelled() {
        if let Err(std::sync::mpsc::RecvTimeoutError::Disconnected) =
            signal.recv_timeout(Duration::from_millis(25))
        {
            break;
        }
        let Some(request) = state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .prepare
            .take()
        else {
            continue;
        };
        let result = resolve_source(
            &snapshots,
            request.pane,
            request.navigation,
            &request.files,
            &request.cancel,
        );
        if request.cancel.is_cancelled() {
            continue;
        }
        let event = {
            let mut state = state.lock().unwrap_or_else(|e| e.into_inner());
            if state
                .host
                .as_ref()
                .is_none_or(|host| host.host.lifetime.is_cancelled())
            {
                continue;
            }
            let Some(source) = state
                .source
                .as_mut()
                .filter(|source| source.id == request.id)
            else {
                continue;
            };
            match result {
                Ok(files) => {
                    let item_count = files.paths.len();
                    source.files = Some(Arc::new(files));
                    DragDropEvent::Prepared {
                        id: request.id,
                        item_count,
                    }
                }
                Err(error) => DragDropEvent::Failed {
                    id: Some(request.id),
                    error,
                },
            }
        };
        send_worker_event(&events, event, &request.cancel);
    }
}

fn send_worker_event(
    events: &mpsc::Sender<DragDropEvent>,
    mut event: DragDropEvent,
    cancel: &Cancellation,
) -> bool {
    while !cancel.is_cancelled() {
        match events.try_send(event) {
            Ok(()) => return true,
            Err(mpsc::error::TrySendError::Closed(_)) => return false,
            Err(mpsc::error::TrySendError::Full(value)) => event = value,
        }
        std::thread::park_timeout(Duration::from_millis(10));
    }
    false
}

async fn forward(
    mut events: mpsc::Receiver<DragDropEvent>,
    outbox: Outbox,
    hover: Hover,
    cancel: Cancellation,
) {
    let mut tick = tokio::time::interval(Duration::from_millis(25));
    let mut previous = None;
    while !cancel.is_cancelled() {
        let event = tokio::select! {
            event = events.recv() => match event { Some(event) => event, None => break },
            _ = tick.tick() => {
                let latest = *hover.lock().unwrap_or_else(|e| e.into_inner());
                if previous == latest { continue; }
                previous = latest;
                let Some((host_generation, pane)) = latest else { continue; };
                DragDropEvent::Hover { host_generation, pane }
            },
        };
        let send = outbox.send(Event::DragDrop(event));
        tokio::pin!(send);
        loop {
            tokio::select! {
                result = &mut send => { if !result { return; } break; },
                _ = tick.tick() => if cancel.is_cancelled() { return; },
            }
        }
    }
}

#[path = "drag_drop_actor.rs"]
mod actor;

#[cfg(test)]
#[path = "drag_drop_tests.rs"]
mod tests;
