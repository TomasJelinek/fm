use super::*;
#[path = "operation_discovery.rs"]
mod discovery;
use std::{
    ffi::OsString,
    io,
    path::{Component, Path},
};

pub(super) struct Job {
    pub(super) cancel: Cancellation,
    pub(super) status: Status,
    pub(super) targets: Vec<Arc<dyn PreparedMutation>>,
    directory: Option<Arc<dyn PreparedDirectory>>,
    pub(super) name: Option<OsString>,
    pub(super) rename_conflicts: Arc<super::rename::ConflictState>,
    preview: OperationPreview,
    pub(super) metadata: Option<super::metadata::Edit>,
    pub(super) metadata_budget: Arc<Semaphore>,
    pub(super) predecessors: Arc<crate::copy_fence::Predecessors>,
    // Declared after targets so native resources close before permits are returned.
    metadata_permits: Vec<tokio::sync::OwnedSemaphorePermit>,
}

pub(super) fn initial(request: &PrepareOperation) -> OperationProgress {
    OperationProgress {
        id: request.id,
        kind: request.kind,
        state: OperationState::Preparing,
        pause_requested: false,
        paused: false,
        total: 0,
        completed: 0,
        discovery: Default::default(),
        failed: 0,
        processed_entries: 0,
        throughput: Arc::default(),
        current_path: None,
        message: String::new(),
        errors: vec![],
        source_directory: request.source.path.clone(),
        renamed_to: None,
    }
}
fn error(path: &Path, message: &str) -> FsError {
    FsError::from_io(
        "file operation",
        path,
        io::Error::new(io::ErrorKind::InvalidInput, message),
    )
}
pub(super) fn failed_status(status: &Status, error: FsError) -> OperationProgress {
    let mut progress = status.lock().unwrap_or_else(|e| e.into_inner()).clone();
    progress.state = if error.kind == io::ErrorKind::Interrupted {
        OperationState::Cancelled
    } else {
        OperationState::Failed
    };
    progress.pause_requested = false;
    progress.paused = false;
    progress.failed = u64::from(progress.state == OperationState::Failed);
    progress.message = error.to_string();
    progress.errors.push(error);
    progress
}
pub(super) fn panicked(status: &Status, message: String) -> OperationProgress {
    let path = status
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .source_directory
        .clone();
    failed_status(
        status,
        error(&path, &format!("Native worker failed: {message}")),
    )
}

pub(super) fn prepare(
    request: PrepareOperation,
    service: Option<Arc<dyn MutationService>>,
    cancel: Cancellation,
    status: Status,
    metadata_budget: Arc<Semaphore>,
) -> Result<Job, FsError> {
    let source = &request.source.path;
    let service =
        service.ok_or_else(|| error(source, "This filesystem does not support mutations."))?;
    let mut job = Job {
        cancel,
        status,
        targets: vec![],
        directory: None,
        name: None,
        rename_conflicts: Arc::default(),
        metadata: None,
        metadata_budget,
        predecessors: Default::default(),
        metadata_permits: vec![],
        preview: OperationPreview {
            id: request.id,
            kind: request.kind,
            files: 0,
            folders: 0,
            file_bytes: Some(0),
            sample: vec![],
        },
    };
    if job.cancel.is_cancelled() {
        return Err(FsError::from_io(
            "prepare",
            source,
            io::ErrorKind::Interrupted.into(),
        ));
    }
    if matches!(
        request.kind,
        MutationKind::CreateDirectory | MutationKind::CreateFile
    ) {
        job.directory = Some(service.prepare_create(source, &job.cancel)?);
    } else {
        let selected = matches!(
            request.kind,
            MutationKind::Delete(_) | MutationKind::EditMetadata
        ) && !request.selection.is_empty();
        // Resolve on a worker in visible listing order; IDs alone are never paths.
        for entry in request.source.entries.iter() {
            if job.cancel.is_cancelled() {
                return Err(FsError::from_io(
                    "prepare",
                    source,
                    io::ErrorKind::Interrupted.into(),
                ));
            }
            if !request.view.by_path.contains_key(&entry.path) {
                continue;
            }
            if !(if selected {
                request.selection.contains_path(&entry.path)
            } else {
                request.focused == Some(entry.id)
            }) {
                continue;
            }
            if entry.path.parent() != Some(source)
                || entry.path.file_name().is_none()
                || entry.name == ".."
                || entry.name == "."
            {
                return Err(error(
                    &entry.path,
                    "Only immediate entries of the captured folder are allowed.",
                ));
            }
            if request.kind == MutationKind::EditMetadata
                && job.targets.len() >= MAX_METADATA_TARGETS
            {
                return Err(error(
                    source,
                    "Metadata selection exceeds the 256 pinned-target budget.",
                ));
            }
            if request.kind == MutationKind::EditMetadata {
                let permit = job.metadata_budget.clone().try_acquire_owned()
                    .map_err(|_| error(source, "The shared metadata handle budget is in use. Close another preview or wait for its job."))?;
                job.metadata_permits.push(permit);
            }
            let prepared = service.prepare(&entry.path, &job.cancel)?;
            if prepared.item().path != entry.path {
                return Err(error(&entry.path, "Provider returned a different target."));
            }
            if matches!(
                prepared.item().kind,
                EntryKind::Directory | EntryKind::DirectoryLink
            ) {
                job.preview.folders += 1;
            } else {
                job.preview.files += 1;
                let item = prepared.item();
                let size = (item.kind == EntryKind::File)
                    .then_some(item.metadata.size)
                    .flatten();
                job.preview.file_bytes = job
                    .preview
                    .file_bytes
                    .zip(size)
                    .and_then(|(total, size)| total.checked_add(size));
            }
            if job.preview.sample.len() < 32 {
                job.preview.sample.push(prepared.item().clone());
            }
            job.targets.push(prepared);
        }
        if job.targets.is_empty() || (selected && job.targets.len() != request.selection.len()) {
            return Err(error(
                source,
                "Selection is no longer completely visible. Select the targets again.",
            ));
        }
    }
    job.status.lock().unwrap_or_else(|e| e.into_inner()).total = if job.directory.is_some() {
        1
    } else {
        job.targets.len() as u64
    };
    if request.kind == MutationKind::EditMetadata {
        job.metadata = Some(super::metadata::Edit::prepare(
            request.id,
            &job.targets,
            &job.cancel,
        )?);
    }
    Ok(job)
}

impl Job {
    pub(super) fn progress(&self) -> OperationProgress {
        self.status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    pub(super) fn set_state(&self, state: OperationState) {
        self.status.lock().unwrap_or_else(|e| e.into_inner()).state = state;
    }
    pub(super) fn preview(&self) -> OperationPreview {
        self.preview.clone()
    }
    pub(super) async fn failure(self, error: FsError) -> OperationProgress {
        let result = failed_status(&self.status, error);
        self.dispose(result).await
    }
    pub(super) async fn cancelled(self) -> OperationProgress {
        self.set_state(OperationState::Cancelled);
        let result = self.progress();
        self.dispose(result).await
    }
    async fn dispose(self, mut result: OperationProgress) -> OperationProgress {
        if let Err(error) = tokio::task::spawn_blocking(move || drop(self)).await {
            result.state = OperationState::Failed;
            result.message = format!("Native resource cleanup failed: {error}");
        }
        result
    }
    pub(super) fn confirm(&mut self, name: Option<String>) -> Result<(), FsError> {
        let progress = self.progress();
        match progress.kind {
            MutationKind::Rename | MutationKind::CreateDirectory | MutationKind::CreateFile => {
                let name =
                    name.ok_or_else(|| error(&progress.source_directory, "A name is required."))?;
                let path = Path::new(&name);
                let components: Vec<_> = path.components().collect();
                if name.is_empty()
                    || name.contains('\0')
                    || (progress.kind != MutationKind::CreateDirectory
                        && (components.len() != 1
                            || !matches!(components[0], Component::Normal(_))
                            || path.file_name() != Some(path.as_os_str())))
                {
                    return Err(error(
                        path,
                        "Enter a path for directory creation, or a single name for rename and empty-file creation.",
                    ));
                }
                self.name = Some(name.into());
            }
            MutationKind::Delete(_) if name.is_some() => {
                return Err(error(
                    &progress.source_directory,
                    "Delete confirmation cannot rename an entry.",
                ));
            }
            MutationKind::Delete(_) => {}
            MutationKind::EditMetadata => {
                return Err(error(
                    &progress.source_directory,
                    "Metadata changes require a matching preview revision.",
                ));
            }
        }
        Ok(())
    }
    pub(super) fn execute(self, outbox: Outbox) -> OperationProgress {
        if self.metadata.is_some() {
            return super::metadata::execute(self, outbox);
        }
        if self.progress().kind == MutationKind::Rename {
            return super::rename::execute(self, outbox);
        }
        let mut progress = self.progress();
        self.discover_deletion(&mut progress);
        let count = progress.total as usize;
        for index in 0..count {
            if !self.cancel.wait_if_paused() {
                break;
            }
            let path = self
                .targets
                .get(index)
                .map(|target| target.item().path.clone())
                .unwrap_or_else(|| progress.source_directory.clone());
            progress.current_path = Some(path);
            self.store_progress(&mut progress);
            let result = match progress.kind {
                MutationKind::CreateFile => self
                    .directory
                    .as_ref()
                    .expect("prepared base")
                    .create_file(self.name.as_deref().expect("validated name"), &self.cancel)
                    .map(Some),
                MutationKind::Rename => self.targets[index]
                    .rename(self.name.as_deref().expect("validated name"), &self.cancel)
                    .map(Some),
                MutationKind::CreateDirectory => self
                    .directory
                    .as_ref()
                    .expect("prepared base")
                    .create(
                        Path::new(self.name.as_deref().expect("validated path")),
                        &self.cancel,
                    )
                    .map(Some),
                MutationKind::Delete(mode) => self.targets[index]
                    .delete(mode, &self.cancel, &mut |path| {
                        progress.processed_entries += 1;
                        if progress.discovery.state == DiscoveryState::Complete
                            && progress.discovery.entries().is_some_and(|total| progress.processed_entries > total) {
                            progress.discovery.unavailable("The source changed after discovery; total progress is unavailable.");
                        }
                        progress.current_path = Some(path.to_owned());
                        // Publish counters to the timer even if the next native call stalls.
                        // Only the timer builds and sends bounded display snapshots.
                        let mut status = self.status.lock().unwrap_or_else(|e| e.into_inner());
                        status.processed_entries = progress.processed_entries;
                        status.discovery = progress.discovery.clone();
                        status.current_path = progress.current_path.clone();
                    })
                    .map(|()| None),
                MutationKind::EditMetadata => unreachable!("metadata jobs use their typed worker"),
            };
            match result {
                Ok(destination) => {
                    progress.completed += 1;
                    if destination.is_some() {
                        progress.processed_entries += 1;
                        progress.renamed_to = destination;
                    }
                }
                Err(error) => {
                    progress.failed += 1;
                    if progress.errors.len() < 16 {
                        progress.errors.push(error);
                    }
                }
            }
            self.store_progress(&mut progress);
        }
        if progress.completed == progress.total
            && progress.failed == 0
            && progress.discovery.state == DiscoveryState::Complete
            && progress.discovery.scope == DiscoveryScope::Recursive
            && progress.discovery.entries() != Some(progress.processed_entries)
        {
            progress.discovery.unavailable(
                "Completed work differs from the metadata discovery snapshot; the source changed.",
            );
        }
        progress.state = if progress.completed == progress.total {
            OperationState::Completed
        } else if self.cancel.is_cancelled() {
            OperationState::Cancelled
        } else if progress.completed > 0 || progress.processed_entries > 0 {
            OperationState::CompletedWithErrors
        } else {
            OperationState::Failed
        };
        progress.pause_requested = false;
        progress.paused = false;
        progress.message = progress
            .errors
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        if progress.failed > 0 && progress.processed_entries > 0 {
            progress
                .message
                .push_str("\nSome changes were already completed and have not been undone.");
        }
        progress
    }

    fn store_progress(&self, progress: &mut OperationProgress) {
        let mut status = self.status.lock().unwrap_or_else(|e| e.into_inner());
        progress.throughput = status.throughput.clone();
        progress.pause_requested = self.cancel.pause_requested();
        progress.paused = self.cancel.is_paused();
        if self.cancel.is_cancelled() {
            progress.state = OperationState::CancelRequested;
        }
        *status = progress.clone();
    }
}
