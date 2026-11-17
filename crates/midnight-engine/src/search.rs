//! Cancellable in-memory Unicode prefix matching over immutable directory snapshots.

use crate::{Outbox, Snapshots};
use midnight_core::{
    DirectoryIndex, Entry, EntryKind, Event, FsError, PaneId, PanelFilter, SearchMatch,
    SearchRequest, SearchResult, SearchRevision,
};
use std::{
    io,
    sync::{Arc, Weak},
};
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
    if state.network_navigation == Some(request.navigation)
        && state.network.as_ref().is_some_and(|(navigation, entries)| {
            *navigation == request.navigation && Arc::ptr_eq(entries, &request.source)
        })
    {
        return true;
    }
    if state.archive.as_ref().is_some_and(|(navigation, entries)| {
        *navigation == request.navigation && Arc::ptr_eq(entries, &request.source)
    }) {
        return true;
    }
    state.pending.is_none()
        && state.has_native_view()
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
    // The persistent-filter projection depends only on (source, filter); quick-search
    // keystrokes reuse it instead of re-indexing the whole directory.
    let mut projection: Option<Projection> = None;
    loop {
        if !pending && input.changed().await.is_err() {
            return;
        }
        pending = false;
        let intent = input.borrow_and_update().clone();
        let Some(request) = intent.request else {
            // Cancellation or navigation: release the old listing's projection.
            projection = None;
            continue;
        };
        let valid_source = source_matches(&snapshots, &request);
        let computation = if valid_source {
            let work_request = request.clone();
            let observer = input.clone();
            let cached = projection.take().filter(|cached| cached.serves(&request));
            match tokio::task::spawn_blocking(move || {
                compute(&work_request, cached, &|| {
                    observer.has_changed().unwrap_or(true)
                })
            })
            .await
            {
                Ok((computation, kept)) => {
                    projection = kept;
                    computation
                }
                Err(error) => Computation::Failed(FsError::from_io(
                    "search filenames",
                    &request.location,
                    io::Error::other(error.to_string()),
                )),
            }
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

/// Validated persistent-filter projection of one immutable source snapshot.
struct Projection {
    source: Weak<[Entry]>,
    filter: PanelFilter,
    filtered: Arc<[SearchMatch]>,
    index: Arc<DirectoryIndex>,
}

impl Projection {
    /// The weak source pins its allocation, so pointer equality cannot alias a
    /// later listing allocated at the same address.
    fn serves(&self, request: &SearchRequest) -> bool {
        self.filter == request.filter
            && Weak::ptr_eq(&self.source, &Arc::downgrade(&request.source))
    }
}

/// Returns the computation and the projection worth keeping for the next revision.
fn compute(
    request: &SearchRequest,
    cached: Option<Projection>,
    cancelled: &dyn Fn() -> bool,
) -> (Computation, Option<Projection>) {
    if cancelled() {
        return (Computation::Cancelled, cached);
    }
    let projection = match cached {
        Some(projection) => projection,
        None => match project(request, cancelled) {
            Ok(projection) => projection,
            Err(computation) => return (computation, None),
        },
    };
    let query = UniCase::unicode(&request.query).to_folded_case();
    if query.is_empty() {
        let result = SearchResult {
            source: request.source.clone(),
            filtered: projection.filtered.clone(),
            filtered_index: projection.index.clone(),
            matches: projection.filtered.clone(),
            index: projection.index.clone(),
        };
        return (Computation::Ready(result), Some(projection));
    }
    let mut matches = Vec::new();
    let mut index = DirectoryIndex::default();
    for (row, projected) in projection.filtered.iter().enumerate() {
        if row.is_multiple_of(CANCEL_INTERVAL) && cancelled() {
            return (Computation::Cancelled, Some(projection));
        }
        let entry = &request.source[projected.source_row];
        let display_prefix_end = match entry_prefix_end(entry, &query, cancelled) {
            Ok(Some(end)) => end,
            Ok(None) => continue,
            Err(()) => return (Computation::Cancelled, Some(projection)),
        };
        // Matches are a subset of the validated projection, so their identities
        // stay unique and cannot alias an unmatched source row.
        if !append_entry(&mut index, entry, matches.len()) {
            return (ambiguous_projection(request), None);
        }
        matches.push(SearchMatch {
            source_row: projected.source_row,
            display_prefix_end,
        });
    }
    if cancelled() {
        return (Computation::Cancelled, Some(projection));
    }
    let result = SearchResult {
        source: request.source.clone(),
        filtered: projection.filtered.clone(),
        filtered_index: projection.index.clone(),
        matches: matches.into(),
        index: Arc::new(index),
    };
    (Computation::Ready(result), Some(projection))
}

/// Build and validate the persistent-filter projection once per (source, filter).
fn project(
    request: &SearchRequest,
    cancelled: &dyn Fn() -> bool,
) -> Result<Projection, Computation> {
    let filter = midnight_platform_common::filter::CompiledPanelFilter::new(&request.filter)
        .map_err(|error| {
            Computation::Failed(FsError::from_io(
                "filter filenames",
                &request.location,
                io::Error::new(io::ErrorKind::InvalidInput, error.message),
            ))
        })?;
    let mut filtered = Vec::new();
    let mut index = DirectoryIndex::default();
    for (source_row, entry) in request.source.iter().enumerate() {
        if source_row.is_multiple_of(CANCEL_INTERVAL) && cancelled() {
            return Err(Computation::Cancelled);
        }
        if !filter.matches_entry(entry) {
            continue;
        }
        if !append_entry(&mut index, entry, filtered.len()) {
            return Err(ambiguous_projection(request));
        }
        filtered.push(SearchMatch {
            source_row,
            display_prefix_end: 0,
        });
    }
    // A projected id/path must not also refer to a filtered-out source row. Validate
    // against the source without allocating another full-directory identity map.
    for (source_row, entry) in request.source.iter().enumerate() {
        if source_row.is_multiple_of(CANCEL_INTERVAL) && cancelled() {
            return Err(Computation::Cancelled);
        }
        if let Some(row) = index.by_id.get(&entry.id)
            && filtered[*row].source_row != source_row
        {
            return Err(ambiguous_projection(request));
        }
        if let Some(path) = entry.native_path()
            && let Some(id) = index.by_path.get(path)
            && let Some(row) = index.by_id.get(id)
            && filtered[*row].source_row != source_row
        {
            return Err(ambiguous_projection(request));
        }
    }
    Ok(Projection {
        source: Arc::downgrade(&request.source),
        filter: request.filter.clone(),
        filtered: filtered.into(),
        index: Arc::new(index),
    })
}

/// Incremental counterpart of `DirectoryIndex::from_entries` for projected rows.
/// Like it, only native rows have a path identity: archive members and network
/// nodes (whose `path` is empty) are identified by their entry ID alone.
fn append_entry(index: &mut DirectoryIndex, entry: &Entry, row: usize) -> bool {
    if index.by_id.insert(entry.id, row).is_some()
        || entry
            .native_path()
            .is_some_and(|path| index.by_path.insert(path.to_owned(), entry.id).is_some())
    {
        return false;
    }
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
    true
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

/// Match original Unicode names against an already folded prefix. A decorated caption
/// may report a match but cannot borrow an original-name byte range for highlighting.
pub(crate) fn entry_prefix_end(
    entry: &Entry,
    query: &str,
    cancelled: &dyn Fn() -> bool,
) -> Result<Option<usize>, ()> {
    let name = if let Some(node) = &entry.virtual_node {
        std::str::from_utf8(&node.raw_name).ok()
    } else {
        entry.name.to_str()
    };
    let Some(name) = name else { return Ok(None) };
    prefix_end(name, query, cancelled)
        .map(|matched| matched.map(|end| if entry.display_name == name { end } else { 0 }))
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
