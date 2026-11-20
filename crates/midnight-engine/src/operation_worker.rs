use super::*;
#[path = "operation_delete.rs"]
pub(super) mod delete;
#[path = "operation_discovery.rs"]
mod discovery;
#[path = "operation_replace.rs"]
mod replace;
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
    /// Routes a Delete job's failure decisions; the actor indexes it by job.
    pub(super) delete_decisions: Arc<delete::DecisionState>,
    /// Formats the two entries a Rename conflict prompt displays. Only a Rename job
    /// retains it, inside its lease; it drops before the mutation permit.
    pub(super) rename_locale: Option<Arc<dyn LocaleService>>,
    preview: OperationPreview,
    pub(super) metadata: Option<super::metadata::Edit>,
    pub(super) metadata_budget: Arc<Semaphore>,
    pub(super) predecessors: Arc<crate::copy_fence::Predecessors>,
    // Declared after targets so native resources close before permits are returned.
    metadata_permits: Vec<tokio::sync::OwnedSemaphorePermit>,
    // Native target pins must close before the retained provider capture/permit.
    _captured_sources: Option<Arc<dyn CapturedDeleteSources>>,
    _captured_rename: Option<Arc<dyn CapturedRenameTarget>>,
    _captured_metadata: Option<Arc<dyn CapturedMetadataSources>>,
    pub(super) metadata_covered: Vec<bool>,
    /// A confirmed empty file replaces the existing one through this service.
    replace_existing: Option<Arc<dyn TransferService>>,
    // Release serialization only after all native pins and provider owners close.
    mutation_permit: Option<tokio::sync::OwnedSemaphorePermit>,
}

pub(super) fn initial(request: &PrepareOperation) -> OperationProgress {
    initial_for(request.id, request.kind, request.source.path.clone())
}
pub(super) fn initial_for(
    id: OperationId,
    kind: MutationKind,
    source_directory: std::path::PathBuf,
) -> OperationProgress {
    OperationProgress {
        id,
        kind,
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
        source_directory,
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
/// The one issue of a target whose change completed (`done` states it) but left
/// private staging behind. It is counted in `failed` without undoing completion.
/// Its kind is always Other: the cleanup's native kind stays in the residue and
/// never drives conflict, cancellation or retry handling.
pub(super) fn residue_issue(done: &str, residue: &MutationResidue) -> FsError {
    FsError::from_io(
        "remove staging folder",
        &residue.path,
        io::Error::other(format!("{done}, but {residue}.")),
    )
}
pub(super) fn failed_status(
    status: &Status,
    error: FsError,
    cancel_requested: bool,
) -> OperationProgress {
    let mut progress = status.lock().unwrap_or_else(|e| e.into_inner()).clone();
    // A provider may interrupt preparation independently of our cancellation token.
    progress.state = if cancel_requested && error.kind == io::ErrorKind::Interrupted {
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
        false,
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
    let mut job = new_job(request.id, request.kind, cancel, status, metadata_budget);
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
            job.append_target(prepared);
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

fn new_job(
    id: OperationId,
    kind: MutationKind,
    cancel: Cancellation,
    status: Status,
    metadata_budget: Arc<Semaphore>,
) -> Job {
    Job {
        cancel,
        status,
        targets: vec![],
        directory: None,
        name: None,
        rename_conflicts: Arc::default(),
        delete_decisions: Arc::default(),
        rename_locale: None,
        metadata: None,
        metadata_budget,
        predecessors: Default::default(),
        metadata_permits: vec![],
        _captured_sources: None,
        _captured_rename: None,
        _captured_metadata: None,
        metadata_covered: vec![],
        replace_existing: None,
        mutation_permit: None,
        preview: OperationPreview {
            id,
            kind,
            files: 0,
            folders: 0,
            file_bytes: Some(0),
            sample: vec![],
        },
    }
}

pub(super) fn prepare_captured_delete(
    request: PrepareCapturedDelete,
    scope: super::input::DeleteScope,
    service: Option<Arc<dyn MutationService>>,
    cancel: Cancellation,
    status: Status,
    metadata_budget: Arc<Semaphore>,
) -> Result<Job, FsError> {
    let source = &request.source_directory;
    let service =
        service.ok_or_else(|| error(source, "This filesystem does not support mutations."))?;
    let mut job = new_job(
        request.id,
        MutationKind::Delete(scope.mode),
        cancel,
        status,
        metadata_budget,
    );
    job._captured_sources = Some(request.sources.clone());
    if !scope.valid() || super::input::DeleteScope::capture(request.sources.as_ref()) != scope {
        return Err(error(
            source,
            "The captured Delete scope changed before preparation.",
        ));
    }
    if !job.cancel.wait_if_paused() {
        return Err(FsError::from_io(
            "prepare",
            source,
            io::ErrorKind::Interrupted.into(),
        ));
    }
    let targets = request.sources.prepare(service.as_ref(), &job.cancel)?;
    if targets.len() != scope.count
        || super::input::DeleteScope::capture(request.sources.as_ref()) != scope
    {
        return Err(error(
            source,
            "Provider returned a different captured Delete scope.",
        ));
    }
    for prepared in targets {
        job.append_target(prepared);
    }
    job.status.lock().unwrap_or_else(|e| e.into_inner()).total = scope.count as u64;
    Ok(job)
}

impl Job {
    fn append_target(&mut self, prepared: Arc<dyn PreparedMutation>) {
        if matches!(
            prepared.item().kind,
            EntryKind::Directory | EntryKind::DirectoryLink
        ) {
            self.preview.folders += 1;
        } else {
            self.preview.files += 1;
            let item = prepared.item();
            let size = (item.kind == EntryKind::File)
                .then_some(item.metadata.size)
                .flatten();
            self.preview.file_bytes = self
                .preview
                .file_bytes
                .zip(size)
                .and_then(|(total, size)| total.checked_add(size));
        }
        if self.preview.sample.len() < 32 {
            self.preview.sample.push(prepared.item().clone());
        }
        self.targets.push(prepared);
    }

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
    pub(super) fn hold_mutation_permit(&mut self, permit: tokio::sync::OwnedSemaphorePermit) {
        self.mutation_permit = Some(permit);
    }
    /// Only a Rename can prompt with transfer entries, so only it keeps the locale
    /// that formats them; every other kind drops this owner here.
    pub(super) fn retain_rename_locale(&mut self, locale: Option<Arc<dyn LocaleService>>) {
        if self.preview.kind == MutationKind::Rename {
            self.rename_locale = locale;
        }
    }
    #[cfg_attr(not(test), allow(dead_code))] // Tests confirm without replacing.
    pub(super) fn confirm(&mut self, name: Option<String>) -> Result<(), FsError> {
        self.confirm_with(name, false, None)
    }
    pub(super) fn confirm_with(
        &mut self,
        name: Option<String>,
        replace_existing: bool,
        transfer: Option<Arc<dyn TransferService>>,
    ) -> Result<(), FsError> {
        let progress = self.progress();
        if replace_existing {
            if progress.kind != MutationKind::CreateFile {
                return Err(error(
                    &progress.source_directory,
                    "Only a new empty file can replace an existing file.",
                ));
            }
            self.replace_existing = Some(transfer.ok_or_else(|| {
                error(
                    &progress.source_directory,
                    "Replacing a file is not available here.",
                )
            })?);
        }
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
        if matches!(self.progress().kind, MutationKind::Delete(_)) {
            return delete::execute(self, outbox);
        }
        let mut progress = self.progress();
        let count = progress.total as usize;
        // Completed targets that reported an issue (a residue, an unconfirmed step);
        // each is also one `failed` issue.
        let mut residues = 0;
        // Where a replaced file is kept, for the completion message.
        let mut recovery = String::new();
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
                MutationKind::CreateFile => {
                    let base = self.directory.as_ref().expect("prepared base");
                    let name = self.name.as_deref().expect("validated name");
                    match &self.replace_existing {
                        Some(transfer) => replace::replace_with_empty_file(
                            transfer.as_ref(),
                            base.as_ref(),
                            &progress.source_directory,
                            name,
                            &self.cancel,
                        )
                        .map(|replaced| {
                            recovery = replaced.note;
                            (Some(replaced.path), replaced.issue)
                        }),
                        None => base
                            .create_file(name, &self.cancel)
                            .map(|path| (Some(path), None)),
                    }
                }
                MutationKind::Rename => unreachable!("rename jobs use their typed worker"),
                MutationKind::CreateDirectory => self
                    .directory
                    .as_ref()
                    .expect("prepared base")
                    .create(
                        Path::new(self.name.as_deref().expect("validated path")),
                        &self.cancel,
                    )
                    .map(|path| (Some(path), None)),
                MutationKind::Delete(_) => unreachable!("delete jobs use their typed worker"),
                MutationKind::EditMetadata => unreachable!("metadata jobs use their typed worker"),
            };
            match result {
                Ok((destination, issue)) => {
                    progress.completed += 1;
                    if destination.is_some() {
                        progress.processed_entries += 1;
                        progress.renamed_to = destination;
                    }
                    // The change completed; its issue is counted without undoing it.
                    if let Some(issue) = issue {
                        residues += 1;
                        progress.failed += 1;
                        if progress.errors.len() < 16 {
                            progress.errors.push(issue);
                        }
                    }
                }
                Err(error)
                    if matches!(progress.kind, MutationKind::Delete(_))
                        && error.kind == io::ErrorKind::Interrupted
                        && self.cancel.is_cancelled() =>
                {
                    // Native cancellation can follow already-reported deletions.
                    break;
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
            && progress.failed == residues
            && progress.discovery.state == DiscoveryState::Complete
            && progress.discovery.scope == DiscoveryScope::Recursive
            && progress.discovery.entries() != Some(progress.processed_entries)
        {
            progress.discovery.unavailable(
                "Completed work differs from the metadata discovery snapshot; the source changed.",
            );
        }
        progress.state = if progress.completed == progress.total {
            // Every target completed; residues alone are its errors.
            if progress.failed == 0 {
                OperationState::Completed
            } else {
                OperationState::CompletedWithErrors
            }
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
        // A replaced file stays recoverable: say where, so it can be found and removed.
        if !recovery.is_empty() {
            if !progress.message.is_empty() {
                progress.message.push('\n');
            }
            progress.message.push_str(&recovery);
        }
        // A residue undoes nothing; an incomplete target can leave partial work.
        if progress.processed_entries > 0
            && (progress.state == OperationState::Cancelled || progress.failed > residues)
        {
            if !progress.message.is_empty() {
                progress.message.push('\n');
            }
            progress
                .message
                .push_str("Some changes were already completed and have not been undone.");
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

// These futures own a guard before their first poll, including disposal-task abort.
impl WorkerOwned<Job> {
    pub(super) async fn failure(self, error: FsError) -> OperationProgress {
        let result = failed_status(&self.status, error, self.cancel.is_cancelled());
        self.dispose(result).await
    }
    pub(super) async fn cancelled(self) -> OperationProgress {
        let result = {
            let mut status = self.status.lock().unwrap_or_else(|e| e.into_inner());
            status.state = OperationState::Cancelled;
            // A terminal state reports no pause, as on every other terminal path.
            status.pause_requested = false;
            status.paused = false;
            status.clone()
        };
        self.dispose(result).await
    }
    async fn dispose(self, mut result: OperationProgress) -> OperationProgress {
        if let Err(error) = self.retire().await {
            result.state = OperationState::Failed;
            result.message = format!("Native resource cleanup failed: {error:?}");
        }
        result
    }
}

pub(super) fn prepare_captured_rename(
    request: PrepareCapturedRename,
    scope: super::input::RenameScope,
    service: Option<Arc<dyn MutationService>>,
    cancel: Cancellation,
    status: Status,
    metadata_budget: Arc<Semaphore>,
) -> Result<Job, FsError> {
    let source = &request.source_directory;
    let service =
        service.ok_or_else(|| error(source, "This filesystem does not support mutations."))?;
    let mut job = new_job(
        request.id,
        MutationKind::Rename,
        cancel,
        status,
        metadata_budget,
    );
    job._captured_rename = Some(request.source.clone());
    if !scope.valid() || super::input::RenameScope::capture(request.source.as_ref()) != scope {
        return Err(error(
            source,
            "The captured Rename scope changed before preparation.",
        ));
    }
    if !job.cancel.wait_if_paused() {
        return Err(FsError::from_io(
            "prepare",
            source,
            io::ErrorKind::Interrupted.into(),
        ));
    }
    let prepared = request.source.prepare(service.as_ref(), &job.cancel)?;
    if super::input::RenameScope::capture(request.source.as_ref()) != scope {
        return Err(error(
            source,
            "Provider returned a different captured Rename scope.",
        ));
    }
    job.append_target(prepared);
    if job.cancel.is_cancelled() {
        return Err(FsError::from_io(
            "prepare",
            source,
            io::ErrorKind::Interrupted.into(),
        ));
    }
    job.status.lock().unwrap_or_else(|e| e.into_inner()).total = 1;
    Ok(job)
}

pub(super) fn prepare_captured_metadata(
    request: PrepareCapturedMetadata,
    scope: super::input::MetadataScope,
    service: Option<Arc<dyn MutationService>>,
    cancel: Cancellation,
    status: Status,
    metadata_budget: Arc<Semaphore>,
) -> Result<Job, FsError> {
    let source = &request.source_directory;
    let service =
        service.ok_or_else(|| error(source, "This filesystem does not support mutations."))?;
    let mut job = new_job(
        request.id,
        MutationKind::EditMetadata,
        cancel,
        status,
        metadata_budget,
    );
    job._captured_metadata = Some(request.sources.clone());
    if !scope.valid() || super::input::MetadataScope::capture(request.sources.as_ref()) != scope {
        return Err(error(
            source,
            "The captured metadata scope changed before preparation.",
        ));
    }
    if !job.cancel.wait_if_paused() {
        return Err(FsError::from_io(
            "prepare",
            source,
            io::ErrorKind::Interrupted.into(),
        ));
    }
    // Reserve the complete batch before the capture opens any native target.
    let permits = job.metadata_budget.clone().try_acquire_many_owned(scope.count as u32)
        .map_err(|_| error(source,
            "The shared metadata handle budget is in use. Close another preview or wait for its job."))?;
    job.metadata_permits.push(permits);
    let targets = request.sources.prepare(service.as_ref(), &job.cancel)?;
    if targets.len() != scope.count
        || super::input::MetadataScope::capture(request.sources.as_ref()) != scope
    {
        return Err(error(
            source,
            "Provider returned a different captured metadata scope.",
        ));
    }
    for target in targets {
        job.append_target(target);
    }
    if job.cancel.is_cancelled() {
        return Err(FsError::from_io(
            "prepare",
            source,
            io::ErrorKind::Interrupted.into(),
        ));
    }
    job.metadata_covered = scope.covered;
    job.metadata = Some(super::metadata::Edit::prepare(
        request.id,
        &job.targets,
        &job.cancel,
    )?);
    job.status.lock().unwrap_or_else(|e| e.into_inner()).total = scope.count as u64;
    Ok(job)
}
