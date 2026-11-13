//! Resumable traversal and one verified file at a time. Every method runs on a worker.
use super::*;
use midnight_platform_common::transfer::{CopyPhase, VerifiedFileCopy};
use std::{
    collections::{HashSet, VecDeque},
    ffi::OsString,
    path::Component,
};
#[path = "copy_discovery.rs"]
mod discovery;
#[path = "copy_failures.rs"]
mod failures;
#[path = "copy_link_worker.rs"]
mod link_job;
#[path = "copy_metadata_worker.rs"]
mod metadata_job;
#[path = "move_worker.rs"]
mod move_job;
#[path = "move_native.rs"]
mod native_move;
#[path = "copy_work_budget.rs"]
mod work_budget;

pub(super) enum Step {
    Continue,
    Conflict(Arc<CopyConflict>),
    Failure(Arc<CopyFailure>),
    Done,
}
struct Frame {
    source: Box<dyn TransferSource>,
    destination: Arc<dyn TransferDirectory>,
    retain_source: bool,
    enumeration_complete: bool,
    failed_before: u64,
    skipped_before: u64,
}
struct RootCompletion {
    source: CompletedCopySource,
    failed: u64,
    skipped: u64,
}
struct EntryWork {
    entry: TransferEntry,
    native_identity: Option<TransferIdentity>,
    source: Option<Box<dyn TransferSource>>,
    copy: Option<VerifiedFileCopy>,
    link: Option<link_job::LinkCopy>,
    destination: Arc<dyn TransferDirectory>,
    name: OsString,
    target: Option<Arc<dyn TransferTarget>>,
    consent: Option<Consent>,
    replacement_prepared: bool,
    native_fallback: bool,
    move_preflight_done: bool,
    force_prompt: bool,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Consent {
    Replace,
    Merge,
}
impl EntryWork {
    fn native_identity(&self) -> Option<TransferIdentity> {
        self.native_identity
    }
    fn entry(&self) -> &TransferEntry {
        &self.entry
    }
}

pub(super) struct Job {
    status: Status,
    progress: CopyProgress,
    cancel: Cancellation,
    destination: Arc<dyn TransferDirectory>,
    roots: VecDeque<Box<dyn TransferSource>>,
    protected_roots: HashSet<TransferIdentity>,
    stack: Vec<Frame>,
    current: Option<EntryWork>,
    conflict: Option<CopyConflict>,
    failure: Option<CopyFailure>,
    failure_context: failures::Context,
    skipped_errors: HashSet<failures::Scope>,
    halt: bool,
    decision: Option<CopyChoice>,
    version: u64,
    skip_all: bool,
    replace_all: bool,
    merge_all: bool,
    move_work: Option<move_job::MoveWork>,
    metadata: Option<metadata_job::MetadataWork>,
    verification: TransferVerification,
    links: CopyLinkPolicy,
    work_budget: work_budget::Budget,
    active_root: Option<RootCompletion>,
    completed_root: Option<CompletedCopySource>,
    // Native sources, captures and cleanup authority close before
    // the provider registration and shared capture permit are released.
    drop_retention: Option<Arc<crate::drag_drop::Retention>>,
    // Last: cleanup-bearing link work retains the dedicated Copy I/O guard
    // through actual cleanup and every stage/source/provider destruction.
    link_retirement: Arc<link_job::LinkRetirement>,
}
pub(super) fn check_move_bounds<'a>(
    destination: &Path,
    paths: impl IntoIterator<Item = &'a Path>,
) -> Result<(), FsError> {
    move_job::check_root_bounds(destination, paths)
}
impl Job {
    pub(super) fn set_drop_retention(&mut self, retention: Arc<crate::drag_drop::Retention>) {
        self.drop_retention = Some(retention);
    }
    pub(super) fn set_verification(&mut self, verification: TransferVerification) {
        self.verification = verification;
        self.progress.verification = verification;
    }
    pub(super) fn prepare_move_sources(
        destination: std::path::PathBuf,
        roots: VecDeque<PreparedTransferMove>,
        service: &dyn TransferService,
        cancel: Cancellation,
        status: Status,
        metadata: TransferMetadataPolicy,
    ) -> Result<Self, FsError> {
        move_job::prepare_sources(destination, roots, service, cancel, status, metadata)
    }

    pub(super) fn prepare_with_metadata(
        request: CopyRequest,
        service: &dyn TransferService,
        cancel: Cancellation,
        status: Status,
        metadata: TransferMetadataPolicy,
    ) -> Result<Self, FsError> {
        if metadata == TransferMetadataPolicy::ContentOnly {
            return Self::prepare(request, service, cancel, status);
        }
        move_job::check_bounds(&request)?;
        let mut job = Self::prepare(request, service, cancel, status)?;
        let _permit = move_job::permit(&job.cancel, &job.progress.destination_directory)?;
        job.prepare_metadata(metadata)?;
        Ok(job)
    }
    pub(super) fn prepare_move_with_metadata(
        request: CopyRequest,
        service: &dyn TransferService,
        cancel: Cancellation,
        status: Status,
        metadata: TransferMetadataPolicy,
    ) -> Result<Self, FsError> {
        if metadata == TransferMetadataPolicy::ContentOnly {
            return Self::prepare_move(request, service, cancel, status);
        }
        move_job::prepare_with_metadata(request, service, cancel, status, metadata)
    }
    pub(super) fn prepare_move(
        request: CopyRequest,
        service: &dyn TransferService,
        cancel: Cancellation,
        status: Status,
    ) -> Result<Self, FsError> {
        move_job::prepare(request, service, cancel, status)
    }
    pub(super) fn prepare(
        request: CopyRequest,
        service: &dyn TransferService,
        cancel: Cancellation,
        status: Status,
    ) -> Result<Self, FsError> {
        Self::prepare_native(request, service, cancel, status, CopyLinkPolicy::Reject)
    }

    pub(super) fn prepare_with_options(
        request: CopyRequest,
        service: &dyn TransferService,
        cancel: Cancellation,
        status: Status,
        metadata: TransferMetadataPolicy,
        links: CopyLinkPolicy,
    ) -> Result<Self, FsError> {
        if links == CopyLinkPolicy::Reject {
            return Self::prepare_with_metadata(request, service, cancel, status, metadata);
        }
        link_job::validate_options(&request.destination, &status, metadata)?;
        Self::prepare_native(request, service, cancel, status, links)
    }

    fn prepare_native(
        request: CopyRequest,
        service: &dyn TransferService,
        cancel: Cancellation,
        status: Status,
        links: CopyLinkPolicy,
    ) -> Result<Self, FsError> {
        let destination = service.prepare_transfer_destination(&request.destination, &cancel)?;
        if destination.path() != request.destination {
            return Err(error(
                &request.destination,
                io::ErrorKind::InvalidInput,
                "Provider returned a different destination.",
            ));
        }
        let mut seen = HashSet::new();
        let mut roots = VecDeque::new();
        for path in &request.clipboard.paths {
            if cancel.is_cancelled() {
                return Err(error(path, io::ErrorKind::Interrupted, "Copy cancelled."));
            }
            if !seen.insert(path) {
                continue;
            }
            let source = service.prepare_transfer_source_with_links(path, links, &cancel)?;
            if source.entry().item.path != *path {
                return Err(error(
                    path,
                    io::ErrorKind::InvalidInput,
                    "Provider returned a different source.",
                ));
            }
            if source.entry().item.kind == EntryKind::Directory {
                destination.reject_source_ancestor(source.entry().identity, &cancel)?;
            }
            roots.push_back(source);
        }
        link_job::admit_roots(&roots, destination.as_ref(), links, &cancel)?;
        let mut job = Self::from_sources(destination, roots, cancel, status);
        job.links = links;
        job.set_verification(request.verification);
        Ok(job)
    }

    pub(super) fn prepare_archive(
        path: std::path::PathBuf,
        roots: VecDeque<Box<dyn TransferSource>>,
        service: &dyn TransferService,
        cancel: Cancellation,
        status: Status,
    ) -> Result<Self, FsError> {
        let destination = service.prepare_transfer_destination(&path, &cancel)?;
        if destination.path() != path {
            return Err(error(
                &path,
                io::ErrorKind::InvalidInput,
                "Provider returned a different destination.",
            ));
        }
        Ok(Self::from_sources(destination, roots, cancel, status))
    }

    pub(super) fn prepare_sources(
        path: std::path::PathBuf,
        roots: VecDeque<Box<dyn TransferSource>>,
        service: &dyn TransferService,
        cancel: Cancellation,
        status: Status,
        metadata: TransferMetadataPolicy,
    ) -> Result<Self, FsError> {
        Self::prepare_captured(
            path,
            roots,
            service,
            cancel,
            status,
            metadata,
            CopyLinkPolicy::Reject,
        )
    }

    pub(super) fn prepare_sources_with_options(
        path: std::path::PathBuf,
        roots: VecDeque<Box<dyn TransferSource>>,
        service: &dyn TransferService,
        cancel: Cancellation,
        status: Status,
        metadata: TransferMetadataPolicy,
        links: CopyLinkPolicy,
    ) -> Result<Self, FsError> {
        if links == CopyLinkPolicy::Reject {
            return Self::prepare_sources(path, roots, service, cancel, status, metadata);
        }
        Self::prepare_captured(path, roots, service, cancel, status, metadata, links)
    }

    fn prepare_captured(
        path: std::path::PathBuf,
        roots: VecDeque<Box<dyn TransferSource>>,
        service: &dyn TransferService,
        cancel: Cancellation,
        status: Status,
        metadata: TransferMetadataPolicy,
        links: CopyLinkPolicy,
    ) -> Result<Self, FsError> {
        if links == CopyLinkPolicy::Preserve {
            link_job::validate_options(&path, &status, metadata)?;
        }
        if roots.is_empty() || roots.len() > 4096 {
            return Err(error(
                &path,
                io::ErrorKind::InvalidInput,
                "Invalid captured Copy root count.",
            ));
        }
        if metadata != TransferMetadataPolicy::ContentOnly {
            let paths = roots
                .iter()
                .map(|source| source.entry().item.path.clone())
                .collect();
            move_job::check_bounds(&CopyRequest {
                verification: snapshot(&status).verification,
                id: snapshot(&status).id,
                clipboard: FileClipboard {
                    paths,
                    intent: FileClipboardIntent::Copy,
                },
                destination: path.clone(),
            })?;
        }
        let destination = service.prepare_transfer_destination(&path, &cancel)?;
        if destination.path() != path {
            return Err(error(
                &path,
                io::ErrorKind::InvalidInput,
                "Provider returned a different destination.",
            ));
        }
        let mut identities = HashSet::new();
        for source in &roots {
            source.revalidate(&cancel)?;
            if !matches!(
                source.entry().item.kind,
                EntryKind::File | EntryKind::Directory
            ) && !(links == CopyLinkPolicy::Preserve
                && link_job::is_link_kind(source.entry().item.kind))
            {
                return Err(error(
                    &path,
                    io::ErrorKind::Unsupported,
                    "Captured Copy source kind is unsupported by this link policy.",
                ));
            }
            if let Some(id) = source.native_identity() {
                if id != source.entry().identity {
                    return Err(error(
                        &path,
                        io::ErrorKind::InvalidData,
                        "Native Copy source identity is inconsistent.",
                    ));
                }
                if !identities.insert(id) {
                    return Err(error(
                        &path,
                        io::ErrorKind::InvalidInput,
                        "Duplicate native Copy roots.",
                    ));
                }
                if source.entry().item.kind == EntryKind::Directory {
                    destination.reject_source_ancestor(id, &cancel)?;
                }
            }
        }
        // Read-only ancestry authority avoids requiring writes to source parents.
        let directories: Vec<_> = roots
            .iter()
            .filter(|source| source.entry().item.kind == EntryKind::Directory)
            .filter_map(|source| source.native_identity())
            .collect();
        let mut parents = Vec::new();
        if !directories.is_empty() {
            for source in roots
                .iter()
                .filter(|source| source.native_identity().is_some())
            {
                let source_path = &source.entry().item.path;
                let parent = source_path.parent().ok_or_else(|| {
                    error(
                        source_path,
                        io::ErrorKind::InvalidInput,
                        "Copy source needs a parent.",
                    )
                })?;
                let directory = service.prepare_checksum_directory(parent, &cancel)?;
                if directory.path() != parent {
                    return Err(error(
                        parent,
                        io::ErrorKind::InvalidData,
                        "Provider returned a different source parent.",
                    ));
                }
                for id in &directories {
                    directory.reject_source_ancestor(*id, &cancel)?;
                }
                parents.push(directory);
            }
        }
        for source in &roots {
            source.revalidate(&cancel)?;
        }
        link_job::admit_roots(&roots, destination.as_ref(), links, &cancel)?;
        let mut job = Self::from_sources(destination, roots, cancel, status);
        job.links = links;
        if metadata != TransferMetadataPolicy::ContentOnly {
            if job.roots.len() > 64 {
                return Err(error(
                    &path,
                    io::ErrorKind::Unsupported,
                    "Required modified time accepts at most 64 roots.",
                ));
            }
            job.prepare_metadata(metadata)?;
        }
        Ok(job)
    }

    fn from_sources(
        destination: Arc<dyn TransferDirectory>,
        roots: VecDeque<Box<dyn TransferSource>>,
        cancel: Cancellation,
        status: Status,
    ) -> Self {
        let protected_roots = roots
            .iter()
            .filter_map(|source| source.native_identity())
            .collect();
        let mut progress = snapshot(&status);
        progress.requested_roots = roots.len() as u64;
        let link_retirement = Arc::new(link_job::LinkRetirement::new(
            status.clone(),
            cancel.clone(),
        ));
        Self {
            progress,
            status,
            cancel,
            destination,
            roots,
            protected_roots,
            stack: vec![],
            current: None,
            conflict: None,
            failure: None,
            failure_context: failures::Context::Entry,
            skipped_errors: HashSet::new(),
            halt: false,
            decision: None,
            version: 0,
            skip_all: false,
            replace_all: false,
            merge_all: false,
            move_work: None,
            metadata: None,
            verification: TransferVerification::default(),
            links: CopyLinkPolicy::Reject,
            work_budget: work_budget::Budget::default(),
            active_root: None,
            completed_root: None,
            drop_retention: None,
            link_retirement,
        }
    }

    pub(super) fn accept_decision(&mut self, decision: CopyDecision) -> bool {
        let conflict = self.conflict.as_ref().is_some_and(|conflict| {
            conflict.id == decision.id
                && conflict.version == decision.version
                && match decision.choice {
                    CopyChoice::Retry => {
                        self.links == CopyLinkPolicy::Preserve
                            && self
                                .current
                                .as_ref()
                                .is_some_and(|work| link_job::is_link_kind(work.entry.item.kind))
                    }
                    CopyChoice::SkipAllErrors => false,
                    CopyChoice::ReplaceRecoverable { .. } => conflict.can_replace,
                    CopyChoice::Merge { .. } => conflict.can_merge,
                    _ => true,
                }
        });
        let failure = self.failure.as_ref().is_some_and(|failure| {
            failure.id == decision.id
                && failure.version == decision.version
                && match decision.choice {
                    CopyChoice::Retry => failure.can_retry,
                    CopyChoice::Skip | CopyChoice::SkipAllErrors => failure.can_skip,
                    CopyChoice::Cancel => true,
                    _ => false,
                }
        });
        if conflict || failure {
            self.decision = Some(decision.choice);
            true
        } else {
            false
        }
    }

    pub(super) fn step(&mut self) -> Step {
        let step = self.advance();
        // Completion is a root boundary, never an inferred percentage. In Move,
        // pending proof/finalization work must finish before reporting success.
        if self.current.is_none()
            && self.stack.is_empty()
            && self
                .move_work
                .as_ref()
                .is_none_or(|work| !work.has_pending())
            && let Some(root) = self.active_root.take()
            && root.failed == self.progress.failed
            && root.skipped == self.progress.skipped
        {
            self.progress.completed_roots += 1;
            self.completed_root = Some(root.source);
        }
        self.progress.state = if self.cancel.is_cancelled() {
            CopyOperationState::CancelRequested
        } else {
            CopyOperationState::Running
        };
        self.store_progress();
        step
    }

    fn store_progress(&mut self) {
        if let Some(work) = &self.current {
            self.progress.current_path = Some(work.entry.item.path.clone());
        }
        self.progress.current_destination = self
            .current
            .as_ref()
            .map(|work| work.destination.path().join(&work.name));
        // Derive both fields from the same retained work item. Skip, Retry, native
        // rename and terminal cleanup cannot leave bytes associated with another path.
        self.progress.current_file_size = self
            .current
            .as_ref()
            .and_then(|work| work.copy.as_ref().and(work.entry.item.metadata.size));
        self.progress.current_file_copied = self
            .current
            .as_ref()
            .and_then(|work| work.copy.as_ref())
            .map_or(0, VerifiedFileCopy::copied_bytes);
        self.progress.current_file_verified = self
            .current
            .as_ref()
            .and_then(|work| work.copy.as_ref())
            .map_or(0, VerifiedFileCopy::verified_bytes);
        self.check_discovery_progress();
        let proof_credit = self
            .move_work
            .as_ref()
            .map_or(Some(0), |work| work.active_proof_credit(self.verification));
        self.progress.remaining_work = self.work_budget.remaining(&self.progress, proof_credit);
        let mut status = self.status.lock().unwrap_or_else(|e| e.into_inner());
        self.progress.throughput = status.throughput.clone();
        self.progress.pause_requested = self.cancel.pause_requested();
        self.progress.paused = self.cancel.is_paused();
        if self.cancel.is_cancelled() {
            self.progress.state = CopyOperationState::CancelRequested;
            crate::operation_throughput::stop(&mut self.progress.throughput);
        }
        *status = self.progress.clone();
    }
    pub(super) fn take_completed_root(&mut self) -> Option<CompletedCopySource> {
        self.completed_root.take()
    }
    fn advance(&mut self) -> Step {
        if self.cancel.is_cancelled() {
            return Step::Done;
        }
        if self.failure.is_some() {
            return self.advance_failure();
        }
        if let Some(choice) = self.decision.take() {
            let conflict = self.conflict.take();
            match choice {
                CopyChoice::Retry | CopyChoice::SkipAllErrors => return Step::Continue,
                CopyChoice::Cancel => {
                    self.cancel.cancel();
                    return Step::Done;
                }
                CopyChoice::Skip | CopyChoice::SkipAll => {
                    self.skip_all |= matches!(choice, CopyChoice::SkipAll);
                    self.skip_current();
                    return Step::Continue;
                }
                CopyChoice::ReplaceRecoverable { all } | CopyChoice::Merge { all } => {
                    let consent = if matches!(choice, CopyChoice::Merge { .. }) {
                        Consent::Merge
                    } else {
                        Consent::Replace
                    };
                    let allowed = conflict.is_some_and(|value| match consent {
                        Consent::Replace => value.can_replace,
                        Consent::Merge => value.can_merge,
                    });
                    if !allowed {
                        self.record(error(
                            &self.progress.destination_directory,
                            io::ErrorKind::Unsupported,
                            "This conflict action is unavailable.",
                        ));
                        self.drop_current();
                        return Step::Continue;
                    }
                    let Some(mut work) = self.current.take() else {
                        return Step::Continue;
                    };
                    if work
                        .target
                        .as_ref()
                        .is_none_or(|target| target.revalidate(&self.cancel).is_err())
                    {
                        return self.refresh_conflict(work);
                    }
                    work.consent = Some(consent);
                    work.replacement_prepared = false;
                    work.force_prompt = false;
                    match consent {
                        Consent::Replace => self.replace_all |= all,
                        Consent::Merge => self.merge_all |= all,
                    }
                    self.current = Some(work);
                }
                CopyChoice::Rename(name) => {
                    let valid = !name.is_empty()
                        && !name.as_encoded_bytes().contains(&0)
                        && Path::new(&name).file_name() == Some(name.as_os_str())
                        && matches!(
                            (
                                Path::new(&name).components().next(),
                                Path::new(&name).components().count()
                            ),
                            (Some(Component::Normal(_)), 1)
                        );
                    if !valid {
                        self.record(error(
                            &self.progress.destination_directory,
                            io::ErrorKind::InvalidInput,
                            "Enter one native basename without dot or path components.",
                        ));
                        self.drop_current();
                        return Step::Continue;
                    }
                    if let Some(work) = self.current.as_mut() {
                        work.name = name;
                        work.target = None;
                        work.consent = None;
                        work.replacement_prepared = false;
                    }
                }
            }
        }
        if let Some(conflict) = &self.conflict {
            return Step::Conflict(Arc::new(conflict.clone()));
        }
        if self.halt {
            return Step::Done;
        }
        if let Some(work) = self.current.take() {
            return self.process(work);
        }
        if self
            .move_work
            .as_ref()
            .is_some_and(move_job::MoveWork::has_pending)
            || (self.move_work.is_some() && self.roots.is_empty() && self.stack.is_empty())
        {
            return self.advance_move();
        }
        self.progress.phase = CopyOperationPhase::Planning;
        self.progress.current_path = None;
        let next = if let Some(frame) = self.stack.last_mut() {
            let next = if frame.enumeration_complete {
                Ok(None)
            } else if let Some(move_work) = self.move_work.as_mut() {
                frame
                    .source
                    .next_move_child(&self.cancel)
                    .and_then(|child| {
                        child
                            .map(|prepared| {
                                move_work.admit_child(
                                    prepared,
                                    &frame.source.entry().item.path,
                                    &self.cancel,
                                )
                            })
                            .transpose()
                    })
            } else {
                frame.source.next_child(&self.cancel)
            };
            match next {
                Ok(Some(source)) => Some((source, frame.destination.clone())),
                Ok(None) => {
                    // Removal retries must not advance an already consumed iterator.
                    // This remains true even if its provider is not fused after EOF.
                    self.stack
                        .last_mut()
                        .expect("completed traversal frame")
                        .enumeration_complete = true;
                    let frame = self.stack.last().expect("completed traversal frame");
                    let retaining = frame.retain_source;
                    let completed_before = (frame.failed_before, frame.skipped_before);
                    if let Some(mut move_work) = self.move_work.take() {
                        let entry = frame.source.entry().clone();
                        let result = if frame.retain_source {
                            move_work.discard(entry.identity)
                        } else {
                            frame
                                .source
                                .revalidate(&self.cancel)
                                .and_then(|()| frame.destination.revalidate(&self.cancel))
                                .and_then(|()| move_work.remove_directory(&entry, self))
                        };
                        self.move_work = Some(move_work);
                        if let Err(error) = result {
                            if retaining {
                                self.record(error);
                                return Step::Done;
                            }
                            return self
                                .wait_for_failure(error, failures::Context::DirectoryRemoval);
                        }
                    }
                    if completed_before == (self.progress.failed, self.progress.skipped) {
                        self.progress.completed_directories += 1;
                    }
                    self.stack.pop();
                    return Step::Continue;
                }
                Err(error) => {
                    return self.wait_for_failure(error, failures::Context::Enumeration);
                }
            }
        } else {
            self.roots.pop_front().map(|source| {
                self.active_root = Some(RootCompletion {
                    source: source
                        .tree_copy_occurrence()
                        .or_else(|| {
                            self.drop_retention
                                .as_ref()?
                                .completions
                                .get(&source.entry().item.path)
                                .copied()
                        })
                        .map_or_else(
                            || {
                                source.virtual_identity().map_or_else(
                                    || {
                                        CompletedCopySource::Native(
                                            source.entry().item.path.clone(),
                                        )
                                    },
                                    CompletedCopySource::Archive,
                                )
                            },
                            CompletedCopySource::Tree,
                        ),
                    failed: self.progress.failed,
                    skipped: self.progress.skipped,
                });
                (source, self.destination.clone())
            })
        };
        let Some((source, destination)) = next else {
            return self.advance_move();
        };
        let path = &source.entry().item.path;
        let Some(name) = path.file_name() else {
            self.record(error(
                path,
                io::ErrorKind::InvalidInput,
                "A source basename is required.",
            ));
            return Step::Continue;
        };
        let name = name.to_owned();
        self.current = Some(EntryWork {
            entry: source.entry().clone(),
            native_identity: source.native_identity(),
            source: Some(source),
            copy: None,
            link: None,
            destination,
            name,
            target: None,
            consent: None,
            replacement_prepared: false,
            native_fallback: false,
            move_preflight_done: false,
            force_prompt: false,
        });
        Step::Continue
    }

    fn process(&mut self, mut work: EntryWork) -> Step {
        self.progress.current_path = Some(work.entry().item.path.clone());
        let result = if work.link.is_some() {
            self.link_step(&mut work)
        } else if work.copy.is_some() {
            self.copy_step(&mut work)
        } else {
            self.prepare_entry(&mut work)
        };
        match result {
            Ok(Action::Keep) => {
                self.current = Some(work);
                Step::Continue
            }
            Ok(Action::Done) => Step::Continue,
            Ok(Action::Directory(destination)) => {
                self.stack.push(Frame {
                    source: work.source.take().expect("directory source"),
                    destination,
                    retain_source: false,
                    enumeration_complete: false,
                    failed_before: self.progress.failed,
                    skipped_before: self.progress.skipped,
                });
                Step::Continue
            }
            Ok(Action::Conflict(target)) => self.wait_for_conflict(work, target),
            Err(failure) if failure.kind == io::ErrorKind::AlreadyExists => {
                // A late collision needs a new target pin and fresh consent.
                work.consent = None;
                work.replacement_prepared = false;
                work.force_prompt = true;
                match self.observe_target(&mut work) {
                    Ok(Some(target)) => self.wait_for_conflict(work, target),
                    Ok(None) => {
                        self.current = Some(work);
                        Step::Continue
                    }
                    Err(error) => {
                        self.current = Some(work);
                        self.wait_for_failure(error, failures::Context::Entry)
                    }
                }
            }
            Err(error) => {
                self.current = Some(work);
                self.wait_for_failure(error, failures::Context::Entry)
            }
        }
    }

    fn prepare_entry(&mut self, work: &mut EntryWork) -> Result<Action, FsError> {
        work.source
            .as_ref()
            .expect("unstarted entry source")
            .revalidate(&self.cancel)?;
        if link_job::is_link_kind(work.entry.item.kind) {
            return self.prepare_link(work);
        }
        if work.consent.is_none()
            && let Some(target) = self.observe_target(work)?
        {
            // The provider distinguishes an exact same-name no-op, a case-only
            // rename, and a distinct hard-link alias without inferring from text.
            if self.move_work.is_some()
                && work.native_identity() == Some(target.identity)
                && self.try_native_move(work)?
            {
                return Ok(Action::Done);
            }
            return Ok(Action::Conflict(target));
        }
        if self.try_native_move(work)? {
            return Ok(Action::Done);
        }
        self.prepare_move_copy(work)?;
        let source = work.source.as_ref().expect("unstarted entry source");
        match source.entry().item.kind {
            EntryKind::Directory => {
                if self.move_work.is_some()
                    && self.stack.len() >= move_job::MAX_MOVE_DIRECTORY_DEPTH
                {
                    return Err(error(
                        &source.entry().item.path,
                        io::ErrorKind::Unsupported,
                        "Recursive Move exceeds the 128-directory traversal-depth budget.",
                    ));
                }
                if let Some(identity) = source.native_identity() {
                    work.destination
                        .reject_source_ancestor(identity, &self.cancel)?;
                }
                let destination = if work.consent == Some(Consent::Merge) {
                    let target = work.target.as_ref().expect("consented target pin");
                    target.revalidate(&self.cancel)?;
                    work.destination.open_merge_directory(
                        &work.name,
                        target.entry(),
                        &self.cancel,
                    )?
                } else {
                    let created = work
                        .destination
                        .create_directory(&work.name, &self.cancel)?;
                    self.progress.directories_created += 1;
                    created
                };
                if let Some(identity) = source.native_identity() {
                    destination.reject_source_ancestor(identity, &self.cancel)?;
                }
                Ok(Action::Directory(destination))
            }
            EntryKind::File => {
                if source.entry().item.metadata.size.is_none() {
                    return Err(error(
                        &source.entry().item.path,
                        io::ErrorKind::InvalidData,
                        "A regular source file with known length is required.",
                    ));
                }
                // Validate optional metadata before consuming a prepared stage/source.
                if let Some(metadata) = self.metadata.as_ref() {
                    metadata
                        .pins
                        .get(&source.entry().identity)
                        .ok_or_else(|| {
                            error(
                                &source.entry().item.path,
                                io::ErrorKind::InvalidData,
                                "Prepared source metadata pin is missing",
                            )
                        })?
                        .revalidate(&self.cancel)?;
                }
                let stage = if let Some(move_work) = self.move_work.as_mut() {
                    let mut stage =
                        if let Some(stage) = move_work.stages.remove(&source.entry().identity) {
                            stage
                        } else if move_work.metadata_policy == TransferMetadataPolicy::ContentOnly {
                            work.destination.stage_file(&self.cancel)?
                        } else {
                            work.destination
                                .stage_file_with_metadata(move_work.metadata_policy, &self.cancel)?
                        };
                    if let Err(failure) = stage
                        .rewind_for_validation(&self.cancel)
                        .and_then(|guard| guard.revalidate(&self.cancel))
                    {
                        // Retain this exact owned stage through Retry. Discarding
                        // it here can hide a cleanup refusal and accumulate a new
                        // orphan on every attempt. Skip/Cancel use normal cleanup.
                        move_work.stages.insert(source.entry().identity, stage);
                        return Err(failure);
                    }
                    stage
                } else {
                    if let Some(metadata) = self.metadata.as_mut() {
                        metadata
                            .stages
                            .remove(&source.entry().identity)
                            .ok_or_else(|| {
                                error(
                                    &source.entry().item.path,
                                    io::ErrorKind::InvalidData,
                                    "Prepared metadata stage is missing",
                                )
                            })?
                    } else {
                        work.destination.stage_file(&self.cancel)?
                    }
                };
                let source = work.source.take().expect("regular source");
                work.copy = Some(if let Some(metadata) = self.metadata.as_mut() {
                    let pin = metadata
                        .pins
                        .remove(&source.entry().identity)
                        .ok_or_else(|| {
                            error(
                                &source.entry().item.path,
                                io::ErrorKind::InvalidData,
                                "Prepared source metadata pin is missing",
                            )
                        })?;
                    VerifiedFileCopy::new_with_options(source, stage, Some(pin), self.verification)?
                } else {
                    VerifiedFileCopy::new_with_verification(source, stage, self.verification)?
                });
                self.progress.phase = CopyOperationPhase::Copying;
                Ok(Action::Keep)
            }
            _ => Err(error(
                &source.entry().item.path,
                io::ErrorKind::Unsupported,
                "Copying links and special entries is unsupported.",
            )),
        }
    }

    fn copy_step(&mut self, work: &mut EntryWork) -> Result<Action, FsError> {
        if work.consent == Some(Consent::Replace)
            && (!work.replacement_prepared
                || work
                    .copy
                    .as_ref()
                    .is_some_and(|copy| copy.phase() == CopyPhase::ReadyToPublish))
        {
            let target = work.target.as_ref().expect("consented target pin").clone();
            if target.revalidate(&self.cancel).is_err() {
                work.consent = None;
                work.replacement_prepared = false;
                work.force_prompt = true;
                if let Some(target) = self.observe_target(work)? {
                    return Ok(Action::Conflict(target));
                }
            } else if !work.replacement_prepared {
                work.copy
                    .as_mut()
                    .expect("started copy")
                    .prepare_replacement(
                        &work.name,
                        target,
                        TransferReplacementPolicy::RecoverableCurrentEntry,
                        &self.cancel,
                    )?;
                work.replacement_prepared = true;
            }
        }
        let copy = work.copy.as_mut().expect("started copy");
        if copy.phase() == CopyPhase::ReadyToPublish {
            self.progress.phase = CopyOperationPhase::Publishing;
            let publication = if work.consent == Some(Consent::Replace) {
                copy.publish_replace(&work.name, &self.cancel)?
            } else {
                copy.publish(&work.name, &self.cancel)?
            };
            let proof_eligible = matches!(
                &publication,
                TransferPublication::Published(_) | TransferPublication::Replaced { .. }
            );
            let completed_before = self.progress.files_completed;
            match publication {
                TransferPublication::Published(_) => self.progress.files_completed += 1,
                TransferPublication::Replaced { recovery, .. } => {
                    self.progress.files_completed += 1;
                    self.record_recovery(recovery.path);
                }
                TransferPublication::RecoveryRequired {
                    published,
                    recovery,
                    error,
                } => {
                    if published.is_some() {
                        self.progress.files_completed += 1;
                    }
                    for item in recovery {
                        self.record_recovery(item.path);
                    }
                    self.record(error);
                }
                TransferPublication::PublishedDurabilityUnconfirmed { path, mut error } => {
                    error.message = format!(
                        "Target published at {}; durability is unconfirmed: {}",
                        path.display(),
                        error.message
                    );
                    self.record(error);
                }
            }
            if self.progress.files_completed > completed_before {
                self.work_budget
                    .published(work.entry.item.metadata.size, self.verification);
                self.progress.completed_bytes = self
                    .progress
                    .completed_bytes
                    .checked_add(work.entry.item.metadata.size.unwrap_or(0))
                    .ok_or_else(|| {
                        error(
                            &self.progress.destination_directory,
                            io::ErrorKind::InvalidData,
                            "Completed logical byte total overflow.",
                        )
                    })?;
            }
            if proof_eligible && let Some(move_work) = self.move_work.as_mut() {
                let proof = work
                    .copy
                    .take()
                    .expect("published copy")
                    .into_published_proof(&self.cancel)?;
                self.protected_roots.insert(proof.destination_identity());
                move_work.proofs.push(proof);
            }
            return Ok(Action::Done);
        }
        let before_copy = copy.copied_bytes();
        let before_verify = copy.verified_bytes();
        let result = copy.step(&self.cancel);
        self.progress.copied_bytes = self
            .progress
            .copied_bytes
            .checked_add(copy.copied_bytes() - before_copy)
            .ok_or_else(|| {
                error(
                    &self.progress.destination_directory,
                    io::ErrorKind::InvalidData,
                    "Copied byte total overflow.",
                )
            })?;
        self.progress.verified_bytes = self
            .progress
            .verified_bytes
            .checked_add(copy.verified_bytes() - before_verify)
            .ok_or_else(|| {
                error(
                    &self.progress.destination_directory,
                    io::ErrorKind::InvalidData,
                    "Verified byte total overflow.",
                )
            })?;
        self.progress.phase = match result? {
            CopyPhase::Copying => CopyOperationPhase::Copying,
            CopyPhase::Syncing => CopyOperationPhase::Syncing,
            CopyPhase::Verifying => CopyOperationPhase::Verifying,
            CopyPhase::ReadyToPublish | CopyPhase::Published => CopyOperationPhase::Publishing,
            CopyPhase::Failed | CopyPhase::RecoveryRequired => {
                return Err(error(
                    &self.progress.destination_directory,
                    io::ErrorKind::InvalidData,
                    "The copy failed verification.",
                ));
            }
        };
        Ok(Action::Keep)
    }

    fn wait_for_conflict(&mut self, work: EntryWork, target: TransferEntry) -> Step {
        self.current = Some(work);
        if self.skip_all {
            self.skip_current();
            return Step::Continue;
        }
        let Some(version) = self.version.checked_add(1) else {
            self.record(error(
                &self.progress.destination_directory,
                io::ErrorKind::InvalidData,
                "Conflict version exhausted.",
            ));
            self.drop_current();
            return Step::Continue;
        };
        self.version = version;
        let work = self.current.as_ref().expect("conflicting entry");
        let capabilities = work.destination.capabilities();
        let native_replacement = self.move_work.is_some()
            && !work.native_fallback
            && work.copy.is_none()
            && work.native_identity().is_some_and(|identity| {
                work.destination.supports_native_move_replacement(identity)
            });
        let distinct = work.native_identity() != Some(target.identity)
            && !self.protected_roots.contains(&target.identity);
        let guarded = work.target.is_some() && distinct;
        let target_kind = target.item.kind;
        let conflict = CopyConflict {
            id: self.progress.id,
            version,
            source: work.entry().item.clone(),
            target: target.item,
            destination: work.destination.path().join(&work.name),
            can_replace: guarded
                && (capabilities.recoverable_replacement || native_replacement)
                && work.entry().item.kind == EntryKind::File
                && target_kind == EntryKind::File,
            can_merge: guarded
                && capabilities.directory_merge
                && work.entry().item.kind == EntryKind::Directory
                && target_kind == EntryKind::Directory,
        };
        let automatic = !work.force_prompt;
        self.conflict = Some(conflict.clone());
        self.progress.message =
            "The destination exists. Choose how to resolve the conflict.".into();
        if automatic && self.replace_all && conflict.can_replace {
            self.decision = Some(CopyChoice::ReplaceRecoverable { all: true });
            return Step::Continue;
        }
        if automatic && self.merge_all && conflict.can_merge {
            self.decision = Some(CopyChoice::Merge { all: true });
            return Step::Continue;
        }
        Step::Conflict(Arc::new(conflict))
    }
    fn observe_target(&self, work: &mut EntryWork) -> Result<Option<TransferEntry>, FsError> {
        work.target = None;
        match work.destination.prepare_target(&work.name, &self.cancel) {
            Ok(target) => {
                let entry = target.as_ref().map(|target| target.entry().clone());
                work.target = target;
                Ok(entry)
            }
            Err(failure) if failure.kind == io::ErrorKind::Unsupported => {
                work.destination.inspect(&work.name, &self.cancel)
            }
            Err(failure) => Err(failure),
        }
    }
    fn refresh_conflict(&mut self, mut work: EntryWork) -> Step {
        work.consent = None;
        work.replacement_prepared = false;
        work.force_prompt = true;
        match self.observe_target(&mut work) {
            Ok(Some(target)) => self.wait_for_conflict(work, target),
            Ok(None) => {
                self.current = Some(work);
                Step::Continue
            }
            Err(failure) => {
                self.current = Some(work);
                self.wait_for_failure(failure, failures::Context::Entry)
            }
        }
    }
    fn record(&mut self, error: FsError) {
        self.halt |= self.move_work.is_some();
        self.record_decided_error(error);
    }
    fn record_decided_error(&mut self, error: FsError) {
        retain_error(&mut self.progress, error, false);
    }
    fn record_cleanup_error(&mut self, error: FsError) {
        self.halt |= self.move_work.is_some();
        retain_error(&mut self.progress, error, true);
    }
    fn record_recovery(&mut self, path: std::path::PathBuf) {
        self.progress.recovery_count += 1;
        if self.progress.recovery_paths.len() < 16 {
            self.progress.recovery_paths.push(path);
        }
    }
    fn cleanup_work(&mut self, work: &mut EntryWork) {
        if let Some(copy) = &mut work.copy
            && let Err(error) = copy.cleanup()
        {
            self.record_cleanup_error(error);
        }
        if let Some(link) = &mut work.link
            && let Err(failure) = link.cleanup()
        {
            self.record_link_cleanup(failure);
        }
    }
    fn drop_current(&mut self) {
        if let Some(mut work) = self.current.take() {
            self.cleanup_work(&mut work);
        }
        self.progress.current_path = None;
    }
    fn skip_current(&mut self) {
        if let Some(move_work) = self.move_work.as_mut() {
            for frame in &mut self.stack {
                frame.retain_source = true;
            }
            if let Some(current) = &self.current
                && let Err(error) = move_work.discard(current.entry().identity)
            {
                self.record_cleanup_error(error);
            }
        }
        self.drop_current();
        self.progress.skipped += 1;
        self.progress.phase = CopyOperationPhase::Planning;
    }
    pub(super) fn finish(mut self) -> CopyProgress {
        if let Some(failure) = self.failure.take() {
            self.record_decided_error(failure.error);
        }
        // Clean and drop the active native object in the same order as normal
        // disposal. Keep only its diagnostic until the unused stages and Move
        // recovery finish, so they cannot evict the current partial's path.
        let (current_cleanup, current_link_cleanup) =
            self.current.take().map_or((None, None), |mut work| {
                let file = work.copy.as_mut().and_then(|copy| copy.cleanup().err());
                let link = work.link.as_mut().and_then(|copy| copy.cleanup().err());
                (file, link)
            });
        self.progress.current_path = None;
        if let Some(metadata) = self.metadata.as_mut() {
            for failure in metadata.cleanup() {
                self.record_cleanup_error(failure);
            }
        }
        if let Some(mut move_work) = self.move_work.take() {
            for error in move_work.cleanup(&mut self) {
                self.record_cleanup_error(error);
            }
        }
        if let Some(error) = current_cleanup {
            self.record_cleanup_error(error);
        }
        if let Some(failure) = current_link_cleanup {
            self.record_link_cleanup(failure);
        }
        self.finish_discovery();
        // Keep heartbeat status nonterminal until owned native resources finish
        // dropping. The async caller publishes the returned terminal state only
        // after this worker and its I/O/mutation permits have been released.
        self.store_progress();
        self.progress.state = if self.cancel.is_cancelled() {
            CopyOperationState::Cancelled
        } else if self.progress.failed == 0 && self.progress.skipped == 0 {
            CopyOperationState::Completed
        } else if self.progress.failed > 0
            && self.progress.files_completed == 0
            && self.progress.directories_created == 0
            && self.progress.native_entries_moved == 0
        {
            CopyOperationState::Failed
        } else if self.progress.files_completed > 0
            || self.progress.directories_created > 0
            || self.progress.native_entries_moved > 0
            || self.progress.skipped > 0
        {
            CopyOperationState::CompletedWithErrors
        } else {
            CopyOperationState::Failed
        };
        self.progress.message = self
            .progress
            .errors
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        let omitted = self
            .progress
            .failed
            .saturating_sub(self.progress.errors.len() as u64);
        if omitted > 0 {
            self.progress.message.push_str(&format!(
                "\n{omitted} earlier error details omitted from the bounded diagnostic history."
            ));
        }
        if self.progress.skipped > 0 {
            self.progress.message.push_str(&format!(
                "\n{} entries or subtrees skipped; source data retained.",
                self.progress.skipped
            ));
        }
        if self.cancel.is_cancelled() {
            self.progress
                .message
                .push_str(if self.progress.intent == TransferIntent::Move {
                    "\nMove cancelled; completed moves and already published destinations remain."
                } else {
                    "\nCopy cancelled; already published files are retained."
                });
        }
        if self.progress.recovery_count > 0 {
            self.progress.message.push_str(&format!(
                "\n{} recovery files retained. See the recovery locations in job details.",
                self.progress.recovery_count
            ));
        }
        if self.progress.intent == TransferIntent::Move {
            self.progress.message.push_str(&format!("\n{} entries moved by native rename; {} verified source files removed; {} temporary source locations need attention. Unfinished sources were restored unless an error above reports otherwise. Earlier completed moves remain after later errors or cancellation. Open readers can delay storage reclamation. Merged source directories are removed only after completed children and an empty-directory check.", self.progress.native_entries_moved, self.progress.source_files_removed, self.progress.source_residual_count));
        }
        self.progress.clone()
    }
    fn advance_move(&mut self) -> Step {
        let Some(mut work) = self.move_work.take() else {
            return Step::Done;
        };
        let result = work.advance(self);
        self.move_work = Some(work);
        match result {
            Ok(false) => Step::Continue,
            Ok(true) if self.roots.is_empty() && self.stack.is_empty() => Step::Done,
            Ok(true) => Step::Continue,
            Err(error) => self.wait_for_failure(error, failures::Context::Move),
        }
    }
}
enum Action {
    Keep,
    Done,
    Directory(Arc<dyn TransferDirectory>),
    Conflict(TransferEntry),
}

#[cfg(test)]
#[path = "copy_cleanup_contract_tests.rs"]
mod cleanup_contract;
