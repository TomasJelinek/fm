//! Bounded native change hints and polling, owned entirely by backend workers.

use crate::{
    Outbox, hidden_entries,
    retirement::{RetirementLease, WorkerOwned},
    spawn_resident,
    task_lifetime::TaskLifetime,
};
use midnight_core::{
    Cancellation, Entry, Event, FileSystem, FsError, NavigationId, PaneId, TreeViewStamp,
    VisibleDirectory, WatchService, WatchStatus, WatchSubscription, WatchedView,
};
use midnight_work_budget::resident::Spawner;
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::{self, Thread},
    time::{Duration, Instant},
};
use tokio::{sync::mpsc::error::TrySendError, task::JoinSet};

pub(crate) mod signature;
mod status;
mod supervisor;
pub(crate) mod tree;
use supervisor::{Attempt, Supervision};
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

// A cancelled outer task or a panic outside a worker's loop (see
// `supervisor::run`) can bypass the final drain. The last handle still retires
// captured mounts off the caller without moving subscriptions.
type SharedOwner = Arc<WorkerOwned<Shared>>;

/// Two independent blocking workers own native subscriptions and serial probes.
/// There are no unbounded event/path queues and no worker per filesystem event.
/// A worker that panics restarts in place on its own thread (`supervisor::run`).
pub(crate) struct Controller {
    panes: [SharedOwner; RETIREMENT_CAPACITY],
    /// Dispatched flat listings and fences per pane; tree watches never use them.
    listings: [Arc<Mutex<Listings>>; RETIREMENT_CAPACITY],
}

#[derive(Clone)]
pub(crate) struct Handle {
    panes: [SharedOwner; RETIREMENT_CAPACITY],
}

/// Lets a flat listing's task, which outlives its dispatch, report its progress
/// and outcome.
#[derive(Clone)]
pub(crate) struct Reporter {
    panes: [Weak<WorkerOwned<Shared>>; RETIREMENT_CAPACITY],
    listings: [Arc<Mutex<Listings>>; RETIREMENT_CAPACITY],
}

#[derive(Clone, Copy)]
struct Timing {
    quiet: Duration,
    maximum: Duration,
    poll: Duration,
    /// How often a flat folder whose native registration failed is checked
    /// while the registration is retried: about this often for up to 4,000
    /// entries, less often for larger ones (`unwatched_poll`).
    unwatched: Duration,
    /// The wait before a failed native registration of a flat folder is tried
    /// again; it doubles after each failure, up to `poll`.
    retry: Duration,
    /// How long a flat folder's registration may keep failing before
    /// `PollingOnly` is reported: a retry usually succeeds, and checks find
    /// changes meanwhile.
    report: Duration,
    /// The least time between two hints of a flat target whose folder keeps
    /// changing, without `quiet` between its changes; twice its last check
    /// when that took longer, up to four times this.
    /// Each hint makes the UI list the folder again; a folder a build keeps
    /// writing is then listed at a steady rate, not again as soon as the
    /// previous listing commits. Changes that settled are hinted at once: a
    /// change after a quiet moment, and the last changes of a busy folder (see
    /// `hint_deadline`). Zero disables the limit.
    cooldown: Duration,
    /// The least time between two restarts of a pane's worker; the first
    /// restart after a panic is immediate.
    restart: Duration,
}

/// Bursts settle after 150 ms of quiet, at most 500 ms after their first event;
/// a folder that keeps changing is hinted at most once a second, and its last
/// changes as soon as they settle; polling catches missed notifications every
/// 15 s. A folder without notifications is checked about every second while its
/// registration is retried after 1, 2, 4, 8 and then every 15 s; only a minute
/// of failures is reported. A worker restarts at most once a minute after its
/// first restart.
const PRODUCTION_TIMING: Timing = Timing {
    quiet: Duration::from_millis(150),
    maximum: Duration::from_millis(500),
    poll: Duration::from_secs(15),
    unwatched: Duration::from_secs(1),
    retry: Duration::from_secs(1),
    report: Duration::from_secs(60),
    cooldown: Duration::from_secs(1),
    restart: Duration::from_secs(60),
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
    /// `None` when no listing start was recorded for these rows: a resort of
    /// the UI's rows shows rows listed earlier, and a listing can start before
    /// the folder's counter is published. A resort whose rows the backend
    /// cannot use lists the directory again without a fence, which costs at
    /// most one check.
    fence: Option<Fence>,
    /// The rows leave hidden entries out (Show hidden files is off), as the
    /// listing that produced them did, so checks of them leave them out too.
    hide: bool,
    /// How the listing that produced the rows was watched, if it read the
    /// directory to its end (`Started::done`).
    listed: Option<Listed>,
    /// The authorization helper read the rows (`Listings::authorized`): this
    /// process may not read their folder, so they have no target. A check would
    /// fail, a failed check counts as a change, and the re-read that its hint
    /// starts would be denied and recover to another folder.
    authorized: bool,
}

/// How a listing that read its directory to the end was watched: the counter
/// of a subscription that was in place before it finished reading, if any.
/// A change before a folder's subscription takes effect is never counted, so
/// rows read before then may lack it, and only a check finds it (see
/// `Visible::read_unwatched`).
#[derive(Clone)]
struct Listed {
    counted: Option<Arc<AtomicU64>>,
}

impl Visible {
    /// Whether the rows' listing finished reading the directory before the
    /// subscription that counts `counter` took effect: they then may lack a
    /// change that nothing counted, and are checked at once. A folder is
    /// subscribed only after its listing read entries (`Prepared`) or once it
    /// is accepted (`register`), so a small folder is often read completely
    /// before that, and a file written right after it was listed was shown
    /// only by the periodic check (`Timing::poll`). Without a watch service
    /// nothing is counted, and the rows are checked periodically as before.
    fn read_unwatched(&self, worker: &Worker, counter: &Arc<AtomicU64>) -> bool {
        worker.native.is_some()
            && self.listed.as_ref().is_some_and(|listed| {
                !listed
                    .counted
                    .as_ref()
                    .is_some_and(|counted| Arc::ptr_eq(counted, counter))
            })
    }
}

/// A flat change count when a listing of its directory started.
#[derive(Clone)]
struct Fence {
    counter: Arc<AtomicU64>,
    value: u64,
}

/// The newest flat Navigate, Refresh or Resort the backend dispatched for a pane.
struct Started {
    navigation: NavigationId,
    path: PathBuf,
    /// Recorded for a listing of a directory whose change counter is
    /// published: a target's, or a folder's subscribed while it was listed.
    fence: Option<Fence>,
    /// The listing leaves hidden entries out; `None` for a resort of the UI's
    /// rows, which keep the choice they were listed with.
    hide: Option<bool>,
    /// The listing read entries of the directory (`Reporter::listing_read`).
    read: bool,
    /// It read the directory to its end (`Reporter::listing_done`).
    done: bool,
    /// The counter of a subscription of the directory that was in place
    /// before the listing finished reading: when it started, or taking effect
    /// while it read (`Worker::subscribed`).
    counted: Option<Arc<AtomicU64>>,
    /// It ended without rows to accept; the pane shows its previous rows.
    failed: bool,
}

/// A flat change counter and the directory whose notifications it counts.
struct Published {
    path: PathBuf,
    counter: Arc<AtomicU64>,
    /// Its native subscription is in place (`Worker::subscribed`).
    armed: bool,
}

/// Shared by the backend's command loop and one pane's flat worker.
#[derive(Default)]
struct Listings {
    /// The newest flat target's counter, published before it subscribes.
    counter: Option<Published>,
    /// The counter of a folder subscribed while it is listed (`Prepared`),
    /// published before it subscribes.
    prepared: Option<Published>,
    started: Option<Started>,
    /// Listings dispatched from now on leave hidden entries out
    /// (`Controller::set_hide_hidden`).
    hide: bool,
    /// The newest dispatch whose rows the authorization helper read: an
    /// authorized retry (`Controller::authorized_listing_started`) or a resort
    /// of its rows (`Controller::rows_resorted`). Navigation identities only
    /// grow, so a later dispatch never matches it.
    authorized: Option<NavigationId>,
}

struct Shared {
    state: Mutex<Control>,
    worker: OnceLock<Thread>,
    /// The worker's wake permit (`wake`, `park`).
    woken: AtomicBool,
    timing: Timing,
    /// When the worker restarted after a panic, oldest first.
    #[cfg(test)]
    restarts: Mutex<Vec<Instant>>,
}

impl Shared {
    fn new(timing: Timing) -> Self {
        Self {
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
            woken: AtomicBool::new(false),
            timing,
            #[cfg(test)]
            restarts: Mutex::default(),
        }
    }

    fn stop(&self) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.stopped = true;
        state.cancel.cancel();
        drop(state);
        self.wake();
    }

    fn wake(&self) {
        // Set after the change it announces, so the worker sees that change.
        self.woken.store(true, Ordering::Release);
        if let Some(worker) = self.worker.get() {
            worker.unpark();
        }
    }

    /// Waits on the worker thread until `wake` or `timeout`. A wake while the
    /// worker is busy is kept by `woken`, not by the thread's own unpark token:
    /// anything else that parks the thread takes that token, and the native
    /// registration does (the Windows watch service's reply is a channel
    /// receive). A wake taken that way was lost, and a worker that had no
    /// target yet slept through the acceptance of its pane's listing until
    /// the folder changed again; a change the listing missed was then never
    /// shown.
    fn park(&self, timeout: Option<Duration>) {
        if self.woken.swap(false, Ordering::Acquire) {
            return;
        }
        match timeout {
            Some(timeout) => thread::park_timeout(timeout),
            None => thread::park(),
        }
        // Taken with the wake that ended the wait, so the next wait sleeps; a
        // later wake sets it again.
        self.woken.swap(false, Ordering::Acquire);
    }

    fn is_current(&self, revision: u64) -> bool {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        !state.stopped && state.revision == revision
    }

    fn stopped(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .stopped
    }
}

/// What one pane's worker owns for the whole session, across its restarts.
struct Worker {
    pane: PaneId,
    shared: SharedOwner,
    listings: Arc<Mutex<Listings>>,
    filesystem: Arc<dyn FileSystem>,
    native: Option<Arc<dyn WatchService>>,
    outbox: Outbox,
    timing: Timing,
}

impl Controller {
    pub(crate) fn handle(&self) -> Handle {
        Handle {
            panes: self.panes.clone(),
        }
    }
    pub(crate) fn new(
        resident: &Spawner,
        retirement: Retirement,
        scope: TaskLifetime,
        filesystem: Arc<dyn FileSystem>,
        native: Option<Arc<dyn WatchService>>,
        outbox: Outbox,
        tasks: &mut JoinSet<()>,
    ) -> Self {
        Self::with_timing(
            resident,
            retirement,
            scope,
            filesystem,
            native,
            outbox,
            tasks,
            PRODUCTION_TIMING,
        )
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "watch timing is explicit for tests"
    )]
    fn with_timing(
        resident: &Spawner,
        retirement: Retirement,
        scope: TaskLifetime,
        filesystem: Arc<dyn FileSystem>,
        native: Option<Arc<dyn WatchService>>,
        outbox: Outbox,
        tasks: &mut JoinSet<()>,
        timing: Timing,
    ) -> Self {
        let panes = retirement
            .panes
            .map(|lease| Arc::new(lease.own(Shared::new(timing))));
        let listings: [Arc<Mutex<Listings>>; RETIREMENT_CAPACITY] =
            std::array::from_fn(|_| Arc::default());
        for ((pane, shared), listings) in [PaneId::Left, PaneId::Right]
            .into_iter()
            .zip(&panes)
            .zip(&listings)
        {
            let worker = Worker {
                pane,
                shared: shared.clone(),
                listings: listings.clone(),
                filesystem: filesystem.clone(),
                native: native.clone(),
                outbox: outbox.clone(),
                timing,
            };
            let native_scope = scope.clone();
            // Submit only when this task is polled; the resident ticket keeps
            // both the pane supervisor and its captured scope alive after an abort.
            let resident = resident.clone();
            let rejected = Arc::downgrade(shared);
            tasks.spawn(scope.future(async move {
                let work = native_scope.blocking(move || supervisor::run(worker));
                if let Some(ticket) = spawn_resident(&resident, "watch", Box::new(work)) {
                    let _ = ticket.await;
                } else if let Some(shared) = rejected.upgrade() {
                    shared.stop();
                }
            }));
        }
        Self { panes, listings }
    }

    /// Called by the backend when it dispatches a flat listing of `path` (a
    /// Navigate, a Refresh, or a Resort without the UI's rows), before it clears
    /// the pane's watch scope and before the listing reads the directory. The
    /// listing includes every change counted so far for `path`, by the pane's
    /// flat target or by a folder subscribed while an earlier listing read it
    /// (see `Target::retarget` and `Target::adopt`). A target watching the rows
    /// still shown keeps counting until the listing is accepted, whatever
    /// folder it lists, and watches them again if the listing fails (see
    /// `Reporter::listing_failed`). The listing's rows are checked with the
    /// Show hidden files choice it was dispatched with (`set_hide_hidden`).
    pub(crate) fn listing_started(&self, pane: PaneId, navigation: NavigationId, path: &Path) {
        self.started(pane, navigation, path, true);
    }

    /// Like `listing_started` for a flat Resort of the UI's rows, which were
    /// listed earlier and therefore record no fence and keep the Show hidden
    /// files choice they were listed with.
    pub(crate) fn resort_started(&self, pane: PaneId, navigation: NavigationId, path: &Path) {
        self.started(pane, navigation, path, false);
    }

    /// Like `listing_started` for a listing that the authorization helper
    /// reads (`Command::RetryDirectoryWithElevation`). This process may not
    /// read the folder, so once its rows are accepted the pane watches nothing
    /// (`Visible::authorized`). The listing reports no read
    /// (`Reporter::listing_read`), so the folder is not subscribed while it is
    /// listed either.
    pub(crate) fn authorized_listing_started(
        &self,
        pane: PaneId,
        navigation: NavigationId,
        path: &Path,
    ) {
        self.started(pane, navigation, path, true);
        self.listings[pane.index()]
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .authorized = Some(navigation);
    }

    /// Called with `resort_started` for a Resort that shows the UI's rows of
    /// `source` in another order: rows the authorization helper read stay
    /// unwatched.
    pub(crate) fn rows_resorted(
        &self,
        pane: PaneId,
        source: NavigationId,
        navigation: NavigationId,
    ) {
        let mut listings = self.listings[pane.index()]
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if listings.authorized == Some(source) {
            listings.authorized = Some(navigation);
        }
    }

    /// Called by the backend when Show hidden files changes
    /// (`Command::ShowHiddenEntries`), before it dispatches a listing with the
    /// new choice. A check compares the folder with the accepted rows, so it
    /// leaves out exactly what their listing left out: rows accepted before
    /// keep the choice they were listed with until rows listed with the new one
    /// are accepted (the UI lists every pane again after a change).
    pub(crate) fn set_hide_hidden(&self, hide: bool) {
        for listings in &self.listings {
            listings
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .hide = hide;
        }
    }

    pub(crate) fn reporter(&self) -> Reporter {
        Reporter {
            panes: self.panes.each_ref().map(Arc::downgrade),
            listings: self.listings.clone(),
        }
    }

    fn started(&self, pane: PaneId, navigation: NavigationId, path: &Path, listing: bool) {
        let mut listings = self.listings[pane.index()]
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let published = listing
            .then(|| {
                [&listings.prepared, &listings.counter]
                    .into_iter()
                    .flatten()
                    .find(|published| published.path == path)
            })
            .flatten();
        let fence = published.map(|published| Fence {
            value: published.counter.load(Ordering::Acquire),
            counter: published.counter.clone(),
        });
        // A counter is published before its subscription exists.
        let counted = published
            .filter(|published| published.armed)
            .map(|published| published.counter.clone());
        let hide = listing.then_some(listings.hide);
        listings.started = Some(Started {
            navigation,
            path: path.to_path_buf(),
            fence,
            hide,
            read: false,
            done: false,
            counted,
            failed: false,
        });
    }

    /// Called by the backend only after the UI accepts a complete listing.
    pub(crate) fn set_visible(&self, visible: VisibleDirectory) {
        // Left in place: until the worker sees this acceptance, it still tells
        // from that record whether the pane is listing its directory again.
        let (fence, listed_hide, listed, current_hide, authorized) = {
            let listings = self.listings[visible.pane.index()]
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let started = listings
                .started
                .as_ref()
                .filter(|started| started.navigation == visible.navigation);
            (
                started.and_then(|started| started.fence.clone()),
                started.and_then(|started| started.hide),
                started
                    .filter(|started| started.done)
                    .map(|started| Listed {
                        counted: started.counted.clone(),
                    }),
                listings.hide,
                listings.authorized == Some(visible.navigation),
            )
        };
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
        // A dispatched listing's rows have the choice it recorded. A resort's
        // rows, or rows whose record a newer dispatch replaced, were listed as
        // the shown rows of their folder were, else with the current choice; if
        // that guess is wrong, the hint it causes lists the folder again.
        let hide = listed_hide
            .or_else(|| {
                state
                    .visible
                    .as_ref()
                    .filter(|old| old.directory.path == visible.path)
                    .map(|old| old.hide)
            })
            .unwrap_or(current_hide);
        state.cancel.cancel();
        state.cancel = Cancellation::default();
        state.navigation = visible.navigation;
        state.tree_navigation = None;
        state.revision = state.revision.wrapping_add(1);
        state.visible = Some(Visible {
            directory: visible,
            fence,
            hide,
            listed,
            authorized,
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
            shared.stop();
        }
    }
}

impl Reporter {
    /// Called by a flat listing's task when it has read the first entries of
    /// its directory, before it reads the rest. A folder the pane does not
    /// watch yet is then subscribed while the listing continues (`Prepared`).
    /// A dispatched path is never registered before a listing read it.
    pub(crate) fn listing_read(&self, pane: PaneId, navigation: NavigationId) {
        let index = pane.index();
        {
            let mut listings = self.listings[index]
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            match listings.started.as_mut() {
                Some(started) if started.navigation == navigation && !started.read => {
                    started.read = true;
                }
                // Reported already, or a newer dispatch decides.
                _ => return,
            }
        }
        if let Some(shared) = self.panes[index].upgrade() {
            shared.wake();
        }
    }

    /// Called by a flat listing's task when it has read its directory to the
    /// end, before it publishes the rows. A subscription that takes effect
    /// after this cannot count a change the rows lack, so the rows are
    /// checked once accepted (`Visible::read_unwatched`).
    pub(crate) fn listing_done(&self, pane: PaneId, navigation: NavigationId) {
        let mut listings = self.listings[pane.index()]
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(started) = listings
            .started
            .as_mut()
            .filter(|started| started.navigation == navigation)
        {
            started.done = true;
        }
    }

    /// Called by the backend when the flat listing it dispatched as
    /// `navigation` ended without rows to accept (it failed, offered elevation
    /// or fell back to another directory) and was not superseded. The pane
    /// still shows the rows it accepted before, so a target kept for this
    /// listing, of this or another folder, watches them again (see
    /// `Target::rearm`), and a folder subscribed while it was listed is
    /// released.
    pub(crate) fn listing_failed(&self, pane: PaneId, navigation: NavigationId) {
        let index = pane.index();
        {
            let mut listings = self.listings[index]
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            match listings.started.as_mut() {
                Some(started) if started.navigation == navigation => started.failed = true,
                // A newer dispatch decides about the target.
                _ => return,
            }
        }
        if let Some(shared) = self.panes[index].upgrade() {
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

/// When a flat target's pending burst (`debounce`) may be hinted, given the
/// previous hint (`hinted`) and the duration of its last check (`probe_cost`).
///
/// A burst that settled (no change for `Timing::quiet`) is hinted at once: a
/// change after a quiet moment however soon it follows the previous hint, and
/// the last changes of a folder that kept changing. A burst that goes on is
/// hinted once it lasted `Timing::maximum`, and no sooner than the cooldown
/// after the previous hint (twice the last check, at least `Timing::cooldown`,
/// at most four times it): a folder that keeps changing is listed at a steady
/// rate. The last check's duration does not delay a settled burst: a check made
/// while the disk was busy can take seconds in a small folder, and a large
/// folder is not listed back to back anyway, because no hint is sent while a
/// listing of the folder is pending and a check follows the listings that
/// changes outran (see `serve`).
fn hint_deadline(
    debounce: &Debouncer,
    hinted: Option<Instant>,
    probe_cost: Duration,
    timing: Timing,
) -> Option<Instant> {
    let settled = debounce.last? + timing.quiet;
    let lasted = debounce.first? + timing.maximum;
    let Some(hinted) = hinted.filter(|_| !timing.cooldown.is_zero()) else {
        return Some(settled.min(lasted));
    };
    let cooldown = (probe_cost * 2).clamp(timing.cooldown, timing.cooldown * 4);
    Some(settled.min(lasted.max(hinted + cooldown)))
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
    /// Probes leave hidden entries out, as the accepted rows do; set with
    /// `baseline`, so both always describe the same entries.
    hide: bool,
    /// Change count: native notifications and probes that found a change.
    dirty: Arc<AtomicU64>,
    observed: u64,
    /// Change count up to which the accepted rows include every change: the
    /// fence of the listing they came from (0 for a new target, whose counter
    /// starts after its listing read the directory), or the count before a
    /// probe that found them current.
    accepted_at: u64,
    active: Arc<AtomicBool>,
    _subscription: Option<Box<dyn WatchSubscription>>,
    /// The failed native registration, retried until it succeeds.
    retry: Option<Retry>,
    /// How many rows the accepted listing has, which sets how often a folder
    /// without notifications is checked.
    entries: usize,
    debounce: Debouncer,
    poll_at: Instant,
    /// When the last hint was sent; kept while the target watches listings
    /// of its folder (`retarget`), so `Timing::cooldown` spans them.
    hinted_at: Option<Instant>,
    /// How long the last check read the folder.
    probe_cost: Duration,
}

/// A failed native registration of a flat folder and when to try it again.
/// The folder is checked often meanwhile (`unwatched_poll`), and a success
/// switches to notifications without a status; only a registration that
/// keeps failing for `Timing::report` is reported as `PollingOnly`.
struct Retry {
    at: Instant,
    /// The wait after the next failure.
    delay: Duration,
    /// When the registrations of this target began to fail.
    since: Instant,
}

/// How often the rows of a folder without native notifications are checked:
/// every `Timing::unwatched` for up to 4,000 entries, proportionally less often
/// for larger folders up to the regular `poll`, and never more often than ten
/// times as long as the last check took, so checking costs at most a tenth of
/// the time.
fn unwatched_poll(timing: Timing, entries: usize, probe_cost: Duration) -> Duration {
    let blocks = u32::try_from(entries.div_ceil(4_000).max(1)).unwrap_or(u32::MAX);
    timing
        .unwatched
        .saturating_mul(blocks)
        .min(timing.poll.max(timing.unwatched))
        .max(probe_cost.saturating_mul(10))
}

impl Target {
    /// How long until the rows are checked again.
    fn poll_interval(&self, timing: Timing) -> Duration {
        if self.retry.is_some() {
            unwatched_poll(timing, self.entries, self.probe_cost)
        } else {
            timing.poll
        }
    }

    /// Registers the folder again once a failed registration is due; returns
    /// the status to report.
    fn retry_registration(
        &mut self,
        worker: &Worker,
        supervision: &mut Supervision,
    ) -> Option<WatchStatus> {
        if self
            .retry
            .as_ref()
            .is_none_or(|retry| Instant::now() < retry.at)
        {
            return None;
        }
        let registration = subscribe(worker, supervision, &self.path, &self.dirty, &self.active);
        let (subscription, retry, status) =
            outcome(registration, self.retry.as_ref(), worker.timing);
        if subscription.is_some() {
            // Changes since the last check were not counted: check at once.
            self.poll_at = Instant::now();
        }
        // Destruction can join native watcher threads; it stays on this worker.
        drop(std::mem::replace(&mut self._subscription, subscription));
        self.retry = retry;
        status
    }

    /// When the pending burst may be hinted (see `hint_deadline`).
    fn hint_deadline(&self, timing: Timing) -> Option<Instant> {
        hint_deadline(&self.debounce, self.hinted_at, self.probe_cost, timing)
    }

    /// A refresh/resort of the same directory keeps its change counter: the
    /// target stays subscribed from the dispatch until this acceptance, or
    /// until that listing fails (see `relisting` and `rearm`). A new native
    /// subscription (the directory may have been replaced) is in place before
    /// the old one is released, so there is no registration gap.
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
    /// so they keep `accepted_at`; they are no newer than a check that came due
    /// before this acceptance (one the dispatch superseded, or one due while
    /// the target was parked), so that check runs at once. So do rows read
    /// while the target was not subscribed (`Visible::read_unwatched`).
    fn retarget(
        &mut self,
        worker: &Worker,
        supervision: &mut Supervision,
        visible: Visible,
        revision: u64,
        cancel: Cancellation,
    ) -> Option<WatchStatus> {
        // Read before subscribing again: a notification through the new
        // subscription is counted after this and reaches the debounce.
        let version = self.dirty.load(Ordering::Acquire);
        let unwatched = visible.read_unwatched(worker, &self.dirty);
        let fence = visible
            .fence
            .filter(|fence| Arc::ptr_eq(&fence.counter, &self.dirty));
        if let Some(fence) = &fence {
            self.accepted_at = fence.value;
        }
        let activity = version != self.accepted_at || unwatched;
        let (subscription, retry, status) = outcome(
            subscribe(worker, supervision, &self.path, &self.dirty, &self.active),
            self.retry.as_ref(),
            worker.timing,
        );
        // Destruction can join native watcher threads; it stays on this worker.
        drop(std::mem::replace(&mut self._subscription, subscription));
        self.retry = retry;
        self.revision = revision;
        self.navigation = visible.directory.navigation;
        self.cancel = cancel;
        self.baseline = accepted_signature(&visible.directory.entries);
        self.hide = visible.hide;
        self.entries = visible.directory.entries.len();
        self.observed = version;
        self.debounce = Debouncer::default();
        let now = Instant::now();
        self.poll_at = if activity || (fence.is_none() && self.poll_at <= now) {
            now
        } else {
            now + self.poll_interval(worker.timing)
        };
        status
    }

    /// Makes the folder subscribed while its listing read it the target of
    /// the accepted rows, without registering it again. Every notification it
    /// counted may postdate that listing's read of an entry, unless the
    /// listing fenced against this counter (a later listing of the folder,
    /// dispatched while it was subscribed): the rows are probed at once if a
    /// notification was counted after that fence, or at all without one, and
    /// if they were read before the subscription took effect
    /// (`Visible::read_unwatched`).
    ///
    /// A failed preparation is registered once more, as acceptance registers a
    /// folder that was not prepared; the failure can be transient. A counted
    /// notification can mean that the folder was replaced while it was listed,
    /// leaving the subscription on the removed directory, so it is subscribed
    /// again, keeping its counter, before the prepared subscription is
    /// released.
    fn adopt(
        mut prepared: Prepared,
        worker: &Worker,
        supervision: &mut Supervision,
        visible: Visible,
        revision: u64,
        cancel: Cancellation,
    ) -> (Self, Option<WatchStatus>) {
        prepared.adopted = true;
        let path = std::mem::take(&mut prepared.path);
        let dirty = prepared.dirty.clone();
        let active = prepared.active.clone();
        // Read before subscribing again, as `retarget` does.
        let version = dirty.load(Ordering::Acquire);
        let unwatched = visible.read_unwatched(worker, &dirty);
        let accepted_at = visible
            .fence
            .filter(|fence| Arc::ptr_eq(&fence.counter, &dirty))
            .map_or(0, |fence| fence.value);
        let (subscription, retry, status) = match prepared.subscription.take() {
            Some(Ok(subscription)) if version == 0 => {
                (Some(subscription), None, Some(WatchStatus::Native))
            }
            Some(Ok(replaced)) => {
                let again = outcome(
                    subscribe(worker, supervision, &path, &dirty, &active),
                    None,
                    worker.timing,
                );
                // Destruction can join native watcher threads; it stays here.
                drop(replaced);
                again
            }
            Some(Err(_)) => outcome(
                subscribe(worker, supervision, &path, &dirty, &active),
                None,
                worker.timing,
            ),
            None => (None, None, None),
        };
        worker.publish(&path, &dirty);
        let now = Instant::now();
        let mut target = Self {
            revision,
            pane: visible.directory.pane,
            navigation: visible.directory.navigation,
            baseline: accepted_signature(&visible.directory.entries),
            hide: visible.hide,
            entries: visible.directory.entries.len(),
            path,
            cancel,
            dirty,
            observed: version,
            accepted_at,
            active,
            _subscription: subscription,
            retry,
            debounce: Debouncer::default(),
            poll_at: now,
            hinted_at: None,
            probe_cost: Duration::ZERO,
        };
        if version == accepted_at && !unwatched {
            target.poll_at = now + target.poll_interval(worker.timing);
        }
        (target, status)
    }

    /// Watches the accepted rows again after the listing (of this or another
    /// folder) that made the target stale failed as `navigation`: the pane
    /// still shows these rows and accepts their hints. Subscription, counts,
    /// debounce and poll schedule carry on, so notifications counted
    /// meanwhile are hinted, and a listing that keeps failing is retried no
    /// faster than a current target hints and checks. The failed dispatch
    /// cancelled the control's token; probes get a new one, which the next
    /// dispatch or acceptance cancels.
    fn rearm(&mut self, shared: &Shared, navigation: NavigationId) {
        let mut state = shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        // Otherwise the control changed since `relisting` looked at it, and the
        // next look decides again.
        if state.stopped
            || state.navigation != navigation
            || state.tree.is_some()
            || state.tree_navigation == Some(navigation)
        {
            return;
        }
        state.cancel = Cancellation::default();
        self.revision = state.revision;
        self.cancel = state.cancel.clone();
    }
}

impl Drop for Target {
    fn drop(&mut self) {
        self.active.store(false, Ordering::Release);
    }
}

/// A folder the pane navigates to, subscribed on this worker once its first
/// listing has read entries, so that notifications during the rest of that
/// listing are counted (see `wanted`). It becomes the folder's target when the
/// listing is accepted (`Target::adopt`), and is released on this worker when
/// the newest dispatch no longer lists it. Besides the moment a folder is
/// subscribed again before its old subscription is released, a pane holds at
/// most two flat subscriptions: the shown rows' and one prepared folder's.
struct Prepared {
    path: PathBuf,
    dirty: Arc<AtomicU64>,
    active: Arc<AtomicBool>,
    /// `None` without a watch service. A failure is reported only if it
    /// persists when the folder is accepted (`Target::adopt`).
    subscription: Option<Result<Box<dyn WatchSubscription>, FsError>>,
    /// Its callbacks keep counting for the target that adopted it.
    adopted: bool,
}

impl Drop for Prepared {
    fn drop(&mut self) {
        if !self.adopted {
            self.active.store(false, Ordering::Release);
        }
    }
}

impl Worker {
    /// Publishes the counter of the target that watches `path`; a prepared
    /// folder's counter keeps its subscription.
    fn publish(&self, path: &Path, counter: &Arc<AtomicU64>) {
        let mut listings = self
            .listings
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let armed = match listings.prepared.take() {
            Some(prepared) if Arc::ptr_eq(&prepared.counter, counter) => prepared.armed,
            other => {
                listings.prepared = other;
                false
            }
        };
        listings.counter = Some(Published {
            path: path.to_path_buf(),
            counter: counter.clone(),
            armed,
        });
    }

    /// Records the outcome of a registration of `path` that counts with
    /// `counter`. Once it is in place, a listing that starts later, or the
    /// listing of `path` still reading it, can lack only changes it counts.
    fn subscribed(&self, path: &Path, counter: &Arc<AtomicU64>, armed: bool) {
        let mut listings = self
            .listings
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let Listings {
            counter: published,
            prepared,
            started,
            ..
        } = &mut *listings;
        for published in [published, prepared].into_iter().flatten() {
            if Arc::ptr_eq(&published.counter, counter) {
                published.armed = armed;
            }
        }
        if armed
            && let Some(started) = started
                .as_mut()
                .filter(|started| started.path == path && !started.done)
        {
            started.counted.get_or_insert_with(|| counter.clone());
        }
    }

    /// Releases a prepared folder on this worker; its counter is no longer
    /// published, so later listings of the folder record no fence for it.
    fn release(&self, prepared: Option<Prepared>) {
        let Some(prepared) = prepared else {
            return;
        };
        {
            let mut listings = self
                .listings
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if listings
                .prepared
                .as_ref()
                .is_some_and(|published| Arc::ptr_eq(&published.counter, &prepared.dirty))
            {
                listings.prepared = None;
            }
        }
        // Destruction can join native watcher threads; it stays on this worker.
        drop(prepared);
    }
}

/// A native subscription that counts notifications for one flat directory, or
/// `None` without a watch service.
fn subscribe(
    worker: &Worker,
    supervision: &mut Supervision,
    path: &Path,
    dirty: &Arc<AtomicU64>,
    active: &Arc<AtomicBool>,
) -> Option<Result<Box<dyn WatchSubscription>, FsError>> {
    let native = worker.native.as_deref()?;
    let callback_dirty = dirty.clone();
    let callback_active = active.clone();
    let callback_shared = Arc::downgrade(&worker.shared);
    let callback = Arc::new(move || {
        if callback_active.load(Ordering::Acquire) {
            // Pairs with the acquire loads of a fence (`Controller::started`)
            // and of the count a probe can confirm (`serve`).
            callback_dirty.fetch_add(1, Ordering::Release);
            if let Some(shared) = callback_shared.upgrade() {
                shared.wake();
            }
        }
    });
    let registration = supervision.register(Attempt::Flat(path.to_path_buf()), path, || {
        native.watch_directory(path, callback)
    });
    worker.subscribed(path, dirty, registration.is_ok());
    Some(registration)
}

/// The subscription of a registration outcome, the retry a failure schedules
/// (continuing `failing`, the target's earlier failure, so its backoff and
/// start carry on), and the status it means: `Native` clears an earlier
/// status, and a failure is reported only once registrations kept failing for
/// `Timing::report` (the report skips a status the view already shows).
fn outcome(
    registration: Option<Result<Box<dyn WatchSubscription>, FsError>>,
    failing: Option<&Retry>,
    timing: Timing,
) -> (
    Option<Box<dyn WatchSubscription>>,
    Option<Retry>,
    Option<WatchStatus>,
) {
    match registration {
        None => (None, None, None),
        Some(Ok(subscription)) => (Some(subscription), None, Some(WatchStatus::Native)),
        Some(Err(error)) => {
            let now = Instant::now();
            let (delay, since) =
                failing.map_or((timing.retry, now), |retry| (retry.delay, retry.since));
            let retry = Retry {
                at: now + delay,
                delay: delay.saturating_mul(2).min(timing.poll.max(timing.retry)),
                since,
            };
            let status = (now.saturating_duration_since(since) >= timing.report)
                .then_some(WatchStatus::PollingOnly(error));
            (None, Some(retry), status)
        }
    }
}

/// A target for accepted rows of a folder that was not prepared. `check` probes
/// the rows at once (after a restart lost the target's history); so do rows
/// that a listing read before this subscription (`Visible::read_unwatched`).
fn register(
    worker: &Worker,
    supervision: &mut Supervision,
    visible: Visible,
    revision: u64,
    cancel: Cancellation,
    check: bool,
) -> (Target, Option<WatchStatus>) {
    let dirty = Arc::new(AtomicU64::new(0));
    let check = check || visible.read_unwatched(worker, &dirty);
    let Visible {
        directory: visible,
        hide,
        ..
    } = visible;
    // Published before the subscription exists, so a listing that starts from
    // now on fences against this counter. A fence covers counted changes only:
    // a change before the subscription exists is never counted, and a listing
    // that read the directory before then can lack it until a check (see
    // `Visible::read_unwatched`).
    worker.publish(&visible.path, &dirty);
    let active = Arc::new(AtomicBool::new(true));
    let (subscription, retry, status) = outcome(
        subscribe(worker, supervision, &visible.path, &dirty, &active),
        None,
        worker.timing,
    );
    let now = Instant::now();
    let mut target = Target {
        revision,
        pane: visible.pane,
        navigation: visible.navigation,
        baseline: accepted_signature(&visible.entries),
        hide,
        entries: visible.entries.len(),
        path: visible.path,
        cancel,
        dirty,
        observed: 0,
        accepted_at: 0,
        active,
        _subscription: subscription,
        retry,
        debounce: Debouncer::default(),
        poll_at: now,
        hinted_at: None,
        probe_cost: Duration::ZERO,
    };
    if !check {
        target.poll_at = now + target.poll_interval(worker.timing);
    }
    (target, status)
}

/// Subscribes `path` while its first listing is read; see `Prepared`.
fn prepare(worker: &Worker, supervision: &mut Supervision, path: PathBuf) -> Prepared {
    let dirty = Arc::new(AtomicU64::new(0));
    // Published before the subscription exists, as `register` does.
    worker
        .listings
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .prepared = Some(Published {
        path: path.clone(),
        counter: dirty.clone(),
        armed: false,
    });
    let active = Arc::new(AtomicBool::new(true));
    let subscription = subscribe(worker, supervision, &path, &dirty, &active);
    Prepared {
        path,
        dirty,
        active,
        subscription,
        adopted: false,
    }
}

/// Which folder to subscribe while it is listed.
enum Wanted {
    Nothing,
    /// The prepared folder stays subscribed.
    Prepared,
    Folder(PathBuf),
}

/// The folder of the pane's newest flat dispatch once its listing has read
/// entries (or while a folder subscribed for an earlier listing is listed
/// again), when that dispatch did not fail, is not older than the control's
/// navigation, is not a tree view and lists a folder other than the target's.
/// Empty folders, failed or blocked listings and resorts of the UI's rows read
/// no entries; they keep registration at acceptance.
fn wanted(worker: &Worker, target: Option<&Path>, prepared: Option<&Path>) -> Wanted {
    if worker.native.is_none() {
        return Wanted::Nothing;
    }
    let (navigation, tree_view) = {
        let state = worker
            .shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state.stopped {
            return Wanted::Nothing;
        }
        (
            state.navigation,
            state.tree_navigation == Some(state.navigation),
        )
    };
    let listings = worker
        .listings
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let Some(started) = listings.started.as_ref().filter(|started| {
        !tree_view
            && !started.failed
            && started.navigation >= navigation
            && target != Some(started.path.as_path())
    }) else {
        return Wanted::Nothing;
    };
    if prepared == Some(started.path.as_path()) {
        Wanted::Prepared
    } else if started.read {
        Wanted::Folder(started.path.clone())
    } else {
        Wanted::Nothing
    }
}

/// What the pane's newest dispatch means for a stale flat target.
enum Relisting {
    /// A view without a flat listing: release the target.
    Released,
    /// A flat listing or resort of this or another folder that is not
    /// accepted yet; the pane still shows the target's rows.
    Pending,
    /// That listing, dispatched as this navigation, failed.
    Failed(NavigationId),
}

/// Whether the pane still shows a stale flat target's rows: its newest dispatch
/// is a Navigate, Refresh or Resort of any folder that is not older than the
/// navigation that made the target stale (the backend records it before it
/// clears the pane's watch scope), until it is accepted or reported failed.
/// Archive, network and tree views release the target at once.
fn relisting(shared: &Shared, listings: &Mutex<Listings>) -> Relisting {
    let (navigation, tree_view) = {
        let state = shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        (
            state.navigation,
            state.tree_navigation == Some(state.navigation),
        )
    };
    let listings = listings.lock().unwrap_or_else(|error| error.into_inner());
    match &listings.started {
        Some(started) if !tree_view && started.navigation >= navigation => {
            if started.failed {
                Relisting::Failed(started.navigation)
            } else {
                Relisting::Pending
            }
        }
        _ => Relisting::Released,
    }
}

/// What a pane shows.
enum Shown {
    /// Its current tree scope.
    Tree(TreeViewStamp),
    Flat(ShownRows),
}

/// Accepted flat rows the pane shows, and the revision and token of the target
/// that watches them.
struct ShownRows {
    visible: Visible,
    revision: u64,
    cancel: Cancellation,
    /// The rows are the control's current view. Otherwise a newer flat
    /// listing was dispatched (before this worker saw their acceptance, or
    /// before it restarted) and `relisting` keeps them on screen, so their
    /// target gets a stale revision: `relisting` parks it, or re-arms it once
    /// that listing fails.
    current: bool,
}

/// What the pane shows: its current tree scope, else its accepted flat rows
/// while they are current, or while `relisting` keeps them on screen (a flat
/// listing of this or another folder is pending or failed) and no tree
/// replaced them. Watch statuses (`shown_view`) and flat targets
/// (`flat_update`) both follow it.
fn shown(worker: &Worker) -> Option<Shown> {
    let rows = {
        let state = worker
            .shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state.stopped {
            return None;
        }
        if let Some(scope) = tree::current(&state) {
            return Some(Shown::Tree(scope.stamp));
        }
        let visible = state.visible.as_ref()?;
        let current =
            visible.directory.navigation == state.navigation && !state.cancel.is_cancelled();
        // A tree adopted since these rows were accepted replaced them.
        if !current && state.tree_navigation.is_some() {
            return None;
        }
        ShownRows {
            visible: visible.clone(),
            // Revisions only grow, so the previous one is never current again.
            revision: if current {
                state.revision
            } else {
                state.revision.wrapping_sub(1)
            },
            cancel: state.cancel.clone(),
            current,
        }
    };
    if rows.current {
        return Some(Shown::Flat(rows));
    }
    match relisting(&worker.shared, &worker.listings) {
        Relisting::Released => None,
        Relisting::Pending | Relisting::Failed(_) => Some(Shown::Flat(rows)),
    }
}

/// The view whose rows the pane shows (`shown`). Rows the authorization helper
/// read are not watched, so no status describes them.
fn shown_view(worker: &Worker) -> Option<WatchedView> {
    Some(match shown(worker)? {
        Shown::Tree(stamp) => WatchedView::Tree(stamp),
        Shown::Flat(rows) if rows.visible.authorized => return None,
        Shown::Flat(rows) => WatchedView::Flat {
            pane: worker.pane,
            navigation: rows.visible.directory.navigation,
        },
    })
}

/// The flat rows the pane shows (`shown`) when the target does not watch them
/// yet: current rows the target has no current revision for, or shown rows of
/// another navigation than the target's. Rows the authorization helper read
/// are settled once no target is left.
fn flat_update(worker: &Worker, target: Option<&Target>) -> Option<(Visible, u64, Cancellation)> {
    let Shown::Flat(rows) = shown(worker)? else {
        return None;
    };
    let watched = match target {
        _ if rows.visible.authorized => target.is_none(),
        None => false,
        Some(target) if rows.current => target.revision == rows.revision,
        Some(target) => target.navigation == rows.visible.directory.navigation,
    };
    (!watched).then_some((rows.visible, rows.revision, rows.cancel))
}

/// One pane's worker loop and its final drain. A panic unwinds its targets,
/// prepared folder, tree runtime and subscriptions on this worker, and the
/// supervisor (`supervisor::run`) restarts it.
fn serve(worker: &Worker, supervision: &mut Supervision) {
    let shared = &worker.shared;
    let timing = worker.timing;
    let mut target: Option<Target> = None;
    let mut next: Option<Prepared> = None;
    let mut tree_target: Option<tree::Runtime> = None;
    // After a restart, the history of the unwound target is lost, so the first
    // target's rows are probed at once.
    let mut resumed = supervision.take_resumed();
    if resumed {
        // The counters of the unwound targets are gone with them.
        {
            let mut listings = worker
                .listings
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            listings.counter = None;
            listings.prepared = None;
        }
        supervision.restoring(shown_view(worker));
    }
    loop {
        if !supervision.flush(worker) {
            break;
        }
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
            // A tree's targets start unobserved, so they are probed at once.
            resumed = false;
            drop(target.take());
            worker.release(next.take());
            if tree_target
                .as_ref()
                .is_none_or(|runtime| !runtime.same(&scope))
            {
                drop(tree_target.take());
                tree_target = Some(tree::Runtime::new(scope.clone(), cancel));
            }
            let runtime = tree_target.as_mut().expect("tree watch owner");
            let stamp = scope.stamp;
            if let Some(status) =
                runtime.update(scope, shared, worker.native.as_deref(), supervision, timing)
            {
                supervision.record(WatchedView::Tree(stamp), status);
            }
            let delay = runtime.step(shared, worker.filesystem.as_ref(), &worker.outbox, timing);
            supervision.completed();
            if !delay.is_zero() {
                supervision.park(shared, Some(delay));
            }
            continue;
        }
        drop(tree_target.take());
        if let Some((visible, revision, cancel)) = flat_update(worker, target.as_ref()) {
            if visible.authorized {
                // Neither subscribed nor checked (`Visible::authorized`). The
                // rows of the previous target are no longer shown; destruction
                // can join native watcher threads, so it stays on this worker.
                drop(target.take());
                continue;
            }
            let view = WatchedView::Flat {
                pane: worker.pane,
                navigation: visible.directory.navigation,
            };
            let status = if let Some(current) = target
                .as_mut()
                .filter(|old| old.path == visible.directory.path)
            {
                current.retarget(worker, supervision, visible, revision, cancel)
            } else {
                let (new, status) = match next.take() {
                    Some(prepared) if prepared.path == visible.directory.path => {
                        Target::adopt(prepared, worker, supervision, visible, revision, cancel)
                    }
                    other => {
                        worker.release(other);
                        register(
                            worker,
                            supervision,
                            visible,
                            revision,
                            cancel,
                            std::mem::take(&mut resumed),
                        )
                    }
                };
                // The old target's rows are no longer shown. Its release comes
                // after the new target is in place; destruction can join native
                // watcher threads, so it stays on this worker.
                drop(target.replace(new));
                status
            };
            if let Some(status) = status {
                supervision.record(view, status);
            }
            continue;
        }
        match wanted(
            worker,
            target.as_ref().map(|target| target.path.as_path()),
            next.as_ref().map(|next| next.path.as_path()),
        ) {
            Wanted::Prepared => {}
            Wanted::Nothing => worker.release(next.take()),
            Wanted::Folder(path) => {
                worker.release(next.take());
                next = Some(prepare(worker, supervision, path));
            }
        }
        let stale = target
            .as_ref()
            .filter(|target| !shared.is_current(target.revision))
            .map(|_| relisting(shared, &worker.listings));
        match stale {
            Some(Relisting::Pending) => {
                // Counts the shown rows' notifications for the fence of a
                // listing of their folder, without probes or hints, until a
                // listing is accepted or fails.
                supervision.park(shared, None);
                continue;
            }
            Some(Relisting::Failed(navigation)) => {
                if let Some(target) = target.as_mut() {
                    target.rearm(shared, navigation);
                }
                continue;
            }
            Some(Relisting::Released) => drop(target.take()),
            None => {}
        }
        let Some(target) = target.as_mut() else {
            supervision.park(shared, None);
            continue;
        };
        if let Some(status) = target.retry_registration(worker, supervision) {
            let view = WatchedView::Flat {
                pane: worker.pane,
                navigation: target.navigation,
            };
            supervision.record(view, status);
        }
        // Read before the probe below reads the directory.
        let version = target.dirty.load(Ordering::Acquire);
        if version != target.observed {
            target.observed = version;
            target.debounce.record(Instant::now());
        }
        if Instant::now() >= target.poll_at {
            // Recovery of unknown metadata is also a visible change. Baselines
            // stay until UI acceptance, so later checks repeat an ignored hint,
            // and a failed refresh once its target is re-armed (see `rearm`).
            let started = Instant::now();
            let changed = match probe(
                worker.filesystem.as_ref(),
                &target.path,
                target.hide,
                &target.cancel,
            ) {
                Ok(current) => target.baseline != Some(current),
                Err(()) => true,
            };
            target.probe_cost = started.elapsed();
            supervision.completed();
            if !shared.is_current(target.revision) {
                // Superseded: the check stays due, so rows a resort accepts
                // without a fence are checked at once (see `retarget`).
                continue;
            }
            target.poll_at = Instant::now() + target.poll_interval(timing);
            if changed {
                // Counted like a notification, so a listing that started
                // before this probe is checked again when it is accepted.
                target.observed = target.dirty.fetch_add(1, Ordering::Release).wrapping_add(1);
                target.debounce.record(Instant::now());
            } else {
                // Every change counted up to `version` happened before this
                // probe read the directory, and the directory matched the
                // accepted rows, so a later resort of them needs no probe.
                target.accepted_at = version;
            }
        }
        let now = Instant::now();
        if target
            .hint_deadline(timing)
            .is_some_and(|deadline| now >= deadline)
        {
            target.debounce = Debouncer::default();
            if !send_hint(shared, target, &worker.outbox) {
                if shared.is_current(target.revision) {
                    break;
                }
                // Supersession/shutdown may consume the wake in send_hint.
                // Recheck control before parking for the old target's next poll.
                continue;
            }
            target.hinted_at = Some(now);
        }
        let deadline = target
            .hint_deadline(timing)
            .map_or(target.poll_at, |deadline| deadline.min(target.poll_at));
        let deadline = target
            .retry
            .as_ref()
            .map_or(deadline, |retry| deadline.min(retry.at));
        supervision.park(
            shared,
            Some(deadline.saturating_duration_since(Instant::now())),
        );
    }
    // Prefer normal scope destruction on this native worker. The startup-owned
    // Shared guard also covers a cancelled task or a panic outside this loop.
    let retired = supervisor::take_scope(shared);
    drop(tree_target);
    drop(target);
    worker.release(next);
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

/// The `accepted_signature` of what a listing of `path` would show now. With
/// `hide`, hidden entries are left out by the listings' own rule
/// (`hidden_entries`), decided on the enumerated entry as a listing decides,
/// so a folder whose hidden entries the accepted rows lack is not a change,
/// and neither is a change of those entries alone.
fn probe(
    filesystem: &dyn FileSystem,
    path: &Path,
    hide: bool,
    cancel: &Cancellation,
) -> Result<[u8; 32], ()> {
    let mut current = signature::Signature::default();
    let mut failed = false;
    filesystem
        .read_directory(path, cancel, &mut |mut entry| {
            if cancel.is_cancelled() {
                return false;
            }
            if !crate::folder_sizes::is_immediate_child(path, &entry.path) {
                failed = true;
                return false;
            }
            if hide && hidden_entries::is_hidden(&entry) {
                return !cancel.is_cancelled();
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
