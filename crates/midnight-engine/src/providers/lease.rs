use super::{
    MountedFilesystem, OpenedLocation, ProviderEntry, ProviderError,
    registry::{MountPermit, Registration, check_cancel, poisoned},
};
use midnight_core::{Cancellation, EntryKind, MountRef, NodeRef, NodeToken, StoredLocation};
use midnight_plugin_host::{CapturedFile, ContentSource};
use std::{
    collections::{HashMap, HashSet},
    fmt,
    sync::{Arc, Mutex},
};

const MAX_LIST_ENTRIES: usize = 50_000;
const MAX_LIST_BYTES: usize = 32 * 1024 * 1024;
const MAX_ISSUED_NODES: usize = 100_000;

struct MountLease {
    reference: MountRef,
    mount: Arc<dyn MountedFilesystem>,
    lifetime: Cancellation,
    nodes: Mutex<HashMap<NodeToken, EntryKind>>,
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

    pub fn parent(
        &self,
        node: &NodeRef,
        cancel: &Cancellation,
    ) -> Result<Option<NodeRef>, ProviderError> {
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
    /// budgets nor acquire node authority. Directory links are returned for display;
    /// callers must not recursively enumerate them automatically.
    pub fn list_folders(
        &self,
        node: &NodeRef,
        cancel: &Cancellation,
        limit: usize,
    ) -> Result<Vec<MountedEntry>, ProviderError> {
        self.list_selected(node, cancel, true, limit.min(MAX_LIST_ENTRIES))
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

    fn list_selected(
        &self,
        node: &NodeRef,
        cancel: &Cancellation,
        folders_only: bool,
        limit: usize,
    ) -> Result<Vec<MountedEntry>, ProviderError> {
        if !matches!(
            self.lease.check_node(node)?,
            EntryKind::Directory | EntryKind::DirectoryLink
        ) {
            return Err(ProviderError::Unsupported);
        }
        let request = self.lease.lifetime.linked(cancel);
        check_cancel(&request)?;
        let mut entries = Vec::new();
        let mut seen = HashSet::new();
        let mut bytes = 0_usize;
        let mut token_bytes = 0_usize;
        let mut inspected = 0_usize;
        let mut failure = None;
        let result = self.lease.mount.list(&node.token, &request, &mut |entry| {
            if failure.is_some() {
                return false;
            }
            let result = check_cancel(&request).and_then(|()| {
                inspected = inspected.saturating_add(1);
                if inspected > 1_000_000 {
                    return Err(ProviderError::ResourceLimited);
                }
                token_bytes = token_bytes
                    .saturating_add(entry.token.as_bytes().len())
                    .saturating_add(std::mem::size_of::<NodeToken>() + 64);
                if token_bytes > MAX_LIST_BYTES {
                    return Err(ProviderError::ResourceLimited);
                }
                if entry.token == node.token || !seen.insert(entry.token.clone()) {
                    return Err(ProviderError::InvalidResponse(
                        "duplicate or self-referencing listing node",
                    ));
                }
                self.lease.validate_kind(&entry.token, entry.kind)?;
                Ok(())
            });
            if let Err(error) = result {
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
        });
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
        check_cancel(&request)?;
        let source = self.lease.mount.open_content(&node.token, &request)?;
        check_cancel(&request)?;
        let source = CapturedFile::from_source(source)
            .map_err(|error| ProviderError::Failed(error.to_string()))?;
        check_cancel(&request)?;
        Ok(Arc::new(LeasedContent {
            lease: self.lease.clone(),
            source,
        }))
    }
}

impl MountLease {
    fn validate_kind(&self, token: &NodeToken, kind: EntryKind) -> Result<(), ProviderError> {
        let nodes = self.nodes.lock().map_err(|_| poisoned())?;
        if nodes.get(token).is_some_and(|known| *known != kind) {
            return Err(ProviderError::InvalidResponse("issued node changed kind"));
        }
        Ok(())
    }

    fn check_node(&self, node: &NodeRef) -> Result<EntryKind, ProviderError> {
        if self.lifetime.is_cancelled() || node.mount != self.reference {
            return Err(ProviderError::StaleReference);
        }
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
