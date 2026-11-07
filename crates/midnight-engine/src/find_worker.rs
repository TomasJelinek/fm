//! Bounded native traversal; no UI or host filesystem calls live in this worker.
#[path = "find_content.rs"]
mod content;
use content::ContentMatch;
use midnight_core::*;
use midnight_platform_common::filter::CompiledFilter;
use std::{
    io,
    mem::size_of,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

pub(crate) fn search(
    service: &dyn FindService,
    request: &FindRequest,
    cancel: &Cancellation,
    emit: &mut dyn FnMut(FindUpdate) -> bool,
) -> FindSummary {
    let mut work = Work {
        request,
        cancel,
        emit,
        progress: FindProgress::default(),
        budgets: Vec::new(),
        diagnostics: Vec::new(),
        retained: 0,
        batch: Vec::new(),
        cancelled: false,
        failed: false,
        last_progress: Instant::now(),
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
    let filter = match CompiledFilter::new(&PanelFilter {
        enabled: true,
        mode: FilterMode::Masks,
        expression: request.masks.clone(),
        case_sensitive: request.name_case_sensitive,
    }) {
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
    if !(work.emit)(FindUpdate::Started) {
        work.cancelled = true;
        return work.finish();
    }
    'roots: for root in &request.roots {
        if work.stopped() {
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
                Some((source, _)) => source.next_child(cancel),
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
            let mut matches = entry
                .path
                .file_name()
                .is_some_and(|name| filter.matches(name));
            if let Some(query) = &request.content {
                matches &= entry.kind == EntryKind::File;
                if matches {
                    matches = match content::matches(
                        source.as_mut(),
                        query,
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
                        if work.progress.results >= request.limits.max_results as u64 {
                            work.budget(FindBudget::Results);
                            break 'roots;
                        }
                        if !work.reserve(
                            entry_cost(source.entry())
                                + size_of::<FindResult>()
                                + 2 * size_of::<Arc<FindResult>>(),
                        ) {
                            break 'roots;
                        }
                        work.progress.results += 1;
                        work.batch.push(Arc::new(FindResult {
                            id: FindResultId(work.progress.results),
                            entry: source.entry().clone(),
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

struct Work<'a> {
    request: &'a FindRequest,
    cancel: &'a Cancellation,
    emit: &'a mut dyn FnMut(FindUpdate) -> bool,
    progress: FindProgress,
    budgets: Vec<FindBudget>,
    diagnostics: Vec<FsError>,
    retained: usize,
    batch: Vec<Arc<FindResult>>,
    cancelled: bool,
    failed: bool,
    last_progress: Instant,
}
impl Work<'_> {
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
        if self.last_progress.elapsed() >= Duration::from_millis(250) {
            if !(self.emit)(FindUpdate::Progress(self.progress.clone())) {
                self.cancelled = true;
            }
            self.last_progress = Instant::now();
        }
    }
    fn finish(mut self) -> FindSummary {
        self.stopped();
        if !self.cancelled {
            self.flush();
        }
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
