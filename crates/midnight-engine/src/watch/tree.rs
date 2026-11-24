//! Accepted native scope, private dirty epochs, and one serial probe/batch owner.
use super::*;
use crate::{
    providers::{MountedTarget, ProviderTarget, tree::BranchId},
    tree_actor::RootRegistrationLease,
};
use midnight_core::{
    SizeDirectoryIdentity, TreeDiagnostic, TreeEvent, TreeRequestId, TreeRevision, TreeViewStamp,
};
use std::{
    cmp::Reverse,
    collections::{BinaryHeap, HashMap, HashSet},
    path::PathBuf,
};

pub(crate) struct ScopeTarget {
    pub(crate) branch: BranchId,
    pub(crate) path: PathBuf,
    pub(crate) directory_identity: Option<SizeDirectoryIdentity>,
    pub(crate) display: [u8; 32],
    /// Full-directory observation (every entry, with listing metadata) of the
    /// accepted listing, when known. A first probe equal to it is the accepted
    /// state and needs no maintenance; without it, a first probe always does.
    pub(crate) full: Option<[u8; 32]>,
    pub(crate) folders_only: bool,
    /// Probes leave hidden entries out, as the accepted listing did: the root
    /// while its rows are still the flat listing the tree was adopted from and
    /// that listing left them out. The tree's own listings (an expanded branch,
    /// the root once listed again) keep them in the model and so in probes.
    pub(crate) hide: bool,
    /// The branch is a directory link: a denied Windows compatibility junction is
    /// probed through its target, as its listing was (`browse_listing`).
    pub(crate) link: bool,
    pub(crate) path_bytes: usize,
    pub(crate) _mounted: MountedTarget,
    pub(crate) _registration: Arc<RootRegistrationLease>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum NotificationKey {
    Directory(SizeDirectoryIdentity),
    Branch(BranchId),
}

impl ScopeTarget {
    fn notification_key(&self) -> NotificationKey {
        self.directory_identity.map_or(
            NotificationKey::Branch(self.branch),
            NotificationKey::Directory,
        )
    }

    fn same_basis(&self, other: &Self) -> bool {
        self.branch == other.branch
            && self.path == other.path
            && self.directory_identity == other.directory_identity
    }

    fn same_notification_directory(&self, other: &Self) -> bool {
        match (self.directory_identity, other.directory_identity) {
            (Some(left), Some(right)) => left == right,
            _ => self.same_basis(other),
        }
    }

    fn probe(
        &self,
        filesystem: &dyn FileSystem,
        cancel: &Cancellation,
    ) -> Result<signature::Observation, ()> {
        if cancel.is_cancelled() {
            return Err(());
        }
        self.check_native_target()?;
        self.check_directory_identity(filesystem, cancel)?;
        let observation = signature::probe_shown(
            filesystem,
            &self.path,
            self.folders_only,
            self.hide,
            self.link,
            cancel,
        )?;
        self.check_directory_identity(filesystem, cancel)?;
        self.check_native_target()?;
        if cancel.is_cancelled() {
            return Err(());
        }
        Ok(observation)
    }

    fn check_native_target(&self) -> Result<(), ()> {
        match self._mounted.value().map_err(|_| ())? {
            ProviderTarget::Native(path) if path == &self.path => Ok(()),
            _ => Err(()),
        }
    }

    fn check_directory_identity(
        &self,
        filesystem: &dyn FileSystem,
        cancel: &Cancellation,
    ) -> Result<(), ()> {
        let Some(expected) = self.directory_identity else {
            return Ok(());
        };
        let current = filesystem
            .browse_directory_identity(&self.path, cancel)
            .map_err(|_| ())?;
        (current == Some(expected)).then_some(()).ok_or(())
    }
}

#[derive(Clone)]
pub(crate) struct TreeScope {
    pub(crate) stamp: TreeViewStamp,
    pub(crate) targets: Vec<Arc<ScopeTarget>>,
    pub(crate) eligible: HashSet<BranchId>,
    pub(crate) complete: bool,
}

pub(super) struct TreeControl {
    scope: Arc<TreeScope>,
    notification_keys: HashSet<NotificationKey>,
    epoch: u64,
    acknowledged: u64,
    fence: Option<(RefreshTicket, TreeRevision)>,
    active_request: Option<TreeRequestId>,
    retry_at: Instant,
    batch: u64,
    /// The replaced batch while its native subscription is still registered:
    /// its callbacks count until the subscription is released, so swapping
    /// batches leaves no registration gap.
    retiring: Option<u64>,
    diagnostic: Option<TreeDiagnostic>,
    exhausted: bool,
    /// Send times of the session's last `Changed` hints, oldest first.
    #[cfg(test)]
    hints: std::collections::VecDeque<Instant>,
}

/// Test view of a session's dirty epochs and hint pacing.
#[cfg(test)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct TreeFence {
    pub(super) epoch: u64,
    pub(super) acknowledged: u64,
    /// The dirty epoch a successful refresh captured, until an exact Commit
    /// acknowledges it.
    pub(super) fence: Option<u64>,
    /// Send times of the session's last 16 `Changed` hints, oldest first,
    /// recorded by the worker under the control lock.
    pub(super) hints: Vec<Instant>,
}

#[derive(Clone, Copy)]
pub(crate) struct RefreshTicket {
    stamp: TreeViewStamp,
    request: TreeRequestId,
    epoch: u64,
}

fn same_session(left: TreeViewStamp, right: TreeViewStamp) -> bool {
    left.pane == right.pane && left.navigation == right.navigation && left.session == right.session
}

impl TreeControl {
    fn new(scope: TreeScope, timing: Timing, notification_keys: HashSet<NotificationKey>) -> Self {
        Self {
            scope: Arc::new(scope),
            notification_keys,
            epoch: 0,
            acknowledged: 0,
            fence: None,
            active_request: None,
            retry_at: Instant::now() + timing.poll,
            batch: 0,
            retiring: None,
            diagnostic: None,
            exhausted: false,
            #[cfg(test)]
            hints: std::collections::VecDeque::new(),
        }
    }

    #[cfg(test)]
    fn record_hint(&mut self, sent: Instant) {
        if self.hints.len() == 16 {
            self.hints.pop_front();
        }
        self.hints.push_back(sent);
    }

    fn dirty(&mut self) {
        if let Some(next) = self.epoch.checked_add(1) {
            self.epoch = next;
        } else {
            self.exhausted = true;
        }
    }
}

impl Handle {
    /// Called on the model worker after successful exact Commit. Retired scopes
    /// are released here, after unlocking, never on a UI callback/native callback.
    pub(crate) fn set_tree_scope(&self, scope: TreeScope) {
        // Build bounded membership on the model worker, outside the callback's lock.
        let notification_keys = scope
            .targets
            .iter()
            .filter(|target| scope.eligible.contains(&target.branch))
            .map(|target| target.notification_key())
            .collect();
        let shared = &self.panes[scope.stamp.pane.index()];
        let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.stopped || scope.stamp.navigation < state.navigation {
            return;
        }
        state.tree_navigation = Some(scope.stamp.navigation);
        let same = state
            .tree
            .as_ref()
            .is_some_and(|tree| same_session(tree.scope.stamp, scope.stamp))
            && !state.cancel.is_cancelled();
        if same
            && state
                .tree
                .as_ref()
                .is_some_and(|tree| tree.scope.stamp.revision.0 > scope.stamp.revision.0)
        {
            return;
        }
        let retired = if same {
            let tree = state.tree.as_mut().expect("same tree session");
            tree.notification_keys = notification_keys;
            if scope.complete {
                if let Some((ticket, revision)) = tree.fence
                    && scope.stamp.revision.0 >= revision.0
                {
                    tree.acknowledged = tree.acknowledged.max(ticket.epoch);
                    tree.fence = None;
                }
                tree.diagnostic = None;
            }
            Some(std::mem::replace(&mut tree.scope, Arc::new(scope)))
        } else {
            state.cancel.cancel();
            state.cancel = Cancellation::default();
            state.navigation = scope.stamp.navigation;
            let old = state
                .tree
                .replace(TreeControl::new(scope, shared.timing, notification_keys));
            old.map(|tree| tree.scope)
        };
        state.revision = state
            .revision
            .checked_add(1)
            .expect("watch control revision exhausted");
        drop(state);
        drop(retired);
        shared.wake();
    }

    /// Successful Commit survives failed scope admission. Only eligible previous
    /// targets survive; zero-target state still emits paced maintenance retries.
    pub(crate) fn scope_failed(
        &self,
        stamp: TreeViewStamp,
        eligible: HashSet<BranchId>,
        error: TreeDiagnostic,
    ) -> bool {
        let shared = &self.panes[stamp.pane.index()];
        let previous = {
            let state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
            state
                .tree
                .as_ref()
                .filter(|tree| same_session(tree.scope.stamp, stamp))
                .map(|tree| tree.scope.clone())
        };
        let targets = previous.as_ref().map_or_else(Vec::new, |scope| {
            scope
                .targets
                .iter()
                .filter(|target| eligible.contains(&target.branch))
                .cloned()
                .collect()
        });
        self.set_tree_scope(TreeScope {
            stamp,
            targets,
            eligible,
            complete: false,
        });
        let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.stopped || state.cancel.is_cancelled() || state.navigation != stamp.navigation {
            return false;
        }
        let Some(tree) = state
            .tree
            .as_mut()
            .filter(|tree| same_session(tree.scope.stamp, stamp))
        else {
            return false;
        };
        let changed = tree.diagnostic.as_ref() != Some(&error);
        tree.diagnostic = Some(error);
        if changed {
            tree.dirty();
        }
        drop(state);
        shared.wake();
        changed
    }

    pub(crate) fn begin_refresh(
        &self,
        stamp: TreeViewStamp,
        request: TreeRequestId,
    ) -> Option<RefreshTicket> {
        let shared = &self.panes[stamp.pane.index()];
        let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.stopped || state.cancel.is_cancelled() || state.navigation != stamp.navigation {
            return None;
        }
        let tree = state
            .tree
            .as_mut()
            .filter(|tree| same_session(tree.scope.stamp, stamp))?;
        tree.active_request = Some(request);
        Some(RefreshTicket {
            stamp,
            request,
            epoch: tree.epoch,
        })
    }

    pub(crate) fn finish_refresh(
        &self,
        ticket: RefreshTicket,
        success: bool,
        final_revision: TreeRevision,
    ) {
        let shared = &self.panes[ticket.stamp.pane.index()];
        let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.stopped
            || state.cancel.is_cancelled()
            || state.navigation != ticket.stamp.navigation
        {
            return;
        }
        let Some(tree) = state.tree.as_mut().filter(|tree| {
            same_session(tree.scope.stamp, ticket.stamp)
                && tree.active_request == Some(ticket.request)
        }) else {
            return;
        };
        tree.active_request = None;
        tree.retry_at = Instant::now() + shared.timing.poll;
        if success {
            tree.fence = Some((ticket, final_revision));
        }
        drop(state);
        shared.wake();
    }

    /// Immediate logical cancellation; native teardown and large scope retirement
    /// remain with the watch/model workers even if a probe is blocked.
    pub(crate) fn clear_scope(&self, pane: PaneId, navigation: NavigationId) {
        let shared = &self.panes[pane.index()];
        let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
        if navigation <= state.navigation {
            return;
        }
        state.navigation = navigation;
        state.cancel.cancel();
        state.revision = state
            .revision
            .checked_add(1)
            .expect("watch control revision exhausted");
        drop(state);
        shared.wake();
    }

    /// Terminal model failure cannot leave autonomous probes for a dead session.
    /// Keep tree_navigation so a delayed flat acknowledgment cannot revive it.
    pub(crate) fn retire_session(&self, stamp: TreeViewStamp) {
        let shared = &self.panes[stamp.pane.index()];
        let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
        if !state.stopped
            && state.navigation == stamp.navigation
            && state
                .tree
                .as_ref()
                .is_some_and(|tree| same_session(tree.scope.stamp, stamp))
        {
            state.cancel.cancel();
            state.revision = state
                .revision
                .checked_add(1)
                .expect("watch control revision exhausted");
        }
        drop(state);
        shared.wake();
    }

    #[cfg(test)]
    pub(super) fn tree_fence(&self, pane: PaneId) -> Option<TreeFence> {
        let state = self.panes[pane.index()]
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let tree = state.tree.as_ref()?;
        Some(TreeFence {
            epoch: tree.epoch,
            acknowledged: tree.acknowledged,
            fence: tree.fence.map(|(ticket, _)| ticket.epoch),
            hints: tree.hints.iter().copied().collect(),
        })
    }
}

struct Observed {
    basis: Arc<ScopeTarget>,
    full: Option<[u8; 32]>,
    failed: bool,
    poll_at: Instant,
}

pub(super) struct Runtime {
    // Tear down native callbacks before releasing their retained provider scope.
    subscription: Option<Box<dyn WatchSubscription>>,
    scope: Arc<TreeScope>,
    cancel: Cancellation,
    observed: HashMap<BranchId, Observed>,
    /// Next poll deadline of every scope target (by index), rebuilt whenever
    /// the scope changes. With up to 10,000 eligible branches, finding the
    /// next due target and the park deadline is O(log T) instead of O(T).
    due: BinaryHeap<Reverse<(Instant, usize)>>,
    slots: Vec<Arc<ScopeTarget>>,
    batch: u64,
    /// When a batch whose native registration failed may be registered again
    /// with a later scope update.
    retry_batch_at: Option<Instant>,
    epoch: u64,
    /// The dirty epoch the last sent `Changed` hint covered. Only an epoch
    /// hinted already is retried at the poll cadence; a newer one waits for
    /// its debounce.
    hinted: u64,
    debounce: Debouncer,
    hint_pending: bool,
    exhausted_reported: bool,
}

impl Runtime {
    pub(super) fn new(scope: Arc<TreeScope>, cancel: Cancellation) -> Self {
        Self {
            scope,
            cancel,
            observed: HashMap::new(),
            due: BinaryHeap::new(),
            slots: Vec::new(),
            subscription: None,
            batch: 0,
            retry_batch_at: None,
            epoch: 0,
            hinted: 0,
            debounce: Debouncer::default(),
            hint_pending: false,
            exhausted_reported: false,
        }
    }

    pub(super) fn same(&self, scope: &TreeScope) -> bool {
        same_session(self.scope.stamp, scope.stamp)
    }

    /// Unobserved targets and `due_now` are due now, in scope order; other
    /// observed ones keep their next poll time.
    fn schedule(&mut self, due_now: &[Arc<ScopeTarget>]) {
        let now = Instant::now();
        self.due = self
            .scope
            .targets
            .iter()
            .enumerate()
            .map(|(index, target)| {
                let at = match self.observed.get(&target.branch) {
                    Some(old) if !due_now.iter().any(|due| Arc::ptr_eq(due, target)) => old.poll_at,
                    _ => now,
                };
                Reverse((at, index))
            })
            .collect();
    }

    /// Adopts `scope`, registering a new native batch when its slots change.
    /// Returns the status of a batch it registered.
    pub(super) fn update(
        &mut self,
        scope: Arc<TreeScope>,
        shared: &SharedOwner,
        native: Option<&dyn WatchService>,
        supervision: &mut Supervision,
        timing: Timing,
    ) -> Option<WatchStatus> {
        if Arc::ptr_eq(&self.scope, &scope) && !self.slots.is_empty() {
            return None;
        }
        let targets = scope
            .targets
            .iter()
            .map(|target| (target.branch, target))
            .collect::<HashMap<_, _>>();
        self.observed.retain(|branch, observed| {
            let Some(target) = targets.get(branch) else {
                return false;
            };
            // An observation that left hidden entries out, or kept them, cannot
            // be compared with the other kind: the target starts unobserved.
            if !scope.eligible.contains(branch)
                || !observed.basis.same_basis(target)
                || observed.basis.hide != target.hide
            {
                return false;
            }
            observed.basis = Arc::clone(target);
            true
        });
        drop(targets);
        let mut slots = Vec::new();
        if let Some(root) = scope.targets.first() {
            slots.push(root.clone());
        }
        for previous in self.slots.iter().skip(1) {
            if let Some(target) = scope
                .targets
                .iter()
                .find(|target| target.same_basis(previous))
                .or_else(|| {
                    scope
                        .targets
                        .iter()
                        .find(|target| target.same_notification_directory(previous))
                })
                && !slots
                    .iter()
                    .any(|old| target.same_notification_directory(old))
            {
                slots.push(target.clone());
            }
        }
        for target in &scope.targets {
            if slots.len() >= 32 {
                break;
            }
            if !slots
                .iter()
                .any(|old| target.same_notification_directory(old))
            {
                slots.push(target.clone());
            }
        }
        let changed = self.slots.len() != slots.len()
            || self
                .slots
                .iter()
                .zip(&slots)
                .any(|(old, new)| !old.same_notification_directory(new));
        // A batch whose native registration failed is registered again with a
        // later scope update (an expansion, a collapse or a refresh's Commit),
        // at most once per poll interval. A refresh alone keeps the slots.
        let retry = !changed
            && native.is_some()
            && self
                .retry_batch_at
                .is_some_and(|retry_at| Instant::now() >= retry_at);
        self.scope = scope;
        if !changed && !retry {
            self.schedule(&[]);
            // A surviving alias may represent the same physical slot. Retain
            // only its current provider context while keeping the subscription.
            self.slots = slots;
            return None;
        }
        // A directory joining the batch was only polled (or not observed yet),
        // so probe it at once instead of requesting a sweep of every branch.
        // Directories in both batches stay covered by the overlap below. A
        // retried batch had no subscription, so all of its directories join.
        let joined = if retry {
            slots.clone()
        } else {
            slots
                .iter()
                .filter(|new| {
                    !self
                        .slots
                        .iter()
                        .any(|old| old.same_notification_directory(new))
                })
                .cloned()
                .collect::<Vec<_>>()
        };
        self.schedule(&joined);
        let stamp = self.scope.stamp;
        let batch = {
            let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.stopped || self.cancel.is_cancelled() || state.navigation != stamp.navigation {
                return None;
            }
            let tree = state
                .tree
                .as_mut()
                .filter(|tree| same_session(tree.scope.stamp, stamp))?;
            let replaced = tree.batch;
            tree.batch = tree
                .batch
                .checked_add(1)
                .expect("watch batch revision exhausted");
            // Callbacks during setup count for the new batch; those of the
            // replaced one count until its subscription is released below.
            tree.retiring = self.subscription.is_some().then_some(replaced);
            tree.batch
        };
        self.batch = batch;
        // The new subscription is registered before the old one is released,
        // and old slots keep their provider scope until their callbacks end.
        let replaced = self.subscription.take();
        let retired = std::mem::replace(&mut self.slots, slots);
        let status = self.subscribe(stamp, batch, shared, native, supervision, timing);
        // Destruction can join native watcher threads; it stays on this worker.
        drop(replaced);
        drop(retired);
        let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(tree) = state
            .tree
            .as_mut()
            .filter(|tree| same_session(tree.scope.stamp, stamp) && tree.batch == batch)
        {
            tree.retiring = None;
        }
        status
    }

    /// Registers the batch of the current slots. A failed batch leaves its
    /// targets polled (no loop creates single-directory replacements).
    fn subscribe(
        &mut self,
        stamp: TreeViewStamp,
        batch: u64,
        shared: &SharedOwner,
        native: Option<&dyn WatchService>,
        supervision: &mut Supervision,
        timing: Timing,
    ) -> Option<WatchStatus> {
        self.retry_batch_at = None;
        let native = native?;
        if self.slots.is_empty() {
            return None;
        }
        let paths = self
            .slots
            .iter()
            .map(|target| target.path.clone())
            .collect::<Vec<_>>();
        let keys = self
            .slots
            .iter()
            .map(|target| target.notification_key())
            .collect::<Vec<_>>();
        let weak = Arc::downgrade(shared);
        let callback = Arc::new(move |mask: u32| {
            let Some(shared) = weak.upgrade() else {
                return;
            };
            let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.stopped || state.cancel.is_cancelled() || state.navigation != stamp.navigation
            {
                return;
            }
            let Some(tree) = state.tree.as_mut().filter(|tree| {
                same_session(tree.scope.stamp, stamp)
                    && (tree.batch == batch || tree.retiring == Some(batch))
            }) else {
                return;
            };
            if keys
                .iter()
                .enumerate()
                .any(|(slot, key)| mask & (1 << slot) != 0 && tree.notification_keys.contains(key))
            {
                tree.dirty();
                drop(state);
                shared.wake();
            }
        });
        let attempt = Attempt::Tree(stamp.navigation, stamp.session);
        match supervision.register(attempt, &paths[0], || {
            native.watch_directories(&paths, callback)
        }) {
            Ok(subscription) => {
                self.subscription = Some(subscription);
                Some(WatchStatus::Native)
            }
            Err(error) => {
                self.retry_batch_at = Some(Instant::now() + timing.poll);
                Some(WatchStatus::PollingOnly(error))
            }
        }
    }

    pub(super) fn step(
        &mut self,
        shared: &SharedOwner,
        filesystem: &dyn FileSystem,
        outbox: &Outbox,
        timing: Timing,
    ) -> Duration {
        let now = Instant::now();
        {
            let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.stopped
                || self.cancel.is_cancelled()
                || state.navigation != self.scope.stamp.navigation
            {
                return Duration::ZERO;
            }
            let Some(tree) = state
                .tree
                .as_mut()
                .filter(|tree| same_session(tree.scope.stamp, self.scope.stamp))
            else {
                return Duration::ZERO;
            };
            if tree.epoch != self.epoch {
                self.epoch = tree.epoch;
                self.debounce.record(now);
            }
            if tree.epoch == tree.acknowledged && !tree.exhausted {
                self.debounce = Debouncer::default();
                self.hint_pending = false;
            } else if now >= tree.retry_at
                && tree.active_request.is_none()
                && tree.epoch <= self.hinted
            {
                // A hint the UI ignored, or whose sweep failed, is retried at
                // the poll cadence. A newer change waits for its debounce.
                self.hint_pending = true;
            }
            if self.debounce.take_due(now, timing.quiet, timing.maximum) {
                self.hint_pending = true;
            }
            let event = if tree.exhausted && !self.exhausted_reported {
                Some(TreeEvent::WatchFailed {
                    stamp: tree.scope.stamp,
                    error: TreeDiagnostic::new(
                        "Native watch epoch exhausted; reopen this directory.",
                    ),
                })
            } else if self.hint_pending && tree.active_request.is_none() {
                Some(TreeEvent::Changed {
                    stamp: tree.scope.stamp,
                })
            } else {
                None
            };
            if let Some(event) = event {
                let hint = matches!(event, TreeEvent::Changed { .. });
                match outbox.sender.try_send(Event::Tree(Box::new(event))) {
                    Ok(()) => {
                        self.exhausted_reported |= tree.exhausted;
                        self.hint_pending = false;
                        // The hint covers every change counted so far; a later
                        // one arms the debounce again.
                        self.debounce = Debouncer::default();
                        if hint {
                            self.hinted = tree.epoch;
                            #[cfg(test)]
                            tree.record_hint(now);
                        }
                        tree.retry_at = now + timing.poll;
                        drop(state);
                        (outbox.wakeup)();
                    }
                    Err(TrySendError::Closed(_)) => {
                        state.stopped = true;
                        return Duration::ZERO;
                    }
                    Err(TrySendError::Full(_)) => {}
                }
            }
        }
        // Exactly one due target per iteration: control and due output run between
        // probes, and targets are served in deadline order regardless of
        // ongoing native callbacks.
        let due = self
            .due
            .peek()
            .filter(|Reverse((at, _))| now >= *at)
            .map(|Reverse((_, index))| *index);
        if let Some(index) = due {
            self.due.pop();
            let target = &self.scope.targets[index];
            let result = target.probe(filesystem, &self.cancel);
            let checked_scope = {
                let state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
                if state.stopped
                    || self.cancel.is_cancelled()
                    || state.navigation != self.scope.stamp.navigation
                {
                    return Duration::ZERO;
                }
                let Some(tree) = state.tree.as_ref().filter(|tree| {
                    same_session(tree.scope.stamp, self.scope.stamp)
                        && tree.scope.eligible.contains(&target.branch)
                }) else {
                    return Duration::ZERO;
                };
                tree.scope.clone()
            };
            // Scope membership can be large. Inspect it without holding the
            // callback's mutex, then require that exact scope when accepting I/O.
            if !Arc::ptr_eq(&checked_scope, &self.scope)
                && !checked_scope
                    .targets
                    .iter()
                    .any(|current| Arc::ptr_eq(current, target))
            {
                return Duration::ZERO;
            }
            let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.stopped
                || self.cancel.is_cancelled()
                || state.navigation != self.scope.stamp.navigation
            {
                return Duration::ZERO;
            }
            let Some(tree) = state.tree.as_mut().filter(|tree| {
                same_session(tree.scope.stamp, self.scope.stamp)
                    && tree.scope.eligible.contains(&target.branch)
                    && Arc::ptr_eq(&tree.scope, &checked_scope)
            }) else {
                return Duration::ZERO;
            };
            // A first observation compares with the accepted listing, so a
            // target whose directory is unchanged since then stays quiet.
            let old = self.observed.entry(target.branch).or_insert(Observed {
                basis: target.clone(),
                full: target.full,
                failed: false,
                poll_at: now,
            });
            let changed = match result {
                Ok(observation) => {
                    let changed = old.failed
                        || old.full != Some(observation.full)
                        || (observation.display != target.display
                            && tree.epoch == tree.acknowledged);
                    old.failed = false;
                    old.full = Some(observation.full);
                    changed
                }
                Err(()) => {
                    let changed = !old.failed || tree.epoch == tree.acknowledged;
                    old.failed = true;
                    changed
                }
            };
            old.poll_at = Instant::now() + timing.poll;
            self.due.push(Reverse((old.poll_at, index)));
            if changed {
                tree.dirty();
            }
            return Duration::ZERO;
        }
        let mut deadline = self
            .due
            .peek()
            .map_or(now + timing.poll, |Reverse((at, _))| *at);
        if let Some(debounce) = self.debounce.deadline(timing.quiet, timing.maximum) {
            deadline = deadline.min(debounce);
        }
        let state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(tree) = &state.tree
            && tree.epoch > tree.acknowledged
            && tree.epoch <= self.hinted
            && tree.active_request.is_none()
        {
            deadline = deadline.min(tree.retry_at);
        }
        if self.hint_pending {
            deadline = deadline.min(now + Duration::from_millis(10));
        }
        deadline.saturating_duration_since(Instant::now())
    }
}

pub(super) fn current(state: &Control) -> Option<Arc<TreeScope>> {
    state
        .tree
        .as_ref()
        .filter(|tree| {
            !state.cancel.is_cancelled() && tree.scope.stamp.navigation == state.navigation
        })
        .map(|tree| tree.scope.clone())
}

#[cfg(test)]
#[path = "tree_state_tests.rs"]
mod tests;
