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
    files: FileClipboard,
    destination: Arc<std::path::PathBuf>,
    intent: TransferIntent,
    verification: Option<TransferVerification>,
    permit: copy_operations::Reservation,
    retention: Option<Arc<Retention>>,
}

/// Keep provider registrations and capture capacity through normal job cleanup.
/// Pending rejection and final retirement run on backend workers.
pub(crate) struct Retention {
    _source: Option<Arc<dyn CapturedClipboardFiles>>,
    _target: Option<Arc<dyn CapturedDropDirectory>>,
    pub(crate) completions: Arc<HashMap<std::path::PathBuf, TreeCopyOccurrence>>,
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
                || !state.has_native_view()
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
            drag: None,
            pane: request.target.pane,
            item_count: request.files.paths.len(),
            destination: request.target.directory.clone(),
            intent: TransferIntent::Copy,
            verification: None,
        };
        pending.insert(
            token,
            Accepted {
                files: request.files,
                destination: request.target.directory,
                intent: TransferIntent::Copy,
                verification: None,
                permit,
                retention: None,
            },
        );
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
        .filter(|source| {
            state.pending.is_none() && state.has_native_view() && source.navigation == navigation
        })
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
    captured: Option<Arc<dyn CapturedClipboardFiles>>,
    running: bool,
    released: bool,
}
struct Prepare {
    id: DragId,
    pane: PaneId,
    navigation: NavigationId,
    files: PrepareFiles,
    cancel: Cancellation,
}
enum PrepareFiles {
    Flat(SelectedFiles),
    Captured(Arc<dyn CapturedClipboardFiles>),
}
#[derive(Default)]
struct State {
    host: Option<Host>,
    source: Option<Source>,
    prepare: Option<Prepare>,
    internal: Option<internal::Work>,
    pending_internal: Option<InternalDropRequest>,
    retired: Vec<Box<dyn Send>>,
    highest_drag: u64,
    highest_host: u64,
}
type SharedState = Arc<Mutex<State>>;
struct Start {
    id: DragId,
    host: u64,
    files: Arc<FileClipboard>,
    captured: Option<Arc<dyn CapturedClipboardFiles>>,
    cancel: Cancellation,
}

#[path = "drag_drop_internal.rs"]
mod internal;

impl State {
    fn clear_source(&mut self) -> Option<bool> {
        let running = self.source.take().map(|source| {
            source.cancel.cancel();
            let running = source.running;
            self.retired.push(Box::new(source));
            running
        });
        if let Some(prepare) = self.prepare.take() {
            self.retired.push(Box::new(prepare));
        }
        if let Some(work) = self.internal.take() {
            self.retired.push(Box::new(work));
        }
        if let Some(pending) = self.pending_internal.take() {
            self.retired.push(Box::new(pending));
        }
        running
    }
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
    transfers: Option<Arc<dyn TransferService>>,
    events: mpsc::Sender<DragDropEvent>,
    native: Option<mpsc::Sender<Start>>,
    prepare: Option<std::sync::mpsc::SyncSender<()>>,
}

impl Controller {
    pub(crate) async fn prepare_captured(
        &self,
        id: DragId,
        pane: PaneId,
        navigation: NavigationId,
        source: Arc<dyn CapturedClipboardFiles>,
    ) {
        self.prepare_kind(id, pane, navigation, PrepareFiles::Captured(source))
            .await;
    }
    pub(crate) fn new(
        service: Option<Arc<dyn FileDragDropService>>,
        transfers: Option<Arc<dyn TransferService>>,
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
            transfers,
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
            state.clear_source();
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
            state.clear_source();
        }
    }

    pub(crate) async fn prepare(
        &self,
        id: DragId,
        pane: PaneId,
        navigation: NavigationId,
        files: SelectedFiles,
    ) {
        self.prepare_kind(id, pane, navigation, PrepareFiles::Flat(files))
            .await;
    }
    async fn retire_backlog(&self) {
        let retired = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            (state.retired.len() >= 8).then(|| std::mem::take(&mut state.retired))
        };
        if let Some(retired) = retired {
            // One awaited fallback bounds retirement even if the coalescing resolver
            // is inside a non-cancellable provider call. Never drop providers here.
            let _ = tokio::task::spawn_blocking(move || drop(retired)).await;
        }
    }
    fn wake_prepare(&self) {
        if let Some(sender) = &self.prepare {
            let _ = sender.try_send(());
        }
    }
    async fn prepare_kind(
        &self,
        id: DragId,
        pane: PaneId,
        navigation: NavigationId,
        files: PrepareFiles,
    ) {
        self.retire_backlog().await;
        let mut files = Some(files);
        let accepted = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if id.0 == 0 || id.0 <= state.highest_drag {
                state.retired.push(Box::new(files.take()));
                self.wake_prepare();
                return;
            }
            state.highest_drag = id.0;
            state.clear_source();
            if state
                .host
                .as_ref()
                .is_none_or(|host| host.host.lifetime.is_cancelled())
            {
                state.retired.push(Box::new(files.take()));
                false
            } else {
                let cancel = self.cancel.child();
                let files = files.take().expect("single preparation input");
                let captured = match &files {
                    PrepareFiles::Captured(source) => Some(source.clone()),
                    PrepareFiles::Flat(_) => None,
                };
                state.source = Some(Source {
                    id,
                    pane,
                    navigation,
                    cancel: cancel.clone(),
                    files: None,
                    captured,
                    running: false,
                    released: false,
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
        self.wake_prepare();
        if !accepted {
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
                    source.id == id
                        && !source.running
                        && !source.released
                        && !source.cancel.is_cancelled()
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
                        captured: source.captured.clone(),
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

    pub(crate) async fn accept(
        &self,
        token: DropToken,
        id: OperationId,
        verification: midnight_core::TransferVerification,
        copies: &copy_operations::Controller,
    ) {
        let accepted = self
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&token);
        if let Some(accepted) = accepted {
            let request = CopyRequest {
                verification: accepted.verification.unwrap_or(verification),
                id,
                clipboard: accepted.files,
                destination: (*accepted.destination).clone(),
            };
            copies
                .enqueue_drop_reserved(
                    request,
                    accepted.intent,
                    accepted.retention,
                    accepted.permit,
                )
                .await;
        }
    }
    pub(crate) fn reject(&self, token: DropToken) {
        let accepted = self
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&token);
        if let Some(accepted) = accepted {
            self.state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .retired
                .push(Box::new(accepted));
            self.wake_prepare();
        }
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
            let running = state.clear_source();
            self.wake_prepare();
            running
        } else {
            None
        }
    }
    async fn failed(&self, id: Option<DragId>, error: FsError) {
        let _ = self.events.send(DragDropEvent::Failed { id, error }).await;
    }
    pub(crate) fn shutdown(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(host) = state.host.take() {
            host.host.lifetime.cancel();
        }
        state.clear_source();
        let pending = std::mem::take(&mut *self.pending.lock().unwrap_or_else(|e| e.into_inner()));
        if !pending.is_empty() {
            state.retired.push(Box::new(pending));
        }
        self.cancel.cancel();
        self.wake_prepare();
        if self.prepare.is_none() && !state.retired.is_empty() {
            let retired = std::mem::take(&mut state.retired);
            tokio::task::spawn_blocking(move || drop(retired));
        }
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
    loop {
        let retired = std::mem::take(&mut state.lock().unwrap_or_else(|e| e.into_inner()).retired);
        drop(retired);
        if cancel.is_cancelled() {
            break;
        }
        if let Err(std::sync::mpsc::RecvTimeoutError::Disconnected) =
            signal.recv_timeout(Duration::from_millis(25))
        {
            break;
        }
        let work = state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .internal
            .take();
        if let Some(work) = work {
            internal::execute(work, &state, &snapshots, &events, &cancel);
            continue;
        }
        let Some(request) = state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .prepare
            .take()
        else {
            continue;
        };
        let result = match &request.files {
            PrepareFiles::Flat(files) => resolve_source(
                &snapshots,
                request.pane,
                request.navigation,
                files,
                &request.cancel,
            ),
            PrepareFiles::Captured(source) => (|| {
                current_source(
                    &snapshots.lock().unwrap_or_else(|e| e.into_inner())[request.pane.index()],
                    request.navigation,
                )?;
                if source.intent() != FileClipboardIntent::Copy {
                    return Err(error(
                        io::ErrorKind::InvalidInput,
                        "A drag source must retain a Copy offer.",
                    ));
                }
                let files = source.resolve(&request.cancel)?;
                files.validate()?;
                if files.intent != FileClipboardIntent::Copy {
                    return Err(error(
                        io::ErrorKind::InvalidInput,
                        "The captured drag offer changed.",
                    ));
                }
                current_source(
                    &snapshots.lock().unwrap_or_else(|e| e.into_inner())[request.pane.index()],
                    request.navigation,
                )?;
                Ok(files)
            })(),
        };
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
                Err(error) => {
                    state.clear_source();
                    DragDropEvent::Failed {
                        id: Some(request.id),
                        error,
                    }
                }
            }
        };
        let delivery_cancel = if matches!(&event, DragDropEvent::Failed { .. }) {
            &cancel
        } else {
            &request.cancel
        };
        send_worker_event(&events, event, delivery_cancel);
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

#[cfg(test)]
#[path = "drag_drop_device_tests.rs"]
mod device_tests;
