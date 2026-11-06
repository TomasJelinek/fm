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
        let mut snapshots = Vec::with_capacity(targets.len());
        for target in targets {
            snapshots.push(target.metadata_snapshot(cancel)?);
        }
        let mut edit = Self {
            preview: MetadataPreview {
                id,
                revision: 0,
                patch: MetadataPatch::default(),
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
        edit.update(0, MetadataPatch::default(), targets, cancel);
        Ok(edit)
    }

    pub(super) fn update(
        &mut self,
        revision: u64,
        mut patch: MetadataPatch,
        targets: &[Arc<dyn PreparedMutation>],
        cancel: &Cancellation,
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
            let mut issue = validate_patch(&patch, c, path).err();
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
            if self.preview.sample.len() < 32 {
                self.preview.sample.push(MetadataPreviewRow {
                    path: target.item().path.clone(),
                    proposed_name: self.names[index].clone(),
                    current: self.snapshots[index].clone(),
                    issue,
                });
            }
        }
    }
}

struct Frame {
    target: Arc<dyn PreparedMutation>,
    visited: bool,
    depth: usize,
    issue: Option<FsError>,
    name: Option<OsString>,
    // Target is declared first: its native handles close before this lease is returned.
    _permit: Option<tokio::sync::OwnedSemaphorePermit>,
}

pub(super) fn execute(job: worker::Job, outbox: Outbox) -> OperationProgress {
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
    for (root_index, root) in job.targets.iter().enumerate() {
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
            _permit: None,
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
                // Reserve before native enumeration opens any child handles. Other prepared
                // jobs share the same semaphore; no batch may temporarily exceed the budget.
                let mut permits = loop {
                    let available = job.metadata_budget.available_permits();
                    if let Ok(permits) = job
                        .metadata_budget
                        .clone()
                        .try_acquire_many_owned(available as u32)
                    {
                        break permits;
                    }
                };
                let children = if frame.depth >= MAX_METADATA_DEPTH {
                    Err(error(
                        &frame.target.item().path,
                        io::ErrorKind::Other,
                        "metadata traversal exceeds its depth budget",
                    ))
                } else {
                    frame
                        .target
                        .metadata_children(&job.cancel, permits.num_permits())
                };
                match children {
                    Ok(children)
                        if children.len() <= permits.num_permits()
                            && children.len() + stack.len() + job.targets.len()
                                <= MAX_METADATA_TARGETS =>
                    {
                        let mut child_edit =
                            match Edit::prepare(progress.id, &children, &job.cancel) {
                                Ok(edit) => edit,
                                Err(error) => {
                                    frame.issue = Some(error);
                                    stack.push(frame);
                                    continue;
                                }
                            };
                        child_edit.update(
                            edit.preview.revision,
                            patch.clone(),
                            &children,
                            &job.cancel,
                        );
                        frame.visited = true;
                        let depth = frame.depth + 1;
                        stack.push(frame);
                        for (index, child) in children.into_iter().enumerate().rev() {
                            stack.push(Frame {
                                target: child,
                                visited: false,
                                depth,
                                issue: child_edit.issues[index].clone(),
                                name: child_edit.names[index].clone(),
                                _permit: Some(
                                    permits
                                        .split(1)
                                        .expect("native child count fits the reserved budget"),
                                ),
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
                    Err(error) => frame.issue = Some(error),
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
            if frame.depth == 0 && job.targets.len() == 1 && outcome.changes.renamed {
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
