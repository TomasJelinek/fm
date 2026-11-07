//! Independent Space-key scans retain raw paths through listing and sort generations.
use super::*;

pub(super) struct EntrySizeScope {
    pub(super) scan: FolderSizeScanId,
    navigation: NavigationId,
    location: PathBuf,
    path: PathBuf,
    entry: EntryId,
}

impl Presenter {
    pub(super) fn measure_entry(
        &mut self,
        pane: PaneId,
        entry: EntryId,
        send: &dyn Fn(Command) -> bool,
    ) {
        let p = &mut self.panes[pane.index()];
        if p.loading || p.size_scan.is_some() || p.archive.is_some() {
            return;
        }
        let Some(navigation) = p.successful_generation else {
            return;
        };
        let Some(target) = p
            .entry_by_id(entry)
            .filter(|entry| entry.kind == EntryKind::Directory)
        else {
            return;
        };
        let path = target.path.clone();
        if p.entry_size_scans.iter().any(|scope| scope.path == path) {
            return;
        }
        let Some(next) = p.size_scan_generation.checked_add(1) else {
            p.error = "Size request identities are exhausted.".into();
            return;
        };
        let scan = FolderSizeScanId(next);
        if send(Command::ComputeEntrySize {
            pane,
            navigation,
            scan,
            entry,
        }) {
            p.size_scan_generation = next;
            p.folder_sizes.insert(path.clone(), FolderSize::default());
            p.entry_size_scans.push(EntrySizeScope {
                scan,
                navigation,
                location: p.path.clone(),
                path,
                entry,
            });
        } else {
            p.error = "Folder size queue is busy. Try Space again.".into();
        }
    }

    pub(super) fn apply_entry_size_event(&mut self, event: Event) -> Option<Event> {
        let (pane, scan) = match &event {
            Event::FolderSizeUpdated { pane, scan, .. }
            | Event::FolderSizesFinished { pane, scan, .. } => (*pane, *scan),
            _ => return Some(event),
        };
        let p = &mut self.panes[pane.index()];
        let Some(index) = p
            .entry_size_scans
            .iter()
            .position(|scope| scope.scan == scan)
        else {
            return Some(event);
        };
        let scope = &p.entry_size_scans[index];
        match event {
            Event::FolderSizeUpdated {
                navigation,
                entry,
                size,
                ..
            } if navigation == scope.navigation
                && entry == scope.entry
                && p.display_path() == scope.location
                && p.size_scan.is_none() =>
            {
                // A newer full scan's final result must not regress to this partial scan.
                let keep_final = p
                    .folder_sizes
                    .get(&scope.path)
                    .is_some_and(|previous| previous.finished && !size.finished);
                if !keep_final {
                    p.folder_sizes.insert(scope.path.clone(), size);
                }
            }
            Event::FolderSizesFinished { navigation, .. } if navigation == scope.navigation => {
                self.retired
                    .push(Box::new(p.entry_size_scans.remove(index)));
            }
            _ => {}
        }
        None
    }
}

impl Pane {
    pub(crate) fn entry_size_running(&self, path: &Path) -> bool {
        (self.size_scan.is_some()
            && self
                .size_scan_scope
                .as_ref()
                .is_none_or(|scope| scope.contains(path)))
            || self.entry_size_scans.iter().any(|scope| scope.path == path)
    }
}
