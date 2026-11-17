//! Backend-only append indexes retain the displayed streaming order for Shift ranges.
use midnight_core::*;
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

#[derive(Default)]
struct StreamIndex {
    navigation: NavigationId,
    location: PathBuf,
    /// Shared with outstanding range snapshots; the next append copies it once.
    index: Arc<DirectoryIndex>,
    rows: usize,
}

#[derive(Clone, Default)]
pub(crate) struct StreamingSelections(Arc<[Mutex<StreamIndex>; 2]>);

impl StreamingSelections {
    /// Called by the listing worker before publishing each visible batch.
    pub(crate) fn append(
        &self,
        pane: PaneId,
        navigation: NavigationId,
        location: &Path,
        entries: &[Entry],
    ) {
        let mut current = self.0[pane.index()]
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if navigation < current.navigation {
            return;
        }
        if navigation != current.navigation || current.location != location {
            *current = StreamIndex {
                navigation,
                location: location.to_owned(),
                ..Default::default()
            };
        }
        if entries.is_empty() {
            return;
        }
        let StreamIndex { index, rows, .. } = &mut *current;
        let index = Arc::make_mut(index);
        for entry in entries {
            let row = *rows;
            *rows += 1;
            index.by_id.insert(entry.id, row);
            index.by_path.insert(entry.path.clone(), entry.id);
            if row.is_multiple_of(64) {
                index.directory_bits.push(0);
            }
            if matches!(entry.kind, EntryKind::Directory | EntryKind::DirectoryLink) {
                index.directory_bits[row / 64] |= 1 << (row % 64);
            }
            index.file_sizes.push(if entry.kind == EntryKind::File {
                entry.metadata.size
            } else {
                None
            });
        }
    }

    /// Share the current immutable view without copying it under the pane lock.
    /// Repeated ranges between two batches reuse one view; the next append pays
    /// for at most one copy on the listing worker.
    pub(crate) fn snapshot(
        &self,
        request: &SelectionRequest,
    ) -> Result<Arc<DirectoryIndex>, &'static str> {
        let current = self.0[request.pane.index()]
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if current.navigation != request.navigation || current.location != request.location {
            return Err("The streamed range belongs to an obsolete folder listing");
        }
        Ok(current.index.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_order_and_raw_identity_are_retained_across_batches_and_stale_writes() {
        let streams = StreamingSelections::default();
        let mut first = Entry::new(EntryId(80), "root/z".into(), EntryKind::DirectoryLink);
        first.metadata.size = Some(900);
        let mut second = Entry::new(EntryId(3), "root/a".into(), EntryKind::File);
        second.metadata.size = Some(37);
        streams.append(
            PaneId::Left,
            NavigationId(2),
            Path::new("root"),
            std::slice::from_ref(&first),
        );
        streams.append(PaneId::Left, NavigationId(2), Path::new("root"), &[second]);
        streams.append(PaneId::Left, NavigationId(1), Path::new("old"), &[first]);
        let request = SelectionRequest {
            pane: PaneId::Left,
            navigation: NavigationId(2),
            revision: SelectionRevision(1),
            context: NavigationId(2),
            location: "root".into(),
            view: None,
            seed: Arc::default(),
            seed_revision: SelectionRevision(0),
            authoritative: None,
            reset: false,
            preserve: None,
            restore: None,
            action: SelectionAction::Rebase,
            toggle_directory: false,
        };
        let index = streams.snapshot(&request).unwrap();
        assert!(
            Arc::ptr_eq(&index, &streams.snapshot(&request).unwrap()),
            "ranges between two batches share one view instead of copying it"
        );
        assert_eq!(index.by_id[&EntryId(80)], 0);
        assert_eq!(index.by_id[&EntryId(3)], 1);
        assert!(index.is_directory(EntryId(80)));
        assert_eq!(index.file_sizes, [None, Some(37)]);
        let file = SelectionSnapshot::from_bits(index.clone(), vec![0b10]).unwrap();
        assert_eq!(file.file_bytes(), Some(37));
        let with_link = SelectionSnapshot::from_bits(index.clone(), vec![0b11]).unwrap();
        assert_eq!(
            with_link.file_bytes(),
            Some(37),
            "folder-link metadata is excluded from the separate regular-file total"
        );
        assert_eq!((with_link.file_count(), with_link.folder_count()), (1, 1));
        let mut file_link = Entry::new(EntryId(9), "root/file-link".into(), EntryKind::Symlink);
        file_link.metadata.size = Some(900);
        streams.append(
            PaneId::Left,
            NavigationId(2),
            Path::new("root"),
            &[file_link],
        );
        let extended = streams.snapshot(&request).unwrap();
        let with_file_link = SelectionSnapshot::from_bits(extended, vec![0b110]).unwrap();
        assert_eq!(
            (with_file_link.file_count(), with_file_link.folder_count()),
            (2, 0)
        );
        assert_eq!(
            with_file_link.file_bytes(),
            None,
            "a file symlink must not contribute its native link metadata as regular-file bytes"
        );
        streams.append(PaneId::Left, NavigationId(3), Path::new("next"), &[]);
        assert!(streams.snapshot(&request).is_err());
        assert_eq!(
            index.by_path.len(),
            2,
            "a captured range view remains immutable"
        );
    }
}
