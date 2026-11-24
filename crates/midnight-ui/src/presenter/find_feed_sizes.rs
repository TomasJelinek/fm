//! Captured Find-result folder scans. Only input, cached presentation and cancellation live here.
use crate::Presenter;
use midnight_core::*;
use std::{collections::HashMap, sync::Arc};

struct Scope {
    request: Arc<FindFeedSizeRequest>,
    index: Arc<DirectoryIndex>,
    marked: bool,
}

#[derive(Default)]
pub(crate) struct FeedSizeState {
    next: u64,
    scope: Option<Scope>,
    running: bool,
    finished: bool,
    values: HashMap<EntryId, FolderSize>,
}

impl FeedSizeState {
    pub(crate) fn cancel(&mut self) {
        if let Some(scope) = &self.scope {
            scope.request.cancel.cancel();
        }
        self.running = false;
    }

    /// Retire the returned owner on a worker; it may own many cached warnings and paths.
    pub(crate) fn reset(&mut self) -> Box<dyn Send> {
        self.cancel();
        Box::new(std::mem::take(self))
    }

    fn covers(&self, entry: &Entry) -> bool {
        let Some(scope) = &self.scope else {
            return false;
        };
        let Some(&row) = scope.index.by_id.get(&entry.id) else {
            return false;
        };
        scope
            .request
            .entries
            .get(row)
            .is_some_and(|captured| captured.path == entry.path)
            && (!scope.marked || scope.request.selected.get(row) == Some(&true))
            && matches!(entry.kind, EntryKind::Directory | EntryKind::DirectoryLink)
    }

    pub(crate) fn folder_size_text(&self, entry: &Entry, presenter: &Presenter) -> Option<String> {
        if !matches!(entry.kind, EntryKind::Directory | EntryKind::DirectoryLink) {
            return None;
        }
        let value = self.values.get(&entry.id);
        let covered = self.covers(entry);
        if value.is_none() && !covered {
            return None;
        }
        Some(
            crate::size_presentation::folder_size(
                value,
                self.running && covered,
                covered,
                presenter.preferences.values.size_display,
                &presenter.number_locale,
            )
            .0,
        )
    }
}

impl Drop for FeedSizeState {
    fn drop(&mut self) {
        self.cancel();
    }
}

impl Presenter {
    pub(crate) fn request_find_feed_sizes(&mut self, pane: PaneId, send: &dyn Fn(Command) -> bool) {
        if self.operations.closing {
            return;
        }
        let Some(feed) = self.feed_mut(pane) else {
            return;
        };
        if feed.sizes.running {
            feed.sizes.cancel();
            feed.revision = feed.revision.wrapping_add(1);
            self.desktop.status = "Cancelling folder size scan…".into();
            return;
        }
        let Some(next) = feed.sizes.next.checked_add(1) else {
            return;
        };
        let request = Arc::new(FindFeedSizeRequest {
            pane,
            id: FolderSizeScanId(next),
            rows_serial: feed.rows_serial,
            entries: feed.entries.clone(),
            selected: feed.selected.clone(),
            cancel: Cancellation::default(),
        });
        let index = feed.index.clone();
        let marked = feed.selected_count > 0;
        self.submit_find_feed_sizes(request, index, marked, send);
    }

    /// Space takes this one raw row before its selection/cursor gesture advances.
    pub(super) fn measure_find_feed_entry(
        &mut self,
        pane: PaneId,
        entry: Entry,
        send: &dyn Fn(Command) -> bool,
    ) {
        if self.operations.closing
            || !matches!(entry.kind, EntryKind::Directory | EntryKind::DirectoryLink)
        {
            return;
        }
        let Some(feed) = self.feed(pane) else {
            return;
        };
        if feed.sizes.running && feed.sizes.covers(&entry) {
            return;
        }
        let Some(next) = feed.sizes.next.checked_add(1) else {
            return;
        };
        let entries: Arc<[Entry]> = Arc::from([entry]);
        let index = Arc::new(DirectoryIndex::from_entries(&entries));
        let request = Arc::new(FindFeedSizeRequest {
            pane,
            id: FolderSizeScanId(next),
            rows_serial: feed.rows_serial,
            entries,
            selected: Arc::new(vec![false]),
            cancel: Cancellation::default(),
        });
        self.submit_find_feed_sizes(request, index, false, send);
    }

    fn submit_find_feed_sizes(
        &mut self,
        request: Arc<FindFeedSizeRequest>,
        index: Arc<DirectoryIndex>,
        marked: bool,
        send: &dyn Fn(Command) -> bool,
    ) {
        let Some(feed) = self.feed_mut(request.pane) else {
            return;
        };
        if !send(Command::ComputeFindFeedSizes(request.clone())) {
            self.retired.push(Box::new(request));
            self.desktop.report(
                "Folder size queue busy",
                "The scan was not started. Try again.".into(),
            );
            return;
        }
        let next = request.id.0;
        let old = feed.sizes.scope.replace(Scope {
            request,
            index,
            marked,
        });
        if let Some(old) = &old {
            old.request.cancel.cancel();
        }
        feed.sizes.next = next;
        feed.sizes.running = true;
        feed.sizes.finished = false;
        feed.revision = feed.revision.wrapping_add(1);
        self.retired.push(Box::new(old));
    }

    pub(crate) fn apply_find_feed_size(&mut self, event: FindFeedSizeEvent) {
        let Some(feed) = self.feed_mut(event.pane).filter(|feed| {
            feed.rows_serial == event.rows_serial
                && !feed.sizes.finished
                && feed
                    .sizes
                    .scope
                    .as_ref()
                    .is_some_and(|scope| scope.request.id == event.id)
        }) else {
            self.retired.push(Box::new(event));
            return;
        };
        let (old, error) = match event.update {
            FindFeedSizeUpdate::Updated { entry, size } => {
                let valid = feed
                    .sizes
                    .scope
                    .as_ref()
                    .and_then(|scope| {
                        scope
                            .index
                            .by_id
                            .get(&entry)
                            .and_then(|row| scope.request.entries.get(*row))
                    })
                    .is_some_and(|entry| feed.sizes.covers(entry));
                if !valid {
                    self.retired.push(Box::new(size));
                    return;
                }
                (feed.sizes.values.insert(entry, size), None)
            }
            FindFeedSizeUpdate::Finished {
                cancelled: _,
                error,
            } => {
                feed.sizes.running = false;
                feed.sizes.finished = true;
                (None, error)
            }
        };
        feed.revision = feed.revision.wrapping_add(1);
        self.retired.push(Box::new(old));
        if let Some(error) = error {
            self.desktop
                .report("Folder sizes", crate::error_presentation::describe(&error));
        }
    }
}

#[cfg(test)]
#[path = "find_feed_sizes_tests.rs"]
mod tests;
