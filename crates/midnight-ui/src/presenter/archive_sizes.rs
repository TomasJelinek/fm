//! Archive size presentation keeps opaque identities and backend-owned snapshots.
use super::*;

const MAX_ENTRY_SCANS: usize = 64;

#[derive(Clone)]
struct FullScan {
    id: FolderSizeScanId,
    directory: u64,
    selection: Option<Arc<HashSet<EntryId>>>,
}

#[derive(Clone, Default)]
pub(super) struct State {
    snapshot: Arc<ArchiveSizeSnapshot>,
    invalidated: bool,
    full_scan: Option<FullScan>,
    entries: Vec<(EntryId, FolderSizeScanId)>,
    observer: Option<(SelectionSizeId, NavigationId, SelectionRevision)>,
    observer_version: u64,
    summary: Option<SelectedFolderSizes>,
}

impl State {
    fn finish(&mut self, scan: FolderSizeScanId, retired: &mut Vec<DeferredDrop>) -> bool {
        if self
            .full_scan
            .as_ref()
            .is_some_and(|value| value.id == scan)
        {
            retired.push(Box::new(self.full_scan.take()));
            return true;
        }
        if let Some(index) = self.entries.iter().position(|(_, id)| *id == scan) {
            self.entries.swap_remove(index);
            return true;
        }
        false
    }

    pub(super) fn summary(
        &self,
        navigation: NavigationId,
        revision: SelectionRevision,
    ) -> Option<SelectedFolderSizes> {
        self.observer
            .filter(|(_, nav, rev)| *nav == navigation && *rev == revision)
            .and(self.summary)
    }
}

impl Pane {
    fn archive_size_root(&self, pane: PaneId) -> Option<TreeRootSnapshot> {
        if self.loading || self.archive_request || self.tree_view().is_some() {
            return None;
        }
        let archive = self.archive.as_ref()?;
        if archive.sizes.invalidated {
            return None;
        }
        archive.location.mount_id()?;
        Some(TreeRootSnapshot {
            pane,
            navigation: self.successful_generation?,
            entries: self.entries.clone(),
            index: self.index.clone(),
        })
    }

    pub(crate) fn can_measure_folders(&self) -> bool {
        self.tree_view().is_some()
            || self.native_capabilities()
            || self.archive.as_ref().is_some_and(|archive| {
                archive.sizes.full_scan.is_some()
                    || (!self.loading
                        && !self.archive_request
                        && !archive.sizes.invalidated
                        && self.tree_view().is_none()
                        && archive.location.mount_id().is_some())
            })
    }

    pub(crate) fn folder_size(&self, entry: &Entry) -> Option<&FolderSize> {
        if self.tree_view().is_some() {
            return self.tree.sizes.size(TreeRowId(entry.id.0));
        }
        if let Some(node) = &entry.virtual_node {
            return self.archive.as_ref()?.sizes.snapshot.get(node.id);
        }
        self.folder_sizes.get(&entry.path)
    }

    /// Presentation fence only; never used to resolve a filesystem operation.
    pub(crate) fn folder_warning_revision(&self, entry: &Entry) -> String {
        if self.tree_view().is_some() {
            return format!("tree:{}", self.tree.sizes.diagnostic_revision());
        }
        if let Some(archive) = &self.archive {
            return format!(
                "archive:{}:{}",
                archive.sizes.snapshot.mount, archive.sizes.snapshot.version
            );
        }
        let Some(size) = self.folder_size(entry) else {
            return String::new();
        };
        format!(
            "native:{}:{}:{}",
            self.size_scan_generation, size.errors, size.finished
        )
    }

    pub(crate) fn folder_size_running(&self, entry: &Entry) -> bool {
        if self.tree_view().is_some() {
            return self.tree.sizes.running(TreeRowId(entry.id.0));
        }
        let Some(node) = &entry.virtual_node else {
            return self.entry_size_running(&entry.path);
        };
        self.archive.as_ref().is_some_and(|archive| {
            archive.location.mount_id() == Some(node.id.mount)
                && (archive.sizes.entries.iter().any(|(id, _)| *id == entry.id)
                    || archive.sizes.full_scan.as_ref().is_some_and(|scan| {
                        (node.parent == scan.directory
                            && scan
                                .selection
                                .as_ref()
                                .is_none_or(|ids| ids.contains(&entry.id)))
                            || archive
                                .sizes
                                .snapshot
                                .get(node.id)
                                .is_some_and(|s| !s.finished)
                    }))
        })
    }
}

impl Presenter {
    pub(super) fn archive_folder_sizes(&mut self, pane: PaneId, send: &dyn Fn(Command) -> bool) {
        let p = &mut self.panes[pane.index()];
        let Some(archive) = &p.archive else { return };
        if let Some(scan) = &archive.sizes.full_scan {
            if !send(Command::ArchiveSizes(ArchiveSizeCommand::Cancel {
                pane,
                scan: scan.id,
            })) {
                p.set_error(
                    "Folder size queue is busy. Try cancelling again.",
                    &mut self.retired,
                );
            }
            return;
        }
        let Some(root) = p.archive_size_root(pane) else {
            return;
        };
        if archive.selection_pending {
            return;
        }
        let Some(next) = p.size_scan_generation.checked_add(1) else {
            return;
        };
        let scan = FolderSizeScanId(next);
        let selection = (!archive.selected.is_empty()).then(|| archive.selected.clone());
        if send(Command::ArchiveSizes(ArchiveSizeCommand::Compute {
            root,
            scan,
            selection: selection.clone(),
        })) {
            p.size_scan_generation = next;
            if let Some(archive) = &mut p.archive {
                archive.sizes.entries.clear();
                archive.sizes.full_scan = Some(FullScan {
                    id: scan,
                    directory: archive.location.directory,
                    selection,
                });
            }
        } else {
            p.set_error("Folder size queue is busy. Try again.", &mut self.retired);
        }
    }

    pub(super) fn measure_archive_entry(
        &mut self,
        pane: PaneId,
        entry: EntryId,
        send: &dyn Fn(Command) -> bool,
    ) {
        let p = &mut self.panes[pane.index()];
        let Some(root) = p.archive_size_root(pane) else {
            return;
        };
        let Some(archive) = &p.archive else { return };
        if archive.sizes.full_scan.is_some()
            || archive.sizes.entries.len() >= MAX_ENTRY_SCANS
            || archive.sizes.entries.iter().any(|(id, _)| *id == entry)
            || !p
                .entry_by_id(entry)
                .is_some_and(|e| e.kind == EntryKind::Directory)
        {
            return;
        }
        let Some(next) = p.size_scan_generation.checked_add(1) else {
            return;
        };
        let scan = FolderSizeScanId(next);
        if send(Command::ArchiveSizes(ArchiveSizeCommand::Entry {
            root,
            scan,
            entry,
        })) {
            p.size_scan_generation = next;
            if let Some(archive) = &mut p.archive {
                archive.sizes.entries.push((entry, scan));
            }
        } else {
            p.set_error(
                "Folder size queue is busy. Try Space again.",
                &mut self.retired,
            );
        }
    }

    pub(super) fn flush_archive_size_observers(&mut self, send: &dyn Fn(Command) -> bool) {
        for pane in [PaneId::Left, PaneId::Right] {
            // The ACK outlives navigation away from the archive. Retry a full
            // command queue so the backend can keep publication bounded.
            if let Some((mount, version)) = self.archive_size_acks[pane.index()]
                && send(Command::ArchiveSizes(ArchiveSizeCommand::Acknowledge {
                    pane,
                    mount,
                    version,
                }))
            {
                self.archive_size_acks[pane.index()] = None;
            }
            let p = &mut self.panes[pane.index()];
            let Some(root) = p.archive_size_root(pane) else {
                continue;
            };
            let Some(archive) = &mut p.archive else {
                continue;
            };
            let revision = archive.selection_revision;
            let state = &mut archive.sizes;
            if archive.selection_pending
                || state
                    .observer
                    .is_some_and(|(_, nav, rev)| nav == root.navigation && rev == revision)
                || (archive.selected.is_empty() && state.observer.is_none())
            {
                continue;
            }
            let Some(next) = self.archive_size_observer_next[pane.index()].checked_add(1) else {
                continue;
            };
            let request = SelectionSizeId(next);
            let navigation = root.navigation;
            if send(Command::ArchiveSizes(ArchiveSizeCommand::Observe {
                root,
                request,
                revision,
                selected: archive.selected.clone(),
            })) {
                self.archive_size_observer_next[pane.index()] = next;
                state.observer = Some((request, navigation, revision));
                state.observer_version = 0;
                state.summary = None;
            }
        }
    }

    pub(super) fn apply_archive_sizes(&mut self, event: ArchiveSizeEvent) {
        match event {
            ArchiveSizeEvent::Invalidated { pane, mount } => {
                if let Some(archive) = &mut self.panes[pane.index()].archive
                    && archive.location.mount_id() == Some(mount)
                {
                    let empty = Arc::new(ArchiveSizeSnapshot::new(
                        mount,
                        archive.sizes.snapshot.version,
                        HashMap::new(),
                    ));
                    self.retired
                        .push(Box::new(mem::replace(&mut archive.sizes.snapshot, empty)));
                    archive.sizes.invalidated = true;
                    archive.sizes.summary = None;
                }
            }
            ArchiveSizeEvent::Snapshot { pane, sizes, .. } => {
                self.archive_size_acks[pane.index()] = Some((sizes.mount, sizes.version));
                let Some(archive) = &mut self.panes[pane.index()].archive else {
                    self.retired.push(Box::new(sizes));
                    return;
                };
                if archive.location.mount_id() == Some(sizes.mount)
                    && !archive.sizes.invalidated
                    && sizes.version > archive.sizes.snapshot.version
                {
                    self.retired
                        .push(Box::new(mem::replace(&mut archive.sizes.snapshot, sizes)));
                } else {
                    self.retired.push(Box::new(sizes));
                }
            }
            ArchiveSizeEvent::Finished { pane, scan, .. } => {
                if let Some(archive) = &mut self.panes[pane.index()].archive {
                    archive.sizes.finish(scan, &mut self.retired);
                }
            }
            ArchiveSizeEvent::Selected {
                pane,
                request,
                navigation,
                revision,
                version,
                summary,
            } => {
                let p = &mut self.panes[pane.index()];
                if p.tree_view().is_none()
                    && let Some(archive) = &mut p.archive
                    && !archive.sizes.invalidated
                    && p.successful_generation == Some(navigation)
                    && archive.selection_revision == revision
                    && !archive.selection_pending
                    && archive.sizes.observer == Some((request, navigation, revision))
                    && (archive.sizes.summary.is_none() || version > archive.sizes.observer_version)
                {
                    archive.sizes.observer_version = version;
                    archive.sizes.summary = Some(summary);
                }
            }
            ArchiveSizeEvent::Rejected {
                pane,
                scan: Some(scan),
                message,
            } => {
                let p = &mut self.panes[pane.index()];
                if p.archive
                    .as_mut()
                    .is_some_and(|archive| archive.sizes.finish(scan, &mut self.retired))
                {
                    p.set_error(message, &mut self.retired);
                }
            }
            // An observer rejection has no user-operation identity. An old
            // navigation's diagnostic must not replace the current panel error.
            ArchiveSizeEvent::Rejected { scan: None, .. } => {}
        }
    }
}
