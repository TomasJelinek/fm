//! Backend branch occurrences. No row caption or position conveys file authority.
use super::{MountedEntry, MountedLocation, ProviderEntry, ProviderError};
use midnight_core::{Cancellation, EntryKind, NodeRef};
use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
};

static NEXT_TREE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BranchId {
    tree: u64,
    value: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct TreeLimits {
    pub nodes: usize,
    pub bytes: usize,
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
}

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
    error: Option<ProviderError>,
    pending: Option<(u64, Cancellation)>,
}

/// All methods and final destruction run on backend workers. Request execution
/// is separate so blocking providers never hold the tree's actor/state borrow.
pub struct DirectoryTree {
    root: BranchId,
    next_branch: u64,
    next_request: u64,
    nodes: HashMap<BranchId, Node>,
    limits: TreeLimits,
    active: Arc<AtomicUsize>,
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
    root: bool,
    limit: usize,
    cancel: Cancellation,
    permit: Permit,
}
pub struct BranchResult {
    branch: BranchId,
    request: u64,
    kind: TreeRequest,
    rows: Result<Vec<MountedEntry>, ProviderError>,
    _permit: Permit,
}

impl BranchWork {
    /// Run with a shared engine blocking-worker permit, never on the UI thread.
    pub fn run(self) -> BranchResult {
        let rows = if self.root {
            self.location.list(&self.node, &self.cancel)
        } else {
            self.location
                .list_folders(&self.node, &self.cancel, self.limit)
        }
        .and_then(|rows| {
            if rows.iter().any(|row| self.ancestors.contains(&row.node)) {
                return Err(ProviderError::InvalidResponse(
                    "ancestor cycle in directory branch",
                ));
            }
            Ok(rows)
        });
        BranchResult {
            branch: self.branch,
            request: self.request,
            kind: self.kind,
            rows,
            _permit: self.permit,
        }
    }
}

impl DirectoryTree {
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
    pub fn attach_container(
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
        if node.depth >= self.limits.depth {
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
        let bytes = self
            .payload_bytes()
            .saturating_sub(target_charge(node))
            .saturating_add(target.address().address().len());
        if bytes > self.limits.bytes {
            return Err(ProviderError::ResourceLimited);
        }
        let removed = self.descendants(node.children.clone());
        for id in removed {
            if let Some(node) = self.nodes.remove(&id) {
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
        node.expanded = false;
        node.error = None;
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
        })
    }

    pub fn new(location: MountedLocation, limits: TreeLimits) -> Result<Self, ProviderError> {
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
        if charge(&entry).saturating_add(location.address().address().len()) > limits.bytes {
            return Err(ProviderError::ResourceLimited);
        }
        Ok(Self {
            root,
            next_branch: 1,
            next_request: 1,
            limits,
            active: Arc::default(),
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
                    error: None,
                    pending: None,
                },
            )]),
        })
    }

    pub fn root(&self) -> BranchId {
        self.root
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
        if node.entry.kind != EntryKind::Directory && node.target.is_none() {
            return Err(ProviderError::Unsupported);
        }
        if kind == TreeRequest::Probe && node.expanded {
            return Err(ProviderError::Unsupported);
        }
        if node.depth >= self.limits.depth {
            return Err(ProviderError::ResourceLimited);
        }
        let next = self
            .next_request
            .checked_add(1)
            .ok_or(ProviderError::ResourceLimited)?;
        let mut ancestors = Vec::new();
        let mut ancestor = Some(branch);
        while let Some(id) = ancestor {
            let node = &self.nodes[&id];
            ancestors.push(node.node.clone());
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
        node.error = None;
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
            root: branch == self.root,
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
        let rows = self.validate_chain(result.branch).and(result.rows);
        let outcome = rows.and_then(|rows| {
            if result.kind == TreeRequest::Probe {
                self.nodes
                    .get_mut(&result.branch)
                    .ok_or(ProviderError::StaleReference)?
                    .known = Some(!rows.is_empty());
                Ok(())
            } else {
                self.install(result.branch, rows, result.kind == TreeRequest::Expand)
            }
        });
        if let Err(error) = outcome {
            let used = self.payload_bytes();
            if let Some(node) = self.nodes.get_mut(&result.branch) {
                node.error = Some(bounded_error(error, self.limits.bytes.saturating_sub(used)));
            }
        }
        true
    }

    fn install(
        &mut self,
        branch: BranchId,
        rows: Vec<MountedEntry>,
        expand: bool,
    ) -> Result<(), ProviderError> {
        let parent = self
            .nodes
            .get(&branch)
            .ok_or(ProviderError::StaleReference)?;
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
        let mut bytes = self.nodes[&self.root].source.address().address().len();
        for (id, node) in &self.nodes {
            if !removed.contains(id) {
                bytes = bytes.saturating_add(error_charge(&node.error));
                bytes = bytes.saturating_add(target_charge(node));
                if !retained.contains(id) {
                    bytes = bytes.saturating_add(charge(&node.entry));
                }
            }
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
                        error: None,
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
        parent.children = children;
        if expand {
            parent.expanded = true;
        }
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

    fn payload_bytes(&self) -> usize {
        self.nodes
            .values()
            .map(|node| {
                charge(&node.entry)
                    .saturating_add(error_charge(&node.error))
                    .saturating_add(target_charge(node))
            })
            .fold(
                self.nodes[&self.root].source.address().address().len(),
                usize::saturating_add,
            )
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
    if node.entry.kind != EntryKind::Directory && node.target.is_none() {
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

#[cfg(test)]
mod tests {
    use super::*;

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
