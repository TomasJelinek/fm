use super::{
    MountedFilesystem, OpenedLocation, ProviderEntry, ProviderError,
    registry::{MountPermit, Registration, check_cancel, poisoned},
};
use midnight_core::{
    Cancellation, EntryKind, MountRef, NodeRef, NodeToken, SizeDirectoryIdentity, StoredLocation,
};
use midnight_plugin_host::{CapturedFile, ContentSource};
use std::{
    collections::{HashMap, HashSet},
    fmt,
    sync::{Arc, Mutex, RwLock, RwLockReadGuard, TryLockError},
};

const MAX_LIST_ENTRIES: usize = 50_000;
pub(super) const MAX_LIST_BYTES: usize = 32 * 1024 * 1024;
const MAX_ISSUED_NODES: usize = 100_000;
/// Provider callbacks one listing or probe may inspect, rows or not.
const MAX_INSPECTED_ENTRIES: usize = 1_000_000;

#[path = "target.rs"]
mod target;
pub use target::MountedTarget;

struct MountLease {
    reference: MountRef,
    mount: Arc<dyn MountedFilesystem>,
    lifetime: Cancellation,
    nodes: Mutex<HashMap<NodeToken, EntryKind>>,
    // Listings, presence probes and parent lookups hold this shared from their
    // first check through the provider call and issue step. Reclamation takes it
    // exclusively without waiting, so it never releases a reference that a call in
    // flight may bind, emit or issue, and the lease and provider forget together.
    reclaim: RwLock<()>,
    // Plugin-backed mount destructor runs before its registration/library owner.
    _registration: Arc<Registration>,
    _permit: MountPermit,
}

/// Retaining a location/content lease keeps its provider mount alive on the backend.
#[derive(Clone)]
pub struct MountedLocation {
    lease: Arc<MountLease>,
    address: StoredLocation,
    root: NodeRef,
}

#[derive(Clone, Debug)]
pub struct MountedEntry {
    pub node: NodeRef,
    pub entry: ProviderEntry,
}

impl MountedLocation {
    pub(crate) fn release_on_worker(self) -> Result<(), ProviderError> {
        self.lease.release_on_worker()
    }
    pub(super) fn new(
        address: StoredLocation,
        reference: MountRef,
        opened: OpenedLocation,
        lifetime: Cancellation,
        registration: Arc<Registration>,
        permit: MountPermit,
    ) -> Self {
        let root = NodeRef {
            mount: reference,
            token: opened.root,
        };
        let nodes = HashMap::from([(root.token.clone(), EntryKind::Directory)]);
        Self {
            lease: Arc::new(MountLease {
                reference,
                mount: opened.mount,
                lifetime,
                nodes: Mutex::new(nodes),
                reclaim: RwLock::new(()),
                _registration: registration,
                _permit: permit,
            }),
            address,
            root,
        }
    }

    pub fn root(&self) -> &NodeRef {
        &self.root
    }
    pub fn address(&self) -> &StoredLocation {
        &self.address
    }

    /// Resolve browse-only identity through this retained mount. Logical node
    /// targets remain the sole context for later action-specific preparation.
    pub fn browse_directory_identity(
        &self,
        node: &NodeRef,
        cancel: &Cancellation,
    ) -> Result<Option<SizeDirectoryIdentity>, ProviderError> {
        if !matches!(
            self.lease.check_node(node)?,
            EntryKind::Directory | EntryKind::DirectoryLink
        ) {
            return Err(ProviderError::Unsupported);
        }
        let request = self.lease.lifetime.linked(cancel);
        check_cancel(&request)?;
        let identity = self
            .lease
            .mount
            .browse_directory_identity(&node.token, &request)?;
        check_cancel(&request)?;
        Ok(identity)
    }

    pub fn parent(
        &self,
        node: &NodeRef,
        cancel: &Cancellation,
    ) -> Result<Option<NodeRef>, ProviderError> {
        let _listing = self.lease.listing()?;
        self.lease.check_node(node)?;
        let request = self.lease.lifetime.linked(cancel);
        check_cancel(&request)?;
        let token = self.lease.mount.parent(&node.token, &request)?;
        check_cancel(&request)?;
        let Some(token) = token else {
            return Ok(None);
        };
        self.lease
            .issue(std::iter::once((&token, EntryKind::Directory)))?;
        Ok(Some(NodeRef {
            mount: self.lease.reference,
            token,
        }))
    }

    /// Folder-only branch listing. Irrelevant files neither consume retained-row
    /// budgets nor acquire node authority. Following directory links additionally
    /// requires browse identities and ancestor-cycle validation by the tree.
    pub fn list_folders(
        &self,
        node: &NodeRef,
        cancel: &Cancellation,
        limit: usize,
    ) -> Result<Vec<MountedEntry>, ProviderError> {
        self.list_selected(node, cancel, true, limit.min(MAX_LIST_ENTRIES))
    }

    /// Presence-only disclosure probe: whether the directory contains a folder.
    /// Enumeration stops at the first folder; no reference, row or row budget is
    /// issued. Tokens inspected before it share a folder listing's token ledger.
    pub fn has_folders(
        &self,
        node: &NodeRef,
        cancel: &Cancellation,
    ) -> Result<bool, ProviderError> {
        let _listing = self.lease.listing()?;
        if !matches!(
            self.lease.check_node(node)?,
            EntryKind::Directory | EntryKind::DirectoryLink
        ) {
            return Err(ProviderError::Unsupported);
        }
        let request = self.lease.lifetime.linked(cancel);
        check_cancel(&request)?;
        let mut present = false;
        let mut ledger = TokenLedger::new(&self.lease, &node.token, &request);
        let mut failure = None;
        let result = self
            .lease
            .mount
            .list_folders(&node.token, &request, &mut |entry| {
                // A provider that keeps emitting after the stop cannot change the answer.
                if present || failure.is_some() {
                    return false;
                }
                if let Err(error) = ledger.admit(&entry) {
                    failure = Some(error);
                    return false;
                }
                present = matches!(entry.kind, EntryKind::Directory | EntryKind::DirectoryLink);
                !present
            });
        if let Some(error) = failure {
            return Err(error);
        }
        result?;
        check_cancel(&request)?;
        Ok(present)
    }

    /// Complete bounded listing. Partial/failed attempts never issue new references.
    /// Production's streaming projection remains in place during this migration.
    pub fn list(
        &self,
        node: &NodeRef,
        cancel: &Cancellation,
    ) -> Result<Vec<MountedEntry>, ProviderError> {
        self.list_selected(node, cancel, false, MAX_LIST_ENTRIES)
    }

    pub(super) fn validate_reference(&self, node: &NodeRef) -> Result<(), ProviderError> {
        self.lease.check_node(node).map(|_| ())
    }

    /// References this lease currently issues, its root included.
    pub(crate) fn issued_count(&self) -> Result<usize, ProviderError> {
        Ok(self.lease.nodes.lock().map_err(|_| poisoned())?.len())
    }

    /// Node records the provider retains for this mount, when it reports them.
    /// Can exceed the issued count: failed, stopped and presence-only listings
    /// may bind records that are never issued.
    pub(crate) fn provider_node_count(&self) -> Option<usize> {
        self.lease.mount.node_count()
    }

    /// Opt-in reclamation by the mount's single lister: keep the references in
    /// `keep` and the root, release every other issued reference and let the
    /// provider forget the tokens it no longer needs (`retain_nodes`). Released
    /// references fail with `StaleReference`. `keep` must contain every reference
    /// the lister, its checkpoints and its pending work still hold; a resolved
    /// `MountedTarget` keeps working, as it checks registration only.
    ///
    /// Never waits: returns `Ok(None)` without releasing anything while a listing,
    /// presence probe or parent lookup of this mount is in flight. Otherwise
    /// returns the number of references still issued. The provider call runs
    /// outside the issued-table lock.
    pub(crate) fn retain_issued(
        &self,
        keep: &HashSet<NodeToken>,
        cancel: &Cancellation,
    ) -> Result<Option<usize>, ProviderError> {
        check_cancel(cancel)?;
        check_cancel(&self.lease.lifetime)?;
        let _exclusive = match self.lease.reclaim.try_write() {
            Ok(guard) => guard,
            Err(TryLockError::WouldBlock) => return Ok(None),
            Err(TryLockError::Poisoned(_)) => return Err(poisoned()),
        };
        let with_root;
        let keep = if keep.contains(&self.root.token) {
            keep
        } else {
            with_root = keep
                .iter()
                .cloned()
                .chain([self.root.token.clone()])
                .collect::<HashSet<_>>();
            &with_root
        };
        let issued = {
            let mut nodes = self.lease.nodes.lock().map_err(|_| poisoned())?;
            nodes.retain(|token, _| keep.contains(token));
            super::release_spare_capacity(&mut nodes);
            nodes.len()
        };
        // The lease forgot first: every reference it still issues remains known
        // to the provider even if the provider fails to forget its own records.
        self.lease.mount.retain_nodes(keep)?;
        Ok(Some(issued))
    }

    #[cfg(test)]
    pub(crate) fn issued_capacity(&self) -> usize {
        self.lease.nodes.lock().unwrap().capacity()
    }

    fn list_selected(
        &self,
        node: &NodeRef,
        cancel: &Cancellation,
        folders_only: bool,
        limit: usize,
    ) -> Result<Vec<MountedEntry>, ProviderError> {
        let _listing = self.lease.listing()?;
        if !matches!(
            self.lease.check_node(node)?,
            EntryKind::Directory | EntryKind::DirectoryLink
        ) {
            return Err(ProviderError::Unsupported);
        }
        let request = self.lease.lifetime.linked(cancel);
        check_cancel(&request)?;
        let mut entries = Vec::new();
        let mut ledger = TokenLedger::new(&self.lease, &node.token, &request);
        let mut bytes = 0_usize;
        let mut failure = None;
        let mut accept = |entry: ProviderEntry| {
            if failure.is_some() {
                return false;
            }
            if let Err(error) = ledger.admit(&entry) {
                failure = Some(error);
                return false;
            }
            if folders_only
                && !matches!(entry.kind, EntryKind::Directory | EntryKind::DirectoryLink)
            {
                return true;
            }
            let result = (|| {
                let localized = entry
                    .metadata
                    .localized_modified
                    .as_ref()
                    .map_or(0, |value| {
                        value.date.capacity().saturating_add(value.time.capacity())
                    });
                let size = std::mem::size_of::<MountedEntry>()
                    .saturating_add(entry.raw_name.len())
                    .saturating_add(entry.display_name.capacity())
                    .saturating_add(entry.token.as_bytes().len())
                    .saturating_add(localized);
                bytes = bytes.saturating_add(size);
                if entries.len() >= limit || bytes > MAX_LIST_BYTES {
                    return Err(ProviderError::ResourceLimited);
                }
                Ok(())
            })();
            if let Err(error) = result {
                failure = Some(error);
                return false;
            }
            entries.push(MountedEntry {
                node: NodeRef {
                    mount: self.lease.reference,
                    token: entry.token.clone(),
                },
                entry,
            });
            true
        };
        let result = if folders_only {
            self.lease
                .mount
                .list_folders(&node.token, &request, &mut accept)
        } else {
            self.lease.mount.list(&node.token, &request, &mut accept)
        };
        if let Some(error) = failure {
            return Err(error);
        }
        result?;
        check_cancel(&request)?;
        self.lease.issue(
            entries
                .iter()
                .map(|entry| (&entry.node.token, entry.entry.kind)),
        )?;
        Ok(entries)
    }

    /// Viewer/thumbnail brokers consume this source without inventing a native path.
    pub fn open_content(
        &self,
        node: &NodeRef,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn ContentSource>, ProviderError> {
        if self.lease.check_node(node)? != EntryKind::File {
            return Err(ProviderError::Unsupported);
        }
        let request = self.lease.lifetime.linked(cancel);
        let mut construction = crate::copy_nested_retirement::Retained::new(ContentConstruction {
            source: None,
            captured: None,
            lease: Some(self.lease.clone()),
        });
        let result = (|| {
            check_cancel(&request)?;
            let source = self.lease.mount.open_content(&node.token, &request)?;
            construction.get_mut().source = Some(source);
            check_cancel(&request)?;
            // The anchor keeps final responsibility while the host constructor
            // validates and may reject its independently consumed reference.
            let source = construction
                .get()
                .source
                .as_ref()
                .expect("returned content")
                .clone();
            construction.get_mut().captured = Some(
                CapturedFile::from_source(source)
                    .map_err(|error| ProviderError::Failed(error.to_string()))?,
            );
            check_cancel(&request)?;
            construction.get_mut().release_source()?;
            let state = construction.get_mut();
            Ok(Arc::new(LeasedContent {
                lease: state.lease.take().expect("captured mount lease"),
                source: state.captured.take().expect("validated content"),
            }) as Arc<dyn ContentSource>)
        })();
        let result = match result {
            Ok(source) => Ok(source),
            Err(original) => Err(super::registry::construction_failure(
                original,
                construction.get_mut().release(),
            )),
        };
        drop(construction.take());
        result
    }
}

struct ContentConstruction {
    source: Option<Arc<dyn ContentSource>>,
    captured: Option<CapturedFile>,
    lease: Option<Arc<MountLease>>,
}
impl ContentConstruction {
    fn release_source(&mut self) -> Result<(), ProviderError> {
        if let Some(source) = self.source.take() {
            crate::copy_nested_retirement::content_step(|| source.release_on_worker())
                .map_err(|error| ProviderError::Failed(error.to_string()))?;
        }
        Ok(())
    }
    fn release(&mut self) -> Result<(), ProviderError> {
        use crate::copy_nested_retirement::{self as nested, Retained};
        let mut lease = Retained::new(self.lease.take());
        let mut captured = Retained::new(self.captured.take());
        self.release_source()?;
        if let Some(captured) = captured.take() {
            nested::release_captured_file(captured)
                .map_err(|error| ProviderError::Failed(error.to_string()))?;
        }
        if let Some(lease) = lease.take() {
            lease.release_on_worker()?;
        }
        Ok(())
    }
}

/// Validation shared by complete, folder-only and presence listings: cancellation,
/// the inspected-callback ceiling, the 32 MiB token ledger (charged before a token
/// is kept for duplicate detection), duplicate or self-referencing tokens and kind
/// changes of already issued nodes.
struct TokenLedger<'a> {
    lease: &'a MountLease,
    directory: &'a NodeToken,
    request: &'a Cancellation,
    seen: HashSet<NodeToken>,
    inspected: usize,
    bytes: usize,
}

impl<'a> TokenLedger<'a> {
    fn new(lease: &'a MountLease, directory: &'a NodeToken, request: &'a Cancellation) -> Self {
        Self {
            lease,
            directory,
            request,
            seen: HashSet::new(),
            inspected: 0,
            bytes: 0,
        }
    }

    fn admit(&mut self, entry: &ProviderEntry) -> Result<(), ProviderError> {
        check_cancel(self.request)?;
        self.inspected = self.inspected.saturating_add(1);
        if self.inspected > MAX_INSPECTED_ENTRIES {
            return Err(ProviderError::ResourceLimited);
        }
        // Token bytes plus the retained handle and its duplicate-set slot.
        self.bytes = self
            .bytes
            .saturating_add(entry.token.as_bytes().len())
            .saturating_add(std::mem::size_of::<NodeToken>() + 64);
        if self.bytes > MAX_LIST_BYTES {
            return Err(ProviderError::ResourceLimited);
        }
        if entry.token == *self.directory || !self.seen.insert(entry.token.clone()) {
            return Err(ProviderError::InvalidResponse(
                "duplicate or self-referencing listing node",
            ));
        }
        self.lease.validate_kind(&entry.token, entry.kind)
    }
}

impl MountLease {
    fn release_on_worker(self: Arc<Self>) -> Result<(), ProviderError> {
        use crate::copy_nested_retirement::{self as nested, Retained};
        nested::provider_step(|| {
            let Some(owner) = Arc::into_inner(self) else {
                return Ok(());
            };
            let MountLease {
                mount,
                _registration,
                _permit,
                ..
            } = owner;
            let mut tail = Retained::new((_registration, _permit));
            nested::release_mount(mount)?;
            let (registration, permit) = tail.take();
            let mut permit = Retained::new(permit);
            registration.release_on_worker()?;
            drop(permit.take());
            Ok(())
        })
    }
    /// Shared reclaim gate for one call that may bind, emit or issue references.
    fn listing(&self) -> Result<RwLockReadGuard<'_, ()>, ProviderError> {
        self.reclaim.read().map_err(|_| poisoned())
    }

    /// Registration lifetime and mount identity only, not current issuance.
    fn check_registration(&self, node: &NodeRef) -> Result<(), ProviderError> {
        if self.lifetime.is_cancelled() || node.mount != self.reference {
            return Err(ProviderError::StaleReference);
        }
        Ok(())
    }

    fn validate_kind(&self, token: &NodeToken, kind: EntryKind) -> Result<(), ProviderError> {
        let nodes = self.nodes.lock().map_err(|_| poisoned())?;
        if nodes.get(token).is_some_and(|known| *known != kind) {
            return Err(ProviderError::InvalidResponse("issued node changed kind"));
        }
        Ok(())
    }

    fn check_node(&self, node: &NodeRef) -> Result<EntryKind, ProviderError> {
        self.check_registration(node)?;
        self.nodes
            .lock()
            .map_err(|_| poisoned())?
            .get(&node.token)
            .copied()
            .ok_or(ProviderError::StaleReference)
    }

    fn issue<'a>(
        &self,
        values: impl Iterator<Item = (&'a NodeToken, EntryKind)> + Clone,
    ) -> Result<(), ProviderError> {
        let mut nodes = self.nodes.lock().map_err(|_| poisoned())?;
        if values
            .clone()
            .any(|(token, kind)| nodes.get(token).is_some_and(|known| *known != kind))
        {
            return Err(ProviderError::InvalidResponse("issued node changed kind"));
        }
        let additions = values
            .clone()
            .filter(|(token, _)| !nodes.contains_key(*token))
            .count();
        if nodes.len().saturating_add(additions) > MAX_ISSUED_NODES {
            return Err(ProviderError::ResourceLimited);
        }
        for (token, kind) in values {
            nodes.insert(token.clone(), kind);
        }
        Ok(())
    }
}

struct LeasedContent {
    source: CapturedFile,
    // Source destructor runs while the mount and plugin registration are alive.
    lease: Arc<MountLease>,
}

impl fmt::Debug for LeasedContent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LeasedContent")
            .field("mount", &self.lease.reference)
            .finish_non_exhaustive()
    }
}

impl ContentSource for LeasedContent {
    fn cleanup_unconfirmed(&self) -> bool {
        self.source.cleanup_unconfirmed()
    }
    fn release_on_worker(self: Arc<Self>) -> Result<(), midnight_plugin_host::Error> {
        use crate::copy_nested_retirement::{self as nested, Retained};
        let mut reference = Retained::new(self);
        nested::content_step(|| {
            if reference.get().cleanup_unconfirmed() {
                return Err(midnight_plugin_host::Error(
                    nested::unconfirmed().to_string(),
                ));
            }
            let Some(owner) = Arc::into_inner(reference.take()) else {
                return Ok(());
            };
            let mut lease = Retained::new(owner.lease);
            nested::release_captured_file(owner.source)?;
            lease
                .take()
                .release_on_worker()
                .map_err(|error| midnight_plugin_host::Error(error.to_string()))
        })
    }
    fn length(&self) -> u64 {
        self.source.length()
    }
    fn display_name(&self) -> String {
        self.source.display_name()
    }
    fn version(&self) -> Vec<u8> {
        self.source.stable_version()
    }
    fn validate(&self) -> Result<(), midnight_plugin_host::Error> {
        if self.lease.lifetime.is_cancelled() {
            return Err(midnight_plugin_host::Error(
                "Filesystem lease was revoked.".into(),
            ));
        }
        self.source.revalidate()?;
        if self.lease.lifetime.is_cancelled() {
            return Err(midnight_plugin_host::Error(
                "Filesystem lease was revoked.".into(),
            ));
        }
        Ok(())
    }
    fn read_at(&self, offset: u64, length: u32) -> Result<Vec<u8>, midnight_plugin_host::Error> {
        self.validate()?;
        let bytes = self.source.read_at(offset, length)?;
        self.validate()?;
        Ok(bytes)
    }
}
