//! Transitional adapter for an already captured read-only archive session.
use super::{
    ArchiveTarget, MountedFilesystem, OpenedLocation, ProviderEntry, ProviderError, ProviderTarget,
};
use midnight_core::{
    ArchiveLocation, ArchiveMount, ArchiveNode, Cancellation, EntryKind, EntryMetadata, FsError,
    NodeToken,
};
use midnight_plugin_host::{ContentSource, Error};
use std::{
    collections::{HashMap, HashSet},
    fmt, io,
    sync::{Arc, Mutex},
};

const MAX_NODES: usize = 100_000;
const MAX_NAMES_BYTES: usize = 8 * 1024 * 1024;
const MAX_READ_BYTES: u32 = 256 * 1024;
const MAX_COMPONENTS: usize = 256;
const MAX_LOCATOR_BYTES: usize = 64 * 1024;

#[derive(Clone)]
struct Record {
    parent: Option<u64>,
    kind: EntryKind,
    node: Option<ArchiveNode>,
}

struct Records {
    nodes: HashMap<u64, Record>,
    names_bytes: usize,
}

struct CapturedArchive {
    mount: Arc<dyn ArchiveMount>,
    captured: ArchiveLocation,
    records: Mutex<Records>,
}

/// Backend only. Does not discover plugins, open a native path or restore an address.
pub fn captured_archive(
    location: &ArchiveLocation,
    cancel: &Cancellation,
) -> Result<OpenedLocation, ProviderError> {
    super::registry::check_cancel(cancel)?;
    let mount = location.mount.clone().ok_or(ProviderError::Unavailable)?;
    if mount.id() == 0
        || location.address.source_version.is_empty()
        || location.address.source_version.len() > 256
        || location.trail.is_empty()
        || location.trail.len() > MAX_COMPONENTS + 1
        || location.address.components.len() + 1 != location.trail.len()
        || location.trail.first().map(|row| row.0) != Some(1)
        || location.trail.last().map(|row| row.0) != Some(location.directory)
    {
        return Err(ProviderError::InvalidResponse(
            "invalid captured archive location",
        ));
    }
    location
        .address
        .validate()
        .map_err(|_| ProviderError::InvalidResponse("invalid captured archive address"))?;
    let names_bytes = location.trail.iter().try_fold(
        location
            .address
            .components
            .iter()
            .map(Vec::len)
            .sum::<usize>(),
        |total, (_, caption)| {
            let total = total.saturating_add(caption.len());
            if caption.len() > MAX_LOCATOR_BYTES || total > MAX_NAMES_BYTES {
                Err(ProviderError::ResourceLimited)
            } else {
                Ok(total)
            }
        },
    )?;
    check(mount.as_ref(), cancel)?;
    let root = token(mount.id(), location.directory)?;
    let mut nodes = HashMap::new();
    let mut parent = None;
    for (id, _) in &location.trail {
        if *id == 0 || nodes.contains_key(id) {
            return Err(ProviderError::InvalidResponse(
                "invalid archive parent trail",
            ));
        }
        nodes.insert(
            *id,
            Record {
                parent,
                kind: EntryKind::Directory,
                node: None,
            },
        );
        parent = Some(*id);
    }
    let captured = location.clone();
    check(mount.as_ref(), cancel)?;
    Ok(OpenedLocation {
        root,
        mount: Arc::new(CapturedArchive {
            mount,
            captured,
            records: Mutex::new(Records { nodes, names_bytes }),
        }),
    })
}

impl CapturedArchive {
    fn base_index(&self, id: u64) -> Option<usize> {
        self.captured.trail.iter().position(|row| row.0 == id)
    }

    fn validate_member(
        &self,
        id: u64,
        record: &Record,
        node: &ArchiveNode,
    ) -> Result<(), ProviderError> {
        if node.id.mount != self.mount.id()
            || node.id.entry != id
            || record.parent != Some(node.parent)
            || record.kind != node.kind
        {
            return Err(ProviderError::InvalidResponse(
                "invalid retained archive member",
            ));
        }
        if node.raw_name.len() > MAX_LOCATOR_BYTES || node.display_name.len() > MAX_LOCATOR_BYTES {
            return Err(ProviderError::ResourceLimited);
        }
        let raw = node.raw_name.as_ref();
        if raw.is_empty()
            || raw == b"."
            || raw == b".."
            || raw.iter().any(|byte| b"/\\:\0".contains(byte))
        {
            return Err(ProviderError::InvalidResponse(
                "invalid archive member locator",
            ));
        }
        Ok(())
    }

    fn archive_target(
        &self,
        token: &NodeToken,
        cancel: &Cancellation,
    ) -> Result<ArchiveTarget, ProviderError> {
        let (id, record) = self.record(token)?;
        let (directory, member) = match record.kind {
            EntryKind::Directory => (id, None),
            EntryKind::File => {
                let member = record
                    .node
                    .as_ref()
                    .ok_or(ProviderError::InvalidResponse("missing archive member"))?;
                self.validate_member(id, &record, member)?;
                (member.parent, Some(member.clone()))
            }
            EntryKind::DirectoryLink | EntryKind::Symlink | EntryKind::Other => {
                return Err(ProviderError::Unsupported);
            }
        };
        let mut chain = Vec::new();
        let mut seen = HashSet::new();
        let mut current = directory;
        let mut raw_bytes = 0usize;
        let mut names_bytes =
            self.captured.trail[0]
                .1
                .len()
                .saturating_add(member.as_ref().map_or(0, |node| {
                    node.raw_name.len().saturating_add(node.display_name.len())
                }));
        let records = self
            .records
            .lock()
            .map_err(|_| super::registry::poisoned())?;
        loop {
            super::registry::check_cancel(cancel)?;
            if !seen.insert(current) {
                return Err(ProviderError::InvalidResponse(
                    "cyclic archive parent trail",
                ));
            }
            let record = records
                .nodes
                .get(&current)
                .ok_or(ProviderError::InvalidResponse("unknown archive parent"))?;
            if record.kind != EntryKind::Directory {
                return Err(ProviderError::InvalidResponse(
                    "archive parent is not a directory",
                ));
            }
            if current == 1 {
                if record.parent.is_some() {
                    return Err(ProviderError::InvalidResponse(
                        "invalid archive root parent",
                    ));
                }
                break;
            }
            if chain.len() >= MAX_COMPONENTS {
                return Err(ProviderError::ResourceLimited);
            }
            let parent = record
                .parent
                .ok_or(ProviderError::InvalidResponse("missing archive parent"))?;
            let (raw, caption) = if let Some(index) = self.base_index(current) {
                if index == 0 || parent != self.captured.trail[index - 1].0 {
                    return Err(ProviderError::InvalidResponse(
                        "captured archive parent changed",
                    ));
                }
                let raw = &self.captured.address.components[index - 1];
                if let Some(node) = &record.node {
                    self.validate_member(current, record, node)?;
                    if node.raw_name.as_ref() != raw.as_slice() {
                        return Err(ProviderError::InvalidResponse(
                            "captured archive name changed",
                        ));
                    }
                }
                (raw.clone(), self.captured.trail[index].1.clone())
            } else {
                let node = record
                    .node
                    .as_ref()
                    .ok_or(ProviderError::InvalidResponse("missing archive ancestor"))?;
                self.validate_member(current, record, node)?;
                (node.raw_name.to_vec(), node.display_name.clone())
            };
            raw_bytes = raw_bytes.saturating_add(raw.len());
            names_bytes = names_bytes
                .saturating_add(raw.len())
                .saturating_add(caption.len());
            if raw_bytes > MAX_LOCATOR_BYTES || names_bytes > MAX_NAMES_BYTES {
                return Err(ProviderError::ResourceLimited);
            }
            chain.push((current, raw, caption));
            current = parent;
        }
        drop(records);
        let mut trail = vec![self.captured.trail[0].clone()];
        let mut components = Vec::with_capacity(chain.len());
        for (id, raw, caption) in chain.into_iter().rev() {
            trail.push((id, caption));
            components.push(raw);
        }
        let mut address = self.captured.address.clone();
        address.components = components;
        address
            .validate()
            .map_err(|_| ProviderError::InvalidResponse("invalid archive target address"))?;
        Ok(ArchiveTarget {
            location: ArchiveLocation {
                mount: Some(self.mount.clone()),
                address,
                directory,
                trail,
            },
            member,
        })
    }

    fn record(&self, token: &NodeToken) -> Result<(u64, Record), ProviderError> {
        let raw: [u8; 16] = token
            .as_bytes()
            .try_into()
            .map_err(|_| ProviderError::StaleReference)?;
        let owner = u64::from_le_bytes(
            raw[..8]
                .try_into()
                .map_err(|_| ProviderError::StaleReference)?,
        );
        let id = u64::from_le_bytes(
            raw[8..]
                .try_into()
                .map_err(|_| ProviderError::StaleReference)?,
        );
        if owner != self.mount.id() {
            return Err(ProviderError::StaleReference);
        }
        let record = self
            .records
            .lock()
            .map_err(|_| super::registry::poisoned())?
            .nodes
            .get(&id)
            .cloned()
            .ok_or(ProviderError::StaleReference)?;
        Ok((id, record))
    }

    /// Validate a whole listing before publishing callbacks or updating the cache.
    fn validate_rows(&self, directory: u64, rows: &[ArchiveNode]) -> Result<(), ProviderError> {
        if rows.len() > MAX_NODES {
            return Err(ProviderError::ResourceLimited);
        }
        let mut seen = HashSet::new();
        let mut bytes = 0usize;
        for node in rows {
            if node.id.mount != self.mount.id()
                || node.id.entry <= 1
                || node.id.entry == directory
                || node.parent != directory
                || !seen.insert(node.id.entry)
            {
                return Err(ProviderError::InvalidResponse(
                    "invalid archive listing identity",
                ));
            }
            bytes = bytes
                .saturating_add(node.raw_name.len())
                .saturating_add(node.display_name.len());
            if bytes > MAX_NAMES_BYTES
                || node.raw_name.len() > 64 * 1024
                || node.display_name.len() > 64 * 1024
            {
                return Err(ProviderError::ResourceLimited);
            }
        }
        Ok(())
    }

    /// Both validation and cache installation are transactional under this short lock.
    fn cache(&self, rows: &[ArchiveNode], install: bool) -> Result<(), ProviderError> {
        let mut records = self
            .records
            .lock()
            .map_err(|_| super::registry::poisoned())?;
        let mut additions = 0usize;
        let mut bytes = records.names_bytes;
        for node in rows {
            if let Some(old) = records.nodes.get(&node.id.entry) {
                if old.parent != Some(node.parent)
                    || old.kind != node.kind
                    || (old.node.is_none()
                        && self.base_index(node.id.entry).is_some_and(|index| {
                            index == 0
                                || self.captured.address.components[index - 1].as_slice()
                                    != node.raw_name.as_ref()
                        }))
                    || old.node.as_ref().is_some_and(|old| {
                        old.raw_name != node.raw_name
                            || old.size != node.size
                            || old.readable != node.readable
                    })
                {
                    return Err(ProviderError::InvalidResponse(
                        "archive member identity changed",
                    ));
                }
                if old.node.is_some() {
                    continue;
                }
            } else {
                additions += 1;
            }
            bytes = bytes
                .saturating_add(node.raw_name.len())
                .saturating_add(node.display_name.len());
        }
        if records.nodes.len().saturating_add(additions) > MAX_NODES || bytes > MAX_NAMES_BYTES {
            return Err(ProviderError::ResourceLimited);
        }
        if install {
            for node in rows {
                records
                    .nodes
                    .entry(node.id.entry)
                    .and_modify(|record| {
                        // Captured immutable members keep their original metadata/byte accounting.
                        if record.node.is_none() {
                            record.node = Some(node.clone());
                        }
                    })
                    .or_insert_with(|| Record {
                        parent: Some(node.parent),
                        kind: node.kind,
                        node: Some(node.clone()),
                    });
            }
            records.names_bytes = bytes;
        }
        Ok(())
    }
}

impl MountedFilesystem for CapturedArchive {
    fn resolve_target(
        &self,
        node: &NodeToken,
        cancel: &Cancellation,
    ) -> Result<ProviderTarget, ProviderError> {
        check(self.mount.as_ref(), cancel)?;
        let target = self.archive_target(node, cancel)?;
        check(self.mount.as_ref(), cancel)?;
        Ok(ProviderTarget::Archive(Box::new(target)))
    }

    fn list(
        &self,
        directory: &NodeToken,
        cancel: &Cancellation,
        emit: &mut dyn FnMut(ProviderEntry) -> bool,
    ) -> Result<(), ProviderError> {
        check(self.mount.as_ref(), cancel)?;
        let (id, record) = self.record(directory)?;
        if record.kind != EntryKind::Directory {
            return Err(ProviderError::Unsupported);
        }
        let rows = self.mount.list(id, cancel).map_err(map_error)?;
        check(self.mount.as_ref(), cancel)?;
        self.validate_rows(id, &rows)?;
        self.cache(&rows, false)?;
        for node in &rows {
            super::registry::check_cancel(cancel)?;
            if !emit(ProviderEntry {
                token: token(self.mount.id(), node.id.entry)?,
                raw_name: node.raw_name.clone(),
                display_name: node.display_name.clone(),
                kind: node.kind,
                metadata: EntryMetadata {
                    size: node.size,
                    ..EntryMetadata::default()
                },
            }) {
                return Ok(());
            }
        }
        check(self.mount.as_ref(), cancel)?;
        self.cache(&rows, true)
    }

    fn parent(
        &self,
        node: &NodeToken,
        cancel: &Cancellation,
    ) -> Result<Option<NodeToken>, ProviderError> {
        check(self.mount.as_ref(), cancel)?;
        let (_, record) = self.record(node)?;
        record
            .parent
            .map(|parent| token(self.mount.id(), parent))
            .transpose()
    }

    fn open_content(
        &self,
        node: &NodeToken,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn ContentSource>, ProviderError> {
        check(self.mount.as_ref(), cancel)?;
        let (id, record) = self.record(node)?;
        let node = record.node.ok_or(ProviderError::Unsupported)?;
        if node.kind != EntryKind::File || !node.readable {
            return Err(ProviderError::Unsupported);
        }
        let length = node.size.ok_or(ProviderError::Unsupported)?;
        let mut version = self.captured.address.source_version.clone();
        version.extend_from_slice(&self.mount.id().to_le_bytes());
        version.extend_from_slice(&id.to_le_bytes());
        Ok(Arc::new(MemberContent {
            mount: self.mount.clone(),
            entry: id,
            length,
            name: node.display_name,
            version,
        }))
    }
}

struct MemberContent {
    mount: Arc<dyn ArchiveMount>,
    entry: u64,
    length: u64,
    name: String,
    version: Vec<u8>,
}

impl fmt::Debug for MemberContent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ArchiveContent")
            .field("mount", &self.mount.id())
            .field("entry", &self.entry)
            .finish_non_exhaustive()
    }
}

impl ContentSource for MemberContent {
    fn length(&self) -> u64 {
        self.length
    }
    fn display_name(&self) -> String {
        self.name.clone()
    }
    fn version(&self) -> Vec<u8> {
        self.version.clone()
    }
    fn validate(&self) -> Result<(), Error> {
        self.mount
            .validate()
            .map_err(|error| Error(error.to_string()))
    }
    fn read_at(&self, offset: u64, length: u32) -> Result<Vec<u8>, Error> {
        if length == 0 || length > MAX_READ_BYTES || offset.checked_add(u64::from(length)).is_none()
        {
            return Err(Error("Invalid archive content read bounds.".into()));
        }
        self.validate()?;
        let wanted = self.length.saturating_sub(offset).min(u64::from(length)) as u32;
        let bytes = if wanted == 0 {
            Vec::new()
        } else {
            self.mount
                .read(self.entry, offset, wanted, &Cancellation::default())
                .map_err(|error| Error(error.to_string()))?
        };
        self.validate()?;
        if bytes.len() != wanted as usize {
            return Err(Error(
                "Archive content returned an inconsistent read length.".into(),
            ));
        }
        Ok(bytes)
    }
}

fn token(mount: u64, entry: u64) -> Result<NodeToken, ProviderError> {
    NodeToken::new(&[mount.to_le_bytes(), entry.to_le_bytes()].concat())
        .map_err(|_| ProviderError::InvalidResponse("invalid archive node token"))
}

fn check(mount: &dyn ArchiveMount, cancel: &Cancellation) -> Result<(), ProviderError> {
    super::registry::check_cancel(cancel)?;
    mount.validate().map_err(map_error)?;
    super::registry::check_cancel(cancel)
}

fn map_error(error: FsError) -> ProviderError {
    match error.kind {
        io::ErrorKind::Interrupted => ProviderError::Cancelled,
        io::ErrorKind::Unsupported => ProviderError::Unsupported,
        _ => ProviderError::Failed(error.to_string()),
    }
}

#[cfg(test)]
#[path = "archive_tests.rs"]
mod tests;
