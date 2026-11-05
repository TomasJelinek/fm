//! Cancellable in-memory Unicode prefix matching over immutable directory snapshots.

use crate::{Outbox, Snapshots};
use fm_core::{
    DirectoryIndex, Event, FsError, PaneId, SearchMatch, SearchRequest, SearchResult,
    SearchRevision,
};
use std::{io, sync::Arc};
use tokio::{sync::watch, task::JoinSet};
use unicase::UniCase;
use unicode_segmentation::UnicodeSegmentation;

const CANCEL_INTERVAL: usize = 256;

/// One coalescing slot and one actual CPU job per pane. No idle blocking workers.
pub(crate) struct Controller {
    inputs: [watch::Sender<Intent>; 2],
}

#[derive(Clone, Default)]
struct Intent {
    serial: u64,
    revision: Option<SearchRevision>,
    request: Option<Arc<SearchRequest>>,
}

impl Controller {
    pub(crate) fn new(snapshots: Snapshots, outbox: Outbox, tasks: &mut JoinSet<()>) -> Self {
        let inputs = std::array::from_fn(|_| {
            let (sender, receiver) = watch::channel(Intent::default());
            tasks.spawn(run(snapshots.clone(), outbox.clone(), receiver));
            sender
        });
        Self { inputs }
    }

    pub(crate) fn query(&self, request: SearchRequest) {
        let input = &self.inputs[request.pane.index()];
        input.send_if_modified(|intent| {
            if intent
                .revision
                .is_some_and(|latest| request.revision <= latest)
            {
                return false;
            }
            intent.serial = intent.serial.wrapping_add(1);
            intent.revision = Some(request.revision);
            intent.request = Some(Arc::new(request));
            true
        });
    }

    pub(crate) fn cancel(&self, pane: PaneId, revision: SearchRevision) {
        self.inputs[pane.index()].send_if_modified(|intent| {
            if intent.revision.is_some_and(|latest| revision <= latest) {
                return false;
            }
            intent.serial = intent.serial.wrapping_add(1);
            intent.revision = Some(revision);
            intent.request = None;
            true
        });
    }

    /// Navigation invalidates the source immediately without resetting the
    /// caller's monotonically increasing query revision.
    pub(crate) fn invalidate(&self, pane: PaneId) {
        self.inputs[pane.index()].send_modify(|intent| {
            intent.serial = intent.serial.wrapping_add(1);
            intent.request = None;
        });
    }
}

fn source_matches(snapshots: &Snapshots, request: &SearchRequest) -> bool {
    let snapshots = snapshots.lock().unwrap_or_else(|error| error.into_inner());
    let state = &snapshots[request.pane.index()];
    state.pending.is_none()
        && [&state.visible, &state.committed]
            .into_iter()
            .flatten()
            .any(|view| {
                view.navigation == request.navigation
                    && view.path == request.location
                    && Arc::ptr_eq(&view.entries, &request.source)
            })
}

async fn run(snapshots: Snapshots, outbox: Outbox, mut input: watch::Receiver<Intent>) {
    let mut pending = false;
    loop {
        if !pending && input.changed().await.is_err() {
            return;
        }
        pending = false;
        let intent = input.borrow_and_update().clone();
        let Some(request) = intent.request else {
            continue;
        };
        let valid_source = source_matches(&snapshots, &request);
        let computation = if valid_source {
            let work_request = request.clone();
            let observer = input.clone();
            tokio::task::spawn_blocking(move || {
                compute(&work_request, &|| observer.has_changed().unwrap_or(true))
            })
            .await
            .unwrap_or_else(|error| {
                Computation::Failed(FsError::from_io(
                    "search filenames",
                    &request.location,
                    io::Error::other(error.to_string()),
                ))
            })
        } else {
            Computation::Failed(FsError::from_io(
                "search filenames",
                &request.location,
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "The directory snapshot is no longer current.",
                ),
            ))
        };
        let event = match computation {
            Computation::Cancelled => continue,
            Computation::Ready(result) => Event::SearchReady {
                pane: request.pane,
                navigation: request.navigation,
                revision: request.revision,
                result: Arc::new(result),
            },
            Computation::Failed(error) => Event::SearchFailed {
                pane: request.pane,
                navigation: request.navigation,
                revision: request.revision,
                error,
            },
        };
        tokio::select! {
            biased;
            changed = input.changed() => {
                if changed.is_err() { return; }
                pending = true;
            }
            permit = outbox.sender.reserve() => {
                let Ok(permit) = permit else { return; };
                let sent = {
                    let latest = input.borrow();
                    if latest.serial != intent.serial || (valid_source && !source_matches(&snapshots, &request)) {
                        false
                    } else {
                        permit.send(event);
                        true
                    }
                };
                if sent { (outbox.wakeup)(); }
            }
        }
    }
}

enum Computation {
    Ready(SearchResult),
    Cancelled,
    Failed(FsError),
}

fn compute(request: &SearchRequest, cancelled: &dyn Fn() -> bool) -> Computation {
    if cancelled() {
        return Computation::Cancelled;
    }
    let query = UniCase::unicode(&request.query).to_folded_case();
    let mut matches = Vec::new();
    let mut index = DirectoryIndex::default();
    for (source_row, entry) in request.source.iter().enumerate() {
        if source_row.is_multiple_of(CANCEL_INTERVAL) && cancelled() {
            return Computation::Cancelled;
        }
        let display_prefix_end = if query.is_empty() {
            0
        } else {
            // A replacement character in the label must never identify a different
            // native name. Unrepresentable names remain in the unfiltered snapshot.
            let Some(name) = entry.name.to_str() else {
                continue;
            };
            let end = match prefix_end(name, &query, cancelled) {
                Ok(Some(end)) => end,
                Ok(None) => continue,
                Err(()) => return Computation::Cancelled,
            };
            if entry.display_name == name { end } else { 0 }
        };
        let row = matches.len();
        if index.by_id.insert(entry.id, row).is_some()
            || index.by_path.insert(entry.path.clone(), entry.id).is_some()
        {
            return ambiguous_projection(request);
        }
        matches.push(SearchMatch {
            source_row,
            display_prefix_end,
        });
    }
    // A matching id/path must not also refer to an unmatched source row. Validate
    // against the source without allocating another full-directory identity map.
    for (source_row, entry) in request.source.iter().enumerate() {
        if source_row.is_multiple_of(CANCEL_INTERVAL) && cancelled() {
            return Computation::Cancelled;
        }
        if let Some(row) = index.by_id.get(&entry.id)
            && matches[*row].source_row != source_row
        {
            return ambiguous_projection(request);
        }
        if let Some(id) = index.by_path.get(&entry.path)
            && let Some(row) = index.by_id.get(id)
            && matches[*row].source_row != source_row
        {
            return ambiguous_projection(request);
        }
    }
    if cancelled() {
        return Computation::Cancelled;
    }
    Computation::Ready(SearchResult {
        source: request.source.clone(),
        matches: matches.into(),
        index: Arc::new(index),
    })
}

fn ambiguous_projection(request: &SearchRequest) -> Computation {
    Computation::Failed(FsError::from_io(
        "search filenames",
        &request.location,
        io::Error::new(
            io::ErrorKind::InvalidData,
            "Directory identities are ambiguous.",
        ),
    ))
}

fn prefix_end(name: &str, query: &str, cancelled: &dyn Fn() -> bool) -> Result<Option<usize>, ()> {
    if name.is_ascii() && query.is_ascii() {
        if !name
            .as_bytes()
            .get(..query.len())
            .is_some_and(|part| part.eq_ignore_ascii_case(query.as_bytes()))
        {
            return Ok(None);
        }
        let mut end = query.len();
        // CRLF is the only multi-character extended grapheme in pure ASCII.
        if end > 0
            && name.as_bytes().get(end - 1) == Some(&b'\r')
            && name.as_bytes().get(end) == Some(&b'\n')
        {
            end += 1;
        }
        return Ok(Some(end));
    }
    let mut remaining = query.as_bytes();
    for (number, (start, grapheme)) in name.grapheme_indices(true).enumerate() {
        if number.is_multiple_of(CANCEL_INTERVAL) && cancelled() {
            return Err(());
        }
        let folded = UniCase::unicode(grapheme).to_folded_case();
        let compared = remaining.len().min(folded.len());
        if remaining[..compared] != folded.as_bytes()[..compared] {
            return Ok(None);
        }
        if remaining.len() <= folded.len() {
            return Ok(Some(start + grapheme.len()));
        }
        remaining = &remaining[folded.len()..];
    }
    Ok(None)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod actor_tests;
