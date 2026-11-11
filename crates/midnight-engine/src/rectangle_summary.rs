//! One cancellable latest-value summary per panel; never changes committed selection.
use crate::Outbox;
use midnight_core::*;
use std::sync::{Arc, Mutex};
use tokio::{sync::watch, task::JoinSet};

type Mailbox = Arc<Mutex<Option<Arc<RectangleSummaryRequest>>>>;

pub(crate) struct Controller {
    requests: [(Mailbox, watch::Sender<()>); 2],
    stop: Cancellation,
}
impl Controller {
    pub(crate) fn new(outbox: Outbox, tasks: &mut JoinSet<()>) -> Self {
        let stop = Cancellation::default();
        let requests = std::array::from_fn(|_| {
            let (sender, mut receiver) = watch::channel(());
            let mailbox: Mailbox = Arc::default();
            let pending = mailbox.clone();
            let outbox = outbox.clone();
            let stop = stop.clone();
            tasks.spawn(async move {
                while receiver.changed().await.is_ok() && !stop.is_cancelled() {
                    let Some(request) = take_latest(&pending, &mut receiver) else {
                        continue;
                    };
                    let work = request.clone();
                    let stopping = stop.clone();
                    let Ok(Some(totals)) =
                        tokio::task::spawn_blocking(move || summarize(&work, &stopping)).await
                    else {
                        continue;
                    };
                    if request.cancel.is_cancelled() || stop.is_cancelled() {
                        continue;
                    }
                    let event = Event::RectangleSummary {
                        pane: request.pane,
                        token: request.token.clone(),
                        totals,
                    };
                    tokio::select! {
                        biased;
                        _ = receiver.changed() => { receiver.mark_changed(); }
                        sent = outbox.send(event) => { if !sent { break; } }
                    }
                }
            });
            (mailbox, sender)
        });
        Self { requests, stop }
    }
    pub(crate) fn submit(&self, request: Arc<RectangleSummaryRequest>) {
        let (mailbox, wake) = &self.requests[request.pane.index()];
        let mut pending = mailbox.lock().unwrap_or_else(|e| e.into_inner());
        *pending = Some(request);
        wake.send_replace(());
    }
}
fn take_latest(
    mailbox: &Mailbox,
    receiver: &mut watch::Receiver<()>,
) -> Option<Arc<RectangleSummaryRequest>> {
    // Match the consumed wake to the request taken under the same lock used by
    // submit. A newer wake must not discard the very request we just captured.
    let mut pending = mailbox.lock().unwrap_or_else(|e| e.into_inner());
    receiver.borrow_and_update();
    pending.take()
}
impl Drop for Controller {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}
fn summarize(
    request: &RectangleSummaryRequest,
    stop: &Cancellation,
) -> Option<ArchiveSelectionTotals> {
    if request.ranges.len() != request.view.by_id.len() {
        return None;
    }
    let cancelled = || request.cancel.is_cancelled() || stop.is_cancelled();
    if cancelled() {
        return None;
    }
    let mut totals = ArchiveSelectionTotals::default();
    let mut visited = 0usize;
    let mut add = |index: &DirectoryIndex, position: usize| {
        let directory = index
            .directory_bits
            .get(position / 64)
            .is_some_and(|bits| bits & (1 << (position % 64)) != 0);
        totals.change(
            if directory {
                EntryKind::Directory
            } else {
                EntryKind::File
            },
            index.file_sizes.get(position).copied().flatten(),
            true,
        );
    };
    if request.mode == SelectionMode::Replace {
        // Ordinary drag selection visits only intersected rows, not the full directory.
        let tree = if let RectangleSelectionSeed::Tree(seed) = &request.seed {
            if !Arc::ptr_eq(&seed.basis().index.directory, &request.view) {
                return None;
            }
            Some(seed.basis())
        } else {
            None
        };
        for span in request.ranges.spans() {
            for position in span.first..=span.last {
                visited += 1;
                if visited.is_multiple_of(1024) && cancelled() {
                    return None;
                }
                if tree.is_none_or(|view| {
                    view.rows
                        .get(position)
                        .is_some_and(|row| row.selectable && !row.context_only)
                }) {
                    add(&request.view, position);
                }
            }
        }
        return (!cancelled()).then_some(totals);
    }
    match &request.seed {
        RectangleSelectionSeed::Native(seed) => {
            // Retain hidden original marks for additive/toggle selection too.
            for (path, id) in &seed.basis().by_path {
                visited += 1;
                if visited.is_multiple_of(1024) && cancelled() {
                    return None;
                }
                let position = *seed.basis().by_id.get(id)?;
                let hit = request
                    .view
                    .by_path
                    .get(path)
                    .and_then(|id| request.view.by_id.get(id))
                    .is_some_and(|position| request.ranges.contains(*position));
                if request.mode.selected(hit, seed.contains_path(path)) {
                    add(seed.basis(), position);
                }
            }
            for (path, id) in &request.view.by_path {
                visited += 1;
                if visited.is_multiple_of(1024) && cancelled() {
                    return None;
                }
                if !seed.basis().by_path.contains_key(path) {
                    let position = *request.view.by_id.get(id)?;
                    if request
                        .mode
                        .selected(request.ranges.contains(position), false)
                    {
                        add(&request.view, position);
                    }
                }
            }
        }
        RectangleSelectionSeed::Archive { selected, full } => {
            for (id, position) in &full.by_id {
                visited += 1;
                if visited.is_multiple_of(1024) && cancelled() {
                    return None;
                }
                let hit = request
                    .view
                    .by_id
                    .get(id)
                    .is_some_and(|row| request.ranges.contains(*row));
                if request.mode.selected(hit, selected.contains(id)) {
                    add(full, *position);
                }
            }
        }
        RectangleSelectionSeed::Tree(seed) => {
            if !Arc::ptr_eq(&seed.basis().index.directory, &request.view) {
                return None;
            }
            for (position, row) in seed.basis().rows.iter().enumerate() {
                if position.is_multiple_of(1024) && cancelled() {
                    return None;
                }
                if row.selectable
                    && !row.context_only
                    && request.mode.selected(
                        request.ranges.contains(position),
                        seed.contains_position(position),
                    )
                {
                    add(&request.view, position);
                }
            }
        }
    }
    (!cancelled()).then_some(totals)
}

#[cfg(test)]
#[path = "rectangle_summary_tests.rs"]
mod tests;
