//! Bounded native traversal; no UI or host filesystem calls live in this worker.
#[path = "find_content.rs"]
mod content;
#[path = "find_duplicates.rs"]
mod duplicates;
use content::ContentMatch;
use midnight_core::*;
#[path = "find_predicate.rs"]
mod predicate;
use std::{
    io,
    mem::size_of,
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime},
};

/// Progress samples and partial result batches are published at most this often.
pub(crate) const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Clone, Default)]
pub(crate) struct CurrentDirectory(Arc<Mutex<Option<Arc<std::path::PathBuf>>>>);
impl CurrentDirectory {
    pub(crate) fn get(&self) -> Option<Arc<std::path::PathBuf>> {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }
    fn set(&self, path: &Path) -> bool {
        // Status resource bound, never path truncation or a replacement identity.
        if path.as_os_str().as_encoded_bytes().len() > 1024 * 1024 {
            return false;
        }
        let mut current = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if current.as_deref().is_none_or(|old| old != path) {
            *current = Some(Arc::new(path.to_owned()));
        }
        true
    }
}

/// Searches the native `service`. `locale` formats the modification time of each
/// published result: never of an examined node that does not match, nor of a
/// duplicate candidate that joins no group or whose group the result limit omits.
/// A group is formatted before its strings are charged, so the one group that the
/// retained-bytes budget refuses (which ends the search) is formatted unpublished.
pub(crate) fn search_with_location(
    service: &dyn FindService,
    locale: Option<&dyn LocaleService>,
    request: &FindRequest,
    cancel: &Cancellation,
    emit: &mut dyn FnMut(FindUpdate) -> bool,
    location: &CurrentDirectory,
    accepted_at: SystemTime,
) -> FindSummary {
    search_with_progress_interval(
        service,
        locale,
        request,
        cancel,
        emit,
        location,
        accepted_at,
        PROGRESS_INTERVAL,
    )
}

/// `search_with_location` publishing progress and partial batches at most
/// once per `progress_interval` (tests drive the cadence without real waits).
#[allow(
    clippy::too_many_arguments,
    reason = "the production seam plus its test cadence"
)]
pub(crate) fn search_with_progress_interval(
    service: &dyn FindService,
    locale: Option<&dyn LocaleService>,
    request: &FindRequest,
    cancel: &Cancellation,
    emit: &mut dyn FnMut(FindUpdate) -> bool,
    location: &CurrentDirectory,
    accepted_at: SystemTime,
    progress_interval: Duration,
) -> FindSummary {
    let mut work = Work {
        request,
        cancel,
        emit,
        locale,
        progress: FindProgress::default(),
        budgets: Vec::new(),
        diagnostics: Vec::new(),
        retained: 0,
        batch: Vec::new(),
        cancelled: false,
        failed: false,
        last_progress: Instant::now(),
        progress_interval,
    };
    if cancel.is_cancelled() {
        work.cancelled = true;
        return work.finish();
    }
    if let Err(error) = request.validate() {
        work.failed = true;
        work.error(find_error(
            Path::new(""),
            io::ErrorKind::InvalidInput,
            error,
        ));
        return work.finish();
    }
    let filter = match predicate::Predicate::new(request) {
        Ok(filter) => filter,
        Err(error) => {
            work.failed = true;
            work.error(find_error(
                Path::new(""),
                io::ErrorKind::InvalidInput,
                &error,
            ));
            return work.finish();
        }
    };
    let mut content = match request
        .content
        .as_ref()
        .map(content::PreparedContent::new)
        .transpose()
    {
        Ok(content) => content,
        Err(message) => {
            work.failed = true;
            work.error(find_error(
                Path::new(""),
                io::ErrorKind::InvalidInput,
                message,
            ));
            return work.finish();
        }
    };
    if !(work.emit)(FindUpdate::Started) {
        work.cancelled = true;
        return work.finish();
    }
    let mut candidates = duplicates::Candidates::default();
    'roots: for root in &request.roots {
        if work.stopped() {
            break;
        }
        if !location.set(root) {
            work.budget(FindBudget::RetainedBytes);
            break;
        }
        let source = match service.open_find_source(root, cancel) {
            Ok(source) if source.entry().kind == EntryKind::Directory => source,
            Ok(_) => {
                work.error(find_error(
                    root,
                    io::ErrorKind::InvalidInput,
                    "Find starting locations must be ordinary directories.",
                ));
                continue;
            }
            Err(error) => {
                work.error(error);
                continue;
            }
        };
        let root_cost = entry_cost(source.entry());
        if !work.reserve(root_cost) {
            break;
        }
        let mut stack = vec![(source, root_cost)];
        work.progress.directories += 1;
        while !stack.is_empty() {
            if work.stopped() {
                break 'roots;
            }
            let next = match stack.last_mut() {
                Some((source, _)) => {
                    if !location.set(&source.entry().path) {
                        work.budget(FindBudget::RetainedBytes);
                        break 'roots;
                    }
                    source.next_child(cancel)
                }
                None => break,
            };
            let child = match next {
                Ok(Some(child)) => child,
                Ok(None) => {
                    if let Some((_, cost)) = stack.pop() {
                        work.retained -= cost;
                    }
                    continue;
                }
                Err(error) => {
                    work.error(error);
                    if let Some((_, cost)) = stack.pop() {
                        work.retained -= cost;
                    }
                    continue;
                }
            };
            if work.progress.examined >= request.limits.max_nodes {
                work.budget(FindBudget::Nodes);
                break 'roots;
            }
            work.progress.examined += 1;
            let mut source = match child {
                FindChild::Entry(source) => source,
                FindChild::Skipped { reason, .. } => {
                    match reason {
                        FindSkipReason::Link => work.progress.skipped_links += 1,
                        FindSkipReason::Special => work.progress.skipped_special += 1,
                        FindSkipReason::MountBoundary => work.progress.skipped_mounts += 1,
                    }
                    work.progress_if_due();
                    continue;
                }
                FindChild::Failed(error) => {
                    work.error(error);
                    work.progress_if_due();
                    continue;
                }
            };
            match source.entry().kind {
                EntryKind::Directory => work.progress.directories += 1,
                EntryKind::File => work.progress.files += 1,
                _ => {
                    work.error(find_error(
                        &source.entry().path,
                        io::ErrorKind::Unsupported,
                        "Find provider returned a non-ordinary object.",
                    ));
                    continue;
                }
            }
            let entry = source.entry();
            let mut matches = filter.matches(entry)
                && request.modified_within.is_none_or(|maximum| {
                    entry.metadata.modified.is_some_and(|modified| {
                        accepted_at
                            .duration_since(modified)
                            .is_ok_and(|age| age <= maximum)
                    })
                });
            if request.duplicates {
                matches &= entry.kind == EntryKind::File;
            }
            if let Some(content) = &mut content {
                matches &= entry.kind == EntryKind::File;
                if matches {
                    matches = match content.matches(
                        source.as_mut(),
                        request.limits,
                        &mut work.progress,
                        cancel,
                    ) {
                        ContentMatch::Match(matches) => matches,
                        ContentMatch::Limited(budget) => {
                            work.progress.limited_files += 1;
                            work.budget(budget);
                            false
                        }
                        ContentMatch::Failed(error) => {
                            work.error(error);
                            false
                        }
                        ContentMatch::Cancelled => {
                            work.cancelled = true;
                            break 'roots;
                        }
                    };
                }
            }
            if matches {
                match source.revalidate(cancel) {
                    Ok(()) => {
                        if request.duplicates {
                            // Candidates stay unformatted; a published group formats
                            // the copies it publishes.
                            if !candidates.collect(source.entry(), &mut work) {
                                break 'roots;
                            }
                            work.progress_if_due();
                            continue;
                        }
                        if work.progress.results >= request.limits.max_results as u64 {
                            work.budget(FindBudget::Results);
                            break 'roots;
                        }
                        let entry = work.localized(source.entry());
                        if !work.reserve(
                            entry_cost(&entry)
                                + size_of::<FindResult>()
                                + 2 * size_of::<Arc<FindResult>>(),
                        ) {
                            break 'roots;
                        }
                        work.progress.results += 1;
                        work.batch.push(Arc::new(FindResult {
                            id: FindResultId(work.progress.results),
                            entry,
                            duplicate_group: None,
                            duplicate_group_size: 0,
                        }));
                        if work.batch.len() == FIND_BATCH_SIZE && !work.flush() {
                            break 'roots;
                        }
                    }
                    Err(error) => work.error(error),
                }
            }
            if request.recursive && source.entry().kind == EntryKind::Directory {
                if stack.len() >= request.limits.max_depth {
                    work.budget(FindBudget::Depth);
                } else if stack
                    .iter()
                    .any(|(ancestor, _)| ancestor.entry().identity == source.entry().identity)
                {
                    work.error(find_error(
                        &source.entry().path,
                        io::ErrorKind::InvalidData,
                        "Find refused a directory identity cycle.",
                    ));
                } else {
                    let cost = entry_cost(source.entry());
                    if !work.reserve(cost) {
                        break 'roots;
                    }
                    stack.push((source, cost));
                }
            }
            work.progress_if_due();
        }
    }
    if request.duplicates && !work.stopped() {
        candidates.examine(service, &mut work, location);
    }
    work.finish()
}

pub(crate) fn validate_result(
    service: &dyn FindService,
    captured: &FindEntry,
    cancel: &Cancellation,
) -> Result<FindEntry, FsError> {
    let source = service.open_find_source(&captured.path, cancel)?;
    if !source.entry().matches_capture(captured) {
        return Err(find_error(
            &captured.path,
            io::ErrorKind::InvalidData,
            "Find result identity or revision changed; search again.",
        ));
    }
    source.revalidate(cancel)?;
    Ok(source.entry().clone())
}

/// Fills a missing localized modification time of a Find entry, as panel
/// listings do (`localize_metadata`).
pub(crate) fn localize(entry: &mut FindEntry, locale: Option<&dyn LocaleService>) {
    localize_metadata(&mut entry.metadata, locale);
}

/// The engine's one fill-when-absent rule for localized modification times, used by
/// listings, archive and transfer timestamps and Find results (lib.rs re-exports it
/// as `crate::localize_metadata`). It lives here because tests compile this worker
/// standalone. A time the provider formatted is kept; without a locale the UI shows
/// its labelled UTC fallback.
pub(crate) fn localize_metadata(metadata: &mut EntryMetadata, locale: Option<&dyn LocaleService>) {
    if metadata.localized_modified.is_none()
        && let (Some(locale), Some(modified)) = (locale, metadata.modified)
    {
        metadata.localized_modified = locale.format_datetime(modified);
    }
}

struct Work<'a> {
    request: &'a FindRequest,
    cancel: &'a Cancellation,
    emit: &'a mut dyn FnMut(FindUpdate) -> bool,
    locale: Option<&'a dyn LocaleService>,
    progress: FindProgress,
    budgets: Vec<FindBudget>,
    diagnostics: Vec<FsError>,
    retained: usize,
    batch: Vec<Arc<FindResult>>,
    cancelled: bool,
    failed: bool,
    last_progress: Instant,
    progress_interval: Duration,
}
impl Work<'_> {
    /// A published copy of a result or duplicate group member with its localized
    /// time. Nodes that never match and candidates that join no group are not
    /// formatted: a name search over a million nodes formats only its results.
    fn localized(&self, entry: &FindEntry) -> FindEntry {
        let mut entry = entry.clone();
        localize(&mut entry, self.locale);
        entry
    }
    fn stopped(&mut self) -> bool {
        self.cancelled |= self.cancel.is_cancelled();
        self.cancelled
    }
    fn budget(&mut self, budget: FindBudget) {
        if !self.budgets.contains(&budget) {
            self.budgets.push(budget);
        }
    }
    fn reserve(&mut self, bytes: usize) -> bool {
        match self.retained.checked_add(bytes) {
            Some(total) if total <= self.request.limits.max_retained_bytes => {
                self.retained = total;
                true
            }
            _ => {
                self.budget(FindBudget::RetainedBytes);
                false
            }
        }
    }
    fn error(&mut self, error: FsError) {
        if self.cancel.is_cancelled() {
            self.cancelled = true;
            return;
        }
        self.progress.errors += 1;
        if self.diagnostics.len() < MAX_FIND_DIAGNOSTICS {
            let cost = size_of::<FsError>()
                + error.path.as_os_str().as_encoded_bytes().len()
                + error.message.len();
            if self.reserve(cost) {
                self.diagnostics.push(error);
            }
        }
    }
    fn flush(&mut self) -> bool {
        if self.batch.is_empty() {
            return true;
        }
        let batch = std::mem::take(&mut self.batch).into();
        if !(self.emit)(FindUpdate::Batch(batch)) {
            self.cancelled = true;
            return false;
        }
        true
    }
    fn progress_if_due(&mut self) {
        if self.last_progress.elapsed() >= self.progress_interval {
            // A partial batch follows the progress cadence, so a sparse search
            // shows its matches while traversal continues.
            if self.flush() && !(self.emit)(FindUpdate::Progress(self.progress.clone())) {
                self.cancelled = true;
            }
            self.last_progress = Instant::now();
        }
    }
    fn finish(mut self) -> FindSummary {
        self.stopped();
        // Stop retains every match already found and revalidated: the controller
        // still accepts this generation's final batch after cancellation, while
        // Close, restart and shutdown reject it.
        self.flush();
        let completion = if self.cancelled {
            FindCompletion::Cancelled
        } else if self.failed {
            FindCompletion::Failed
        } else if self.progress.errors != 0 || !self.budgets.is_empty() {
            FindCompletion::Incomplete
        } else {
            FindCompletion::Completed
        };
        FindSummary {
            completion,
            progress: self.progress,
            reached_budgets: self.budgets,
            diagnostics: self.diagnostics,
        }
    }
}
fn entry_cost(entry: &FindEntry) -> usize {
    size_of::<FindEntry>()
        + entry.path.as_os_str().as_encoded_bytes().len()
        + entry
            .metadata
            .localized_modified
            .as_ref()
            .map_or(0, |date| date.date.len() + date.time.len())
}
