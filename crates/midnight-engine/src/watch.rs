//! Bounded native change hints and polling, owned entirely by backend workers.

use crate::{
    Outbox,
    retirement::{RetirementLease, WorkerOwned},
    task_lifetime::TaskLifetime,
};
use midnight_core::{
    Cancellation, Entry, Event, FileSystem, NavigationId, PaneId, VisibleDirectory, WatchService,
    WatchSubscription,
};
use std::{
    path::{Component, Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::{self, Thread},
    time::{Duration, Instant},
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
    /// Dispatched flat listings and fences per pane; tree watches never use them.
    listings: [Arc<Mutex<Listings>>; RETIREMENT_CAPACITY],
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

/// Bursts settle after 150 ms of quiet, at most 500 ms after their first event;
/// polling catches missed notifications every 15 s.
const PRODUCTION_TIMING: Timing = Timing {
    quiet: Duration::from_millis(150),
    maximum: Duration::from_millis(500),
    poll: Duration::from_secs(15),
};

struct Control {
    visible: Option<Visible>,
    revision: u64,
    cancel: Cancellation,
    stopped: bool,
    navigation: NavigationId,
    tree_navigation: Option<NavigationId>,
    tree: Option<tree::TreeControl>,
}

/// A directory the UI accepted, with the fence of the listing that produced it.
#[derive(Clone)]
struct Visible {
    directory: VisibleDirectory,
    /// `None` when that listing's start was not recorded: a resort shows rows
    /// listed earlier, and a listing can start before a target has published
    /// its counter.
    fence: Option<Fence>,
}

/// A flat target's change count when a listing of its directory started.
#[derive(Clone)]
struct Fence {
    counter: Arc<AtomicU64>,
    value: u64,
}

/// The newest flat Navigate, Refresh or Resort the backend dispatched for a pane.
struct Started {
    navigation: NavigationId,
    path: PathBuf,
    /// Recorded for a Navigate/Refresh listing once a target has published its
    /// counter.
    fence: Option<Fence>,
}

/// Shared by the backend's command loop and one pane's flat worker.
#[derive(Default)]
struct Listings {
    /// Change counter of the newest flat target, published before it subscribes.
    counter: Option<Arc<AtomicU64>>,
    started: Option<Started>,
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
            PRODUCTION_TIMING,
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
        let listings: [Arc<Mutex<Listings>>; RETIREMENT_CAPACITY] =
            std::array::from_fn(|_| Arc::default());
        for (shared, listings) in panes.iter().zip(&listings) {
            let shared = shared.clone();
            let listings = listings.clone();
            let filesystem = filesystem.clone();
            let native = native.clone();
            let outbox = outbox.clone();
            let native_scope = scope.clone();
            // The worker starts when its task first runs; an unpolled task
            // leaves the captured scope to the retained handles.
            tasks.spawn(scope.future(async move {
                crate::actor_thread::run(
                    "midnight-watch",
                    native_scope.blocking(move || {
                        run(shared, listings, filesystem, native, outbox, timing)
                    }),
                )
                .await;
            }));
        }
        Self { panes, listings }
    }

    /// Called by the backend when it dispatches a flat Navigate/Refresh listing
    /// of `path`, before it clears the pane's watch scope and before the listing
    /// reads the directory. The listing includes every change the pane's flat
    /// target has counted so far (see `Target::retarget`), and a target already
    /// watching `path` keeps counting until the listing is accepted.
    pub(crate) fn listing_started(&self, pane: PaneId, navigation: NavigationId, path: &Path) {
        self.started(pane, navigation, path, true);
    }

    /// Like `listing_started` for a flat Resort, which shows rows listed earlier
    /// and therefore records no fence.
    pub(crate) fn resort_started(&self, pane: PaneId, navigation: NavigationId, path: &Path) {
        self.started(pane, navigation, path, false);
    }

    fn started(&self, pane: PaneId, navigation: NavigationId, path: &Path, listing: bool) {
        let mut listings = self.listings[pane.index()]
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let fence = listings
            .counter
            .as_ref()
            .filter(|_| listing)
            .map(|counter| Fence {
                value: counter.load(Ordering::Acquire),
                counter: counter.clone(),
            });
        listings.started = Some(Started {
            navigation,
            path: path.to_path_buf(),
            fence,
        });
    }

    /// Called by the backend only after the UI accepts a complete listing.
    pub(crate) fn set_visible(&self, visible: VisibleDirectory) {
        // Left in place: until the worker sees this acceptance, it still tells
        // from that record whether the pane is listing its directory again.
        let fence = self.listings[visible.pane.index()]
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .started
            .as_ref()
            .filter(|started| started.navigation == visible.navigation)
            .and_then(|started| started.fence.clone());
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
                let old = &old.directory;
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
        state.visible = Some(Visible {
            directory: visible,
            fence,
        });
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
    /// Constant-space multiset signature of the accepted rows (`None` only if
    /// it could not be represented, which makes every probe a change).
    baseline: Option<[u8; 32]>,
    /// Change count: native notifications and probes that found a change.
    dirty: Arc<AtomicU64>,
    observed: u64,
    /// Change count up to which the accepted rows include every change: the
    /// fence of the listing they came from (0 for a new target, whose counter
    /// starts after its listing).
    accepted_at: u64,
    active: Arc<AtomicBool>,
    _subscription: Option<Box<dyn WatchSubscription>>,
    debounce: Debouncer,
    poll_at: Instant,
}

impl Target {
    /// A refresh/resort of the same directory keeps its change counter: the
    /// target stays subscribed from the dispatch until this acceptance (see
    /// `relisting`). A new native subscription (the directory may have been
    /// replaced) is in place before the old one is released, so there is no
    /// registration gap.
    ///
    /// The new rows are probed at once only if a change was counted that they
    /// may lack; no hint reported it for them. A change is counted only after it
    /// happened, and the backend reads the fence before the listing reads the
    /// directory, so the listing includes every change counted up to its fence;
    /// a later count may postdate it. The count when a hint was sent cannot
    /// serve as the fence: a user refresh can start before the hinted change,
    /// and its listing can lack that change although the hint precedes the
    /// acceptance (the UI ignores hints while it loads). Rows without a fence
    /// for this target (a resort, or a listing that started before the target
    /// published its counter) are the accepted rows or were listed after them,
    /// so they keep `accepted_at`.
    fn retarget(
        &mut self,
        shared: &SharedOwner,
        native: Option<&dyn WatchService>,
        visible: Visible,
        revision: u64,
        cancel: Cancellation,
        timing: Timing,
    ) {
        // Read before subscribing again: a notification through the new
        // subscription is counted after this and reaches the debounce.
        let version = self.dirty.load(Ordering::Acquire);
        if let Some(fence) = visible
            .fence
            .filter(|fence| Arc::ptr_eq(&fence.counter, &self.dirty))
        {
            self.accepted_at = fence.value;
        }
        let activity = version != self.accepted_at;
        let subscription = subscribe(shared, native, &self.path, &self.dirty, &self.active);
        // Destruction can join native watcher threads; it stays on this worker.
        drop(std::mem::replace(&mut self._subscription, subscription));
        self.revision = revision;
        self.navigation = visible.directory.navigation;
        self.cancel = cancel;
        self.baseline = accepted_signature(&visible.directory.entries);
        self.observed = version;
        self.debounce = Debouncer::default();
        let now = Instant::now();
        self.poll_at = if activity { now } else { now + timing.poll };
    }
}

impl Drop for Target {
    fn drop(&mut self) {
        self.active.store(false, Ordering::Release);
    }
}

/// A native subscription that counts notifications for one target.
fn subscribe(
    shared: &SharedOwner,
    native: Option<&dyn WatchService>,
    path: &Path,
    dirty: &Arc<AtomicU64>,
    active: &Arc<AtomicBool>,
) -> Option<Box<dyn WatchSubscription>> {
    let callback_dirty = dirty.clone();
    let callback_active = active.clone();
    let callback_shared = Arc::downgrade(shared);
    let callback = Arc::new(move || {
        if callback_active.load(Ordering::Acquire) {
            // Pairs with the fence's acquire load in `Controller::listing_started`.
            callback_dirty.fetch_add(1, Ordering::Release);
            if let Some(shared) = callback_shared.upgrade() {
                shared.wake();
            }
        }
    });
    native.and_then(|native| native.watch_directory(path, callback).ok())
}

fn register(
    shared: &SharedOwner,
    native: Option<&dyn WatchService>,
    listings: &Mutex<Listings>,
    visible: VisibleDirectory,
    revision: u64,
    cancel: Cancellation,
    timing: Timing,
) -> Target {
    let dirty = Arc::new(AtomicU64::new(0));
    // Published before the subscription exists, so a listing that starts from
    // now on fences against the counter that will see its changes.
    listings
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .counter = Some(dirty.clone());
    let active = Arc::new(AtomicBool::new(true));
    let subscription = subscribe(shared, native, &visible.path, &dirty, &active);
    Target {
        revision,
        pane: visible.pane,
        navigation: visible.navigation,
        baseline: accepted_signature(&visible.entries),
        path: visible.path,
        cancel,
        dirty,
        observed: 0,
        accepted_at: 0,
        active,
        _subscription: subscription,
        debounce: Debouncer::default(),
        poll_at: Instant::now() + timing.poll,
    }
}

/// Whether the pane's newest dispatch shows `path` again: a Navigate, Refresh or
/// Resort that is not older than the navigation that made the target stale
/// (the backend records it before it clears the pane's watch scope). Archive,
/// network and tree views, and other directories, release the target at once.
fn relisting(shared: &Shared, listings: &Mutex<Listings>, path: &Path) -> bool {
    let navigation = shared
        .state
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .navigation;
    listings
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .started
        .as_ref()
        .is_some_and(|started| started.navigation >= navigation && started.path == path)
}

fn run(
    shared: SharedOwner,
    listings: Arc<Mutex<Listings>>,
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
                        visible.directory.navigation == state.navigation
                            && !state.cancel.is_cancelled()
                    })
                    .map(|visible| (visible, state.revision, state.cancel.clone()))
            } else {
                None
            }
        };
        if let Some((visible, revision, cancel)) = update {
            if let Some(current) = target
                .as_mut()
                .filter(|old| old.path == visible.directory.path)
            {
                current.retarget(
                    &shared,
                    native.as_deref(),
                    visible,
                    revision,
                    cancel,
                    timing,
                );
                continue;
            }
            // Destruction can join native watcher threads; it always stays here.
            drop(target.take());
            target = Some(register(
                &shared,
                native.as_deref(),
                &listings,
                visible.directory,
                revision,
                cancel,
                timing,
            ));
            continue;
        }
        if let Some(stale) = target
            .as_ref()
            .filter(|target| !shared.is_current(target.revision))
        {
            if relisting(&shared, &listings, &stale.path) {
                // Counts this directory's notifications for the fence of its
                // next listing, without probes or hints, until it is accepted.
                thread::park();
                continue;
            }
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
            // Recovery of unknown metadata is also a visible change. Baselines
            // stay until UI acceptance, so ignored hints and failed refreshes
            // are periodically retried.
            let changed = match probe(filesystem.as_ref(), &target.path, &target.cancel) {
                Ok(current) => target.baseline != Some(current),
                Err(()) => true,
            };
            if !shared.is_current(target.revision) {
                continue;
            }
            target.poll_at = Instant::now() + timing.poll;
            if changed {
                // Counted like a notification, so a listing that started
                // before this probe is checked again when it is accepted.
                target.observed = target.dirty.fetch_add(1, Ordering::Release).wrapping_add(1);
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

/// Order-independent signature of the rows a listing shows: name, kind, file
/// length, time, compression, attributes and mode. Directory sizes are
/// excluded because they can be computed recursive totals.
fn accepted_signature(entries: &[Entry]) -> Option<[u8; 32]> {
    let mut accepted = signature::Signature::default();
    for entry in entries {
        accepted.add(entry).ok()?;
    }
    Some(accepted.digest(false))
}

fn probe(filesystem: &dyn FileSystem, path: &Path, cancel: &Cancellation) -> Result<[u8; 32], ()> {
    let mut current = signature::Signature::default();
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
            signature::listing_metadata(filesystem, &mut entry);
            if current.add(&entry).is_err() {
                failed = true;
                return false;
            }
            !cancel.is_cancelled()
        })
        .map_err(|_| ())?;
    if cancel.is_cancelled() || failed {
        return Err(());
    }
    Ok(current.digest(false))
}

#[cfg(test)]
#[path = "watch/tests.rs"]
mod tests;
