//! Additive virtual pane state. Native paths remain only the captured return location.
use super::*;

#[derive(Clone)]
pub(super) struct ArchivePane {
    pub(super) location: ArchiveLocation,
    pub(super) origin: PathBuf,
    pub(super) history: Vec<ArchiveLocation>,
    pub(super) history_index: usize,
    requested_history: Option<usize>,
    pub(super) selected: Arc<HashSet<EntryId>>,
    totals: ArchiveSelectionTotals,
    selection_pending: bool,
}
impl ArchivePane {
    pub(super) fn unavailable(address: ArchiveAddress, origin: PathBuf) -> Self {
        let location = ArchiveLocation::unavailable(address);
        Self {
            history: vec![location.clone()],
            location,
            origin,
            history_index: 0,
            requested_history: None,
            selected: Arc::default(),
            totals: ArchiveSelectionTotals::default(),
            selection_pending: false,
        }
    }
}

impl Presenter {
    pub(crate) fn can_browse_archive(&self, entry: &Entry) -> bool {
        entry.virtual_node.is_none()
            && entry.kind == EntryKind::File
            && entry.archive_association.is_some_and(|association| {
                self.archive_formats.contains(&association.format)
                    && self
                        .preferences
                        .values
                        .archive_association_enabled(association.extension)
            })
    }
    pub(super) fn archive_navigate(
        &mut self,
        pane: PaneId,
        action: ArchiveAction,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let p = &mut self.panes[pane.index()];
        let Some(navigation) = p.generation.checked_add(1).map(NavigationId) else {
            return false;
        };
        let same = matches!(&action, ArchiveAction::List(location)
            if p.archive.as_ref().is_some_and(|a| a.location.address == location.address));
        let unavailable = match &action {
            ArchiveAction::List(location) if location.mount.is_none() => Some(location.clone()),
            _ => None,
        };
        if !send(Command::ArchiveNavigate {
            pane,
            navigation,
            action,
            sort: p.sort,
        }) {
            p.error = "Archive navigation queue is busy".into();
            return false;
        }
        p.generation = navigation.0;
        p.archive_request = true;
        p.loading = true;
        p.error.clear();
        if let Some(a) = &mut p.archive {
            a.selection_pending = false;
            a.requested_history = None;
            if let Some(location) = unavailable {
                a.location = location;
            }
        }
        if let Some(scan) = p.size_scan.take() {
            send(Command::CancelFolderSizes { pane, scan });
        }
        self.search_navigation_accepted(pane, !same, send);
        drag_drop::invalidate(&self.drag_drop.targets);
        true
    }
    pub(super) fn apply_archive_event(&mut self, event: Event) -> Option<Event> {
        match event {
            Event::ArchiveSelection {
                pane,
                navigation,
                selected,
                totals,
            } => {
                let p = &mut self.panes[pane.index()];
                if p.generation == navigation.0
                    && let Some(a) = &mut p.archive
                {
                    self.retired
                        .push(Box::new(mem::replace(&mut a.selected, selected)));
                    a.selection_pending = false;
                    a.totals = totals;
                } else {
                    self.retired.push(Box::new(selected));
                }
            }
            Event::ArchiveAvailability(formats) => self.archive_formats = formats,
            Event::ArchiveListed {
                pane,
                navigation,
                result,
            } => {
                let p = &mut self.panes[pane.index()];
                if navigation.0 != p.generation {
                    self.retired.push(Box::new(result));
                    return None;
                }
                p.loading = false;
                p.archive_request = false;
                match result {
                    Err(error) => {
                        p.sort = p.committed_sort;
                        if let Some(a) = &mut p.archive {
                            a.requested_history = None;
                        }
                        p.error = error.to_string();
                    }
                    Ok(listing) => {
                        p.committed_sort = p.sort;
                        let same = p.archive.as_ref().is_some_and(|a| {
                            a.location.mount_id() == listing.location.mount_id()
                                && a.location.directory == listing.location.directory
                        });
                        if let Some(archive) = &mut p.archive {
                            if archive.location.mount.is_none() {
                                self.retired.push(Box::new(mem::replace(
                                    &mut archive.history,
                                    vec![listing.location.clone()],
                                )));
                                archive.history_index = 0;
                            } else if let Some(index) = archive.requested_history.take() {
                                archive.history_index = index;
                            } else if !same {
                                archive.history.truncate(archive.history_index + 1);
                                archive.history.push(listing.location.clone());
                                if archive.history.len() > 128 {
                                    self.retired.push(Box::new(archive.history.remove(0)));
                                }
                                archive.history_index = archive.history.len() - 1;
                            }
                            if !same {
                                self.retired
                                    .push(Box::new(mem::take(&mut archive.selected)));
                                archive.totals = ArchiveSelectionTotals::default();
                            }
                            archive.selection_pending = false;
                            archive.location = listing.location;
                        } else {
                            p.archive = Some(ArchivePane {
                                origin: p.path.clone(),
                                location: listing.location.clone(),
                                history: vec![listing.location],
                                history_index: 0,
                                requested_history: None,
                                selected: Arc::default(),
                                totals: ArchiveSelectionTotals::default(),
                                selection_pending: false,
                            });
                        }
                        self.retired
                            .push(Box::new(mem::replace(&mut p.entries, listing.entries)));
                        self.retired
                            .push(Box::new(mem::replace(&mut p.index, listing.index)));
                        self.retired.push(Box::new(p.pending.take()));
                        self.retired.push(Box::new(mem::take(&mut p.selected)));
                        self.retired.push(Box::new(mem::take(&mut p.streaming)));
                        self.retired.push(Box::new(mem::take(&mut p.metadata)));
                        self.retired.push(Box::new(mem::take(&mut p.folder_sizes)));
                        p.selection_preview.clear();
                        p.selection_rebase_pending = false;
                        p.size_scan = None;
                        self.retired.push(Box::new(p.size_scan_scope.take()));
                        self.retired
                            .push(Box::new(mem::take(&mut p.entry_size_scans)));
                        p.successful_generation = Some(navigation);
                        p.cursor = p.cursor.filter(|id| same && p.index.by_id.contains_key(id));
                        p.parent_cursor = p.cursor.is_none();
                        p.error.clear();
                    }
                }
            }
            other => return Some(other),
        }
        None
    }
    pub(super) fn archive_up(&mut self, pane: PaneId, send: &dyn Fn(Command) -> bool) -> bool {
        let Some(archive) = &self.panes[pane.index()].archive else {
            return false;
        };
        let mut location = archive.location.clone();
        if location.trail.len() > 1 {
            location.trail.pop();
            location.address.components.pop();
            location.directory = location.trail.last().map_or(1, |v| v.0);
            self.archive_navigate(pane, ArchiveAction::List(location), send)
        } else {
            let origin = archive.origin.clone();
            let container = archive.location.address.container.clone();
            if !self.navigate(pane, origin, None, send) {
                return false;
            }
            let p = &mut self.panes[pane.index()];
            if let Some(request) = &mut p.pending {
                request.wanted_cursor = Some(container);
            }
            p.parent_cursor = false;
            true
        }
    }
    pub(super) fn archive_open(
        &mut self,
        pane: PaneId,
        row: usize,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        if row == 0 {
            return self.archive_up(pane, send);
        }
        let p = &self.panes[pane.index()];
        let Some(node) = p
            .display_entry(row - 1)
            .and_then(|e| e.virtual_node.as_ref())
            .cloned()
        else {
            return false;
        };
        let Some(archive) = &p.archive else {
            return false;
        };
        if node.kind != EntryKind::Directory {
            self.desktop.report("Archive member", "Use F3 to view a readable member. External launch and extraction are unavailable for this read-only provider.".into());
            return false;
        }
        let mut location = archive.location.clone();
        location.directory = node.id.entry;
        location.trail.push((node.id.entry, node.display_name));
        location.address.components.push(node.raw_name.to_vec());
        if let Err(error) = location.address.validate() {
            self.desktop.report("Archive location limit", error.into());
            return false;
        }
        self.archive_navigate(pane, ArchiveAction::List(location), send)
    }
    pub(crate) fn native_action_allowed(&mut self, pane: PaneId) -> bool {
        if self.panes[pane.index()].native_capabilities() {
            return true;
        }
        self.desktop.report(
            "Read-only archive",
            "This action needs a native filesystem target and is unavailable inside an archive."
                .into(),
        );
        false
    }
    pub(crate) fn refresh_location(
        &mut self,
        pane: PaneId,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        if let Some(a) = &self.panes[pane.index()].archive {
            return self.archive_navigate(pane, ArchiveAction::List(a.location.clone()), send);
        }
        self.navigate(
            pane,
            self.panes[pane.index()].display_path().to_owned(),
            None,
            send,
        )
    }
    pub(super) fn archive_select_row(
        &mut self,
        pane: PaneId,
        row: usize,
        control: bool,
        shift: bool,
        send: &dyn Fn(Command) -> bool,
    ) {
        if row == 0 {
            self.focus_display_row(pane, row);
            return;
        }
        let p = &self.panes[pane.index()];
        if p.loading || p.search_view_waiting() {
            return;
        }
        let Some((entry, kind, size)) = p
            .display_entry(row - 1)
            .map(|e| (e.id, e.kind, e.metadata.size))
        else {
            return;
        };
        let first = p
            .selection_anchor_id
            .and_then(|id| p.display_index().by_id.get(&id))
            .copied()
            .unwrap_or(row - 1);
        self.focus_display_row(pane, row);
        if shift {
            self.archive_bulk_selection(
                pane,
                ArchiveSelectionAction::Range {
                    first: first.min(row - 1),
                    last: first.max(row - 1),
                    additive: control,
                },
                send,
            );
        } else {
            if control
                && self.panes[pane.index()]
                    .archive
                    .as_ref()
                    .is_some_and(|a| Arc::strong_count(&a.selected) > 1)
            {
                self.archive_bulk_selection(pane, ArchiveSelectionAction::Toggle(entry), send);
                return;
            }
            let p = &mut self.panes[pane.index()];
            p.selection_anchor_id = Some(entry);
            if control
                && let Some(a) = &mut p.archive
                && !a.selection_pending
            {
                let selected = Arc::make_mut(&mut a.selected);
                let removed = selected.remove(&entry);
                if !removed {
                    selected.insert(entry);
                }
                a.totals.change(kind, size, !removed);
            }
        }
    }
    pub(super) fn archive_bulk_selection(
        &mut self,
        pane: PaneId,
        action: ArchiveSelectionAction,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let p = &self.panes[pane.index()];
        let Some(a) = &p.archive else {
            return false;
        };
        if a.selection_pending || p.loading || p.search_view_waiting() {
            return false;
        }
        if !send(Command::ArchiveSelect {
            pane,
            navigation: NavigationId(p.generation),
            entries: p.entries.clone(),
            view: p.display_index().clone(),
            selected: a.selected.clone(),
            action,
        }) {
            return false;
        }
        if let Some(a) = &mut self.panes[pane.index()].archive {
            a.selection_pending = true;
        }
        true
    }
    pub(super) fn archive_history(
        &mut self,
        pane: PaneId,
        delta: isize,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let Some(a) = &self.panes[pane.index()].archive else {
            return false;
        };
        let Some(index) = a
            .history_index
            .checked_add_signed(delta)
            .filter(|i| *i < a.history.len())
        else {
            return false;
        };
        let location = a.history[index].clone();
        if !self.archive_navigate(pane, ArchiveAction::List(location), send) {
            return false;
        }
        let archive = self.panes[pane.index()]
            .archive
            .as_mut()
            .expect("archive retained");
        archive.requested_history = Some(index);
        true
    }
}
impl Pane {
    pub(super) fn archive_selection_summary(&self) -> Option<(usize, usize, Option<u64>, bool)> {
        let archive = self.archive.as_ref()?;
        (!archive.selected.is_empty() || archive.selection_pending).then_some((
            archive.totals.files(),
            archive.totals.folders(),
            archive.totals.file_bytes(),
            archive.selection_pending,
        ))
    }
    pub(crate) fn native_capabilities(&self) -> bool {
        self.archive.is_none() && !self.archive_request
    }
    pub(crate) fn display_location_text(&self) -> String {
        self.archive.as_ref().map_or_else(
            || self.display_path().to_string_lossy().into_owned(),
            |a| a.location.display_text(),
        )
    }
    pub(crate) fn selected_count(&self) -> usize {
        self.archive
            .as_ref()
            .map_or_else(|| self.selected.len(), |a| a.selected.len())
    }
}
