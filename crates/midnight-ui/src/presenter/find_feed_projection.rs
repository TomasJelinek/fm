//! Coalesced worker projections for fed rows. The hidden folder is never a target.
use super::*;

/// Build and projection share the backend's two coalescing pane slots.
pub(crate) fn next_feed_work_id() -> Option<u64> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    NEXT.try_update(Ordering::Relaxed, Ordering::Relaxed, |old| {
        old.checked_add(1)
    })
    .ok()
    .map(|previous| previous + 1)
}

pub(crate) struct FeedProjectionState {
    all_entries: Arc<[Entry]>,
    pub(crate) options: FindFeedOptions,
    pub(crate) desired: FindFeedOptions,
    pub(crate) view_revision: u64,
    mask: Option<Arc<MaskSelection>>,
    removed: Vec<Arc<[PathBuf]>>,
    pending: Option<(Arc<FindFeedProjectionRequest>, bool)>,
    dirty: bool,
}

impl FeedProjectionState {
    pub(crate) fn new(entries: Arc<[Entry]>, sort: FindSortSpec) -> Self {
        let options = FindFeedOptions {
            sort,
            show_hidden: true,
            ..Default::default()
        };
        Self {
            all_entries: entries,
            desired: options.clone(),
            options,
            view_revision: 0,
            mask: None,
            removed: vec![],
            pending: None,
            dirty: false,
        }
    }
    pub(crate) fn cancel(&mut self) {
        if let Some((request, _)) = &self.pending {
            request.cancel.cancel();
        }
    }
    pub(crate) fn changed(&mut self) {
        self.cancel();
        self.dirty = true;
    }
}

impl Drop for FeedProjectionState {
    fn drop(&mut self) {
        self.cancel();
    }
}

impl Presenter {
    pub(crate) fn request_feed_sort(
        &mut self,
        pane: PaneId,
        sort: FindSortSpec,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let Some(feed) = self.feed_mut(pane) else {
            return false;
        };
        feed.projection.desired.sort = sort;
        feed.projection.changed();
        self.flush_feed_projections(send);
        true
    }

    pub(crate) fn change_feed_hidden(
        &mut self,
        pane: PaneId,
        show: Option<bool>,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let Some(feed) = self.feed_mut(pane) else {
            return false;
        };
        feed.projection.desired.show_hidden = show.unwrap_or(!feed.projection.desired.show_hidden);
        feed.projection.changed();
        self.flush_feed_projections(send);
        true
    }

    pub(crate) fn request_feed_filter(
        &mut self,
        pane: PaneId,
        filter: PanelFilter,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let Some(feed) = self.feed_mut(pane) else {
            return false;
        };
        feed.projection.desired.filter = filter;
        feed.projection.changed();
        self.flush_feed_projections(send);
        true
    }

    pub(crate) fn request_feed_mask(
        &mut self,
        pane: PaneId,
        mask: Arc<MaskSelection>,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let Some(feed) = self.feed_mut(pane) else {
            return false;
        };
        if feed.projection.mask.is_some() {
            return false;
        }
        feed.projection.mask = Some(mask);
        feed.projection.changed();
        self.flush_feed_projections(send);
        true
    }

    /// Completed mutations retire stale paths on a worker, including originals
    /// hidden by the current filter. Up to 64 batches / 100,000 paths are retained.
    pub(crate) fn remove_feed_projection_paths(&mut self, pane: PaneId, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        let Some(feed) = self.feed_mut(pane) else {
            return;
        };
        let count = feed
            .projection
            .removed
            .iter()
            .map(|batch| batch.len())
            .sum::<usize>();
        if feed.projection.removed.len() >= 64
            || count.saturating_add(paths.len()) > FindLimits::default().max_results
        {
            self.feed_close(pane);
            self.desktop.report("Search results changed", "Too many completed file changes are waiting for the results worker. Run Find again to refresh the list.".into());
            return;
        }
        feed.projection.removed.push(paths.into());
        feed.projection.changed();
        feed.selection_changed();
    }

    pub(super) fn flush_feed_projections(&mut self, send: &dyn Fn(Command) -> bool) {
        for pane in [PaneId::Left, PaneId::Right] {
            if self.finds.feed_builds[pane.index()].is_some() {
                continue;
            }
            let Some(feed) = self.feed_mut(pane) else {
                continue;
            };
            if feed
                .projection
                .pending
                .as_ref()
                .is_some_and(|(request, _)| {
                    request.selection_revision != feed.selection_revision
                        || request.rows_serial != feed.rows_serial
                        || request.cancel.is_cancelled()
                })
            {
                if feed
                    .projection
                    .pending
                    .as_ref()
                    .is_some_and(|(request, _)| !Arc::ptr_eq(&request.selected, &feed.selected))
                {
                    feed.projection.mask = None;
                }
                feed.projection.changed();
            }
            let old = if feed.projection.dirty {
                let Some(id) = next_feed_work_id() else {
                    self.desktop.report(
                        "Search results",
                        "Result view identities are exhausted. Restart the application.".into(),
                    );
                    continue;
                };
                let request = Arc::new(FindFeedProjectionRequest {
                    id,
                    pane,
                    rows_serial: feed.rows_serial,
                    selection_revision: feed.selection_revision,
                    entries: feed.projection.all_entries.clone(),
                    visible: feed.entries.clone(),
                    selected: feed.selected.clone(),
                    cursor: feed.focused().map(|entry| entry.id),
                    options: feed.projection.desired.clone(),
                    mask: feed.projection.mask.clone(),
                    removed: feed.projection.removed.clone().into(),
                    folder_sizes: feed.sizes.sort_sizes(feed.projection.desired.sort),
                    cancel: Cancellation::default(),
                });
                feed.projection.dirty = false;
                feed.projection.pending.replace((request, false))
            } else {
                None
            };
            if let Some((request, sent)) = &mut feed.projection.pending
                && !*sent
                && send(Command::ProjectFindFeed(request.clone()))
            {
                *sent = true;
            }
            if let Some(old) = old {
                self.retired.push(Box::new(old));
            }
        }
    }

    pub(super) fn apply_feed_projection_event(&mut self, event: Event) -> Option<Event> {
        let event = match event {
            Event::FindFeedSize(event) => {
                self.apply_find_feed_size(*event);
                return None;
            }
            event => event,
        };
        let Event::FindFeedProjected(receipt) = event else {
            return Some(event);
        };
        let pane = receipt.request.pane;
        let Some(feed) = self.feed_mut(pane) else {
            self.retired.push(receipt);
            return None;
        };
        if !feed
            .projection
            .pending
            .as_ref()
            .is_some_and(|(request, _)| request.id == receipt.request.id)
        {
            self.retired.push(receipt);
            return None;
        }
        if receipt.request.rows_serial != feed.rows_serial
            || receipt.request.selection_revision != feed.selection_revision
            || feed
                .projection
                .pending
                .as_ref()
                .is_some_and(|(request, _)| request.cancel.is_cancelled())
        {
            if feed
                .projection
                .pending
                .as_ref()
                .is_some_and(|(request, _)| !Arc::ptr_eq(&request.selected, &feed.selected))
            {
                feed.projection.mask = None;
            }
            feed.projection.changed();
            self.retired.push(receipt);
            return None;
        }
        let FindFeedProjectionResult { result, .. } = *receipt;
        let pending = feed.projection.pending.take();
        let Some((request, _)) = &pending else {
            return None;
        };
        let request = request.clone();
        match result {
            Ok(view) => {
                let retired = (
                    mem::replace(&mut feed.entries, view.entries),
                    mem::replace(&mut feed.index, view.index),
                    mem::replace(&mut feed.selected, view.selected),
                    mem::replace(&mut feed.projection.all_entries, view.all_entries),
                    mem::take(&mut feed.projection.removed),
                    feed.projection.mask.take(),
                    pending,
                    request.clone(),
                );
                feed.selected_count = view.selected_count;
                feed.selected_bytes = view.selected_bytes;
                feed.cursor = view.cursor;
                feed.projection.options = request.options.clone();
                feed.projection.view_revision = feed.projection.view_revision.wrapping_add(1);
                let sizes = if !request.removed.is_empty() {
                    feed.rows_serial = super::find::feed::next_rows_serial();
                    Some(feed.sizes.reset())
                } else {
                    None
                };
                feed.selection_changed();
                feed.changed();
                self.retired.push(Box::new(retired));
                if let Some(sizes) = sizes {
                    self.retired.push(sizes);
                }
            }
            Err(error) => {
                feed.projection.mask = None;
                let stale_source = !feed.projection.removed.is_empty();
                if stale_source {
                    self.feed_close(pane);
                }
                self.desktop.report(
                    "Search results",
                    crate::error_presentation::describe(&error),
                );
                self.retired.push(Box::new((pending, request, error)));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::super::find::FindForm;
    use super::*;
    use std::cell::RefCell;

    fn fed() -> (Presenter, FindSessionId) {
        let path = std::env::temp_dir().join("feed-worker-state");
        let mut p = Presenter::new(path.clone(), path.clone());
        let id = p.create_find(PaneId::Left).unwrap();
        p.start_find(id, FindForm::default(), &|_| true);
        let result = Arc::new(FindResult {
            id: FindResultId(1),
            entry: FindEntry {
                path: path.join("file.txt"),
                kind: EntryKind::File,
                metadata: EntryMetadata::default(),
                identity: TransferIdentity {
                    device: 1,
                    object: [0; 16],
                },
                revision: FindRevision([0; 6]),
            },
            duplicate_group: None,
            duplicate_group_size: 0,
            archive: None,
        });
        for update in [
            FindUpdate::Batch(vec![result].into()),
            FindUpdate::Finished(FindSummary {
                completion: FindCompletion::Completed,
                progress: FindProgress::default(),
                reached_budgets: vec![],
                diagnostics: vec![],
            }),
        ] {
            p.apply(Event::Find(Box::new(FindEvent {
                session: id,
                generation: FindGeneration(1),
                update,
            })));
        }
        p.feed_find_results(id).unwrap();
        (p, id)
    }
    fn offer(commands: &RefCell<Vec<Arc<FindFeedProjectionRequest>>>, command: Command) -> bool {
        if let Command::ProjectFindFeed(request) = command {
            commands.borrow_mut().push(request);
        }
        true
    }
    fn stamp(request: &FindFeedProjectionRequest) -> FindFeedProjectionStamp {
        FindFeedProjectionStamp {
            id: request.id,
            pane: request.pane,
            rows_serial: request.rows_serial,
            selection_revision: request.selection_revision,
        }
    }
    #[test]
    fn queue_refusal_retains_latest_sort_and_stale_marks_rebase_on_worker() {
        let (mut p, _) = fed();
        let sort = FindSortSpec {
            column: FindSortColumn::Size,
            descending: true,
        };
        assert!(p.request_feed_sort(PaneId::Left, sort, &|_| false));
        let commands = RefCell::new(vec![]);
        p.flush_feed_projections(&|command| offer(&commands, command));
        let first = commands.borrow_mut().pop().unwrap();
        assert_eq!(first.options.sort, sort);
        p.feed_click(PaneId::Left, 0, true, false);
        p.apply(Event::FindFeedProjected(Box::new(
            FindFeedProjectionResult {
                request: stamp(&first),
                result: Err(FsError::from_io(
                    "stale",
                    Path::new(""),
                    std::io::ErrorKind::Other.into(),
                )),
            },
        )));
        assert!(p.feed(PaneId::Left).unwrap().selected[0]);
        p.flush_feed_projections(&|command| offer(&commands, command));
        let second = commands.borrow_mut().pop().unwrap();
        assert!(second.id > first.id);
        assert!(first.cancel.is_cancelled());
        assert!(second.selected[0]);
    }
    #[test]
    fn replaced_feed_ignores_old_reply_and_pruning_captures_original_source() {
        let (mut p, id) = fed();
        let commands = RefCell::new(vec![]);
        p.change_feed_hidden(PaneId::Left, Some(false), &|command| {
            offer(&commands, command)
        });
        let old = commands.borrow_mut().pop().unwrap();
        p.feed_find_results(id).unwrap();
        p.apply(Event::FindFeedProjected(Box::new(
            FindFeedProjectionResult {
                request: stamp(&old),
                result: Err(FsError::from_io(
                    "stale",
                    Path::new(""),
                    std::io::ErrorKind::Other.into(),
                )),
            },
        )));
        assert!(p.feed(PaneId::Left).unwrap().projection.desired.show_hidden);
        let path = p.feed(PaneId::Left).unwrap().entries[0].path.clone();
        p.remove_feed_projection_paths(PaneId::Left, vec![path.clone()]);
        p.flush_feed_projections(&|command| offer(&commands, command));
        let prune = commands.borrow_mut().pop().unwrap();
        assert_eq!(prune.entries.len(), 1);
        assert_eq!(prune.removed[0][0], path);
    }

    #[test]
    fn sorting_during_folder_progress_preserves_scan_receipts_and_completion() {
        let (mut p, _) = fed();
        p.preferences.values.size_display = SizeDisplay::Bytes;
        let feed = p.feed_mut(PaneId::Left).unwrap();
        let mut folder = feed.entries[0].clone();
        folder.kind = EntryKind::Directory;
        feed.entries = vec![folder].into();
        feed.index = Arc::new(DirectoryIndex::from_entries(&feed.entries));
        feed.projection = FeedProjectionState::new(feed.entries.clone(), FindSortSpec::default());
        let scan = RefCell::new(None);
        p.request_find_feed_sizes(PaneId::Left, &|command| {
            if let Command::ComputeFindFeedSizes(request) = command {
                *scan.borrow_mut() = Some(request);
            }
            true
        });
        let scan = scan.into_inner().unwrap();
        let commands = RefCell::new(vec![]);
        let sort = FindSortSpec {
            column: FindSortColumn::Modified,
            descending: true,
        };
        p.request_feed_sort(PaneId::Left, sort, &|command| offer(&commands, command));
        let request = commands.borrow_mut().pop().unwrap();
        p.apply_find_feed_size(FindFeedSizeEvent {
            pane: scan.pane,
            id: scan.id,
            rows_serial: scan.rows_serial,
            update: FindFeedSizeUpdate::Updated {
                entry: EntryId(1),
                size: FolderSize {
                    bytes: 123,
                    ..Default::default()
                },
            },
        });
        p.apply(Event::FindFeedProjected(Box::new(
            FindFeedProjectionResult {
                request: stamp(&request),
                result: Ok(FindFeedProjection {
                    all_entries: request.entries.clone(),
                    entries: request.visible.clone(),
                    index: Arc::new(DirectoryIndex::from_entries(&request.visible)),
                    selected: request.selected.clone(),
                    selected_count: 0,
                    selected_bytes: 0,
                    cursor: 0,
                }),
            },
        )));
        let feed = p.feed(PaneId::Left).unwrap();
        assert_eq!(feed.rows_serial, scan.rows_serial);
        assert_eq!(
            feed.projection.options.sort, sort,
            "folder repaint cannot invalidate a sort receipt"
        );
        assert!(
            feed.sizes
                .folder_size_text(&feed.entries[0], &p)
                .unwrap()
                .contains("123")
        );
        p.apply_find_feed_size(FindFeedSizeEvent {
            pane: scan.pane,
            id: scan.id,
            rows_serial: scan.rows_serial,
            update: FindFeedSizeUpdate::Finished {
                cancelled: false,
                error: None,
            },
        });
        let next = RefCell::new(false);
        p.request_find_feed_sizes(PaneId::Left, &|command| {
            *next.borrow_mut() = matches!(command, Command::ComputeFindFeedSizes(_));
            true
        });
        assert!(
            *next.borrow(),
            "completed scan can start again after sorting"
        );
    }

    /// Sorted by Size, the end of a folder calculation projects the list again with
    /// the finished totals; running or cancelled totals are not sent.
    #[test]
    fn size_sorted_feed_projects_again_with_finished_folder_totals() {
        let (mut p, _) = fed();
        let feed = p.feed_mut(PaneId::Left).unwrap();
        let mut folder = feed.entries[0].clone();
        folder.kind = EntryKind::Directory;
        feed.entries = vec![folder].into();
        feed.index = Arc::new(DirectoryIndex::from_entries(&feed.entries));
        let sort = FindSortSpec {
            column: FindSortColumn::Size,
            descending: true,
        };
        feed.projection = FeedProjectionState::new(feed.entries.clone(), sort);
        let scan = RefCell::new(None);
        p.request_find_feed_sizes(PaneId::Left, &|command| {
            if let Command::ComputeFindFeedSizes(request) = command {
                *scan.borrow_mut() = Some(request);
            }
            true
        });
        let scan = scan.into_inner().unwrap();
        let commands = RefCell::new(vec![]);
        let update = |p: &mut Presenter, update| {
            p.apply_find_feed_size(FindFeedSizeEvent {
                pane: scan.pane,
                id: scan.id,
                rows_serial: scan.rows_serial,
                update,
            });
            p.flush_feed_projections(&|command| offer(&commands, command));
        };
        update(
            &mut p,
            FindFeedSizeUpdate::Updated {
                entry: EntryId(1),
                size: FolderSize {
                    bytes: 123,
                    finished: true,
                    errors: 2,
                    ..Default::default()
                },
            },
        );
        assert!(commands.borrow().is_empty(), "progress does not re-sort");
        update(
            &mut p,
            FindFeedSizeUpdate::Finished {
                cancelled: false,
                error: None,
            },
        );
        let request = commands.borrow_mut().pop().expect("a new projection");
        assert_eq!(request.options.sort, sort);
        assert_eq!(request.folder_sizes.get(&EntryId(1)), Some(&123));
    }

    #[test]
    fn newer_explicit_marks_supersede_pending_mask_instead_of_being_overwritten() {
        let (mut p, _) = fed();
        let commands = RefCell::new(vec![]);
        assert!(p.request_feed_mask(
            PaneId::Left,
            Arc::new(MaskSelection {
                pattern: "*".into(),
                select: true,
                folders: false
            }),
            &|command| offer(&commands, command)
        ));
        let first = commands.borrow_mut().pop().unwrap();
        p.feed_click(PaneId::Left, 0, true, false);
        p.feed_click(PaneId::Left, 0, true, false);
        p.flush_feed_projections(&|command| offer(&commands, command));
        let latest = commands.borrow_mut().pop().unwrap();
        assert!(first.cancel.is_cancelled());
        assert!(latest.mask.is_none());
        assert!(!latest.selected[0], "later explicit deselection wins");
    }
}
