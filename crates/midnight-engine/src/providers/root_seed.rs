//! Captured initial rows, with an explicit native-only subsequent-read policy.
use super::{MountedFilesystem, OpenedLocation, ProviderEntry, ProviderError, ProviderTarget};
use midnight_core::{Cancellation, Entry, NodeToken, SizeDirectoryIdentity};
use midnight_plugin_host::ContentSource;
use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};

const MAX_ENTRIES: usize = 50_000;
const MAX_BYTES: usize = 32 * 1024 * 1024;

pub(super) enum RootReadPolicy {
    CapturedOnce,
    NativeRefresh,
}

/// Check borrowed captured input before cloning it or constructing adapter caches.
pub(super) fn preflight(entries: &[Entry], cancel: &Cancellation) -> Result<(), ProviderError> {
    super::registry::check_cancel(cancel)?;
    if entries.len() > MAX_ENTRIES {
        return Err(ProviderError::ResourceLimited);
    }
    let mut bytes = 0usize;
    for entry in entries {
        super::registry::check_cancel(cancel)?;
        bytes = bytes
            .saturating_add(std::mem::size_of::<Entry>())
            .saturating_add(entry.path.as_os_str().as_encoded_bytes().len())
            .saturating_add(entry.name.as_encoded_bytes().len())
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
            .saturating_add(entry.virtual_node.as_ref().map_or(0, |node| {
                node.raw_name
                    .len()
                    .saturating_add(node.display_name.capacity())
            }));
        if bytes > MAX_BYTES {
            return Err(ProviderError::ResourceLimited);
        }
    }
    Ok(())
}

pub(super) fn wrap(
    opened: OpenedLocation,
    rows: Vec<ProviderEntry>,
    policy: RootReadPolicy,
    cancel: &Cancellation,
) -> Result<OpenedLocation, ProviderError> {
    let mut tokens = HashSet::new();
    let mut bytes = 0usize;
    for row in &rows {
        super::registry::check_cancel(cancel)?;
        if row.token == opened.root || !tokens.insert(&row.token) {
            return Err(ProviderError::InvalidResponse(
                "duplicate captured root node",
            ));
        }
        bytes = bytes
            .saturating_add(std::mem::size_of::<ProviderEntry>())
            .saturating_add(row.raw_name.len())
            .saturating_add(row.display_name.capacity())
            .saturating_add(row.token.as_bytes().len())
            .saturating_add(row.metadata.localized_modified.as_ref().map_or(0, |value| {
                value.date.capacity().saturating_add(value.time.capacity())
            }));
        if bytes > MAX_BYTES {
            return Err(ProviderError::ResourceLimited);
        }
    }
    super::registry::check_cancel(cancel)?;
    Ok(OpenedLocation {
        root: opened.root.clone(),
        mount: Arc::new(RootSeed {
            source: opened.mount,
            root: opened.root,
            rows: Mutex::new(Some(rows)),
            policy,
        }),
    })
}

struct RootSeed {
    source: Arc<dyn MountedFilesystem>,
    root: NodeToken,
    rows: Mutex<Option<Vec<ProviderEntry>>>,
    policy: RootReadPolicy,
}

impl MountedFilesystem for RootSeed {
    fn browse_directory_identity(
        &self,
        node: &NodeToken,
        cancel: &Cancellation,
    ) -> Result<Option<SizeDirectoryIdentity>, ProviderError> {
        self.source.browse_directory_identity(node, cancel)
    }

    fn list(
        &self,
        directory: &NodeToken,
        cancel: &Cancellation,
        emit: &mut dyn FnMut(ProviderEntry) -> bool,
    ) -> Result<(), ProviderError> {
        if directory != &self.root {
            return self.source.list(directory, cancel, emit);
        }
        super::registry::check_cancel(cancel)?;
        // The adapter validates its captured source, without enumerating this root.
        self.source.resolve_target(&self.root, cancel)?;
        let rows = self
            .rows
            .lock()
            .map_err(|_| super::registry::poisoned())?
            .take();
        let Some(rows) = rows else {
            return match self.policy {
                RootReadPolicy::CapturedOnce => Err(ProviderError::StaleReference),
                RootReadPolicy::NativeRefresh => self.source.list(directory, cancel, emit),
            };
        };
        for row in rows {
            super::registry::check_cancel(cancel)?;
            if !emit(row) {
                break;
            }
        }
        self.source.resolve_target(&self.root, cancel)?;
        super::registry::check_cancel(cancel)
    }

    fn resolve_target(
        &self,
        node: &NodeToken,
        cancel: &Cancellation,
    ) -> Result<ProviderTarget, ProviderError> {
        self.source.resolve_target(node, cancel)
    }
    fn parent(
        &self,
        node: &NodeToken,
        cancel: &Cancellation,
    ) -> Result<Option<NodeToken>, ProviderError> {
        self.source.parent(node, cancel)
    }
    fn open_content(
        &self,
        node: &NodeToken,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn ContentSource>, ProviderError> {
        self.source.open_content(node, cancel)
    }
}
