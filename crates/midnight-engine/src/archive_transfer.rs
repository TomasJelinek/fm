//! Archive members enter the ordinary verified-copy pipeline through provider IDs.
use super::*;
use std::{
    collections::{HashSet, VecDeque},
    path::PathBuf,
};
#[cfg(windows)]
#[path = "archive_transfer_name/windows.rs"]
mod names;
#[cfg(target_os = "linux")]
#[path = "archive_transfer_name/linux.rs"]
mod names;

pub(super) fn resolve(
    request: &ArchiveCopyRequest,
    cancel: &Cancellation,
) -> Result<VecDeque<Box<dyn TransferSource>>, FsError> {
    let mount = request
        .location
        .mount
        .clone()
        .ok_or_else(|| failure("Archive is unavailable."))?;
    check(&*mount, cancel)?;
    let wanted: HashSet<_> = if request.selected.is_empty() {
        request
            .fallback
            .filter(|id| request.view.by_id.contains_key(id))
            .into_iter()
            .collect()
    } else {
        request
            .selected
            .iter()
            .filter(|id| request.view.by_id.contains_key(id))
            .copied()
            .collect()
    };
    if wanted.is_empty() || wanted.len() > 4096 {
        return Err(failure(
            "Archive copy requires between 1 and 4096 captured entries.",
        ));
    }
    let actual = mount.list(request.location.directory, cancel)?;
    let mut actual_map = std::collections::HashMap::new();
    for node in actual {
        if actual_map.insert(node.id.entry, node).is_some() {
            return Err(failure("Duplicate archive member identity."));
        }
    }
    let actual = actual_map;
    let visited = Arc::new(Mutex::new(Traversal::default()));
    let mut sources = VecDeque::new();
    for entry in request
        .entries
        .iter()
        .filter(|entry| wanted.contains(&entry.id))
    {
        let expected = entry
            .virtual_node
            .as_ref()
            .ok_or_else(|| failure("Captured entry is not an archive member."))?;
        let node = actual
            .get(&expected.id.entry)
            .ok_or_else(|| failure("Captured archive member is unavailable."))?;
        if node.id != expected.id
            || node.id.mount != mount.id()
            || node.parent != request.location.directory
            || node.raw_name != expected.raw_name
            || node.kind != expected.kind
            || node.size != expected.size
            || node.readable != expected.readable
        {
            return Err(failure("Captured archive member identity changed."));
        }
        sources.push_back(Box::new(Source::new(
            mount.clone(),
            node.clone(),
            0,
            visited.clone(),
        )?) as Box<dyn TransferSource>);
    }
    if sources.len() != wanted.len() {
        return Err(failure("Captured archive selection is stale."));
    }
    Ok(sources)
}

#[derive(Default)]
pub(crate) struct Traversal {
    seen: HashSet<VirtualEntryId>,
    listed: usize,
    names: usize,
}

struct Source {
    mount: Arc<dyn ArchiveMount>,
    node: ArchiveNode,
    entry: TransferEntry,
    offset: u64,
    children: Option<VecDeque<ArchiveNode>>,
    depth: usize,
    visited: Arc<Mutex<Traversal>>,
}
impl Source {
    fn new(
        mount: Arc<dyn ArchiveMount>,
        node: ArchiveNode,
        depth: usize,
        visited: Arc<Mutex<Traversal>>,
    ) -> Result<Self, FsError> {
        if depth > 256
            || node.id.mount != mount.id()
            || node.id.entry <= 1
            || !matches!(node.kind, EntryKind::File | EntryKind::Directory)
            || (node.kind == EntryKind::File && (!node.readable || node.size.is_none()))
        {
            return Err(failure(
                "Archive copy refuses links, unsafe members and excessive nesting.",
            ));
        }
        // Raw provider bytes are authoritative. Undecodable names fail explicitly rather
        // than using a lossy display label as a destination identity.
        let raw = node.raw_name.as_ref();
        if raw.is_empty()
            || raw == b"."
            || raw == b".."
            || raw.iter().any(|byte| b"/\\:\0".contains(byte))
        {
            return Err(failure(
                "Archive member is not a safe destination basename.",
            ));
        }
        let name = names::native_name(raw)?;
        {
            let mut ids = visited.lock().unwrap_or_else(|e| e.into_inner());
            if ids.seen.len() >= 100_000 || !ids.seen.insert(node.id) {
                return Err(failure(
                    "Archive traversal is cyclic or exceeds its entry budget.",
                ));
            }
        }
        let entry = TransferEntry {
            // This is a presentation basename only. Source reads always use mount/entry IDs.
            item: MutationItem {
                path: PathBuf::from(name),
                kind: node.kind,
                metadata: EntryMetadata {
                    size: node.size,
                    ..EntryMetadata::default()
                },
            },
            // Never supplied to native ancestry checks: native_identity returns None.
            identity: TransferIdentity {
                device: 0,
                object: [0; 16],
            },
        };
        Ok(Self {
            mount,
            node,
            entry,
            offset: 0,
            children: None,
            depth,
            visited,
        })
    }
}
impl TransferSource for Source {
    fn virtual_identity(&self) -> Option<VirtualEntryId> {
        Some(self.node.id)
    }
    fn entry(&self) -> &TransferEntry {
        &self.entry
    }
    fn native_identity(&self) -> Option<TransferIdentity> {
        None
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        check(&*self.mount, cancel)
    }
    fn read_chunk(&mut self, buffer: &mut [u8], cancel: &Cancellation) -> Result<usize, FsError> {
        self.revalidate(cancel)?;
        if self.node.kind != EntryKind::File {
            return Err(failure("Archive directory has no content."));
        }
        if buffer.is_empty() {
            return Ok(0);
        }
        let length = buffer.len().min(256 * 1024) as u32;
        let bytes = self
            .mount
            .read(self.node.id.entry, self.offset, length, cancel)?;
        if bytes.len() > length as usize {
            return Err(failure("Archive provider exceeded the read budget."));
        }
        self.offset = self
            .offset
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| failure("Archive source offset overflow."))?;
        if self.node.size.is_some_and(|size| self.offset > size) {
            return Err(failure("Archive member grew beyond its declared length."));
        }
        buffer[..bytes.len()].copy_from_slice(&bytes);
        Ok(bytes.len())
    }
    fn supports_read_retry(&self) -> bool {
        self.node.kind == EntryKind::File
    }
    fn retry_read_from(&mut self, offset: u64, cancel: &Cancellation) -> Result<(), FsError> {
        self.revalidate(cancel)?;
        if !self.supports_read_retry() || self.node.size.is_none_or(|size| offset > size) {
            return Err(failure("Invalid archive member retry offset."));
        }
        // Archive reads are explicitly addressed by the retained mount/member IDs.
        self.offset = offset;
        Ok(())
    }
    fn next_child(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Option<Box<dyn TransferSource>>, FsError> {
        self.revalidate(cancel)?;
        if self.node.kind != EntryKind::Directory {
            return Err(failure("Archive file has no children."));
        }
        if self.children.is_none() {
            let children = self.mount.list(self.node.id.entry, cancel)?;
            {
                let mut budget = self.visited.lock().unwrap_or_else(|e| e.into_inner());
                budget.listed = budget.listed.saturating_add(children.len());
                for child in &children {
                    budget.names = budget
                        .names
                        .saturating_add(child.raw_name.len())
                        .saturating_add(child.display_name.len());
                }
                if budget.listed > 100_000 || budget.names > 8 * 1024 * 1024 {
                    return Err(failure(
                        "Archive traversal exceeds its aggregate listing budget.",
                    ));
                }
            }
            self.children = Some(children.into());
        }
        let Some(node) = self.children.as_mut().and_then(VecDeque::pop_front) else {
            return Ok(None);
        };
        if node.parent != self.node.id.entry {
            return Err(failure("Archive child belongs to a different directory."));
        }
        Source::new(
            self.mount.clone(),
            node,
            self.depth + 1,
            self.visited.clone(),
        )
        .map(|source| Some(Box::new(source) as Box<dyn TransferSource>))
    }
}

fn check(mount: &dyn ArchiveMount, cancel: &Cancellation) -> Result<(), FsError> {
    if cancel.is_cancelled() {
        return Err(error(
            Path::new(""),
            io::ErrorKind::Interrupted,
            "Archive copy cancelled.",
        ));
    }
    mount.validate()
}
fn failure(message: &str) -> FsError {
    error(Path::new(""), io::ErrorKind::InvalidData, message)
}

/// Shared aggregate traversal accounting across the complete captured batch.
pub(crate) fn traversal() -> Arc<Mutex<Traversal>> {
    Arc::default()
}
pub(crate) fn captured_root(
    target: &crate::providers::ArchiveTarget,
    traversal: Arc<Mutex<Traversal>>,
    cancel: &Cancellation,
) -> Result<Box<dyn TransferSource>, FsError> {
    let mount = target
        .location
        .mount
        .clone()
        .ok_or_else(|| failure("Archive is unavailable."))?;
    check(&*mount, cancel)?;
    let expected = target
        .original
        .as_ref()
        .or(target.member.as_ref())
        .ok_or_else(|| failure("Archive root has no original member record."))?;
    let actual = mount.list(expected.parent, cancel)?;
    {
        let mut budget = traversal.lock().unwrap_or_else(|e| e.into_inner());
        budget.listed = budget.listed.saturating_add(actual.len());
        for node in &actual {
            budget.names = budget
                .names
                .saturating_add(node.raw_name.len())
                .saturating_add(node.display_name.len());
        }
        if budget.listed > 100_000 || budget.names > 8 * 1024 * 1024 {
            return Err(failure(
                "Archive admission exceeds its aggregate listing budget.",
            ));
        }
    }
    let mut ids = HashSet::new();
    let mut found = None;
    for node in actual {
        if node.id.mount != mount.id() || node.parent != expected.parent || !ids.insert(node.id) {
            return Err(failure("Invalid archive parent listing."));
        }
        if node.id == expected.id {
            found = Some(node);
        }
    }
    let node = found.ok_or_else(|| failure("Captured archive member disappeared."))?;
    if node.raw_name != expected.raw_name
        || node.kind != expected.kind
        || node.size != expected.size
        || node.readable != expected.readable
        || node.modified != expected.modified
    {
        return Err(failure("Captured archive member changed."));
    }
    Ok(Box::new(Source::new(mount, node, 0, traversal)?))
}
