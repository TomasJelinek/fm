//! Bounded native change hints and polling, owned entirely by backend workers.

use crate::{
    Outbox,
    retirement::{RetirementLease, WorkerOwned},
    task_lifetime::TaskLifetime,
};
use midnight_core::{
    Cancellation, CompressionStatus, Entry, EntryKind, Event, FileSystem, NavigationId, PaneId,
    VisibleDirectory, WatchService, WatchSubscription,
};
use std::{
    ffi::OsString,
    path::{Component, Path},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::{self, Thread},
    time::{Duration, Instant, SystemTime},
};
use tokio::{sync::mpsc::error::TrySendError, task::JoinSet};

pub(crate) mod signature;
pub(crate) mod tree;
pub(crate) use tree::RefreshTicket;

/// One shared control owner per pane; reserve before accepting any captured scope.
pub(crate) const RETIREMENT_CAPACITY: usize = 2;
pub(crate) struct Retirement {
    pub(crate) panes: [RetirementLease; RETIREMENT_CAPACITY],
}
#[cfg(test)]
impl Retirement {
    fn testing() -> Self {
        let domain =
            crate::retirement::RetirementDomain::try_start(RETIREMENT_CAPACITY, 0).unwrap();
        Self {
            panes: std::array::from_fn(|_| domain.try_reserve().unwrap()),
        }
    }
}

// A cancelled outer task or a native panic can bypass run's final drain. The last
// handle still retires captured mounts off the caller without moving subscriptions.
type SharedOwner = Arc<WorkerOwned<Shared>>;

/// Two independent blocking workers own native subscriptions and serial probes.
/// There are no unbounded event/path queues and no worker per filesystem event.
pub(crate) struct Controller {
    panes: [SharedOwner; RETIREMENT_CAPACITY],
}

#[derive(Clone)]
pub(crate) struct Handle {
    panes: [SharedOwner; RETIREMENT_CAPACITY],
}

#[derive(Clone, Copy)]
struct Timing {
    quiet: Duration,
    maximum: Duration,
    poll: Duration,
}

struct Control {
    visible: Option<VisibleDirectory>,
    revision: u64,
    cancel: Cancellation,
    stopped: bool,
    navigation: NavigationId,
    tree_navigation: Option<NavigationId>,
    tree: Option<tree::TreeControl>,
}

struct Shared {
    state: Mutex<Control>,
    worker: OnceLock<Thread>,
    timing: Timing,
}

impl Shared {
    fn wake(&self) {
        if let Some(worker) = self.worker.get() {
            // Unpark retains its wake permit if a callback races with parking.
            worker.unpark();
        }
    }

    fn is_current(&self, revision: u64) -> bool {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        !state.stopped && state.revision == revision
    }
}

impl Controller {
    pub(crate) fn handle(&self) -> Handle {
        Handle {
            panes: self.panes.clone(),
        }
    }
    pub(crate) fn new(
        retirement: Retirement,
        scope: TaskLifetime,
        filesystem: Arc<dyn FileSystem>,
        native: Option<Arc<dyn WatchService>>,
        outbox: Outbox,
        tasks: &mut JoinSet<()>,
    ) -> Self {
        Self::with_timing(
            retirement,
            scope,
            filesystem,
            native,
            outbox,
            tasks,
            Timing {
                quiet: Duration::from_secs(1),
                maximum: Duration::from_secs(3),
                poll: Duration::from_secs(15),
            },
        )
    }

    fn with_timing(
        retirement: Retirement,
        scope: TaskLifetime,
        filesystem: Arc<dyn FileSystem>,
        native: Option<Arc<dyn WatchService>>,
        outbox: Outbox,
        tasks: &mut JoinSet<()>,
        timing: Timing,
    ) -> Self {
        let panes = retirement.panes.map(|lease| {
            Arc::new(lease.own(Shared {
                state: Mutex::new(Control {
                    visible: None,
                    revision: 0,
                    cancel: Cancellation::default(),
                    stopped: false,
                    navigation: NavigationId(0),
                    tree_navigation: None,
                    tree: None,
                }),
                worker: OnceLock::new(),
                timing,
            }))
        });
        for shared in &panes {
            let shared = shared.clone();
            let filesystem = filesystem.clone();
            let native = native.clone();
            let outbox = outbox.clone();
            let native_scope = scope.clone();
            tasks.spawn(scope.future(async move {
                let _ = tokio::task::spawn_blocking(native_scope.blocking(move || {
                    run(shared, filesystem, native, outbox, timing);
                }))
                .await;
            }));
        }
        Self { panes }
    }

    /// Called by the backend only after the UI accepts a complete listing.
    pub(crate) fn set_visible(&self, visible: VisibleDirectory) {
        let shared = &self.panes[visible.pane.index()];
        let mut state = shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state.stopped
            || visible.navigation < state.navigation
            || state.tree_navigation == Some(visible.navigation)
            || tree::current(&state)
                .is_some_and(|scope| scope.stamp.navigation == visible.navigation)
            || state.visible.as_ref().is_some_and(|old| {
                old.navigation > visible.navigation
                    || (old.navigation == visible.navigation
                        && old.path == visible.path
                        && Arc::ptr_eq(&old.entries, &visible.entries))
            })
        {
            return;
        }
        state.cancel.cancel();
        state.cancel = Cancellation::default();
        state.navigation = visible.navigation;
        state.tree_navigation = None;
        state.revision = state.revision.wrapping_add(1);
        state.visible = Some(visible);
        drop(state);
        shared.wake();
    }

    pub(crate) fn shutdown(&self) {
        self.handle().shutdown();
    }
}

impl Handle {
    pub(crate) fn shutdown(&self) {
        for shared in &self.panes {
            let mut state = shared
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            state.stopped = true;
            state.cancel.cancel();
            drop(state);
            shared.wake();
        }
    }
}

impl Drop for Controller {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[derive(Default)]
struct Debouncer {
    first: Option<Instant>,
    last: Option<Instant>,
}

impl Debouncer {
    fn record(&mut self, now: Instant) {
        self.first.get_or_insert(now);
        self.last = Some(now);
    }

    fn deadline(&self, quiet: Duration, maximum: Duration) -> Option<Instant> {
        Some((self.last? + quiet).min(self.first? + maximum))
    }

    fn take_due(&mut self, now: Instant, quiet: Duration, maximum: Duration) -> bool {
        if self
            .deadline(quiet, maximum)
            .is_some_and(|deadline| now >= deadline)
        {
            *self = Self::default();
            return true;
        }
        false
    }
}

struct Target {
    revision: u64,
    pane: PaneId,
    navigation: NavigationId,
    path: std::path::PathBuf,
    cancel: Cancellation,
    baseline: Vec<Row>,
    dirty: Arc<AtomicU64>,
    observed: u64,
    active: Arc<AtomicBool>,
    _subscription: Option<Box<dyn WatchSubscription>>,
    debounce: Debouncer,
    poll_at: Instant,
}

impl Drop for Target {
    fn drop(&mut self) {
        self.active.store(false, Ordering::Release);
    }
}

fn register(
    shared: &SharedOwner,
    native: Option<&dyn WatchService>,
    visible: VisibleDirectory,
    revision: u64,
    cancel: Cancellation,
    timing: Timing,
) -> Target {
    let dirty = Arc::new(AtomicU64::new(0));
    let active = Arc::new(AtomicBool::new(true));
    let callback_dirty = dirty.clone();
    let callback_active = active.clone();
    let callback_shared = Arc::downgrade(shared);
    let callback = Arc::new(move || {
        if callback_active.load(Ordering::Acquire) {
            callback_dirty.fetch_add(1, Ordering::Relaxed);
            if let Some(shared) = callback_shared.upgrade() {
                shared.wake();
            }
        }
    });
    let subscription =
        native.and_then(|native| native.watch_directory(&visible.path, callback).ok());
    let mut baseline = visible
        .entries
        .iter()
        .map(Row::from_entry)
        .collect::<Vec<_>>();
    baseline.sort_unstable_by(|a, b| a.name.cmp(&b.name));
    Target {
        revision,
        pane: visible.pane,
        navigation: visible.navigation,
        path: visible.path,
        cancel,
        baseline,
        dirty,
        observed: 0,
        active,
        _subscription: subscription,
        debounce: Debouncer::default(),
        poll_at: Instant::now() + timing.poll,
    }
}

fn run(
    shared: SharedOwner,
    filesystem: Arc<dyn FileSystem>,
    native: Option<Arc<dyn WatchService>>,
    outbox: Outbox,
    timing: Timing,
) {
    let _ = shared.worker.set(thread::current());
    let mut target: Option<Target> = None;
    let mut tree_target: Option<tree::Runtime> = None;
    loop {
        let (scope, cancel, retired) = {
            let mut state = shared
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if state.stopped {
                break;
            }
            let scope = tree::current(&state);
            let retired = if scope.is_none() {
                state.tree.take()
            } else {
                None
            };
            (scope, state.cancel.clone(), retired)
        };
        drop(retired);
        if let Some(scope) = scope {
            drop(target.take());
            if tree_target
                .as_ref()
                .is_none_or(|runtime| !runtime.same(&scope))
            {
                drop(tree_target.take());
                tree_target = Some(tree::Runtime::new(scope.clone(), cancel));
            }
            let runtime = tree_target.as_mut().expect("tree watch owner");
            runtime.update(scope, &shared, native.as_deref());
            let delay = runtime.step(&shared, filesystem.as_ref(), &outbox, timing);
            if !delay.is_zero() {
                thread::park_timeout(delay);
            }
            continue;
        }
        drop(tree_target.take());
        let update = {
            let state = shared
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if state.stopped {
                break;
            }
            if target
                .as_ref()
                .is_none_or(|target| target.revision != state.revision)
            {
                state
                    .visible
                    .clone()
                    .filter(|visible| {
                        visible.navigation == state.navigation && !state.cancel.is_cancelled()
                    })
                    .map(|visible| (visible, state.revision, state.cancel.clone()))
            } else {
                None
            }
        };
        if let Some((visible, revision, cancel)) = update {
            // Destruction can join native watcher threads; it always stays here.
            drop(target.take());
            target = Some(register(
                &shared,
                native.as_deref(),
                visible,
                revision,
                cancel,
                timing,
            ));
            continue;
        }
        if target
            .as_ref()
            .is_some_and(|target| !shared.is_current(target.revision))
        {
            drop(target.take());
        }
        let Some(target) = target.as_mut() else {
            thread::park();
            continue;
        };
        let version = target.dirty.load(Ordering::Relaxed);
        if version != target.observed {
            target.observed = version;
            target.debounce.record(Instant::now());
        }
        if Instant::now() >= target.poll_at {
            let changed = match probe(filesystem.as_ref(), &target.path, &target.cancel) {
                Ok(rows) => differs(&target.baseline, &rows),
                Err(()) => true,
            };
            if !shared.is_current(target.revision) {
                continue;
            }
            target.poll_at = Instant::now() + timing.poll;
            if changed {
                target.debounce.record(Instant::now());
            }
        }
        let now = Instant::now();
        if target.debounce.take_due(now, timing.quiet, timing.maximum)
            && !send_hint(&shared, target, &outbox)
        {
            if shared.is_current(target.revision) {
                break;
            }
            // Supersession/shutdown may consume the wake in send_hint. Recheck
            // control before parking for the old target's next poll.
            continue;
        }
        let deadline = target
            .debounce
            .deadline(timing.quiet, timing.maximum)
            .map_or(target.poll_at, |deadline| deadline.min(target.poll_at));
        thread::park_timeout(deadline.saturating_duration_since(Instant::now()));
    }
    // Prefer normal scope destruction on this native worker. The startup-owned
    // Shared guard also covers a cancelled task or a panic before this drain.
    let retired = {
        let mut state = shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        (state.tree.take(), state.visible.take())
    };
    drop(tree_target);
    drop(target);
    drop(retired);
}

fn send_hint(shared: &Shared, target: &Target, outbox: &Outbox) -> bool {
    let mut event = Event::DirectoryChanged {
        pane: target.pane,
        navigation: target.navigation,
    };
    loop {
        let result = {
            let state = shared
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if state.stopped || state.revision != target.revision {
                return false;
            }
            outbox.sender.try_send(event)
        };
        match result {
            Ok(()) => {
                (outbox.wakeup)();
                return true;
            }
            Err(TrySendError::Closed(_)) => return false,
            Err(TrySendError::Full(unsent)) => event = unsent,
        }
        // A full result queue must not prevent shutdown or target switching.
        thread::park_timeout(Duration::from_millis(10));
    }
}

#[derive(PartialEq, Eq)]
struct Row {
    name: OsString,
    kind: EntryKind,
    size: Option<u64>,
    modified: Option<SystemTime>,
    compression: CompressionStatus,
    native_attributes: Option<u32>,
    unix_mode: Option<u32>,
}

impl Row {
    fn from_entry(entry: &Entry) -> Self {
        Self {
            name: entry.name.clone(),
            kind: entry.kind,
            // Directory size can be a computed recursive total after sorting.
            size: (entry.kind == EntryKind::File)
                .then_some(entry.metadata.size)
                .flatten(),
            modified: entry.metadata.modified,
            compression: entry.metadata.compression,
            native_attributes: entry.metadata.native_attributes,
            unix_mode: entry.metadata.unix_mode,
        }
    }
}

fn probe(filesystem: &dyn FileSystem, path: &Path, cancel: &Cancellation) -> Result<Vec<Row>, ()> {
    let mut rows = Vec::new();
    let mut failed = false;
    filesystem
        .read_directory(path, cancel, &mut |mut entry| {
            if cancel.is_cancelled() {
                return false;
            }
            let valid_child = entry.path.strip_prefix(path).ok().is_some_and(|relative| {
                let mut parts = relative.components();
                matches!(parts.next(), Some(Component::Normal(_))) && parts.next().is_none()
            });
            if !valid_child {
                failed = true;
                return false;
            }
            match filesystem.entry_metadata(&entry.path) {
                Ok(metadata) => entry.metadata = metadata,
                Err(_) => {
                    failed = true;
                    return false;
                }
            }
            rows.push(Row::from_entry(&entry));
            !cancel.is_cancelled()
        })
        .map_err(|_| ())?;
    if cancel.is_cancelled() || failed {
        return Err(());
    }
    rows.sort_unstable_by(|a, b| a.name.cmp(&b.name));
    Ok(rows)
}

/// Recovery of unknown metadata is also a visible change. Keep baselines until
/// UI acceptance so ignored hints or failed refreshes are periodically retried.
fn differs(baseline: &[Row], current: &[Row]) -> bool {
    baseline != current
}

#[cfg(test)]
#[path = "watch/tests.rs"]
mod tests;
