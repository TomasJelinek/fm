//! Typed metadata previews and iterative postorder execution on the mutation worker.
use super::*;
use midnight_platform_common::metadata::{case_name, validate_patch};
use std::{
    collections::HashMap,
    ffi::OsString,
    io,
    path::Path,
    time::{Instant, SystemTime},
};
use tokio::sync::{OwnedSemaphorePermit, TryAcquireError};

pub(super) struct Edit {
    pub(super) preview: MetadataPreview,
    snapshots: Vec<MetadataSnapshot>,
    names: Vec<Option<OsString>>,
    issues: Vec<Option<FsError>>,
    pub(super) conflicts: MetadataConflictPolicy,
    pub(super) conflict_state: Arc<ConflictState>,
    pub(super) result: Arc<Mutex<Option<Arc<MetadataResult>>>>,
}

#[derive(Default)]
pub(super) struct ConflictState {
    next: AtomicU64,
    pending: Mutex<Option<(u64, std::sync::mpsc::SyncSender<MetadataConflictPolicy>)>>,
}

impl ConflictState {
    /// Whether the worker currently awaits a decision (it registers before sending).
    #[cfg(test)]
    pub(super) fn is_pending(&self) -> bool {
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
    }
    pub(super) fn resolve(&self, token: u64, choice: MetadataConflictPolicy) {
        let pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((expected, sender)) = pending.as_ref()
            && *expected == token
        {
            let _ = sender.try_send(choice);
        }
    }
    fn ask(
        &self,
        id: OperationId,
        source: std::path::PathBuf,
        destination: std::path::PathBuf,
        outbox: &Outbox,
        cancel: &Cancellation,
    ) -> MetadataConflictPolicy {
        let token = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        *self.pending.lock().unwrap_or_else(|e| e.into_inner()) = Some((token, sender));
        let sent = outbox.blocking_send(
            Event::Operation(OperationEvent::MetadataConflict(Arc::new(
                MetadataConflict {
                    id,
                    token,
                    source,
                    destination,
                },
            ))),
            cancel,
        );
        let decision = if sent {
            loop {
                if cancel.is_cancelled() {
                    break MetadataConflictPolicy::Cancel;
                }
                match receiver.recv_timeout(Duration::from_millis(25)) {
                    Ok(choice) => break choice,
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    Err(_) => break MetadataConflictPolicy::Cancel,
                }
            }
        } else {
            MetadataConflictPolicy::Cancel
        };
        *self.pending.lock().unwrap_or_else(|e| e.into_inner()) = None;
        decision
    }
}

fn error(path: &Path, kind: io::ErrorKind, text: &str) -> FsError {
    FsError::from_io("metadata operation", path, io::Error::new(kind, text))
}

impl Edit {
    pub(super) fn prepare(
        id: OperationId,
        targets: &[Arc<dyn PreparedMutation>],
        cancel: &Cancellation,
    ) -> Result<Self, FsError> {
        Self::with_snapshots(id, 0, MetadataPatch::default(), targets, cancel)
    }

    /// Recursive execution: freshly enumerated and pinned children are read once
    /// and the confirmed patch is applied directly, without a preview refresh.
    pub(super) fn prepare_confirmed(
        id: OperationId,
        revision: u64,
        patch: MetadataPatch,
        targets: &[Arc<dyn PreparedMutation>],
        cancel: &Cancellation,
    ) -> Result<Self, FsError> {
        Self::with_snapshots(id, revision, patch, targets, cancel)
    }

    fn with_snapshots(
        id: OperationId,
        revision: u64,
        patch: MetadataPatch,
        targets: &[Arc<dyn PreparedMutation>],
        cancel: &Cancellation,
    ) -> Result<Self, FsError> {
        let mut snapshots = Vec::with_capacity(targets.len());
        for target in targets {
            snapshots.push(target.metadata_snapshot(cancel)?);
        }
        let mut edit = Self {
            preview: MetadataPreview {
                id,
                revision: 0,
                patch: MetadataPatch::default(),
                localized_times: Default::default(),
                time_input_format: None,
                capabilities: MetadataCapabilities::default(),
                total: targets.len() as u64,
                conflicts: 0,
                unsupported: 0,
                hard_linked: 0,
                sample: vec![],
                errors: vec![],
            },
            snapshots,
            names: vec![],
            issues: vec![],
            conflicts: MetadataConflictPolicy::Skip,
            conflict_state: Arc::default(),
            result: Arc::default(),
        };
        edit.apply(revision, patch, targets, cancel, false);
        Ok(edit)
    }

    /// Native locale discovery and formatting stay on this preparation worker.
    pub(super) fn localize(&mut self, locale: Option<&dyn LocaleService>) {
        self.preview.localized_times.clear();
        self.preview.time_input_format = locale.and_then(LocaleService::datetime_input_format);
        let Some(locale) = locale else {
            return;
        };
        let times = self
            .preview
            .sample
            .iter()
            .flat_map(|row| {
                [&row.current, &row.proposed]
                    .into_iter()
                    .flat_map(|snapshot| {
                        [snapshot.created, snapshot.modified, snapshot.accessed]
                            .into_iter()
                            .flatten()
                    })
            })
            .chain(
                [
                    self.preview.patch.created,
                    self.preview.patch.modified,
                    self.preview.patch.accessed,
                ]
                .into_iter()
                .filter_map(|edit| match edit {
                    TimeEdit::Set(value) => Some(value),
                    _ => None,
                }),
            );
        for value in times {
            if let std::collections::hash_map::Entry::Vacant(entry) =
                self.preview.localized_times.entry(value)
                && let Some(text) = locale.format_datetime(value)
            {
                entry.insert(text);
            }
        }
    }

    pub(super) fn update(
        &mut self,
        revision: u64,
        patch: MetadataPatch,
        targets: &[Arc<dyn PreparedMutation>],
        cancel: &Cancellation,
    ) {
        self.apply(revision, patch, targets, cancel, revision != 0);
    }

    fn apply(
        &mut self,
        revision: u64,
        mut patch: MetadataPatch,
        targets: &[Arc<dyn PreparedMutation>],
        cancel: &Cancellation,
        refresh: bool,
    ) {
        patch.resolve_now(SystemTime::now());
        self.preview.revision = revision;
        self.preview.patch = patch.clone();
        self.preview.conflicts = 0;
        self.preview.unsupported = 0;
        self.preview.hard_linked = 0;
        self.preview.errors.clear();
        self.preview.sample.clear();
        self.names.clear();
        self.issues.clear();
        // Re-read the pinned targets for every explicit refresh. Native adapters
        // revalidate identity; failed reads must not publish cached values as current.
        let snapshot_errors = targets
            .iter()
            .zip(&mut self.snapshots)
            .map(|(target, snapshot)| {
                if !refresh {
                    return None;
                }
                match target.metadata_snapshot(cancel) {
                    Ok(current) => {
                        *snapshot = current;
                        None
                    }
                    Err(error) => {
                        *snapshot = MetadataSnapshot::default();
                        Some(error)
                    }
                }
            })
            .collect::<Vec<_>>();
        let mut caps = self
            .snapshots
            .first()
            .map(|s| s.capabilities)
            .unwrap_or_default();
        let mut destinations = HashMap::<std::path::PathBuf, usize>::new();
        for (index, (target, snapshot)) in targets.iter().zip(&self.snapshots).enumerate() {
            let path = &target.item().path;
            let c = snapshot.capabilities;
            caps.windows_attributes &= c.windows_attributes;
            caps.unix_permissions &= c.unix_permissions;
            caps.compressed &= c.compressed;
            caps.created &= c.created;
            caps.modified &= c.modified;
            caps.accessed &= c.accessed;
            caps.rename &= c.rename;
            self.preview.hard_linked += u64::from(snapshot.hard_links > 1);
            let mut issue = snapshot_errors[index]
                .clone()
                .or_else(|| validate_patch(&patch, c, path).err());
            let mut name = None;
            if cancel.is_cancelled() {
                issue = Some(error(path, io::ErrorKind::Interrupted, "preview cancelled"));
            }
            if issue.is_none()
                && let Some(change) = patch.case
            {
                match case_name(path.file_name().unwrap_or_default(), change) {
                    Ok(proposed) => {
                        issue = target.metadata_name_available(&proposed, cancel).err();
                        let destination = path.with_file_name(&proposed);
                        if let Some(previous) = destinations.insert(destination, index) {
                            let conflict = error(
                                path,
                                io::ErrorKind::AlreadyExists,
                                "multiple selected names produce the same destination",
                            );
                            self.issues[previous] = Some(conflict.clone());
                            issue = Some(conflict);
                        }
                        name = Some(proposed);
                    }
                    Err(error) => issue = Some(error),
                }
            }
            self.names.push(name);
            self.issues.push(issue);
        }
        self.preview.capabilities = caps;
        for (index, target) in targets.iter().enumerate() {
            let issue = self.issues[index].clone();
            if let Some(error) = &issue {
                if error.kind == io::ErrorKind::AlreadyExists {
                    self.preview.conflicts += 1;
                } else {
                    self.preview.unsupported += 1;
                }
                if self.preview.errors.len() < 16 {
                    self.preview.errors.push(error.clone());
                }
            }
            if self.preview.sample.len() < MAX_METADATA_TARGETS {
                self.preview.sample.push(MetadataPreviewRow {
                    path: target.item().path.clone(),
                    proposed_name: self.names[index].clone(),
                    current: self.snapshots[index].clone(),
                    proposed: proposed_metadata(&self.snapshots[index], &patch),
                    issue,
                });
            }
        }
    }
}

fn proposed_metadata(current: &MetadataSnapshot, patch: &MetadataPatch) -> MetadataSnapshot {
    let mut proposed = current.clone();
    if let Some(value) = proposed.windows_attributes {
        proposed.windows_attributes =
            Some((value & !patch.windows_mask) | (patch.windows_value & patch.windows_mask));
    }
    if let Some(value) = proposed.unix_mode {
        proposed.unix_mode =
            Some((value & !patch.unix_mask) | (patch.unix_value & patch.unix_mask));
    }
    if let Some(value) = patch.compressed {
        proposed.compressed = Some(value);
    }
    for (value, edit) in [
        (&mut proposed.created, patch.created),
        (&mut proposed.modified, patch.modified),
        (&mut proposed.accessed, patch.accessed),
    ] {
        if let TimeEdit::Set(time) = edit {
            *value = Some(time);
        }
    }
    proposed
}

struct Frame {
    target: Arc<dyn PreparedMutation>,
    visited: bool,
    depth: usize,
    issue: Option<FsError>,
    name: Option<OsString>,
    // Target is declared first: its native handles close before this pin is returned.
    pin: Option<Pin>,
}

impl Frame {
    fn holds_shared_pin(&self) -> bool {
        self.pin.as_ref().is_some_and(|pin| pin.shared)
    }
}

/// One pinned traversal target; dropping it returns the slot to its pool.
struct Pin {
    _permit: OwnedSemaphorePermit,
    /// Borrowed from the shared budget rather than the executing job's reserve.
    shared: bool,
}

/// Pins taken for one directory's children before native enumeration opens any of them.
/// Declared before the enumerated children, so their native handles close first.
#[derive(Default)]
struct Lease {
    reserve: Option<OwnedSemaphorePermit>,
    shared: Option<OwnedSemaphorePermit>,
}

impl Lease {
    fn count(&self) -> usize {
        self.reserve_count() + self.shared_count()
    }
    fn reserve_count(&self) -> usize {
        self.reserve
            .as_ref()
            .map_or(0, OwnedSemaphorePermit::num_permits)
    }
    fn shared_count(&self) -> usize {
        self.shared
            .as_ref()
            .map_or(0, OwnedSemaphorePermit::num_permits)
    }
    /// Returns the pins the listed children do not need, borrowed shared ones first.
    fn trim(&mut self, needed: usize) {
        let mut excess = self.count().saturating_sub(needed);
        for permits in [&mut self.shared, &mut self.reserve].into_iter().flatten() {
            let released = excess.min(permits.num_permits());
            drop(permits.split(released));
            excess -= released;
        }
    }
    /// One child pin, from the reserve first.
    fn pin(&mut self) -> Option<Pin> {
        if let Some(permit) = self.reserve.as_mut().and_then(|permits| permits.split(1)) {
            return Some(Pin {
                _permit: permit,
                shared: false,
            });
        }
        let permit = self.shared.as_mut()?.split(1)?;
        Some(Pin {
            _permit: permit,
            shared: true,
        })
    }
}

/// Takes up to `at_most` currently free permits and never waits: pending metadata jobs
/// keep their pins while they wait for the executing job, so any wait could deadlock.
/// A failed attempt means another acquirer took permits after they were counted.
fn take_free(pool: &Arc<Semaphore>, at_most: usize) -> Option<OwnedSemaphorePermit> {
    loop {
        let count = pool.available_permits().min(at_most);
        if count == 0 {
            return None;
        }
        match pool.clone().try_acquire_many_owned(count as u32) {
            Ok(permits) => return Some(permits),
            Err(TryAcquireError::Closed) => return None,
            Err(TryAcquireError::NoPermits) => {}
        }
    }
}

/// Snapshots and pins a directory's children with at most `cap` pins. The executing
/// job's reserve is used first. Free shared pins are borrowed only when no reserve pin
/// is free or the adapter refused the directory as larger than the pins taken; both
/// native adapters refuse that with `ErrorKind::Other` before opening any child.
fn list_children(
    target: &dyn PreparedMutation,
    job: &worker::Job,
    cap: usize,
    lease: &mut Lease,
) -> Result<Vec<Arc<dyn PreparedMutation>>, FsError> {
    if let Some(permits) = job
        .metadata_traversal
        .as_ref()
        .and_then(|reserve| take_free(reserve, cap))
    {
        lease.reserve = Some(permits);
        match target.metadata_children(&job.cancel, lease.count()) {
            Err(refusal) if refusal.kind == io::ErrorKind::Other && lease.count() < cap => {
                lease.shared = take_free(&job.metadata_budget, cap - lease.count());
                if lease.shared.is_none() {
                    return Err(refusal);
                }
            }
            listed => return listed,
        }
    } else {
        // Still list with no pins at all, so that an empty folder succeeds.
        lease.shared = take_free(&job.metadata_budget, cap);
    }
    target.metadata_children(&job.cancel, lease.count())
}

/// Names why a directory could not be pinned, instead of the adapter's size refusal.
/// Fewer pins than the per-job cap means borrowing took every free shared target, so
/// all shared targets then belonged to this job (`own_shared`) or to other pending jobs.
fn pin_shortfall(path: &Path, pinned: usize, cap: usize, own_shared: usize) -> FsError {
    let size = match pinned {
        0 => "none of this folder's entries could be pinned".to_owned(),
        pinned => format!("this folder has more entries than the {pinned} that could be pinned"),
    };
    let cause = if pinned < cap {
        let others = MAX_METADATA_TARGETS.saturating_sub(own_shared);
        format!(
            "other pending metadata jobs hold {others} of the {MAX_METADATA_TARGETS} \
             shared targets"
        )
    } else {
        format!(
            "one metadata job pins at most {MAX_METADATA_TARGETS} targets, including its \
             selection and the folders it is still processing"
        )
    };
    error(
        path,
        io::ErrorKind::Other,
        &format!("{size}, because {cause}; this folder and its contents were not changed"),
    )
}

pub(super) fn execute(mut job: worker::Job, outbox: Outbox) -> OperationProgress {
    let edit = job.metadata.as_ref().expect("metadata job");
    let patch = &edit.preview.patch;
    let mut progress = job.progress();
    let mut result = MetadataResult {
        id: progress.id,
        processed: 0,
        changed: 0,
        skipped: 0,
        failed: 0,
        sample: vec![],
    };
    let mut last = Instant::now();
    let mut conflict_policy = edit.conflicts;
    if patch.recursive {
        progress.total = progress.total.saturating_sub(
            job.metadata_covered
                .iter()
                .filter(|covered| **covered)
                .count() as u64,
        );
    }
    // Completed descendant pins must close before a selected ancestor is renamed.
    // Windows can refuse parent rename while a completed child still has open handles.
    // The retained capture stays in job until every root/descendant pin has closed.
    let root_count = job.targets.len();
    let roots = std::mem::take(&mut job.targets);
    for (root_index, root) in roots.into_iter().enumerate() {
        // Typed occurrence ancestry, captured before preview, handles selected overlaps.
        // The covering root walks this subtree once; nonrecursive edits keep every root.
        if patch.recursive && job.metadata_covered.get(root_index) == Some(&true) {
            continue;
        }
        if job.cancel.is_cancelled() {
            break;
        }
        let before_failures = result.failed;
        let mut stack = vec![Frame {
            target: root.clone(),
            visited: false,
            depth: 0,
            issue: edit.issues[root_index].clone(),
            name: edit.names[root_index].clone(),
            pin: None,
        }];
        while let Some(mut frame) = stack.pop() {
            if job.cancel.is_cancelled() {
                break;
            }
            if !frame.visited
                && frame.issue.is_none()
                && patch.recursive
                && frame.target.item().kind == EntryKind::Directory
            {
                if frame.depth >= MAX_METADATA_DEPTH {
                    // Checked first: a depth refusal never reserves or borrows pins.
                    frame.issue = Some(error(
                        &frame.target.item().path,
                        io::ErrorKind::Other,
                        "metadata traversal exceeds its depth budget",
                    ));
                } else {
                    // Pins are taken before native enumeration opens any child. This job's
                    // selection, pending frames and these children stay within the
                    // per-job budget, and no pin is ever waited for (E6-7).
                    let cap = MAX_METADATA_TARGETS.saturating_sub(stack.len() + root_count);
                    let mut lease = Lease::default();
                    match list_children(frame.target.as_ref(), &job, cap, &mut lease) {
                        Ok(children) if children.len() <= lease.count() => {
                            // Unused pins return before the snapshot reads below.
                            lease.trim(children.len());
                            let child_edit = match Edit::prepare_confirmed(
                                progress.id,
                                edit.preview.revision,
                                patch.clone(),
                                &children,
                                &job.cancel,
                            ) {
                                Ok(edit) => edit,
                                Err(error) => {
                                    frame.issue = Some(error);
                                    stack.push(frame);
                                    continue;
                                }
                            };
                            // Children are processed in listing order. The first ones take
                            // the reserve pins, so the reserve refills as they finish.
                            let pins: Vec<_> = (0..children.len())
                                .map(|_| lease.pin().expect("one trimmed pin per child"))
                                .collect();
                            frame.visited = true;
                            let depth = frame.depth + 1;
                            stack.push(frame);
                            for ((index, child), pin) in
                                children.into_iter().enumerate().zip(pins).rev()
                            {
                                stack.push(Frame {
                                    target: child,
                                    visited: false,
                                    depth,
                                    issue: child_edit.issues[index].clone(),
                                    name: child_edit.names[index].clone(),
                                    pin: Some(pin),
                                });
                            }
                            continue;
                        }
                        Ok(_) => {
                            frame.issue = Some(error(
                                &frame.target.item().path,
                                io::ErrorKind::Other,
                                "metadata traversal exceeds its pinned-target budget",
                            ))
                        }
                        Err(refusal) if refusal.kind == io::ErrorKind::Other => {
                            let own_shared = root_count
                                + lease.shared_count()
                                + usize::from(frame.holds_shared_pin())
                                + stack
                                    .iter()
                                    .filter(|other| other.holds_shared_pin())
                                    .count();
                            frame.issue = Some(pin_shortfall(
                                &frame.target.item().path,
                                lease.count(),
                                cap,
                                own_shared,
                            ));
                        }
                        Err(error) => frame.issue = Some(error),
                    }
                }
            }
            let outcome = if let Some(error) = frame.issue {
                let mut outcome = MetadataOutcome::unchanged(frame.target.item().path.clone());
                outcome.error = Some(error);
                outcome
            } else {
                frame
                    .target
                    .edit_metadata(patch, frame.name.as_deref(), &job.cancel)
            };
            result.processed += 1;
            result.changed += u64::from(outcome.changes.any());
            if frame.depth == 0 && root_count == 1 && outcome.changes.renamed {
                progress.renamed_to = Some(outcome.current_path.clone());
            }
            progress.current_path = Some(outcome.current_path.clone());
            if outcome.changes.any() {
                progress.processed_entries += 1;
            }
            if let Some(error) = &outcome.error {
                if error.kind == io::ErrorKind::AlreadyExists {
                    let choice = if conflict_policy == MetadataConflictPolicy::Skip {
                        let destination = frame.name.as_ref().map_or_else(
                            || outcome.current_path.clone(),
                            |name| outcome.original_path.with_file_name(name),
                        );
                        edit.conflict_state.ask(
                            progress.id,
                            outcome.original_path.clone(),
                            destination,
                            &outbox,
                            &job.cancel,
                        )
                    } else {
                        conflict_policy
                    };
                    match choice {
                        MetadataConflictPolicy::Cancel => job.cancel.cancel(),
                        MetadataConflictPolicy::Skip => result.skipped += 1,
                        MetadataConflictPolicy::SkipAll => {
                            result.skipped += 1;
                            conflict_policy = MetadataConflictPolicy::SkipAll;
                        }
                    }
                } else if error.kind != io::ErrorKind::Interrupted {
                    result.failed += 1;
                }
                if progress.errors.len() < 16 {
                    progress.errors.push(error.clone());
                }
            }
            if result.sample.len() < 32 {
                result.sample.push(outcome);
            }
            if last.elapsed() >= Duration::from_millis(250) {
                *job.status.lock().unwrap_or_else(|e| e.into_inner()) = progress.clone();
                if outbox
                    .sender
                    .try_send(Event::Operation(OperationEvent::Progress(Arc::new(
                        progress.clone(),
                    ))))
                    .is_ok()
                {
                    (outbox.wakeup)();
                }
                last = Instant::now();
            }
        }
        if result.failed > before_failures {
            progress.failed += 1;
        } else if !job.cancel.is_cancelled() {
            progress.completed += 1;
        }
    }
    progress.state = if job.cancel.is_cancelled() {
        OperationState::Cancelled
    } else if result.failed > 0 || result.skipped > 0 {
        OperationState::CompletedWithErrors
    } else {
        OperationState::Completed
    };
    progress.message = format!(
        "{} examined; {} changed; {} skipped; {} failed. Completed changes are retained.",
        result.processed, result.changed, result.skipped, result.failed
    );
    // Terminal delivery belongs to the async controller, after this worker releases native
    // handles and its mutation permit. Cancellation must retain all completed item outcomes.
    *edit.result.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(result));
    if job.cancel.is_cancelled() {
        progress.state = OperationState::Cancelled;
    }
    progress
}
