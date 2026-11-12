//! Accepted native scope, private dirty epochs, and one serial probe/batch owner.
use super::*;
use crate::{
    providers::{MountedTarget, tree::BranchId},
    tree_actor::RootRegistrationLease,
};
use midnight_core::{TreeDiagnostic, TreeEvent, TreeRequestId, TreeRevision, TreeViewStamp};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};

pub(crate) struct ScopeTarget {
    pub(crate) branch: BranchId,
    pub(crate) path: PathBuf,
    pub(crate) display: [u8; 32],
    pub(crate) folders_only: bool,
    pub(crate) path_bytes: usize,
    pub(crate) _mounted: MountedTarget,
    pub(crate) _registration: Arc<RootRegistrationLease>,
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
    epoch: u64,
    acknowledged: u64,
    fence: Option<(RefreshTicket, TreeRevision)>,
    active_request: Option<TreeRequestId>,
    retry_at: Instant,
    batch: u64,
    diagnostic: Option<TreeDiagnostic>,
    exhausted: bool,
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
    fn new(scope: TreeScope, timing: Timing) -> Self {
        Self {
            scope: Arc::new(scope),
            epoch: 0,
            acknowledged: 0,
            fence: None,
            active_request: None,
            retry_at: Instant::now() + timing.poll,
            batch: 0,
            diagnostic: None,
            exhausted: false,
        }
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
            let old = state.tree.replace(TreeControl::new(scope, shared.timing));
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
}

struct Observed {
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
    slots: Vec<Arc<ScopeTarget>>,
    batch: u64,
    cursor: usize,
    epoch: u64,
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
            slots: Vec::new(),
            subscription: None,
            batch: 0,
            cursor: 0,
            epoch: 0,
            debounce: Debouncer::default(),
            hint_pending: false,
            exhausted_reported: false,
        }
    }

    pub(super) fn same(&self, scope: &TreeScope) -> bool {
        same_session(self.scope.stamp, scope.stamp)
    }

    pub(super) fn update(
        &mut self,
        scope: Arc<TreeScope>,
        shared: &SharedOwner,
        native: Option<&dyn WatchService>,
    ) {
        if Arc::ptr_eq(&self.scope, &scope) && !self.slots.is_empty() {
            return;
        }
        self.observed
            .retain(|branch, _| scope.eligible.contains(branch));
        let mut slots = Vec::new();
        if let Some(root) = scope.targets.first() {
            slots.push(root.clone());
        }
        for previous in self.slots.iter().skip(1) {
            if let Some(target) = scope
                .targets
                .iter()
                .find(|target| target.branch == previous.branch && target.path == previous.path)
            {
                slots.push(target.clone());
            }
        }
        for target in &scope.targets {
            if slots.len() >= 32 {
                break;
            }
            if !slots.iter().any(|old| old.branch == target.branch) {
                slots.push(target.clone());
            }
        }
        let changed = self.slots.len() != slots.len()
            || self
                .slots
                .iter()
                .zip(&slots)
                .any(|(old, new)| old.branch != new.branch || old.path != new.path);
        self.scope = scope;
        if !changed {
            return;
        }
        let stamp = self.scope.stamp;
        let batch = {
            let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.stopped || self.cancel.is_cancelled() || state.navigation != stamp.navigation {
                return;
            }
            let Some(tree) = state
                .tree
                .as_mut()
                .filter(|tree| same_session(tree.scope.stamp, stamp))
            else {
                return;
            };
            tree.batch = tree
                .batch
                .checked_add(1)
                .expect("watch batch revision exhausted");
            tree.dirty(); // Covers the registration gap, including callbacks during setup.
            tree.batch
        };
        self.batch = batch;
        drop(self.subscription.take());
        self.slots = slots;
        if self.slots.is_empty() {
            return;
        }
        let paths = self
            .slots
            .iter()
            .map(|target| target.path.clone())
            .collect::<Vec<_>>();
        let branches = self
            .slots
            .iter()
            .map(|target| target.branch)
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
            let Some(tree) = state
                .tree
                .as_mut()
                .filter(|tree| same_session(tree.scope.stamp, stamp) && tree.batch == batch)
            else {
                return;
            };
            if branches.iter().enumerate().any(|(slot, branch)| {
                mask & (1 << slot) != 0 && tree.scope.eligible.contains(branch)
            }) {
                tree.dirty();
                drop(state);
                shared.wake();
            }
        });
        self.subscription =
            native.and_then(|native| native.watch_directories(&paths, callback).ok());
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
            } else if now >= tree.retry_at && tree.active_request.is_none() {
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
                match outbox.sender.try_send(Event::Tree(Box::new(event))) {
                    Ok(()) => {
                        self.exhausted_reported |= tree.exhausted;
                        self.hint_pending = false;
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
        // probes, and the cursor advances regardless of ongoing native callbacks.
        let count = self.scope.targets.len();
        let due = (0..count)
            .map(|offset| (self.cursor + offset) % count)
            .find(|index| {
                self.observed
                    .get(&self.scope.targets[*index].branch)
                    .is_none_or(|old| now >= old.poll_at)
            });
        if let Some(index) = due {
            self.cursor = (index + 1) % count;
            let target = &self.scope.targets[index];
            let result =
                signature::probe(filesystem, &target.path, target.folders_only, &self.cancel);
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
            }) else {
                return Duration::ZERO;
            };
            let old = self.observed.entry(target.branch).or_insert(Observed {
                full: None,
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
            if changed {
                tree.dirty();
            }
            return Duration::ZERO;
        }
        let mut deadline = self
            .observed
            .values()
            .map(|old| old.poll_at)
            .min()
            .unwrap_or(now + timing.poll);
        if let Some(debounce) = self.debounce.deadline(timing.quiet, timing.maximum) {
            deadline = deadline.min(debounce);
        }
        let state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(tree) = &state.tree
            && tree.epoch > tree.acknowledged
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
