//! Weak, exact-source lookup of the last immutable map queued to a pane.
use crate::Snapshots;
use midnight_core::{ArchiveMount, ArchiveSizeSnapshot, Entry, EntryKind, PaneId};
use std::sync::{Arc, Weak};

pub(crate) struct Published {
    source: Weak<dyn ArchiveMount>,
    sizes: Weak<ArchiveSizeSnapshot>,
}
impl Published {
    pub(super) fn new(source: &Arc<dyn ArchiveMount>, sizes: &Arc<ArchiveSizeSnapshot>) -> Self {
        Self {
            source: Arc::downgrade(source),
            sizes: Arc::downgrade(sizes),
        }
    }
    fn matches(&self, source: &Arc<dyn ArchiveMount>) -> bool {
        self.source.ptr_eq(&Arc::downgrade(source))
    }
    /// Borrow the caller's already retained mount; this helper acquires no lock
    /// and creates no transient native owner outside the guarded snapshot state.
    pub(crate) fn snapshot_for(
        &self,
        source: &Arc<dyn ArchiveMount>,
    ) -> Option<Arc<ArchiveSizeSnapshot>> {
        if !self.matches(source) {
            return None;
        }
        self.sizes
            .upgrade()
            .filter(|sizes| sizes.mount == source.id())
    }
    #[cfg(test)]
    pub(crate) fn testing(
        source: &Arc<dyn ArchiveMount>,
        sizes: &Arc<ArchiveSizeSnapshot>,
    ) -> Self {
        Self::new(source, sizes)
    }
}

pub(super) fn invalidate(snapshots: &Snapshots, pane: PaneId, source: &Arc<dyn ArchiveMount>) {
    let mut states = snapshots.lock().unwrap_or_else(|e| e.into_inner());
    let published = &mut states[pane.index()].archive_sort_sizes;
    if published
        .as_ref()
        .is_some_and(|value| value.matches(source))
    {
        *published = None;
    }
}

/// Caller validates the source before and after listing/sorting. Keeping this
/// lookup weak avoids retaining an idle archive worker or a retired large map.
pub(crate) fn snapshot(
    snapshots: &Snapshots,
    pane: PaneId,
    source: &Arc<dyn ArchiveMount>,
) -> Option<Arc<ArchiveSizeSnapshot>> {
    let states = snapshots.lock().unwrap_or_else(|e| e.into_inner());
    let published = states[pane.index()].archive_sort_sizes.as_ref()?;
    published.snapshot_for(source)
}

pub(crate) fn size(entry: &Entry, sizes: Option<&ArchiveSizeSnapshot>) -> Option<u64> {
    match entry.kind {
        EntryKind::Directory => {
            let node = entry.virtual_node.as_ref()?;
            let size = sizes?.get(node.id)?;
            (size.finished && size.errors == 0).then_some(size.bytes)
        }
        EntryKind::DirectoryLink => None,
        _ => entry.metadata.size,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use midnight_core::{ArchiveNode, FolderSize, VirtualEntryId};
    use std::collections::HashMap;

    #[test]
    fn unfinished_error_and_link_totals_remain_unknown_without_changing_raw_sizes() {
        let id = VirtualEntryId { mount: 3, entry: 2 };
        let mut entry = Entry::from_archive(ArchiveNode {
            id,
            parent: 1,
            raw_name: Arc::from(b"folder".as_slice()),
            display_name: "folder".into(),
            kind: EntryKind::Directory,
            size: Some(999),
            readable: false,
            modified: None,
        });
        for (finished, errors, expected) in [(false, 0, None), (true, 1, None), (true, 0, Some(12))]
        {
            let sizes = ArchiveSizeSnapshot::new(
                3,
                1,
                HashMap::from([(
                    id,
                    FolderSize {
                        bytes: 12,
                        finished,
                        errors,
                        ..FolderSize::default()
                    },
                )]),
            );
            assert_eq!(size(&entry, Some(&sizes)), expected);
            assert_eq!(entry.metadata.size, Some(999));
            assert_eq!(entry.virtual_node.as_ref().unwrap().size, Some(999));
            entry.kind = EntryKind::DirectoryLink;
            assert_eq!(size(&entry, Some(&sizes)), None);
            entry.kind = EntryKind::Directory;
        }
        assert_eq!(size(&entry, None), None);
    }
}
