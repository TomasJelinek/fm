//! Bind cached size data to the captured listing, then freeze it on a backend worker.
use crate::{SharedSizes, Snapshots, file_list_records::check};
use midnight_core::*;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};

#[derive(Default)]
pub(crate) struct Captured {
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    native: Option<SharedSizes>,
    archive: Option<Arc<ArchiveSizeSnapshot>>,
    frozen: Option<Arc<Frozen>>,
}

#[derive(Default)]
pub(crate) struct Frozen {
    native: HashMap<PathBuf, u64>,
    archive: Option<Arc<ArchiveSizeSnapshot>>,
}

impl Captured {
    pub(crate) fn bind(capture: &FileListCapture, snapshots: &Snapshots) -> Self {
        let (entries, native_allowed, archive_allowed, tree_scope) = match &capture.source {
            FileListSource::Native { entries, .. } => (entries, true, false, None),
            FileListSource::Archive { entries, .. } => (entries, false, true, None),
            FileListSource::Tree { view, .. } => (
                &view.root.entries,
                true,
                true,
                Some((view.root.pane, view.root.navigation)),
            ),
        };
        let mut native = None;
        let mut archive_scope = None;
        {
            let states = snapshots.lock().unwrap_or_else(|error| error.into_inner());
            for pane in [PaneId::Left, PaneId::Right] {
                if tree_scope.is_some_and(|(root_pane, _)| root_pane != pane) {
                    continue;
                }
                let state = &states[pane.index()];
                if native_allowed && state.has_native_view() {
                    native = [&state.visible, &state.committed]
                        .into_iter()
                        .flatten()
                        .find(|snapshot| {
                            Arc::ptr_eq(&snapshot.entries, entries)
                                && tree_scope.is_none_or(|(_, nav)| nav == snapshot.navigation)
                        })
                        .map(|snapshot| snapshot.sizes.clone());
                    if native.is_some() {
                        break;
                    }
                }
                if archive_allowed
                    && let Some(ready) = &state.tree_root
                    && Arc::ptr_eq(&ready.snapshot().entries, entries)
                    && tree_scope.is_none_or(|(_, nav)| nav == ready.snapshot().navigation)
                    && let Some(source) =
                        ready.archive().and_then(|location| location.mount.as_ref())
                {
                    archive_scope = Some((pane, source.clone()));
                    break;
                }
            }
        }
        // The published lookup verifies the exact mount capability and refuses
        // an invalidated/retired map. Do not nest its snapshots lock above.
        let archive = archive_scope.and_then(|(pane, source)| {
            crate::archive_sizes::published::snapshot(snapshots, pane, &source)
        });
        Self {
            state: Mutex::new(State {
                native,
                archive,
                frozen: None,
            }),
        }
    }

    pub(crate) fn freeze(
        &self,
        entries: &[&Entry],
        cancel: &Cancellation,
    ) -> Result<Arc<Frozen>, FsError> {
        check(cancel)?;
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        check(cancel)?;
        if let Some(frozen) = &state.frozen {
            return Ok(frozen.clone());
        }
        let mut native = HashMap::new();
        if let Some(cache) = &state.native {
            let cache = cache.lock().unwrap_or_else(|error| error.into_inner());
            for entry in entries {
                check(cancel)?;
                if matches!(entry.kind, EntryKind::Directory | EntryKind::DirectoryLink)
                    && let Some(path) = entry.native_path()
                    && let Some(size) = cache.get(path)
                    && size.finished
                    && size.errors == 0
                {
                    native.insert(path.to_owned(), size.bytes);
                }
            }
        }
        check(cancel)?;
        let frozen = Arc::new(Frozen {
            native,
            archive: state.archive.clone(),
        });
        state.frozen = Some(frozen.clone());
        // Only selected completed native values remain retained. All copying
        // and final cache destruction happen on this backend worker.
        state.native = None;
        Ok(frozen)
    }
}

impl Frozen {
    pub(crate) fn directory_size(&self, entry: &Entry) -> Option<u64> {
        if !matches!(entry.kind, EntryKind::Directory | EntryKind::DirectoryLink) {
            return None;
        }
        if entry.virtual_node.is_some() {
            return crate::archive_sizes::published::size(entry, self.archive.as_deref());
        }
        self.native.get(entry.native_path()?).copied()
    }
}

#[cfg(test)]
#[path = "file_list_sizes_tests.rs"]
mod tests;
