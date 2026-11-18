//! One native actor, one coalescing source resolver, and reserved copy admission.
use crate::{
    Outbox, Snapshots, copy_operations,
    engine_lifetime::DragRetirement,
    file_targets,
    retirement::{RetirementWaiter, WorkerOwned},
};
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
    sync::{Notify, mpsc},
    task::JoinSet,
};

type Pending = Arc<WorkerOwned<Mutex<HashMap<DropToken, Accepted>>>>;
type Hover = Arc<Report<Option<(u64, Option<PaneId>)>>>;
/// The latest undelivered refused native drop, with the lifetime of its drop window.
type Refused = Arc<Report<Option<(Cancellation, FsError)>>>;

/// The latest native report of one kind, which `forward` delivers to the UI.
struct Report<T> {
    value: Mutex<T>,
    /// Wakes `forward`. A controller's hover and refusal reports share it, and its
    /// shutdown signals it.
    changed: Arc<Notify>,
}
impl<T> Report<T> {
    fn lock(&self) -> std::sync::MutexGuard<'_, T> {
        self.value.lock().unwrap_or_else(|e| e.into_inner())
    }
}
/// Empty hover and refusal reports that wake the same `forward`.
fn reports() -> (Hover, Refused) {
    let changed = Arc::new(Notify::new());
    (
        Arc::new(Report {
            value: Mutex::default(),
            changed: changed.clone(),
        }),
        Arc::new(Report {
            value: Mutex::default(),
            changed,
        }),
    )
}

struct Accepted {
    files: FileClipboard,
    destination: Arc<std::path::PathBuf>,
    intent: TransferIntent,
    verification: Option<TransferVerification>,
    permit: copy_operations::Reservation,
}

/// Keep provider registrations and capture capacity through normal job cleanup.
/// Pending rejection and final retirement run on backend workers.
pub(crate) struct Retention {
    _source: Option<Arc<dyn CapturedClipboardFiles>>,
    _target: Option<Arc<dyn CapturedDropDirectory>>,
    pub(crate) completions: Arc<HashMap<std::path::PathBuf, TreeCopyOccurrence>>,
}

impl Retention {
    fn release_on_worker(self: Arc<Self>) -> Result<(), FsError> {
        use crate::copy_nested_retirement::{self as nested, Retained};
        let context = nested::current().unwrap_or_default();
        if context.is_quarantined() {
            std::mem::forget(self);
            return Err(nested::unconfirmed());
        }
        let Some(owner) = Arc::into_inner(self) else {
            return Ok(());
        };
        let mut owner = Retained::new(owner);
        let mut result = Ok(());
        let released = context.with(|| {
            nested::checked_step(|| {
                result = (|| {
                    if let Some(source) = owner.get_mut()._source.take() {
                        nested::release_clipboard_sources(source)?;
                    }
                    if let Some(target) = owner.get_mut()._target.take() {
                        nested::release_drop_directory(target)?;
                    }
                    Ok(())
                })();
            })
        });
        if !released && result.is_ok() {
            result = Err(nested::unconfirmed());
        }
        if let Err(error) = result {
            context.record(error.clone());
            return Err(error);
        }
        if !context.with(|| nested::checked_step(|| drop(owner.take()))) {
            return Err(nested::unconfirmed());
        }
        Ok(())
    }
}

/// The original per-registration owner lives inside Copy's sealed reservation.
/// Completion metadata can be cloned independently and contains no authority.
pub(crate) struct RegistrationOwner(Option<Arc<Retention>>);
impl RegistrationOwner {
    pub(crate) fn new(retention: Arc<Retention>) -> Self {
        Self(Some(retention))
    }
}
impl crate::copy_nested_retirement::CheckedRetirement for RegistrationOwner {
    fn retirement_is_checked(&self) -> bool {
        true
    }
}
impl Drop for RegistrationOwner {
    fn drop(&mut self) {
        if let Some(owner) = self.0.take() {
            if crate::copy_nested_retirement::current().is_some() {
                let _ = owner.release_on_worker();
            } else {
                drop(owner);
            }
        }
    }
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
    refused: Refused,
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
        let staged = self.capacity.stage_drop()?;
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
        let permit = staged.publish()?;
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
        let latest = Some((self.host.generation, pane));
        let mut hover = self.hover.lock();
        // Native targets report every pointer motion; only a change needs delivery.
        if *hover != latest {
            *hover = latest;
            self.hover.changed.notify_one();
        }
    }
    fn reject(&self, error: FsError) {
        // Runs inside the native drop callback, so it never waits and never takes capacity
        // from the event queue, whose saturation is itself a refusal reason. The latest
        // refusal replaces an undelivered one; `forward` reports it once the UI accepts
        // events, unless the window has closed by then.
        if self.cancel.is_cancelled() || self.host.lifetime.is_cancelled() {
            return;
        }
        *self.refused.lock() = Some((self.host.lifetime.clone(), error));
        self.refused.changed.notify_one();
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
    preview: Option<preview::Work>,
    pending_preview: Option<InternalDropPreviewRequest>,
    preview_epoch: Option<preview::Epoch>,
    preview_revision: DragPreviewRevision,
    retired: Retired,
    highest_drag: u64,
    highest_host: u64,
    /// Times the native actor returned from waiting for work.
    #[cfg(test)]
    actor_wakeups: u64,
    /// Times the preparation worker returned from waiting for work.
    #[cfg(test)]
    prepare_wakeups: u64,
}
type SharedState = Arc<WorkerOwned<Mutex<State>>>;

/// Owners that the preparation worker drops, off the async executors. Every push wakes
/// that worker once it runs, so it sleeps until there is something to drop.
#[derive(Default)]
struct Retired {
    owners: Vec<Box<dyn Send>>,
    /// The preparation worker's signal. `take` keeps it; `std::mem::take` would not.
    wake: Option<std::sync::mpsc::SyncSender<()>>,
}
impl Retired {
    fn push(&mut self, owner: Box<dyn Send>) {
        self.owners.push(owner);
        if let Some(wake) = &self.wake {
            // A full channel already holds a wakeup the worker has not taken.
            let _ = wake.try_send(());
        }
    }
    fn take(&mut self) -> Vec<Box<dyn Send>> {
        std::mem::take(&mut self.owners)
    }
    fn len(&self) -> usize {
        self.owners.len()
    }
    #[cfg(test)]
    fn is_empty(&self) -> bool {
        self.owners.is_empty()
    }
}

struct Start {
    id: DragId,
    host: u64,
    files: Arc<FileClipboard>,
    captured: Option<Arc<dyn CapturedClipboardFiles>>,
    cancel: Cancellation,
}

#[path = "drag_drop_internal.rs"]
mod internal;

#[path = "drag_drop_preview.rs"]
mod preview;

impl State {
    fn clear_source(&mut self) -> Option<bool> {
        self.clear_preview();
        self.preview_revision = DragPreviewRevision::default();
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
    backlog: tokio::sync::Mutex<RetirementWaiter>,
    snapshots: Snapshots,
    pending: Pending,
    capacity: copy_operations::Admission,
    next: Arc<AtomicU64>,
    hover: Hover,
    refused: Refused,
    cancel: Cancellation,
    service: Option<Arc<dyn FileDragDropService>>,
    transfers: Option<Arc<dyn TransferService>>,
    events: mpsc::Sender<DragDropEvent>,
    native: Option<mpsc::Sender<Start>>,
    /// Wakes the native actor after a host change, a queued start or shutdown.
    native_wake: Option<actor::Wake>,
    /// Wakes the preparation worker after queued work; retired owners wake it themselves.
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
        retirement: DragRetirement,
        service: Option<Arc<dyn FileDragDropService>>,
        transfers: Option<Arc<dyn TransferService>>,
        snapshots: Snapshots,
        capacity: copy_operations::Admission,
        outbox: Outbox,
        tasks: &mut JoinSet<()>,
    ) -> Self {
        let (events, receiver) = mpsc::channel(64);
        let cancel = Cancellation::default();
        let (hover, refused) = reports();
        tasks.spawn(forward(
            receiver,
            outbox,
            hover.clone(),
            refused.clone(),
            cancel.clone(),
        ));
        Self {
            state: Arc::new(retirement.state.own(Mutex::new(State::default()))),
            backlog: tokio::sync::Mutex::new(retirement.backlog),
            snapshots,
            pending: Arc::new(retirement.pending.own(Mutex::new(HashMap::new()))),
            capacity,
            next: Arc::new(AtomicU64::new(0)),
            hover,
            refused,
            cancel,
            service,
            transfers,
            events,
            native: None,
            native_wake: None,
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
                    refused: self.refused.clone(),
                    cancel: self.cancel.clone(),
                }),
            });
        }
        // A running native actor opens the new host's session now; a new one
        // reads the host first.
        self.wake_native();
        if self.native.is_none() {
            let (sender, receiver) = mpsc::channel(1);
            self.native = Some(sender);
            let (wake, woken) = actor::wake();
            self.native_wake = Some(wake);
            let state = self.state.clone();
            let events = self.events.clone();
            let cancel = self.cancel.clone();
            let service = self.service.clone();
            // Both actors live for the rest of the session: keep them off the
            // bounded blocking pool that listings and scans need.
            crate::actor_thread::spawn("midnight-drag-drop", tasks, move || {
                actor::run(state, service, receiver, woken, events, cancel)
            });
            let (sender, receiver) = std::sync::mpsc::sync_channel(1);
            // Owners retired from now on wake the worker; it drops earlier ones first.
            self.state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .retired
                .wake = Some(sender.clone());
            self.prepare = Some(sender);
            let state = self.state.clone();
            let events = self.events.clone();
            let snapshots = self.snapshots.clone();
            let cancel = self.cancel.clone();
            crate::actor_thread::spawn("midnight-drag-prepare", tasks, move || {
                prepare_worker(state, snapshots, receiver, events, cancel)
            });
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
            // The native actor closes the detached host's session now.
            self.wake_native();
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
        // Reserve before taking provider owners out of guarded shared state.
        let mut waiter = self.backlog.lock().await;
        let lease = waiter.reserve().await;
        let retired = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            (state.retired.len() >= 8).then(|| state.retired.take())
        };
        if let Some(retired) = retired {
            // One awaited fallback bounds retirement even if the coalescing resolver
            // is inside a non-cancellable provider call. Never drop providers here.
            let _ = lease.retire(retired).await;
        }
    }
    fn wake_prepare(&self) {
        if let Some(sender) = &self.prepare {
            let _ = sender.try_send(());
        }
    }
    fn wake_native(&self) {
        if let Some(wake) = &self.native_wake {
            wake.wake();
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
            state.clear_preview();
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
            .is_some_and(|sender| sender.try_send(command).is_ok())
        {
            self.wake_native();
        } else {
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
                start_policy: midnight_core::TransferStartPolicy::Immediate,
                verification: accepted.verification.unwrap_or(verification),
                id,
                clipboard: accepted.files,
                destination: (*accepted.destination).clone(),
            };
            copies
                .enqueue_drop_reserved(request, accepted.intent, accepted.permit)
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
            state.clear_source()
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
        self.wake_native();
        // `forward` has no timer; this ends its wait, also for an undeliverable event.
        self.hover.changed.notify_one();
        // Without a resolver, the guarded state retires the backlog on final release.
        // A stopping Tokio blocking pool must never become a destructor fallback.
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
        let retired = state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retired
            .take();
        drop(retired);
        if cancel.is_cancelled() {
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
        let preview = state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .preview
            .take();
        if let Some(preview) = preview {
            preview::execute(preview, &state, &snapshots, &events);
            continue;
        }
        let request = state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .prepare
            .take();
        let Some(request) = request else {
            // Nothing is queued or retired: sleep, without the state lock, until queued
            // work, a retired owner or shutdown signals.
            let woken = signal.recv();
            #[cfg(test)]
            {
                state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .prepare_wakeups += 1;
            }
            if woken.is_err() {
                break;
            }
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

/// Delivers the drag events and the latest hover and refusal reports to the UI. It has
/// no timer: an event, a report's `changed` signal or shutdown wakes it.
async fn forward(
    mut events: mpsc::Receiver<DragDropEvent>,
    outbox: Outbox,
    hover: Hover,
    refused: Refused,
    cancel: Cancellation,
) {
    let changed = hover.changed.clone();
    let mut previous = None;
    // A report may have changed since both were last found unchanged. `changed` stores
    // at most one signal, which a wait during a delivery can take.
    let mut reported = true;
    while !cancel.is_cancelled() {
        let report = if reported {
            next_report(&hover, &refused, &mut previous)
        } else {
            None
        };
        let event = match report {
            Some(event) => event,
            None => {
                reported = false;
                tokio::select! {
                    event = events.recv() => match event { Some(event) => event, None => break },
                    () = changed.notified() => {
                        reported = true;
                        continue;
                    }
                }
            }
        };
        let send = outbox.send(Event::DragDrop(event));
        tokio::pin!(send);
        loop {
            tokio::select! {
                result = &mut send => { if !result { return; } break; },
                () = changed.notified() => {
                    if cancel.is_cancelled() { return; }
                    reported = true;
                },
            }
        }
    }
}

/// The next report for the UI: a refused drop of a window that is still open, else a
/// changed hover. A refusal precedes a hover that changed with it.
fn next_report(
    hover: &Hover,
    refused: &Refused,
    previous: &mut Option<(u64, Option<PaneId>)>,
) -> Option<DragDropEvent> {
    let refusal = refused.lock().take();
    // A refusal outside any gesture leaves the native session usable.
    if let Some((lifetime, error)) = refusal
        && !lifetime.is_cancelled()
    {
        return Some(DragDropEvent::Failed { id: None, error });
    }
    let latest = *hover.lock();
    if *previous == latest {
        return None;
    }
    *previous = latest;
    let (host_generation, pane) = latest?;
    Some(DragDropEvent::Hover {
        host_generation,
        pane,
    })
}

#[path = "drag_drop_actor.rs"]
mod actor;

#[cfg(test)]
#[path = "drag_drop_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "drag_drop_device_tests.rs"]
mod device_tests;
