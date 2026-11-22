//! Backend branch occurrences. No row caption or position conveys file authority.
use super::{MountedEntry, MountedLocation, ProviderEntry, ProviderError};
use midnight_core::{Cancellation, EntryKind, GeneratedTextSource, NodeRef, SizeDirectoryIdentity};
use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
};

static NEXT_TREE: AtomicU64 = AtomicU64::new(1);

/// Released root-mount references that trigger reclamation: at least this many,
/// and at least as many as occurrences still hold, so each pass is amortized.
const RECLAIM_MIN: usize = 4_096;
/// Branch completions after which any released references are reclaimed.
const RECLAIM_COMPLETIONS: u32 = 1_024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BranchId {
    tree: u64,
    value: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct TreeLimits {
    pub nodes: usize,
    pub bytes: usize,
    /// The deepest row: the shown folder's own entries are depth 0, the folders
    /// inside them depth 1 (`TreeProjection::MAX_DEPTH` counts the same way). A
    /// folder at this depth still lists, so an empty one expands; a listing that
    /// would add a deeper row is refused on that folder instead.
    pub depth: usize,
    pub requests: usize,
}
impl Default for TreeLimits {
    fn default() -> Self {
        Self {
            nodes: 10_000,
            bytes: 16 * 1024 * 1024,
            depth: 64,
            requests: 8,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Disclosure {
    None,
    Unknown,
    Loading,
    Collapsed,
    Expanded,
    Empty,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreeRequest {
    Probe,
    Expand,
    Refresh,
}

/// Presentation-only snapshot. Mount resources stay in the backend tree/work.
#[derive(Clone, Debug)]
pub struct TreeRow {
    pub id: BranchId,
    pub parent: Option<BranchId>,
    pub node: NodeRef,
    pub entry: Arc<ProviderEntry>,
    pub depth: usize,
    pub disclosure: Disclosure,
    pub expanded: bool,
    pub error: Option<ProviderError>,
    pub error_details: Option<Arc<dyn GeneratedTextSource>>,
}

#[derive(Clone)]
struct Node {
    source: MountedLocation,
    target: Option<MountedLocation>,
    node: NodeRef,
    entry: Arc<ProviderEntry>,
    parent: Option<BranchId>,
    depth: usize,
    children: Vec<BranchId>,
    expanded: bool,
    known: Option<bool>,
    listed: bool,
    // Every successful listing installs a new version: child metadata feeds the
    // watch signature. The children version changes only with the ordered child
    // occurrence sequence, so an unchanged re-listing keeps traversal cursors.
    listing_version: u64,
    children_version: u64,
    // The provider's full watch observation of the directory from the installed
    // listing (every entry, files included), when it supplied one.
    observation: Option<[u8; 32]>,
    browse_identity: Option<SizeDirectoryIdentity>,
    recursive_excluded: u64,
    error: Option<ProviderError>,
    error_details: Option<Arc<dyn GeneratedTextSource>>,
    pending: Option<(u64, Cancellation)>,
}

/// All methods and final destruction run on backend workers. Request execution
/// is separate so blocking providers never hold the tree's actor/state borrow.
pub struct DirectoryTree {
    root: BranchId,
    next_branch: u64,
    next_request: u64,
    nodes: HashMap<BranchId, Node>,
    // Root address plus every retained node's entry, error, and target charge.
    payload_bytes: usize,
    limits: TreeLimits,
    active: Arc<AtomicUsize>,
    reports: crate::size_warnings::ReportLog,
    // Branch completions since the root mount's references were last reclaimed.
    completions: u32,
    #[cfg(test)]
    eager_reclaim: bool,
    // Expansions ask the provider for a full observation of what they list.
    observe: bool,
}

/// Bounded rollback state for an atomic model/projection publication. It holds
/// one branch subtree: listing a branch changes only that branch and its
/// descendants, so the rest of the model is never copied.
pub(crate) struct TreeCheckpoint {
    branch: BranchId,
    nodes: Vec<(BranchId, Node)>,
    payload_bytes: usize,
}

/// Display-relevant outcome of one branch listing. A re-listing that changes none
/// of it needs no new projection.
pub(crate) struct ListingState {
    failed: bool,
    known: Option<bool>,
    children: Vec<(BranchId, Arc<ProviderEntry>)>,
}

impl ListingState {
    /// No error before or after, the same disclosure knowledge and the same
    /// ordered child occurrences with equal entries.
    pub(crate) fn unchanged(&self, after: &Self) -> bool {
        !self.failed
            && !after.failed
            && self.known == after.known
            && self.children.len() == after.children.len()
            && self.children.iter().zip(&after.children).all(
                |((before_id, before), (after_id, after))| {
                    before_id == after_id && same_entry(before, after)
                },
            )
    }
}

fn same_entry(left: &ProviderEntry, right: &ProviderEntry) -> bool {
    left.token.as_bytes() == right.token.as_bytes()
        && left.raw_name == right.raw_name
        && left.display_name == right.display_name
        && left.kind == right.kind
        && left.metadata == right.metadata
}

struct Permit(Arc<AtomicUsize>, Cancellation);
impl Drop for Permit {
    fn drop(&mut self) {
        self.1.cancel();
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

pub struct BranchWork {
    branch: BranchId,
    request: u64,
    kind: TreeRequest,
    location: MountedLocation,
    node: NodeRef,
    ancestors: Vec<NodeRef>,
    ancestor_identities: Vec<SizeDirectoryIdentity>,
    directory_link: bool,
    root: bool,
    observe: bool,
    limit: usize,
    cancel: Cancellation,
    permit: Permit,
}
pub struct BranchResult {
    branch: BranchId,
    request: u64,
    kind: TreeRequest,
    listing: Result<Listing, ProviderError>,
    identity: Option<SizeDirectoryIdentity>,
    _permit: Permit,
}

impl BranchResult {
    /// The listing or probe failed: references it bound may be orphaned.
    pub(crate) fn failed(&self) -> bool {
        self.listing.is_err()
    }
}

/// What one branch request observed.
enum Listing {
    /// Listed children: an expansion or refresh, or a probe of the hidden root,
    /// with the provider's full watch observation of the directory if requested
    /// (boxed: completions wait in a small fixed mailbox slot).
    Rows(Vec<MountedEntry>, Option<Box<[u8; 32]>>),
    /// A disclosure probe's answer: whether the branch contains a folder.
    Presence(bool),
}

/// Version, children and full watch observation of an installed listing.
pub(crate) type WatchListing<'a> = (u64, &'a [BranchId], Option<[u8; 32]>);

impl BranchWork {
    fn browse_identity(&self) -> Result<Option<SizeDirectoryIdentity>, ProviderError> {
        match self
            .location
            .browse_directory_identity(&self.node, &self.cancel)
        {
            Ok(Some(value)) => Ok(Some(value)),
            Ok(None) => Err(ProviderError::Failed(
                "Directory target is no longer a directory.".into(),
            )),
            Err(ProviderError::Unsupported) if !self.directory_link => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Link pane cancellation without revoking an independently retained target.
    pub(crate) fn with_cancellation(mut self, lifetime: &Cancellation) -> Self {
        self.cancel = self.cancel.linked(lifetime);
        self
    }

    /// Run with a shared engine blocking-worker permit, never on the UI thread.
    pub fn run(self) -> BranchResult {
        let mut identity = None;
        let listing = (|| {
            identity = self.browse_identity()?;
            if identity.is_some_and(|identity| self.ancestor_identities.contains(&identity)) {
                return Err(ProviderError::Failed(
                    "Directory target repeats an ancestor.".into(),
                ));
            }
            let listing = if self.kind == TreeRequest::Probe && !self.root {
                // Disclosure needs presence only: enumeration stops at the first
                // folder and issues no child reference or row. A child that repeats
                // an ancestor reference is therefore reported by Expand instead.
                Listing::Presence(self.location.has_folders(&self.node, &self.cancel)?)
            } else {
                let (rows, observation) = if self.root {
                    (self.location.list(&self.node, &self.cancel)?, None)
                } else if self.observe {
                    // The first watch probe of this branch compares with it.
                    self.location
                        .list_folders_observed(&self.node, &self.cancel, self.limit)?
                } else {
                    let rows = self
                        .location
                        .list_folders(&self.node, &self.cancel, self.limit)?;
                    (rows, None)
                };
                if rows.iter().any(|row| self.ancestors.contains(&row.node)) {
                    return Err(ProviderError::InvalidResponse(
                        "ancestor cycle in directory branch",
                    ));
                }
                Listing::Rows(rows, observation.map(Box::new))
            };
            // Two live observations reject a stable A-to-B retarget around this
            // listing. They do not promise atomic namespace or snapshot identity.
            if self.browse_identity()? != identity {
                return Err(ProviderError::Failed(
                    "Directory target changed while listing.".into(),
                ));
            }
            Ok(listing)
        })();
        BranchResult {
            branch: self.branch,
            request: self.request,
            kind: self.kind,
            listing,
            identity,
            _permit: self.permit,
        }
    }
}

impl DirectoryTree {
    pub(crate) fn watch_listing(&self, branch: BranchId) -> Option<WatchListing<'_>> {
        let node = self.nodes.get(&branch)?;
        node.listed.then_some((
            node.listing_version,
            node.children.as_slice(),
            node.observation,
        ))
    }

    /// Watched trees: later expansions also ask the provider for the full
    /// observation of each listed directory (files included), so a branch's
    /// first watch probe can be compared with what its listing saw.
    pub(crate) fn observe_listings(&mut self) {
        self.observe = true;
    }

    /// Ordered children of a listed branch with the version of that exact sequence.
    /// Re-listing without inserting, removing or reordering children keeps it.
    pub(crate) fn listed_children(&self, branch: BranchId) -> Option<(u64, &[BranchId])> {
        let node = self.nodes.get(&branch)?;
        node.listed
            .then_some((node.children_version, node.children.as_slice()))
    }

    pub(crate) fn contains(&self, branch: BranchId) -> bool {
        self.nodes.contains_key(&branch)
    }

    /// Allocation-free alternative to `state(branch).disclosure`.
    pub(crate) fn disclosure(&self, branch: BranchId) -> Option<Disclosure> {
        self.nodes.get(&branch).map(disclosure)
    }

    /// Allocation-free alternative to `state(branch).expanded`.
    pub(crate) fn expanded(&self, branch: BranchId) -> bool {
        self.nodes.get(&branch).is_some_and(|node| node.expanded)
    }

    pub(crate) fn listing_state(&self, branch: BranchId) -> Option<ListingState> {
        let node = self.nodes.get(&branch)?;
        Some(ListingState {
            failed: node.error.is_some(),
            known: node.known,
            children: node
                .children
                .iter()
                .filter_map(|id| Some((*id, self.nodes.get(id)?.entry.clone())))
                .collect(),
        })
    }

    /// Snapshot `branch` and its descendants. Listing results, recursive batches
    /// and maintenance steps change only the subtree of the branch they list.
    pub(crate) fn checkpoint(&self, branch: BranchId) -> TreeCheckpoint {
        let nodes = self
            .descendants(vec![branch])
            .into_iter()
            .filter_map(|id| {
                let mut node = self.nodes.get(&id)?.clone();
                node.pending = None;
                Some((id, node))
            })
            .collect();
        TreeCheckpoint {
            branch,
            nodes,
            payload_bytes: self.payload_bytes,
        }
    }

    fn restore_subtree(&mut self, checkpoint: TreeCheckpoint) {
        if !self.nodes.contains_key(&checkpoint.branch) {
            // Its parent no longer references this subtree; restored nodes would be orphans.
            return;
        }
        for id in self.descendants(vec![checkpoint.branch]) {
            if let Some(node) = self.nodes.remove(&id) {
                cancel_pending(&node);
            }
        }
        self.nodes.extend(checkpoint.nodes);
        self.payload_bytes = checkpoint.payload_bytes;
    }

    /// Restore the published model while preserving all identity high-water marks.
    /// Cancellation never releases an outstanding worker's native I/O ownership.
    pub(crate) fn rollback_unpublished(&mut self, checkpoint: TreeCheckpoint) {
        self.restore_subtree(checkpoint);
    }

    pub(crate) fn restore(
        &mut self,
        checkpoint: TreeCheckpoint,
        branch: BranchId,
        error: ProviderError,
    ) {
        self.restore_subtree(checkpoint);
        let available = self.limits.bytes.saturating_sub(self.payload_bytes);
        let details = self.failure_details(branch, &error);
        if let Some(node) = self.nodes.get_mut(&branch) {
            self.payload_bytes -= error_charge(&node.error);
            node.error = Some(bounded_error(error, available));
            self.payload_bytes += error_charge(&node.error);
            node.error_details = details;
        }
    }

    pub(crate) fn root_target(
        &self,
        cancel: &Cancellation,
    ) -> Result<super::MountedTarget, ProviderError> {
        let root = &self.nodes[&self.root];
        root.source.resolve_target(&root.node, cancel)
    }

    /// References the root mount's lease issues, and the node records its provider
    /// retains when it reports them.
    pub(crate) fn source_reference_counts(&self) -> Result<(usize, Option<usize>), ProviderError> {
        let source = &self.nodes[&self.root].source;
        Ok((source.issued_count()?, source.provider_node_count()))
    }

    /// Occurrences holding a reference of the root's own mount, the hidden root
    /// included. Attached containers are other leases with their own tables.
    fn source_occurrences(&self) -> impl Iterator<Item = &Node> {
        let mount = self.nodes[&self.root].node.mount;
        self.nodes
            .values()
            .filter(move |node| node.node.mount == mount)
    }

    /// The tree is its mount's only lister and owns reference liveness. Count one
    /// branch completion after the session applied, restored or discarded it, then
    /// check whether to release the root mount's references that no occurrence
    /// holds: once their count (the larger of the issued and the provider's
    /// records, which includes records failed or presence-only listings bound)
    /// reaches the larger of the live count and 4,096, once it just exceeds 4,096
    /// after a failed or unpublished listing, or once 1,024 completions passed.
    ///
    /// Skips, keeping the completion count, unless `settled` (no unpublished
    /// checkpoint can restore other occurrences) and no `BranchWork` or
    /// `BranchResult` of this tree is outstanding: a listing that returned but is
    /// not applied yet issued references that no occurrence holds yet. The lease's
    /// reclaim gate covers only provider calls in flight. Best effort: a skip or an
    /// error leaves the tables to a later completion. Returns the references kept
    /// when it reclaimed.
    pub(crate) fn reclaim_after_completion(
        &mut self,
        settled: bool,
        unsettled_listing: bool,
        cancel: &Cancellation,
    ) -> Option<usize> {
        self.completions = self.completions.saturating_add(1);
        // Every outstanding work item and result holds one permit.
        if !settled || self.active.load(Ordering::Acquire) != 0 {
            return None;
        }
        let (issued, provider) = self.source_reference_counts().ok()?;
        let held = issued.max(provider.unwrap_or(0));
        let periodic = self.completions >= RECLAIM_COMPLETIONS;
        #[cfg(test)]
        let periodic = periodic || self.eager_reclaim;
        // Cheap common case: below the minimum, released references cannot reach it.
        if !periodic && held <= RECLAIM_MIN {
            return None;
        }
        let live = self.source_occurrences().count();
        let released = held.saturating_sub(live);
        let due = released >= live.max(RECLAIM_MIN)
            || (unsettled_listing && released > RECLAIM_MIN)
            || (periodic && released > 0);
        if !due {
            if periodic {
                self.completions = 0;
            }
            return None;
        }
        let keep = self
            .source_occurrences()
            .map(|node| node.node.token.clone())
            .collect::<HashSet<_>>();
        let kept = self.nodes[&self.root]
            .source
            .retain_issued(&keep, cancel)
            .ok()
            .flatten()?;
        self.completions = 0;
        Some(kept)
    }

    /// Reclaim after every settled completion with any released reference.
    #[cfg(test)]
    pub(crate) fn reclaim_eagerly(&mut self) {
        self.eager_reclaim = true;
    }

    #[cfg(test)]
    pub(crate) fn source_occurrence_count(&self) -> usize {
        self.source_occurrences().count()
    }

    /// Parent-first retained native directory scope, including collapsed cached nodes.
    /// Call only after establishing that the root context is native.
    pub(crate) fn loaded_native_branches(&self) -> Vec<BranchId> {
        let mut result = Vec::new();
        let mut pending = vec![self.root];
        while let Some(id) = pending.pop() {
            let node = &self.nodes[&id];
            if node.target.is_some()
                || !matches!(
                    node.entry.kind,
                    EntryKind::Directory | EntryKind::DirectoryLink
                )
            {
                continue;
            }
            if node.listed {
                result.push(id);
            }
            pending.extend(node.children.iter().rev().copied());
        }
        result
    }

    pub(crate) fn loaded_native_branch(&self, branch: BranchId) -> bool {
        let Some(node) = self.nodes.get(&branch) else {
            return false;
        };
        if !node.listed
            || !matches!(
                node.entry.kind,
                EntryKind::Directory | EntryKind::DirectoryLink
            )
        {
            return false;
        }
        let mut current = Some(branch);
        while let Some(id) = current {
            let Some(node) = self.nodes.get(&id) else {
                return false;
            };
            if node.target.is_some()
                || !matches!(
                    node.entry.kind,
                    EntryKind::Directory | EntryKind::DirectoryLink
                )
            {
                return false;
            }
            current = node.parent;
        }
        true
    }

    /// Identity of a successfully installed listing, not a later disclosure probe.
    /// Linked ancestry requires a complete native identity basis for polling.
    pub(crate) fn loaded_native_identity(
        &self,
        branch: BranchId,
    ) -> Result<Option<SizeDirectoryIdentity>, ProviderError> {
        if !self.loaded_native_branch(branch) {
            return Err(ProviderError::StaleReference);
        }
        let identity = self.nodes[&branch].browse_identity;
        let mut linked = false;
        let mut missing = false;
        let mut current = Some(branch);
        while let Some(id) = current {
            let node = &self.nodes[&id];
            linked |= node.entry.kind == EntryKind::DirectoryLink;
            missing |= node.browse_identity.is_none();
            current = node.parent;
        }
        if linked && missing {
            return Err(ProviderError::Unsupported);
        }
        Ok(identity)
    }
    /// Resolve a visible source entry, not a container mounted on that entry.
    /// The downstream service still establishes action-specific authority.
    pub fn resolve_entry_target(
        &self,
        branch: BranchId,
        cancel: &Cancellation,
    ) -> Result<super::MountedTarget, ProviderError> {
        super::registry::check_cancel(cancel)?;
        let node = self
            .nodes
            .get(&branch)
            .ok_or(ProviderError::StaleReference)?;
        if branch == self.root {
            return Err(ProviderError::Unsupported);
        }
        // Entering a container uses navigation_target instead. An unavailable
        // mount on this entry must not make its enclosing source file unusable.
        let mut contexts = Vec::new();
        let mut ancestor = node.parent;
        while let Some(id) = ancestor {
            let parent = self.nodes.get(&id).ok_or(ProviderError::StaleReference)?;
            if !parent.expanded {
                return Err(ProviderError::StaleReference);
            }
            contexts.push((parent.source.clone(), parent.node.clone()));
            if let Some(container) = &parent.target {
                contexts.push((container.clone(), container.root().clone()));
            }
            ancestor = parent.parent;
        }
        node.source
            .resolve_target_with_context(&node.node, cancel, contexts)
    }

    /// Bind an explicitly opened provider container, such as a captured archive.
    /// This never guesses a format from a caption or opens a container itself.
    /// Test-only until tree container discovery and attachment exist (TREE-01).
    #[cfg(test)]
    pub(crate) fn attach_container(
        &mut self,
        branch: BranchId,
        target: MountedLocation,
    ) -> Result<(), ProviderError> {
        target.validate_reference(target.root())?;
        let node = self
            .nodes
            .get(&branch)
            .ok_or(ProviderError::StaleReference)?;
        // Replacing a revoked container must not require that old mount to be
        // alive. Its enclosing source and all ancestors must still be valid.
        node.source.validate_reference(&node.node)?;
        if let Some(parent) = node.parent {
            self.validate_chain(parent)?;
        }
        if node.entry.kind != EntryKind::File {
            return Err(ProviderError::Unsupported);
        }
        // The container's entries would be one row deeper than this file.
        if node.depth > self.limits.depth {
            return Err(ProviderError::ResourceLimited);
        }
        let targets = self
            .nodes
            .values()
            .filter(|node| node.target.is_some())
            .count();
        if node.target.is_none() && targets >= 64 {
            return Err(ProviderError::ResourceLimited);
        }
        let mut bytes = self
            .payload_bytes
            .saturating_sub(target_charge(node))
            .saturating_add(target.address().address().len());
        if bytes > self.limits.bytes {
            return Err(ProviderError::ResourceLimited);
        }
        let removed = self.descendants(node.children.clone());
        for id in removed {
            if let Some(node) = self.nodes.remove(&id) {
                bytes -= node_charge(&node);
                cancel_pending(&node);
            }
        }
        let node = self
            .nodes
            .get_mut(&branch)
            .ok_or(ProviderError::StaleReference)?;
        cancel_pending(node);
        node.pending = None;
        node.children.clear();
        node.target = Some(target);
        node.known = None;
        node.listed = false;
        node.expanded = false;
        bytes -= error_charge(&node.error);
        node.error = None;
        node.error_details = None;
        self.payload_bytes = bytes;
        Ok(())
    }
    /// Read-only navigation context. A separate operation service must establish
    /// mutation authority; these leases cannot authorize Copy/Move/Delete.
    pub fn navigation_target(
        &self,
        branch: BranchId,
    ) -> Result<(MountedLocation, NodeRef), ProviderError> {
        self.validate_chain(branch)?;
        let node = self
            .nodes
            .get(&branch)
            .ok_or(ProviderError::StaleReference)?;
        if let Some(target) = &node.target {
            return Ok((target.clone(), target.root().clone()));
        }
        if !matches!(
            node.entry.kind,
            EntryKind::Directory | EntryKind::DirectoryLink
        ) {
            return Err(ProviderError::Unsupported);
        }
        Ok((node.source.clone(), node.node.clone()))
    }
    /// Includes the invisible root so root errors differ from an empty listing.
    pub fn state(&self, branch: BranchId) -> Option<TreeRow> {
        self.nodes.get(&branch).map(|node| TreeRow {
            id: branch,
            parent: node.parent,
            node: node.node.clone(),
            entry: node.entry.clone(),
            depth: node.depth.saturating_sub(1),
            disclosure: disclosure(node),
            expanded: node.expanded,
            error: node.error.clone(),
            error_details: node.error_details.clone(),
        })
    }

    /// Uses the caller's startup arena for reports retained beyond this tree. Only the
    /// engine can construct that arena, so only the engine creates trees.
    pub(crate) fn new(
        location: MountedLocation,
        limits: TreeLimits,
        reports: crate::ReportArena,
    ) -> Result<Self, ProviderError> {
        if limits.nodes == 0
            || limits.nodes > 10_000
            || limits.bytes == 0
            || limits.bytes > 16 * 1024 * 1024
            || limits.depth == 0
            || limits.depth > 64
            || limits.requests == 0
            || limits.requests > 8
        {
            return Err(ProviderError::ResourceLimited);
        }
        let tree = NEXT_TREE
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            })
            .map_err(|_| ProviderError::ResourceLimited)?;
        let root = BranchId { tree, value: 0 };
        let node = location.root().clone();
        location.validate_reference(&node)?;
        let entry = Arc::new(ProviderEntry {
            token: node.token.clone(),
            raw_name: Arc::from([]),
            display_name: String::new(),
            kind: EntryKind::Directory,
            metadata: Default::default(),
        });
        let payload_bytes = charge(&entry).saturating_add(location.address().address().len());
        if payload_bytes > limits.bytes {
            return Err(ProviderError::ResourceLimited);
        }
        Ok(Self {
            root,
            next_branch: 1,
            next_request: 1,
            payload_bytes,
            limits,
            active: Arc::default(),
            reports: crate::tree_diagnostics::log(&reports),
            completions: 0,
            #[cfg(test)]
            eager_reclaim: false,
            observe: false,
            nodes: HashMap::from([(
                root,
                Node {
                    source: location,
                    target: None,
                    node,
                    entry,
                    parent: None,
                    depth: 0,
                    children: Vec::new(),
                    expanded: false,
                    known: None,
                    listed: false,
                    listing_version: 0,
                    children_version: 0,
                    observation: None,
                    browse_identity: None,
                    recursive_excluded: 0,
                    error: None,
                    error_details: None,
                    pending: None,
                },
            )]),
        })
    }

    pub fn root(&self) -> BranchId {
        self.root
    }

    /// The deepest row this tree admits; the shown folder's own entries are depth 0.
    pub(crate) fn depth_limit(&self) -> usize {
        self.limits.depth
    }

    /// Whether the branch's last listing was refused only because its folders
    /// would be deeper than the depth budget allows.
    pub(crate) fn depth_limited(&self, branch: BranchId) -> bool {
        self.nodes.get(&branch).is_some_and(|node| {
            node.depth > self.limits.depth
                && node.error.as_ref() == Some(&depth_error(self.limits.depth))
        })
    }

    pub fn request(
        &mut self,
        branch: BranchId,
        kind: TreeRequest,
    ) -> Result<BranchWork, ProviderError> {
        self.validate_chain(branch)?;
        let node = self
            .nodes
            .get(&branch)
            .ok_or(ProviderError::StaleReference)?;
        if !matches!(
            node.entry.kind,
            EntryKind::Directory | EntryKind::DirectoryLink
        ) && node.target.is_none()
        {
            return Err(ProviderError::Unsupported);
        }
        if kind == TreeRequest::Probe && node.expanded {
            return Err(ProviderError::Unsupported);
        }
        // Rows are one level shallower than nodes: the hidden root is node depth 0,
        // its entries node depth 1 and rows of depth 0. A folder on the deepest
        // admitted row lists too, so an empty one expands, and `install` refuses a
        // listing that would add a deeper row. Deeper nodes are never created; this
        // only guards that invariant.
        if node.depth.saturating_sub(1) > self.limits.depth {
            return Err(ProviderError::ResourceLimited);
        }
        let next = self
            .next_request
            .checked_add(1)
            .ok_or(ProviderError::ResourceLimited)?;
        let mut ancestors = Vec::new();
        let mut ancestor_identities = Vec::new();
        let mut ancestor = Some(branch);
        while let Some(id) = ancestor {
            let node = &self.nodes[&id];
            ancestors.push(node.node.clone());
            if id != branch
                && let Some(identity) = node.browse_identity
            {
                ancestor_identities.push(identity);
            }
            if let Some(target) = &node.target {
                ancestors.push(target.root().clone());
            }
            ancestor = node.parent;
        }
        self.active
            .try_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                (value < self.limits.requests).then_some(value + 1)
            })
            .map_err(|_| ProviderError::ResourceLimited)?;
        let cancel = Cancellation::default();
        let permit = Permit(self.active.clone(), cancel.clone());
        let node = self
            .nodes
            .get_mut(&branch)
            .ok_or(ProviderError::StaleReference)?;
        if let Some((_, previous)) = node.pending.replace((self.next_request, cancel.clone())) {
            previous.cancel();
        }
        self.payload_bytes -= error_charge(&node.error);
        node.error = None;
        node.error_details = None;
        let (location, target_node) = node.target.as_ref().map_or_else(
            || (node.source.clone(), node.node.clone()),
            |target| (target.clone(), target.root().clone()),
        );
        let result = BranchWork {
            branch,
            request: self.next_request,
            kind,
            location,
            node: target_node,
            ancestors,
            ancestor_identities,
            directory_link: node.entry.kind == EntryKind::DirectoryLink && node.target.is_none(),
            root: branch == self.root,
            // Only an expansion starts a branch's watch; a refresh or a probe
            // never needs a new first-probe baseline.
            observe: self.observe && kind == TreeRequest::Expand && branch != self.root,
            limit: self.limits.nodes,
            cancel,
            permit,
        };
        self.next_request = next;
        Ok(result)
    }

    pub fn apply(&mut self, result: BranchResult) -> bool {
        let Some(node) = self.nodes.get_mut(&result.branch) else {
            return false;
        };
        let Some((request, cancel)) = &node.pending else {
            return false;
        };
        if *request != result.request || cancel.is_cancelled() {
            return false;
        }
        node.pending = None;
        let listing = self.validate_chain(result.branch).and(result.listing);
        let outcome = listing.and_then(|listing| {
            let known = match listing {
                Listing::Presence(present) => present,
                Listing::Rows(rows, _) if result.kind == TreeRequest::Probe => !rows.is_empty(),
                Listing::Rows(rows, observation) => {
                    return self.install(
                        result.branch,
                        rows,
                        result.kind == TreeRequest::Expand,
                        result.request,
                        observation.map(|observation| *observation),
                    );
                }
            };
            self.nodes
                .get_mut(&result.branch)
                .ok_or(ProviderError::StaleReference)?
                .known = Some(known);
            Ok(())
        });
        if outcome.is_ok()
            && result.kind != TreeRequest::Probe
            && let Some(node) = self.nodes.get_mut(&result.branch)
        {
            node.browse_identity = result.identity;
        }
        if let Err(error) = outcome {
            let available = self.limits.bytes.saturating_sub(self.payload_bytes);
            let details = self.failure_details(result.branch, &error);
            if let Some(node) = self.nodes.get_mut(&result.branch) {
                self.payload_bytes -= error_charge(&node.error);
                node.error = Some(bounded_error(error, available));
                self.payload_bytes += error_charge(&node.error);
                node.error_details = details;
            }
        }
        true
    }

    fn failure_details(
        &self,
        branch: BranchId,
        error: &ProviderError,
    ) -> Option<Arc<dyn GeneratedTextSource>> {
        let node = self.nodes.get(&branch)?;
        let message = match error {
            ProviderError::Failed(message) => std::borrow::Cow::Borrowed(message.as_str()),
            _ => std::borrow::Cow::Owned(error.to_string()),
        };
        Some(crate::tree_diagnostics::report(
            &self.reports,
            format_args!(
                "{} (provider {}, occurrence {branch:?})",
                node.entry.display_name,
                node.source.address().provider().as_str(),
            ),
            &message,
        ))
    }

    fn install(
        &mut self,
        branch: BranchId,
        rows: Vec<MountedEntry>,
        expand: bool,
        listing_version: u64,
        observation: Option<[u8; 32]>,
    ) -> Result<(), ProviderError> {
        let parent = self
            .nodes
            .get(&branch)
            .ok_or(ProviderError::StaleReference)?;
        // A folder on the deepest admitted row lists only while it has no folder of
        // its own: those would be one row too deep. Refuse the whole listing, as
        // every other budget does, instead of hiding them or calling the folder empty.
        // The parent's node depth is its children's row depth.
        if parent.depth > self.limits.depth && !rows.is_empty() {
            return Err(depth_error(self.limits.depth));
        }
        let existing = parent
            .children
            .iter()
            .map(|id| (self.nodes[id].node.clone(), *id))
            .collect::<HashMap<_, _>>();
        let retained = rows
            .iter()
            .filter_map(|row| existing.get(&row.node).copied())
            .collect::<HashSet<_>>();
        let removed = parent
            .children
            .iter()
            .filter(|id| !retained.contains(id))
            .copied()
            .collect::<Vec<_>>();
        let removed = self
            .descendants(removed)
            .into_iter()
            .collect::<HashSet<_>>();
        let additions = rows.len() - retained.len();
        if self
            .nodes
            .len()
            .saturating_sub(removed.len())
            .saturating_add(additions)
            > self.limits.nodes
        {
            return Err(ProviderError::ResourceLimited);
        }
        let next = self
            .next_branch
            .checked_add(additions as u64)
            .ok_or(ProviderError::ResourceLimited)?;
        let mut bytes = self.payload_bytes;
        for id in &removed {
            bytes -= node_charge(&self.nodes[id]);
        }
        for id in &retained {
            bytes -= charge(&self.nodes[id].entry);
        }
        for row in &rows {
            bytes = bytes.saturating_add(charge(&row.entry));
        }
        if bytes > self.limits.bytes {
            return Err(ProviderError::ResourceLimited);
        }
        let source = parent.target.as_ref().unwrap_or(&parent.source).clone();
        let depth = parent.depth + 1;
        for id in removed {
            if let Some(node) = self.nodes.remove(&id) {
                cancel_pending(&node);
            }
        }
        let mut children = Vec::with_capacity(rows.len());
        for row in rows {
            let id = if let Some(id) = existing.get(&row.node) {
                self.nodes
                    .get_mut(id)
                    .ok_or(ProviderError::StaleReference)?
                    .entry = Arc::new(row.entry);
                *id
            } else {
                let id = BranchId {
                    tree: self.root.tree,
                    value: self.next_branch,
                };
                self.next_branch += 1;
                self.nodes.insert(
                    id,
                    Node {
                        source: source.clone(),
                        target: None,
                        node: row.node,
                        entry: Arc::new(row.entry),
                        parent: Some(branch),
                        depth,
                        children: Vec::new(),
                        expanded: false,
                        known: None,
                        listed: false,
                        listing_version: 0,
                        children_version: 0,
                        observation: None,
                        browse_identity: None,
                        recursive_excluded: 0,
                        error: None,
                        error_details: None,
                        pending: None,
                    },
                );
                id
            };
            children.push(id);
        }
        self.next_branch = next;
        let parent = self
            .nodes
            .get_mut(&branch)
            .ok_or(ProviderError::StaleReference)?;
        parent.known = Some(!children.is_empty());
        parent.listed = true;
        parent.listing_version = listing_version;
        parent.observation = observation;
        if parent.children != children {
            parent.children_version = listing_version;
        }
        parent.children = children;
        if expand {
            parent.expanded = true;
        }
        self.payload_bytes = bytes;
        Ok(())
    }

    fn descendants(&self, mut pending: Vec<BranchId>) -> Vec<BranchId> {
        let mut result = Vec::new();
        while let Some(id) = pending.pop() {
            if let Some(node) = self.nodes.get(&id) {
                pending.extend(node.children.iter().copied());
                result.push(id);
            }
        }
        result
    }

    fn validate_chain(&self, branch: BranchId) -> Result<(), ProviderError> {
        let mut ancestor = Some(branch);
        while let Some(id) = ancestor {
            let node = self.nodes.get(&id).ok_or(ProviderError::StaleReference)?;
            node.source.validate_reference(&node.node)?;
            if let Some(target) = &node.target {
                target.validate_reference(target.root())?;
            }
            ancestor = node.parent;
        }
        Ok(())
    }

    pub fn collapse(&mut self, branch: BranchId) -> bool {
        if !self.nodes.contains_key(&branch) {
            return false;
        }
        for id in self.descendants(vec![branch]) {
            if let Some(node) = self.nodes.get_mut(&id) {
                cancel_pending(node);
                node.pending = None;
                node.expanded = false;
            }
        }
        true
    }

    pub(crate) fn exclude_recursive(&mut self, branch: BranchId, walk: u64) {
        if let Some(node) = self.nodes.get_mut(&branch) {
            node.recursive_excluded = walk;
        }
    }

    pub(crate) fn recursive_excluded(&self, branch: BranchId, walk: u64) -> bool {
        self.nodes
            .get(&branch)
            .is_none_or(|node| node.recursive_excluded == walk)
    }

    pub(crate) fn cancel_work(&mut self, branch: BranchId) {
        if let Some(node) = self.nodes.get_mut(&branch) {
            cancel_pending(node);
            node.pending = None;
        }
    }

    /// Preorder snapshot, built on the backend. Sibling sorting/filter ancestor
    /// closure is a subsequent projection step; this does not authorize actions.
    pub fn rows(&self) -> Vec<TreeRow> {
        let root = &self.nodes[&self.root];
        if !root.expanded {
            return Vec::new();
        }
        let mut pending = root.children.iter().rev().copied().collect::<Vec<_>>();
        let mut result = Vec::new();
        while let Some(id) = pending.pop() {
            let node = &self.nodes[&id];
            if let Some(row) = self.state(id) {
                result.push(row);
            }
            if node.expanded {
                pending.extend(node.children.iter().rev().copied());
            }
        }
        result
    }
}

impl Drop for DirectoryTree {
    fn drop(&mut self) {
        for node in self.nodes.values() {
            cancel_pending(node);
        }
    }
}

fn cancel_pending(node: &Node) {
    if let Some((_, cancel)) = &node.pending {
        cancel.cancel();
    }
}
fn disclosure(node: &Node) -> Disclosure {
    if !matches!(
        node.entry.kind,
        EntryKind::Directory | EntryKind::DirectoryLink
    ) && node.target.is_none()
    {
        Disclosure::None
    } else if node
        .pending
        .as_ref()
        .is_some_and(|(_, cancel)| !cancel.is_cancelled())
    {
        Disclosure::Loading
    } else if node.error.is_some() {
        Disclosure::Failed
    } else if node.known == Some(false) {
        Disclosure::Empty
    } else if node.expanded {
        Disclosure::Expanded
    } else if node.known == Some(true) {
        Disclosure::Collapsed
    } else {
        Disclosure::Unknown
    }
}
fn charge(entry: &ProviderEntry) -> usize {
    std::mem::size_of::<Node>()
        .saturating_add(entry.token.as_bytes().len())
        .saturating_add(entry.raw_name.len())
        .saturating_add(entry.display_name.capacity())
        .saturating_add(
            entry
                .metadata
                .localized_modified
                .as_ref()
                .map_or(0, |value| {
                    value.date.capacity().saturating_add(value.time.capacity())
                }),
        )
}

fn target_charge(node: &Node) -> usize {
    node.target
        .as_ref()
        .map_or(0, |target| target.address().address().len())
}

fn node_charge(node: &Node) -> usize {
    charge(&node.entry)
        .saturating_add(error_charge(&node.error))
        .saturating_add(target_charge(node))
}

fn error_charge(error: &Option<ProviderError>) -> usize {
    match error {
        Some(ProviderError::Failed(message)) => message.len(),
        _ => 0,
    }
}
fn bounded_error(mut error: ProviderError, available: usize) -> ProviderError {
    if let ProviderError::Failed(message) = &mut error {
        *message = message[..message.floor_char_boundary(available.min(4096))].to_owned();
    }
    error
}

/// The refusal of a folder on the deepest admitted row that has folders of its own.
fn depth_error(limit: usize) -> ProviderError {
    let levels = if limit == 1 { "level" } else { "levels" };
    ProviderError::Failed(format!(
        "Subfolders here are more than {limit} {levels} deep and are not shown."
    ))
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    pub(in crate::providers) fn assert_payload_accounting(tree: &DirectoryTree) {
        let expected = tree
            .nodes
            .values()
            .map(|node| {
                charge(&node.entry)
                    .saturating_add(error_charge(&node.error))
                    .saturating_add(target_charge(node))
            })
            .fold(
                tree.nodes[&tree.root].source.address().address().len(),
                usize::saturating_add,
            );
        assert_eq!(tree.payload_bytes, expected);
        assert!(expected <= tree.limits.bytes);
    }

    #[test]
    fn bounded_diagnostic_releases_excess_capacity_even_when_its_text_is_short() {
        for text in ["文", &"文".repeat(20_000)] {
            let mut message = String::with_capacity(100_000);
            message.push_str(text);
            let ProviderError::Failed(message) =
                bounded_error(ProviderError::Failed(message), 4096)
            else {
                panic!("expected textual diagnostic");
            };
            assert!(message.len() <= 4096);
            assert!(message.capacity() <= 4096);
            assert!(message.starts_with('文'));
        }
    }
}
