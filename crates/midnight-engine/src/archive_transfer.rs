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

/// Rows of a selection or listing handled between two checks of Stop and the source.
const CHECK_STRIDE: usize = 4096;

/// The roots of a captured selection of any size, in the order the panel captured
/// them. Only the selected members are kept from the folder's listing, moved rather
/// than copied, and Stop and the source are checked every few thousand rows. The copy
/// then takes the roots one at a time; each costs its retained source until copied.
pub(super) fn resolve(
    request: &ArchiveCopyRequest,
    cancel: &Cancellation,
) -> Result<VecDeque<Box<dyn TransferSource>>, FsError> {
    let mount = MountReference::new(
        request
            .location
            .mount
            .clone()
            .ok_or_else(|| failure("Archive is unavailable."))?,
    );
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
    if wanted.is_empty() {
        return Err(failure("Archive copy requires a captured entry."));
    }
    let selected = || {
        request
            .entries
            .iter()
            .filter(|entry| wanted.contains(&entry.id))
    };
    let mut members = std::collections::HashMap::with_capacity(wanted.len());
    for (index, entry) in selected().enumerate() {
        if index.is_multiple_of(CHECK_STRIDE) {
            check(&*mount, cancel)?;
        }
        let expected = entry
            .virtual_node
            .as_ref()
            .ok_or_else(|| failure("Captured entry is not an archive member."))?;
        members.insert(expected.id.entry, None);
    }
    let actual = mount.list(request.location.directory, cancel);
    crate::copy_nested_retirement::observe_archive(&*mount);
    let actual = actual?;
    // Every listed identity is unique; only the selected members are kept.
    let mut listed = HashSet::with_capacity(actual.len());
    for (index, node) in actual.into_iter().enumerate() {
        if index.is_multiple_of(CHECK_STRIDE) {
            check(&*mount, cancel)?;
        }
        if !listed.insert(node.id.entry) {
            return Err(failure("Duplicate archive member identity."));
        }
        if let Some(member) = members.get_mut(&node.id.entry) {
            *member = Some(node);
        }
    }
    drop(listed);
    let visited = Arc::new(Mutex::new(Traversal::default()));
    let mut sources = VecDeque::with_capacity(wanted.len());
    for (index, entry) in selected().enumerate() {
        if index.is_multiple_of(CHECK_STRIDE) {
            check(&*mount, cancel)?;
        }
        let expected = entry
            .virtual_node
            .as_ref()
            .ok_or_else(|| failure("Captured entry is not an archive member."))?;
        let node = members
            .get_mut(&expected.id.entry)
            .and_then(Option::take)
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
        sources.push_back(
            Box::new(Source::new(mount.clone(), node, 0, visited.clone())?)
                as Box<dyn TransferSource>,
        );
    }
    if sources.len() != wanted.len() {
        return Err(failure("Captured archive selection is stale."));
    }
    Ok(sources)
}

/// The members a captured batch has entered, so that a member listed twice (a cycle
/// or a duplicate identity) fails instead of copying again. Nothing limits how many
/// members a batch copies or how long their names are; the set costs a few dozen
/// bytes per member, beside the listing of each folder on the current path.
#[derive(Default)]
pub(crate) struct Traversal {
    seen: HashSet<VirtualEntryId>,
}

/// One member being copied. A copy holds a source for each selected member until it
/// is copied, so a source keeps only what reading the member needs: its names went
/// into `entry` when it was made, and only a folder that is entered lists children.
struct Source {
    mount: MountReference,
    member: Member,
    entry: TransferEntry,
    offset: u64,
    children: Option<VecDeque<ArchiveNode>>,
    depth: usize,
    visited: Arc<Mutex<Traversal>>,
}

/// The identity, kind and declared length of a source's member.
#[derive(Clone, Copy)]
struct Member {
    id: VirtualEntryId,
    kind: EntryKind,
    size: Option<u64>,
}

/// Every Copy-owned clone, including a failed construction or recursive source,
/// consumes the archive capability instead of assuming Arc Drop is native close.
struct MountReference(Option<Arc<dyn ArchiveMount>>);
impl MountReference {
    fn new(mount: Arc<dyn ArchiveMount>) -> Self {
        Self(Some(mount))
    }
    fn release(&mut self) -> Result<(), FsError> {
        match self.0.take() {
            Some(mount) => crate::copy_nested_retirement::release_archive(mount),
            None => Ok(()),
        }
    }
}
impl Clone for MountReference {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
impl std::ops::Deref for MountReference {
    type Target = dyn ArchiveMount;
    fn deref(&self) -> &Self::Target {
        &**self.0.as_ref().expect("archive reference remains owned")
    }
}
impl Drop for MountReference {
    fn drop(&mut self) {
        // TransferSource currently has no consuming close method. Its actual
        // checked destruction is therefore the fallback acknowledgement point;
        // a failed release latches Copy's context and never panics from Drop.
        let _ = self.release();
    }
}

#[cfg(test)]
pub(crate) fn retirement_source_for_test(mount: Arc<dyn ArchiveMount>) -> Box<dyn TransferSource> {
    let node = ArchiveNode {
        id: VirtualEntryId {
            mount: mount.id(),
            entry: 2,
        },
        parent: 1,
        raw_name: Arc::from(&b"member"[..]),
        display_name: "member".into(),
        kind: EntryKind::File,
        size: Some(0),
        modified: None,
        readable: true,
        access: Default::default(),
    };
    Box::new(
        Source::new(
            MountReference::new(mount),
            node,
            0,
            Arc::new(Mutex::new(Traversal::default())),
        )
        .unwrap(),
    )
}
impl Source {
    fn new(
        mount: MountReference,
        node: ArchiveNode,
        depth: usize,
        visited: Arc<Mutex<Traversal>>,
    ) -> Result<Self, FsError> {
        if depth > 256 || node.id.mount != mount.id() || node.id.entry <= 1 {
            return Err(failure(
                "Archive copy refuses foreign members and excessive nesting.",
            ));
        }
        // A member that cannot be read says why (its provider's reason).
        if !matches!(node.kind, EntryKind::File | EntryKind::Directory)
            || (node.kind == EntryKind::File && !node.readable)
        {
            return Err(error(
                Path::new(""),
                io::ErrorKind::Unsupported,
                &node.unreadable_reason(),
            ));
        }
        if node.kind == EntryKind::File && node.size.is_none() {
            return Err(failure("Archive member has no declared length."));
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
            if !ids.seen.insert(node.id) {
                return Err(failure(
                    "Archive traversal is cyclic: a member was listed twice.",
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
            member: Member {
                id: node.id,
                kind: node.kind,
                size: node.size,
            },
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
        Some(self.member.id)
    }
    fn entry(&self) -> &TransferEntry {
        &self.entry
    }
    fn native_identity(&self) -> Option<TransferIdentity> {
        None
    }
    /// The archive file is validated by each exchange with its worker (before it starts)
    /// and around each read of the file's bytes (`midnight_filesystem_host::Mount`), so a
    /// member's copy, which reads only through those exchanges, checks Stop here and
    /// leaves the file to them: reopening it for every chunk and step cost up to six
    /// opens of the archive per copied member (WIN-8). A Retry still validates it.
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        stopped(&*self.mount, cancel)
    }
    fn read_chunk(&mut self, buffer: &mut [u8], cancel: &Cancellation) -> Result<usize, FsError> {
        self.revalidate(cancel)?;
        if self.member.kind != EntryKind::File {
            return Err(failure("Archive directory has no content."));
        }
        if buffer.is_empty() {
            return Ok(0);
        }
        let length = buffer.len().min(256 * 1024) as u32;
        // Cancelling a copy must not terminate the shared archive worker.
        let bytes = crate::archive_shared::read(
            &*self.mount,
            self.member.id.entry,
            self.offset,
            length,
            cancel,
        );
        crate::copy_nested_retirement::observe_archive(&*self.mount);
        let bytes = bytes?;
        if bytes.len() > length as usize {
            return Err(failure("Archive provider exceeded the read budget."));
        }
        self.offset = self
            .offset
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| failure("Archive source offset overflow."))?;
        if self.member.size.is_some_and(|size| self.offset > size) {
            return Err(failure("Archive member grew beyond its declared length."));
        }
        buffer[..bytes.len()].copy_from_slice(&bytes);
        Ok(bytes.len())
    }
    fn supports_read_retry(&self) -> bool {
        self.member.kind == EntryKind::File
    }
    fn retry_read_from(&mut self, offset: u64, cancel: &Cancellation) -> Result<(), FsError> {
        check(&*self.mount, cancel)?;
        if !self.supports_read_retry() || self.member.size.is_none_or(|size| offset > size) {
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
        if self.member.kind != EntryKind::Directory {
            return Err(failure("Archive file has no children."));
        }
        if self.children.is_none() {
            // A folder is listed once, from the validated archive file. The host pages
            // the listing and ends it between pages when the copy is cancelled, however
            // many members the folder holds.
            check(&*self.mount, cancel)?;
            let children = self.mount.list(self.member.id.entry, cancel);
            crate::copy_nested_retirement::observe_archive(&*self.mount);
            self.children = Some(children?.into());
        }
        let Some(node) = self
            .children
            .as_mut()
            .and_then(|children| children.pop_front())
        else {
            return Ok(None);
        };
        if node.parent != self.member.id.entry {
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

/// Stop, without validating the archive file.
fn stopped(mount: &dyn ArchiveMount, cancel: &Cancellation) -> Result<(), FsError> {
    crate::copy_nested_retirement::observe_archive(mount);
    if cancel.is_cancelled() {
        return Err(error(
            Path::new(""),
            io::ErrorKind::Interrupted,
            "Archive copy cancelled.",
        ));
    }
    Ok(())
}
/// Stop, then the archive file itself (`ArchiveMount::validate`).
fn check(mount: &dyn ArchiveMount, cancel: &Cancellation) -> Result<(), FsError> {
    stopped(mount, cancel)?;
    let result = mount.validate();
    crate::copy_nested_retirement::observe_archive(mount);
    result
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
    let mount = MountReference::new(
        target
            .location
            .mount
            .clone()
            .ok_or_else(|| failure("Archive is unavailable."))?,
    );
    check(&*mount, cancel)?;
    let expected = target
        .original
        .as_ref()
        .or(target.member.as_ref())
        .ok_or_else(|| failure("Archive root has no original member record."))?;
    let actual = mount.list(expected.parent, cancel);
    crate::copy_nested_retirement::observe_archive(&*mount);
    let actual = actual?;
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

#[cfg(test)]
#[path = "archive_transfer_taint_tests.rs"]
mod taint_tests;

#[cfg(test)]
#[path = "archive_transfer_cancel_tests.rs"]
mod cancel_tests;

#[cfg(test)]
#[path = "archive_transfer_size_tests.rs"]
mod size_tests;
