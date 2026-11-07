//! Resumable traversal and one verified file at a time. Every method runs on a worker.
use super::*;
use midnight_platform_common::transfer::{CopyPhase, VerifiedFileCopy};
use std::{
    collections::{HashSet, VecDeque},
    ffi::OsString,
    path::Component,
};
#[path = "copy_metadata_worker.rs"]
mod metadata_job;
#[path = "move_worker.rs"]
mod move_job;

pub(super) enum Step {
    Continue,
    Conflict(Arc<CopyConflict>),
    Done,
}
struct Frame {
    source: Box<dyn TransferSource>,
    destination: Arc<dyn TransferDirectory>,
}
struct EntryWork {
    source: Option<Box<dyn TransferSource>>,
    copy: Option<VerifiedFileCopy>,
    destination: Arc<dyn TransferDirectory>,
    name: OsString,
    target: Option<Arc<dyn TransferTarget>>,
    consent: Option<Consent>,
    replacement_prepared: bool,
    force_prompt: bool,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Consent {
    Replace,
    Merge,
}
impl EntryWork {
    fn native_identity(&self) -> Option<TransferIdentity> {
        self.source.as_ref().map_or_else(
            || {
                self.copy
                    .as_ref()
                    .and_then(VerifiedFileCopy::source_native_identity)
            },
            |source| source.native_identity(),
        )
    }
    fn entry(&self) -> &TransferEntry {
        if let Some(source) = &self.source {
            source.entry()
        } else {
            self.copy
                .as_ref()
                .expect("entry owns a source or verified copy")
                .source_entry()
        }
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
    decision: Option<CopyChoice>,
    version: u64,
    skip_all: bool,
    replace_all: bool,
    merge_all: bool,
    move_work: Option<move_job::MoveWork>,
    metadata: Option<metadata_job::MetadataWork>,
}
impl Job {
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
        let pins = metadata_job::pins(&job.roots, &job.cancel)?;
        let stages =
            metadata_job::stages(&job.roots, &job.destination, metadata, false, &job.cancel)?;
        job.metadata = Some(metadata_job::MetadataWork { pins, stages });
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
            let source = service.prepare_transfer_source(path, &cancel)?;
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
        Ok(Self::from_sources(destination, roots, cancel, status))
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
        Self {
            progress: snapshot(&status),
            status,
            cancel,
            destination,
            roots,
            protected_roots,
            stack: vec![],
            current: None,
            conflict: None,
            decision: None,
            version: 0,
            skip_all: false,
            replace_all: false,
            merge_all: false,
            move_work: None,
            metadata: None,
        }
    }

    pub(super) fn accept_decision(&mut self, decision: CopyDecision) -> bool {
        if self.conflict.as_ref().is_some_and(|conflict| {
            conflict.id == decision.id && conflict.version == decision.version
        }) {
            self.decision = Some(decision.choice);
            true
        } else {
            false
        }
    }

    pub(super) fn step(&mut self) -> Step {
        let step = self.advance();
        self.progress.state = if self.cancel.is_cancelled() {
            CopyOperationState::CancelRequested
        } else {
            CopyOperationState::Running
        };
        *self.status.lock().unwrap_or_else(|e| e.into_inner()) = self.progress.clone();
        step
    }
    fn advance(&mut self) -> Step {
        if self.cancel.is_cancelled() {
            return Step::Done;
        }
        if let Some(choice) = self.decision.take() {
            let conflict = self.conflict.take();
            match choice {
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
        if let Some(work) = self.current.take() {
            return self.process(work);
        }
        if self.move_work.is_some() && self.roots.is_empty() && self.stack.is_empty() {
            return self.advance_move();
        }
        self.progress.phase = CopyOperationPhase::Planning;
        self.progress.current_path = None;
        let next = if let Some(frame) = self.stack.last_mut() {
            match frame.source.next_child(&self.cancel) {
                Ok(Some(source)) => Some((source, frame.destination.clone())),
                Ok(None) => {
                    self.stack.pop();
                    return Step::Continue;
                }
                Err(error) => {
                    self.record(error);
                    self.stack.pop();
                    return Step::Continue;
                }
            }
        } else {
            self.roots
                .pop_front()
                .map(|source| (source, self.destination.clone()))
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
            source: Some(source),
            copy: None,
            destination,
            name,
            target: None,
            consent: None,
            replacement_prepared: false,
            force_prompt: false,
        });
        Step::Continue
    }

    fn process(&mut self, mut work: EntryWork) -> Step {
        self.progress.current_path = Some(work.entry().item.path.clone());
        let result = if work.copy.is_some() {
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
                        self.cleanup_work(&mut work);
                        self.record(error);
                        Step::Continue
                    }
                }
            }
            Err(error) => {
                self.cleanup_work(&mut work);
                self.record(error);
                Step::Continue
            }
        }
    }

    fn prepare_entry(&mut self, work: &mut EntryWork) -> Result<Action, FsError> {
        work.source
            .as_ref()
            .expect("unstarted entry source")
            .revalidate(&self.cancel)?;
        if work.consent.is_none()
            && let Some(target) = self.observe_target(work)?
        {
            return Ok(Action::Conflict(target));
        }
        let source = work.source.as_ref().expect("unstarted entry source");
        match source.entry().item.kind {
            EntryKind::Directory => {
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
                let stage = if let Some(move_work) = self.move_work.as_mut() {
                    move_work
                        .stages
                        .remove(&source.entry().identity)
                        .ok_or_else(|| {
                            error(
                                &source.entry().item.path,
                                io::ErrorKind::InvalidData,
                                "Prepared Move stage is missing; all sources retained.",
                            )
                        })?
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
                    pin.revalidate(&self.cancel)?;
                    VerifiedFileCopy::new_with_modified_time(source, stage, pin)?
                } else {
                    VerifiedFileCopy::new(source, stage)?
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
                && capabilities.recoverable_replacement
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
                self.cleanup_work(&mut work);
                self.record(failure);
                Step::Continue
            }
        }
    }
    fn record(&mut self, error: FsError) {
        self.progress.failed += 1;
        if self.progress.errors.len() < 16 {
            self.progress.errors.push(error);
        }
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
            self.record(error);
        }
    }
    fn drop_current(&mut self) {
        if let Some(mut work) = self.current.take() {
            self.cleanup_work(&mut work);
        }
        self.progress.current_path = None;
    }
    fn skip_current(&mut self) {
        self.drop_current();
        self.progress.skipped += 1;
        self.progress.phase = CopyOperationPhase::Planning;
    }
    pub(super) fn finish(mut self) -> CopyProgress {
        self.drop_current();
        if let Some(metadata) = self.metadata.as_mut() {
            for failure in metadata.cleanup() {
                self.record(failure);
            }
        }
        if let Some(move_work) = self.move_work.as_mut() {
            for error in move_work.cleanup() {
                self.record(error);
            }
        }
        self.progress.state = if self.cancel.is_cancelled() {
            CopyOperationState::Cancelled
        } else if self.progress.failed == 0 && self.progress.skipped == 0 {
            CopyOperationState::Completed
        } else if self.progress.files_completed > 0
            || self.progress.directories_created > 0
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
        if self.progress.skipped > 0 {
            self.progress.message.push_str(&format!(
                "\n{} entries or subtrees skipped; source data retained.",
                self.progress.skipped
            ));
        }
        if self.cancel.is_cancelled() {
            self.progress
                .message
                .push_str("\nCopy cancelled; already published files are retained.");
        }
        if self.progress.recovery_count > 0 {
            self.progress.message.push_str(&format!(
                "\n{} recovery files retained. See the recovery locations in job details.",
                self.progress.recovery_count
            ));
        }
        if self.progress.intent == TransferIntent::RecoverableMove {
            self.progress.message.push_str(&format!("\n{} source names relocated; {} originals retained in source recovery. Disk space has not been reclaimed. Directory Move is not supported in this increment.", self.progress.source_names_captured, self.progress.source_recovery_count));
        }
        *self.status.lock().unwrap_or_else(|e| e.into_inner()) = self.progress.clone();
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
            Ok(true) => Step::Done,
            Err(error) => {
                self.record(error);
                Step::Done
            }
        }
    }
}
enum Action {
    Keep,
    Done,
    Directory(Arc<dyn TransferDirectory>),
    Conflict(TransferEntry),
}
