//! Backend-owned inline-tree session and bounded branch work.
//!
//! Construction, projection rebuilding and final destruction run on backend
//! workers. Only branch jobs enumerate; never hold a session lock while they run.
use crate::{
    providers::{
        FilesystemProvider, OpenedLocation, ProviderError, ProviderRegistry,
        tree::{
            BranchId, BranchResult, BranchWork, DirectoryTree, Disclosure, TreeLimits, TreeRequest,
        },
    },
    tree_projection::{ProjectionOptions, ProjectionState},
    tree_selection::Selection,
};
use midnight_core::{
    Cancellation, ProviderId, ProviderInstanceId, SelectionRevision, StoredLocation,
    TreeProjection, TreePublication, TreeRequestId, TreeRootSelection, TreeRootSnapshot, TreeRowId,
    TreeSelectionRequest, TreeSessionId, TreeViewStamp,
};
use std::{
    collections::{HashMap, VecDeque},
    num::NonZeroU32,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, TryAcquireError};

pub(crate) use midnight_core::TreeBranchAction as BranchAction;

#[path = "tree_action_capture.rs"]
mod action_capture;
#[path = "tree_checksums.rs"]
mod checksums;
#[path = "tree_navigation.rs"]
mod navigation;
#[path = "tree_refresh.rs"]
mod refresh;
#[path = "tree_user_commands.rs"]
mod user_command_capture;
#[path = "tree_watch_scope.rs"]
mod watch_scope;
use refresh::Maintenance;
#[cfg(test)]
#[path = "tree_clipboard_capture_tests.rs"]
mod clipboard_capture_tests;
#[cfg(test)]
#[path = "tree_sizes_capture_tests.rs"]
mod size_capture_tests;
#[cfg(test)]
#[path = "tree_sizes_runtime_tests.rs"]
mod size_runtime_tests;
pub(crate) use action_capture::CapturedSelection;

static NEXT_SESSION: AtomicU64 = AtomicU64::new(1);
const QUEUE_CAPACITY: usize = 8;
const VIEWPORT_CAPACITY: usize = 256;

pub(crate) struct Session {
    reports: crate::ReportArena,
    tree: DirectoryTree,
    registration: Arc<RootRegistrationLease>,
    budget: WorkerBudget,
    lifetime: Cancellation,
    projection_state: ProjectionState,
    projection: Arc<TreeProjection>,
    selection: Selection,
    selection_seeded: bool,
    accepted: Option<Arc<TreeProjection>>,
    options: ProjectionOptions,
    queue: VecDeque<PendingBranch>,
    versions: HashMap<BranchId, u64>,
    next_work: u64,
    pub(crate) last_request: TreeRequestId,
    maintenance: Option<Maintenance>,
    prefer_maintenance: bool,
    watch_cache: HashMap<BranchId, (u64, Arc<crate::watch::tree::ScopeTarget>)>,
    watch_failure: Option<watch_scope::Failure>,
}

/// Retain this per-pane owner across navigation/session replacement: a blocked
/// old provider must not admit a second branch call when its session is cancelled.
#[derive(Clone)]
pub(crate) struct WorkerBudget {
    listing: Arc<Semaphore>,
    branch: Arc<Semaphore>,
    mutation: Arc<Semaphore>,
}

struct WorkerPermit {
    _listing: OwnedSemaphorePermit,
    _branch: OwnedSemaphorePermit,
    _mutation: Option<OwnedSemaphorePermit>,
}

enum WorkOrigin {
    User { _row: TreeRowId },
    Maintenance,
}

struct PendingBranch {
    branch: BranchId,
    origin: WorkOrigin,
    kind: TreeRequest,
    version: u64,
    request: TreeRequestId,
}

pub(crate) struct BranchJob {
    stamp: TreeViewStamp,
    pending: PendingBranch,
    work: BranchWork,
    permit: WorkerPermit,
    registration: Arc<RootRegistrationLease>,
}

pub(crate) struct BranchCompletion {
    stamp: TreeViewStamp,
    pending: PendingBranch,
    result: BranchResult,
    _registration: Arc<RootRegistrationLease>,
    _permit: WorkerPermit,
}

/// Accepted action ownership. Navigation drops its session without revoking
/// this target; the operation service must still establish its mutation proof.
pub(crate) struct CapturedTarget {
    pub(crate) target: crate::providers::MountedTarget,
    // Ownership only: keep the registration valid until the target resources
    // above have been destroyed, including after the original session closes.
    pub(crate) _registration: Arc<RootRegistrationLease>,
}

pub(crate) struct RootRegistrationLease {
    registry: Arc<ProviderRegistry>,
    provider: ProviderId,
    instance: ProviderInstanceId,
}

impl Drop for RootRegistrationLease {
    fn drop(&mut self) {
        let _ = self.registry.unregister(&self.provider, self.instance);
    }
}

struct CapturedRootProvider {
    address: StoredLocation,
    opened: Mutex<Option<OpenedLocation>>,
}

impl FilesystemProvider for CapturedRootProvider {
    fn resolve(
        &self,
        address: &StoredLocation,
        cancel: &Cancellation,
    ) -> Result<OpenedLocation, ProviderError> {
        check_cancel(cancel)?;
        if address != &self.address {
            return Err(ProviderError::StaleReference);
        }
        self.opened
            .lock()
            .map_err(|_| ProviderError::Failed("Captured root is unavailable.".into()))?
            .take()
            .ok_or(ProviderError::StaleReference)
    }
}

impl WorkerBudget {
    pub(crate) fn listing(&self) -> Arc<Semaphore> {
        self.listing.clone()
    }

    pub(crate) fn new(listing: Arc<Semaphore>) -> Self {
        Self {
            listing,
            branch: Arc::new(Semaphore::new(1)),
            mutation: Arc::new(Semaphore::new(1)),
        }
    }

    pub(crate) fn with_mutation_gate(mut self, mutation: Arc<Semaphore>) -> Self {
        self.mutation = mutation;
        self
    }

    fn acquire(&self, maintenance: bool) -> Result<Option<WorkerPermit>, ProviderError> {
        let mutation = if maintenance {
            match self.mutation.clone().try_acquire_owned() {
                Ok(permit) => Some(permit),
                Err(TryAcquireError::NoPermits) => return Ok(None),
                Err(TryAcquireError::Closed) => return Err(ProviderError::Unavailable),
            }
        } else {
            None
        };
        let branch = match self.branch.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(TryAcquireError::NoPermits) => return Ok(None),
            Err(TryAcquireError::Closed) => return Err(ProviderError::Unavailable),
        };
        let listing = match self.listing.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(TryAcquireError::NoPermits) => return Ok(None),
            Err(TryAcquireError::Closed) => return Err(ProviderError::Unavailable),
        };
        Ok(Some(WorkerPermit {
            _listing: listing,
            _branch: branch,
            _mutation: mutation,
        }))
    }
}

impl Session {
    pub(crate) fn retained_rows(&self) -> std::collections::HashSet<TreeRowId> {
        self.projection_state.retained_rows()
    }
    pub(crate) fn size_sort(
        &mut self,
        sizes: Arc<HashMap<TreeRowId, midnight_core::FolderSize>>,
    ) -> Arc<HashMap<TreeRowId, midnight_core::FolderSize>> {
        std::mem::replace(&mut self.options.sizes, sizes)
    }
    pub(crate) fn reproject_request(
        &mut self,
        request: &midnight_core::TreeReprojectRequest,
        cancel: &Cancellation,
    ) -> Result<TreePublication, ProviderError> {
        let cancel = self.lifetime.linked(cancel);
        check_cancel(&cancel)?;
        self.check_view(&request.view)?;
        if request.request.0 == 0
            || request.request.0 <= self.last_request.0
            || request.selection_revision != self.selection.revision
            || !Arc::ptr_eq(&request.selection, &self.selection.snapshot)
            || !Arc::ptr_eq(request.selection.basis(), &request.view)
        {
            return Err(ProviderError::StaleReference);
        }
        request
            .options
            .filter
            .validate_bounds()
            .map_err(ProviderError::InvalidResponse)?;
        request
            .options
            .search
            .validate_bounds()
            .map_err(ProviderError::InvalidResponse)?;
        let options = ProjectionOptions {
            sizes: self.options.sizes.clone(),
            sort: request.options.sort,
            filter: request.options.filter.clone(),
            search: request.options.search.clone(),
            locale: self.options.locale.clone(),
        };
        // build() also updates occurrence records and locale keys. Keep that
        // bounded bookkeeping private until the complete replacement succeeds.
        let mut state = self.projection_state.clone();
        let projection = state.build(
            &self.tree,
            &self.projection.root,
            self.projection.stamp.session,
            Some(&self.projection),
            &options,
            &cancel,
        )?;
        let selection = self.selection.rebased(projection.clone())?;
        let publication = TreePublication::new(
            projection.clone(),
            selection.revision,
            selection.snapshot.clone(),
        )
        .map_err(ProviderError::InvalidResponse)?;
        check_cancel(&cancel)?;
        self.projection_state = state;
        self.projection = projection;
        self.selection = selection;
        self.options = options;
        self.last_request = request.request;
        self.selection_seeded = true;
        Ok(publication)
    }

    pub(crate) fn new(
        root: TreeRootSnapshot,
        opened: OpenedLocation,
        registry: Arc<ProviderRegistry>,
        budget: WorkerBudget,
        options: ProjectionOptions,
        cancel: &Cancellation,
        reports: crate::ReportArena,
    ) -> Result<Self, ProviderError> {
        preflight_root(&root, cancel)?;
        let session = next_session(&NEXT_SESSION)?;
        let provider = ProviderId::new(&format!("inline-tree.{}", session.0))
            .map_err(|_| ProviderError::ResourceLimited)?;
        let address = StoredLocation::new(
            provider.clone(),
            NonZeroU32::MIN,
            Arc::from(session.0.to_le_bytes()),
        );
        let instance = registry.register(
            provider.clone(),
            Arc::new(CapturedRootProvider {
                address: address.clone(),
                opened: Mutex::new(Some(opened)),
            }),
        )?;
        let registration = Arc::new(RootRegistrationLease {
            registry: registry.clone(),
            provider,
            instance,
        });
        let location = registry.resolve(&address, cancel)?;
        let mut tree = DirectoryTree::new(location, TreeLimits::default(), reports.clone())?;
        let work = tree
            .request(tree.root(), TreeRequest::Expand)?
            .with_cancellation(cancel);
        if !tree.apply(work.run()) {
            return Err(ProviderError::StaleReference);
        }
        if let Some(error) = tree.state(tree.root()).and_then(|root| root.error) {
            return Err(error);
        }
        check_cancel(cancel)?;
        let mut projection_state = ProjectionState::new(&tree, &root)?;
        let projection = projection_state.build(&tree, &root, session, None, &options, cancel)?;
        let selection = Selection::from_root(
            projection.clone(),
            SelectionRevision(0),
            &TreeRootSelection::Empty,
        )?;
        Ok(Self {
            reports,
            tree,
            registration,
            budget,
            lifetime: cancel.child(),
            projection_state,
            projection,
            selection,
            selection_seeded: false,
            accepted: None,
            options,
            queue: VecDeque::new(),
            versions: HashMap::new(),
            next_work: 1,
            last_request: TreeRequestId(0),
            maintenance: None,
            prefer_maintenance: true,
            watch_cache: HashMap::new(),
            watch_failure: None,
        })
    }

    pub(crate) fn projection(&self) -> &Arc<TreeProjection> {
        &self.projection
    }

    /// Migrate the flat selection before its first publication/acknowledgment.
    pub(crate) fn seed_selection(
        &mut self,
        revision: SelectionRevision,
        selected: &TreeRootSelection,
    ) -> Result<(), ProviderError> {
        if self.accepted.is_some() || revision < self.selection.revision {
            return Err(ProviderError::StaleReference);
        }
        let selection = Selection::from_root(self.projection.clone(), revision, selected)?;
        if self.selection_seeded && revision == self.selection.revision {
            return if selection.snapshot.bits() == self.selection.snapshot.bits() {
                Ok(())
            } else {
                Err(ProviderError::StaleReference)
            };
        }
        self.selection = selection;
        self.selection_seeded = true;
        Ok(())
    }

    pub(crate) fn publication(&mut self) -> Result<TreePublication, ProviderError> {
        let publication = TreePublication::new(
            self.projection.clone(),
            self.selection.revision,
            self.selection.snapshot.clone(),
        )
        .map_err(|message| ProviderError::Failed(message.into()))?;
        // Publishing the initial empty selection establishes revision zero too.
        self.selection_seeded = true;
        Ok(publication)
    }

    pub(crate) fn select(&mut self, request: &TreeSelectionRequest) -> Result<(), ProviderError> {
        self.check_view(&request.view)?;
        self.selection.apply(request)
    }

    pub(crate) fn reproject(
        &mut self,
        options: ProjectionOptions,
        cancel: &Cancellation,
    ) -> Result<(), ProviderError> {
        let mut state = self.projection_state.clone();
        let projection = state.build(
            &self.tree,
            &self.projection.root,
            self.projection.stamp.session,
            Some(&self.projection),
            &options,
            cancel,
        )?;
        let selection = self.selection.rebased(projection.clone())?;
        self.options = options;
        self.projection_state = state;
        self.projection = projection;
        self.selection = selection;
        Ok(())
    }

    pub(crate) fn update_projection(&mut self, cancel: &Cancellation) -> Result<(), ProviderError> {
        self.reproject(self.options.clone(), cancel)
    }

    pub(crate) fn commit(&mut self, projection: &Arc<TreeProjection>) -> bool {
        if !Arc::ptr_eq(&self.projection, projection) {
            return false;
        }
        self.accepted = Some(projection.clone());
        true
    }

    fn check_stamp(&self, stamp: TreeViewStamp) -> Result<(), ProviderError> {
        if stamp != self.projection.stamp
            || self
                .accepted
                .as_ref()
                .is_none_or(|view| !Arc::ptr_eq(view, &self.projection))
        {
            return Err(ProviderError::StaleReference);
        }
        Ok(())
    }

    fn visible_branch(&self, row: TreeRowId) -> Result<BranchId, ProviderError> {
        if !self.projection.index.row_positions.contains_key(&row) {
            return Err(ProviderError::StaleReference);
        }
        self.projection_state
            .branch(row)
            .ok_or(ProviderError::StaleReference)
    }

    fn check_view(&self, view: &Arc<TreeProjection>) -> Result<(), ProviderError> {
        self.check_stamp(view.stamp)?;
        if !Arc::ptr_eq(view, &self.projection) {
            return Err(ProviderError::StaleReference);
        }
        Ok(())
    }

    pub(crate) fn branch(
        &mut self,
        view: &Arc<TreeProjection>,
        request: TreeRequestId,
        row: TreeRowId,
        action: BranchAction,
    ) -> Result<(), ProviderError> {
        self.check_view(view)?;
        if request.0 == 0 || request.0 <= self.last_request.0 {
            return Err(ProviderError::StaleReference);
        }
        let branch = self.visible_branch(row)?;
        let display = &view.rows[view.index.row_positions[&row]];
        if display.disclosure == midnight_core::TreeDisclosure::None
            || (action == BranchAction::Probe && display.expanded)
        {
            return Err(ProviderError::Unsupported);
        }
        self.last_request = request;
        if action == BranchAction::Collapse {
            self.queue
                .retain(|pending| !in_subtree(&self.tree, pending.branch, branch));
            self.versions
                .retain(|id, _| !in_subtree(&self.tree, *id, branch));
            self.tree.collapse(branch);
            return Ok(());
        }
        let kind = match action {
            BranchAction::Probe => TreeRequest::Probe,
            BranchAction::Expand => TreeRequest::Expand,
            BranchAction::Refresh => TreeRequest::Refresh,
            BranchAction::Collapse => unreachable!(),
        };
        self.enqueue(branch, row, kind, request)
    }

    fn enqueue(
        &mut self,
        branch: BranchId,
        row: TreeRowId,
        kind: TreeRequest,
        request: TreeRequestId,
    ) -> Result<(), ProviderError> {
        let existing = self
            .queue
            .iter()
            .position(|pending| pending.branch == branch);
        if let Some(position) = existing {
            if kind == TreeRequest::Probe {
                return Ok(());
            }
            self.queue.remove(position);
        } else if self.queue.len() >= QUEUE_CAPACITY {
            // Viewport probes are optional read-ahead. Each completed publication
            // requests the visible range again, so defer excess checks instead of
            // reporting ordinary backpressure as a filesystem failure.
            if kind == TreeRequest::Probe {
                return Ok(());
            }
            // A user's expansion must not be rejected because speculative probes
            // filled the queue. Preserve all explicit commands and the hard bound.
            let position = self
                .queue
                .iter()
                .rposition(|pending| pending.kind == TreeRequest::Probe)
                .ok_or(ProviderError::ResourceLimited)?;
            if let Some(retired) = self.queue.remove(position) {
                self.versions.remove(&retired.branch);
            }
        }
        let version = self.next_work;
        self.next_work = version
            .checked_add(1)
            .ok_or(ProviderError::ResourceLimited)?;
        self.versions.insert(branch, version);
        self.queue.push_back(PendingBranch {
            branch,
            origin: WorkOrigin::User { _row: row },
            kind,
            version,
            request,
        });
        Ok(())
    }

    pub(crate) fn viewport(
        &mut self,
        view: &Arc<TreeProjection>,
        visible: &[TreeRowId],
    ) -> Result<(), ProviderError> {
        self.check_view(view)?;
        if visible.len() > VIEWPORT_CAPACITY {
            return Err(ProviderError::ResourceLimited);
        }
        self.queue
            .retain(|pending| pending.kind != TreeRequest::Probe);
        for row in visible {
            let branch = self.visible_branch(*row)?;
            if self
                .tree
                .state(branch)
                .is_some_and(|state| state.disclosure == Disclosure::Unknown)
            {
                self.enqueue(branch, *row, TreeRequest::Probe, TreeRequestId(0))?;
            }
        }
        Ok(())
    }

    pub(crate) fn queued(&self) -> usize {
        self.queue.len()
            + self
                .maintenance
                .as_ref()
                .map_or(0, |maintenance| maintenance.remaining.len())
    }

    pub(crate) fn take_work(&mut self) -> Result<Option<BranchJob>, ProviderError> {
        self.skip_removed_refresh_branches();
        self.queue
            .retain(|pending| self.tree.state(pending.branch).is_some());
        self.versions
            .retain(|branch, _| self.tree.state(*branch).is_some());
        if self.queued() == 0 {
            return Ok(None);
        }
        let maintenance = self.runnable_maintenance();
        let Some(permit) = self.budget.acquire(maintenance)? else {
            return Ok(None);
        };
        let pending = if maintenance {
            self.take_refresh_branch()?
        } else {
            let position = self
                .queue
                .iter()
                .position(|pending| pending.kind != TreeRequest::Probe)
                .unwrap_or(0);
            let pending = self
                .queue
                .remove(position)
                .ok_or(ProviderError::StaleReference)?;
            self.prefer_maintenance = true;
            pending
        };
        let work = self
            .tree
            .request(pending.branch, pending.kind)?
            .with_cancellation(&self.lifetime);
        Ok(Some(BranchJob {
            stamp: self.projection.stamp,
            pending,
            work,
            permit,
            registration: self.registration.clone(),
        }))
    }

    pub(crate) fn apply(
        &mut self,
        completion: BranchCompletion,
        cancel: &Cancellation,
    ) -> Result<bool, ProviderError> {
        if matches!(completion.pending.origin, WorkOrigin::Maintenance) {
            return self.apply_refresh(completion, cancel);
        }
        let current = self.projection.stamp;
        if completion.stamp.pane != current.pane
            || completion.stamp.navigation != current.navigation
            || completion.stamp.session != current.session
            || self.versions.get(&completion.pending.branch) != Some(&completion.pending.version)
        {
            return Ok(false);
        }
        self.versions.remove(&completion.pending.branch);
        if !self.tree.apply(completion.result) {
            return Ok(false);
        }
        self.reproject(self.options.clone(), cancel)?;
        Ok(true)
    }

    pub(crate) fn capture_target(
        &self,
        view: &Arc<TreeProjection>,
        row: TreeRowId,
        cancel: &Cancellation,
    ) -> Result<CapturedTarget, ProviderError> {
        self.check_view(view)?;
        let branch = self.visible_branch(row)?;
        let position = view.index.row_positions[&row];
        if !view.rows[position].selectable || view.rows[position].context_only {
            return Err(ProviderError::Unsupported);
        }
        Ok(CapturedTarget {
            target: self.tree.resolve_entry_target(branch, cancel)?,
            _registration: self.registration.clone(),
        })
    }
}

impl BranchJob {
    #[cfg(test)]
    pub(crate) fn row(&self) -> TreeRowId {
        match self.pending.origin {
            WorkOrigin::User { _row: row } => row,
            WorkOrigin::Maintenance => panic!("maintenance has no visible row"),
        }
    }
    pub(crate) fn maintenance(&self) -> bool {
        matches!(self.pending.origin, WorkOrigin::Maintenance)
    }
    #[cfg(test)]
    pub(crate) fn request(&self) -> TreeRequestId {
        self.pending.request
    }

    pub(crate) fn run(self) -> BranchCompletion {
        let Self {
            stamp,
            pending,
            work,
            permit,
            registration,
        } = self;
        let result = work.run();
        // Keep admission through provider return, completion projection and cleanup.
        // Aborting an async waiter cannot release a running provider's slot.
        BranchCompletion {
            stamp,
            pending,
            result,
            _registration: registration,
            _permit: permit,
        }
    }
}

/// Call before optional provider capture too, to reject an over-limit tree before
/// allocating adapter caches. Its complete flat listing remains independent.
pub(crate) fn preflight_root(
    root: &TreeRootSnapshot,
    cancel: &Cancellation,
) -> Result<(), ProviderError> {
    check_cancel(cancel)?;
    // The hidden root also consumes one of the model's 10,000 occurrences.
    if !root.within_payload_budget() {
        return Err(ProviderError::ResourceLimited);
    }
    let mut bytes = 0usize;
    for entry in root.entries.iter() {
        check_cancel(cancel)?;
        bytes = bytes
            .saturating_add(std::mem::size_of_val(entry))
            .saturating_add(entry.path.capacity().saturating_mul(3))
            .saturating_add(entry.name.capacity().saturating_mul(2))
            .saturating_add(entry.display_name.capacity().saturating_mul(2))
            .saturating_add(entry.virtual_node.as_ref().map_or(0, |node| {
                node.raw_name
                    .len()
                    .saturating_add(node.display_name.capacity())
            }))
            .saturating_add(
                entry
                    .metadata
                    .localized_modified
                    .as_ref()
                    .map_or(0, |value| {
                        value.date.capacity().saturating_add(value.time.capacity())
                    }),
            );
        if bytes > TreeProjection::MAX_BYTES {
            return Err(ProviderError::ResourceLimited);
        }
    }
    Ok(())
}

fn in_subtree(tree: &DirectoryTree, mut branch: BranchId, ancestor: BranchId) -> bool {
    loop {
        if branch == ancestor {
            return true;
        }
        let Some(parent) = tree.state(branch).and_then(|state| state.parent) else {
            return false;
        };
        branch = parent;
    }
}

fn check_cancel(cancel: &Cancellation) -> Result<(), ProviderError> {
    if cancel.is_cancelled() {
        Err(ProviderError::Cancelled)
    } else {
        Ok(())
    }
}

fn next_session(counter: &AtomicU64) -> Result<TreeSessionId, ProviderError> {
    counter
        .try_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            value.checked_add(1)
        })
        .map(TreeSessionId)
        .map_err(|_| ProviderError::ResourceLimited)
}

#[cfg(test)]
#[path = "tree_actor_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tree_actor_selection_tests.rs"]
mod selection_tests;

#[cfg(test)]
#[path = "tree_action_capture_tests.rs"]
mod action_capture_tests;

#[cfg(test)]
#[path = "tree_reproject_tests.rs"]
mod reproject_tests;
