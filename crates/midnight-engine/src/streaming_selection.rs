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
    index: DirectoryIndex,
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
        for entry in entries {
            let row = current.rows;
            current.rows += 1;
            current.index.by_id.insert(entry.id, row);
            current.index.by_path.insert(entry.path.clone(), entry.id);
            if row.is_multiple_of(64) {
                current.index.directory_bits.push(0);
            }
            if matches!(entry.kind, EntryKind::Directory | EntryKind::DirectoryLink) {
                current.index.directory_bits[row / 64] |= 1 << (row % 64);
            }
        }
    }

    /// Clone only on the selection blocking worker, never on the UI/runtime thread.
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
        Ok(Arc::new(current.index.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_order_and_raw_identity_are_retained_across_batches_and_stale_writes() {
        let streams = StreamingSelections::default();
        let first = Entry::new(EntryId(80), "root/z".into(), EntryKind::DirectoryLink);
        let second = Entry::new(EntryId(3), "root/a".into(), EntryKind::File);
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
            reset: false,
            preserve: None,
            restore: None,
            action: SelectionAction::Rebase,
            toggle_directory: false,
        };
        let index = streams.snapshot(&request).unwrap();
        assert_eq!(index.by_id[&EntryId(80)], 0);
        assert_eq!(index.by_id[&EntryId(3)], 1);
        assert!(index.is_directory(EntryId(80)));
        streams.append(PaneId::Left, NavigationId(3), Path::new("next"), &[]);
        assert!(streams.snapshot(&request).is_err());
        assert_eq!(
            index.by_path.len(),
            2,
            "a captured range view remains immutable"
        );
    }
}
