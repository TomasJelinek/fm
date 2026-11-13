//! Bounded Move authority. Native rename is attempted before copy-only proof work.
use super::*;
use midnight_platform_common::transfer::{FileProofPhase, VerifiedFileProof};
use std::collections::HashMap;
#[path = "move_retry.rs"]
mod retry;
#[cfg(test)]
#[path = "move_retry_tests.rs"]
mod retry_tests;

const MAX_MOVE_ROOTS: usize = 64;
const MAX_MOVE_ENTRIES: usize = 100_000;
pub(super) const MAX_MOVE_DIRECTORY_DEPTH: usize = 128;
const MAX_PROOF_PATH_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Copying,
    Before,
    Capture,
    After,
    Finalize,
    Halted,
}

pub(super) struct MoveWork {
    captures: Vec<Arc<dyn TransferMoveCapture>>,
    pub(super) metadata_policy: TransferMetadataPolicy,
    pub(super) stages: HashMap<TransferIdentity, Box<dyn StagedTransferFile>>,
    pub(super) proofs: Vec<VerifiedFileProof>,
    phase: Phase,
    finalizer: Option<Box<dyn TransferMoveFinalizer>>,
    active: Option<(std::path::PathBuf, TransferRecovery)>,
    reading: bool,
    buffer: Vec<u8>,
    admitted: usize,
    refresh_parents: Vec<std::path::PathBuf>,
    failed_phase: Option<Phase>,
    retryable_failure: bool,
}

pub(super) fn prepare(
    request: CopyRequest,
    service: &dyn TransferService,
    cancel: Cancellation,
    status: Status,
) -> Result<Job, FsError> {
    prepare_with_metadata(
        request,
        service,
        cancel,
        status,
        TransferMetadataPolicy::ContentOnly,
    )
}
pub(super) fn check_bounds(request: &CopyRequest) -> Result<(), FsError> {
    request.clipboard.validate()?;
    check_root_bounds(
        &request.destination,
        request.clipboard.paths.iter().map(|path| path.as_path()),
    )
}
pub(super) fn check_root_bounds<'a>(
    destination: &Path,
    paths: impl IntoIterator<Item = &'a Path>,
) -> Result<(), FsError> {
    let paths: Vec<_> = paths.into_iter().collect();
    if paths.len() > MAX_MOVE_ROOTS {
        return Err(unsupported(
            destination,
            "Move accepts at most 64 selected roots. Choose a smaller selection.",
        ));
    }
    // Reserve repeated native source/parent/stage/proof and receipt path storage,
    // including recovery suffixes. Reject the whole input before creating stages.
    let mut bytes = 0usize;
    for source in paths {
        bytes = bytes
            .saturating_add(
                source
                    .as_os_str()
                    .as_encoded_bytes()
                    .len()
                    .saturating_mul(32),
            )
            .saturating_add(
                destination
                    .as_os_str()
                    .as_encoded_bytes()
                    .len()
                    .saturating_mul(16),
            )
            .saturating_add(8192);
        if bytes > MAX_PROOF_PATH_BYTES {
            return Err(unsupported(
                source,
                "Verified transfer proof paths exceed the 4 MiB budget. Choose content-only Copy or a smaller batch.",
            ));
        }
    }
    Ok(())
}
pub(super) fn prepare_with_metadata(
    request: CopyRequest,
    service: &dyn TransferService,
    cancel: Cancellation,
    status: Status,
    metadata: TransferMetadataPolicy,
) -> Result<Job, FsError> {
    check_bounds(&request)?;
    let _permit = permit(&cancel, &request.destination)?;
    let mut prepared_roots = VecDeque::new();
    for path in &request.clipboard.paths {
        let prepared = service.prepare_move_source(path, &cancel)?;
        if prepared.source.entry().item.path != *path || prepared.capture.entry().item.path != *path
        {
            return Err(unsupported(
                path,
                "Provider returned a different Move source.",
            ));
        }
        prepared_roots.push_back(prepared);
    }
    let mut job = prepare_sources_inner(
        request.destination,
        prepared_roots,
        service,
        cancel,
        status,
        metadata,
    )?;
    job.set_verification(request.verification);
    Ok(job)
}

pub(super) fn prepare_sources(
    destination_path: std::path::PathBuf,
    prepared_roots: VecDeque<PreparedTransferMove>,
    service: &dyn TransferService,
    cancel: Cancellation,
    status: Status,
    metadata: TransferMetadataPolicy,
) -> Result<Job, FsError> {
    let _permit = permit(&cancel, &destination_path)?;
    prepare_sources_inner(
        destination_path,
        prepared_roots,
        service,
        cancel,
        status,
        metadata,
    )
}
fn prepare_sources_inner(
    destination_path: std::path::PathBuf,
    prepared_roots: VecDeque<PreparedTransferMove>,
    service: &dyn TransferService,
    cancel: Cancellation,
    status: Status,
    metadata: TransferMetadataPolicy,
) -> Result<Job, FsError> {
    check_root_bounds(
        &destination_path,
        prepared_roots
            .iter()
            .map(|root| root.source.entry().item.path.as_path()),
    )?;
    let admitted = prepared_roots.len();
    let refresh_parents = prepared_roots
        .iter()
        .filter_map(|root| root.source.entry().item.path.parent().map(Path::to_owned))
        .collect();
    let destination = service.prepare_move_destination(&destination_path, &cancel)?;
    if destination.path() != destination_path {
        return Err(unsupported(
            &destination_path,
            "Provider returned a different Move destination.",
        ));
    }
    let mut roots = VecDeque::new();
    let mut captures = Vec::new();
    let mut identities = HashSet::new();
    for prepared in prepared_roots {
        let path = prepared.source.entry().item.path.clone();
        let source = prepared.source.entry();
        if source.item.path != path
            || !matches!(source.item.kind, EntryKind::File | EntryKind::Directory)
            || (source.item.kind == EntryKind::File && source.item.metadata.size.is_none())
            || prepared.source.native_identity() != Some(source.identity)
            || prepared.capture.entry().item.kind != source.item.kind
            || prepared.capture.entry().item.metadata != source.item.metadata
            || prepared.capture.entry().identity != source.identity
            || prepared.capture.entry().item.path != path
        {
            return Err(unsupported(
                &path,
                "Move requires native ordinary files or directories with matching mutation authority. Links, special and virtual sources are unsupported.",
            ));
        }
        if !identities.insert(source.identity) {
            return Err(unsupported(
                &path,
                "The same native source appears more than once. All source names were retained.",
            ));
        }
        prepared.capture.revalidate(&cancel)?;
        if source.item.kind == EntryKind::Directory {
            destination.reject_source_ancestor(source.identity, &cancel)?;
        }
        if metadata == TransferMetadataPolicy::RequireModifiedTime
            && source.item.kind != EntryKind::File
        {
            return Err(unsupported(
                &path,
                "Required modified time accepts native regular files only.",
            ));
        }
        roots.push_back(prepared.source);
        captures.push(prepared.capture);
    }
    // Identity-based ancestry includes aliases/case variants, not just path prefixes.
    if roots
        .iter()
        .any(|source| source.entry().item.kind == EntryKind::Directory)
    {
        for source in &roots {
            let parent = source.entry().item.path.parent().ok_or_else(|| {
                unsupported(&source.entry().item.path, "Move source needs a parent.")
            })?;
            let directory = service.prepare_move_destination(parent, &cancel)?;
            if directory.path() != parent {
                return Err(unsupported(
                    parent,
                    "Provider returned a different Move source parent.",
                ));
            }
            for other in &roots {
                if other.entry().item.kind == EntryKind::Directory {
                    directory.reject_source_ancestor(other.entry().identity, &cancel)?;
                }
            }
        }
    }
    let mut job = Job::from_sources(destination, roots, cancel, status);
    job.progress.intent = TransferIntent::Move;
    job.move_work = Some(MoveWork {
        captures,
        metadata_policy: metadata,
        stages: HashMap::new(),
        proofs: vec![],
        phase: Phase::Copying,
        finalizer: None,
        active: None,
        reading: false,
        buffer: vec![],
        admitted,
        failed_phase: None,
        retryable_failure: false,
        refresh_parents,
    });
    Ok(job)
}

impl MoveWork {
    pub(super) fn capture_for(
        &self,
        entry: &TransferEntry,
    ) -> Result<Arc<dyn TransferMoveCapture>, FsError> {
        self.captures
            .iter()
            .find(|capture| capture.entry().identity == entry.identity)
            .cloned()
            .ok_or_else(|| unsupported(&entry.item.path, "Move source authority missing."))
    }
    pub(super) fn native_completed(
        &mut self,
        entry: &TransferEntry,
        job: &mut Job,
    ) -> Result<(), FsError> {
        self.discard(entry.identity)?;
        self.record_parents(job, &entry.item.path);
        Ok(())
    }
    pub(super) fn record_parents(&self, job: &mut Job, path: &Path) {
        // Reserve the bounded refresh list for selected-root parents first.
        // Deep trees must not crowd out the panels where the move was requested.
        for parent in self
            .refresh_parents
            .iter()
            .map(std::path::PathBuf::as_path)
            .chain(path.parent())
        {
            if job.progress.source_directories.len() < 64
                && !job
                    .progress
                    .source_directories
                    .iter()
                    .any(|old| old == parent)
            {
                job.progress.source_directories.push(parent.to_owned());
            }
        }
    }
    pub(super) fn admit_child(
        &mut self,
        prepared: midnight_core::PreparedTransferMove,
        parent: &Path,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        let entry = prepared.source.entry();
        if self.admitted >= MAX_MOVE_ENTRIES
            || self
                .captures
                .iter()
                .map(|capture| {
                    capture
                        .entry()
                        .item
                        .path
                        .as_os_str()
                        .as_encoded_bytes()
                        .len()
                        .saturating_mul(32)
                        .saturating_add(8192)
                })
                .sum::<usize>()
                .saturating_add(
                    entry
                        .item
                        .path
                        .as_os_str()
                        .as_encoded_bytes()
                        .len()
                        .saturating_mul(32)
                        .saturating_add(8192),
                )
                > MAX_PROOF_PATH_BYTES
        {
            return Err(unsupported(
                &entry.item.path,
                "Recursive Move reached its entry or active-path budget; completed moves remain.",
            ));
        }
        if entry.item.path.parent() != Some(parent)
            || entry.item.path.file_name().is_none()
            || prepared.source.native_identity() != Some(entry.identity)
            || prepared.capture.entry().identity != entry.identity
            || prepared.capture.entry().item.path != entry.item.path
            || !matches!(entry.item.kind, EntryKind::File | EntryKind::Directory)
            || self
                .captures
                .iter()
                .any(|capture| capture.entry().identity == entry.identity)
        {
            return Err(unsupported(
                &entry.item.path,
                "Recursive Move child authority is invalid or overlaps an active source.",
            ));
        }
        prepared.source.revalidate(cancel)?;
        prepared.capture.revalidate(cancel)?;
        self.admitted += 1;
        self.captures.push(prepared.capture);
        Ok(prepared.source)
    }
    pub(super) fn discard(&mut self, identity: TransferIdentity) -> Result<(), FsError> {
        self.captures
            .retain(|capture| capture.entry().identity != identity);
        if let Some(mut stage) = self.stages.remove(&identity) {
            stage.cleanup()?;
        }
        Ok(())
    }
    pub(super) fn remove_directory(
        &mut self,
        entry: &TransferEntry,
        job: &mut Job,
    ) -> Result<(), FsError> {
        let capture = self
            .captures
            .iter()
            .find(|capture| capture.entry().identity == entry.identity)
            .ok_or_else(|| unsupported(&entry.item.path, "Move directory authority missing."))?;
        let _permit = permit(&job.cancel, &entry.item.path)?;
        let outcome = capture.remove_empty_directory(&job.cancel)?;
        if outcome.state == TransferSourceRemoval::DeletePending {
            job.progress.source_delete_pending += 1;
        }
        if !outcome.durability_confirmed {
            job.progress.source_removal_durability_unconfirmed += 1;
        }
        // Directory handles close with this frame before any subsequent traversal.
        self.captures
            .retain(|capture| capture.entry().identity != entry.identity);
        self.record_parents(job, &entry.item.path);
        Ok(())
    }
    /// Logical proof reads retained for the current file, not cumulative retry I/O.
    pub(super) fn active_proof_credit(&self, policy: TransferVerification) -> Option<u64> {
        let Some(proof) = self.proofs.first() else {
            return Some(0);
        };
        if self.proofs.len() != 1 {
            return None;
        }
        let pass = policy
            .readback_bytes(proof.source_entry().item.metadata.size?)
            .checked_mul(2)?;
        let current = if self.reading {
            let read = proof.validation_bytes();
            if read > pass {
                return None;
            }
            read
        } else {
            0
        };
        match self.phase {
            Phase::Copying => Some(0),
            Phase::Before => Some(current),
            Phase::Capture => Some(pass),
            Phase::After => pass.checked_add(current),
            Phase::Finalize => pass.checked_mul(2),
            Phase::Halted => None,
        }
    }
    pub(super) fn has_pending(&self) -> bool {
        !self.proofs.is_empty()
    }
    pub(super) fn advance(&mut self, job: &mut Job) -> Result<bool, FsError> {
        let attempted = self.phase;
        let result = self.advance_inner(job);
        if let Err(error) = &result {
            self.failed_phase = Some(attempted);
            self.retryable_failure = !job.cancel.is_cancelled() && retry::retryable(error);
            self.phase = Phase::Halted;
        } else if self.phase != Phase::Halted {
            self.failed_phase = None;
            self.retryable_failure = false;
        }
        result
    }
    fn advance_inner(&mut self, job: &mut Job) -> Result<bool, FsError> {
        if self.proofs.is_empty() {
            return Ok(true);
        }
        if self.proofs.len() != 1 {
            return Err(unsupported(
                &job.progress.destination_directory,
                "Move requires one active verified file at a time.",
            ));
        }
        match self.phase {
            Phase::Copying => {
                self.buffer = vec![0; 256 * 1024];
                self.phase = Phase::Before;
                self.reading = false;
                job.progress.phase = CopyOperationPhase::ValidatingBeforeMove;
                Ok(false)
            }
            Phase::Before | Phase::After => self.validate(job),
            Phase::Capture => self.capture(job),
            Phase::Finalize => self.finalize(job),
            Phase::Halted => Ok(true),
        }
    }
    fn validate(&mut self, job: &mut Job) -> Result<bool, FsError> {
        let before = matches!(self.phase, Phase::Before);
        job.progress.phase = if before {
            CopyOperationPhase::ValidatingBeforeMove
        } else {
            CopyOperationPhase::ValidatingAfterMove
        };
        let proof = &mut self.proofs[0];
        job.progress.current_path = Some(proof.source_entry().item.path.clone());
        if !self.reading {
            proof.begin_validation(&job.cancel)?;
            self.reading = true;
            return Ok(false);
        }
        let bytes = proof.validation_bytes();
        let result = proof.validation_step(&mut self.buffer, &job.cancel);
        job.progress.validation_bytes = job
            .progress
            .validation_bytes
            .checked_add(proof.validation_bytes() - bytes)
            .ok_or_else(|| {
                unsupported(
                    &job.progress.destination_directory,
                    "Move validation byte count overflow.",
                )
            })?;
        if result? == FileProofPhase::Complete {
            self.reading = false;
            self.phase = if before {
                Phase::Capture
            } else {
                Phase::Finalize
            };
        }
        Ok(false)
    }
    fn capture(&mut self, job: &mut Job) -> Result<bool, FsError> {
        let proof = &self.proofs[0];
        proof.revalidate_finalization(&job.cancel)?;
        let expected = proof.source_entry().identity;
        let capture = self
            .captures
            .iter()
            .find(|capture| capture.entry().identity == expected)
            .ok_or_else(|| {
                unsupported(
                    &proof.source_entry().item.path,
                    "Move source authority missing.",
                )
            })?;
        let path = capture.entry().item.path.clone();
        job.progress.current_path = Some(path.clone());
        job.progress.phase = CopyOperationPhase::CapturingSources;
        let outcome = {
            let _permit = permit(&job.cancel, &path)?;
            capture.revalidate(&job.cancel)?;
            capture.capture(&job.cancel)?
        };
        self.record_parents(job, &path);
        match outcome {
            TransferMoveCaptureOutcome::Captured { source, recovery } => {
                let matched = source == path && recovery.identity == Some(expected);
                self.active = Some((source, recovery));
                if !matched {
                    return Err(unsupported(
                        &path,
                        "Source capture returned an unexpected identity. Captured data was retained; later sources remain untouched.",
                    ));
                }
                job.progress.source_names_staged += 1;
                let _permit = permit(&job.cancel, &path)?;
                self.finalizer = Some(capture.prepare_finalization(&job.cancel)?);
                self.phase = Phase::After;
            }
            TransferMoveCaptureOutcome::Restored { error, .. } => return Err(error),
            TransferMoveCaptureOutcome::RecoveryRequired {
                source,
                recovery,
                error,
            } => {
                for retained in recovery {
                    self.active = Some((source.clone(), retained));
                }
                return Err(error);
            }
        }
        Ok(false)
    }
    fn finalize(&mut self, job: &mut Job) -> Result<bool, FsError> {
        self.proofs[0].revalidate_finalization(&job.cancel)?;
        let finalizer = self.finalizer.as_mut().ok_or_else(|| {
            unsupported(
                &job.progress.destination_directory,
                "Move finalization authority missing.",
            )
        })?;
        let outcome = {
            let _permit = permit(&job.cancel, &job.progress.destination_directory)?;
            finalizer.revalidate(&job.cancel)?;
            finalizer.finalize(&job.cancel)
        };
        match outcome {
            TransferMoveFinalizeOutcome::Retained {
                source,
                recovery,
                error,
            } => {
                if !self.active.as_ref().is_some_and(|(path, expected)| {
                    *path == source
                        && expected.path == recovery.path
                        && expected.identity == recovery.identity
                }) {
                    return Err(unsupported(
                        &source,
                        "Move finalizer returned an unexpected retained source receipt.",
                    ));
                }
                return Err(error);
            }
            TransferMoveFinalizeOutcome::Removed {
                source,
                recovery,
                state,
                durability_confirmed,
            } => {
                job.progress.source_files_removed += 1;
                job.work_budget.removed(
                    self.proofs[0].source_entry().item.metadata.size,
                    job.verification,
                );
                if state == TransferSourceRemoval::DeletePending {
                    job.progress.source_delete_pending += 1;
                }
                if !durability_confirmed {
                    job.progress.source_removal_durability_unconfirmed += 1;
                }
                let identity = self.proofs[0].source_entry().identity;
                let cleanup = self
                    .captures
                    .iter()
                    .find(|capture| capture.entry().identity == identity)
                    .and_then(|capture| capture.take_stage_cleanup());
                // No source queries after disposition. Close EVERY source/DELETE
                // owner before removing the empty transient directory or copying
                // the next file. Destination proof owners also end here.
                self.proofs.clear();
                self.finalizer = None;
                self.captures
                    .retain(|capture| capture.entry().identity != identity);
                self.buffer = Vec::new();
                self.phase = Phase::Copying;
                self.active = None;
                if let Some(cleanup) = cleanup {
                    let path = cleanup.path().to_owned();
                    if let Err(error) = cleanup.cleanup() {
                        record_source(
                            job,
                            source.clone(),
                            TransferRecovery {
                                path,
                                identity: None,
                            },
                            false,
                        );
                        return Err(error);
                    }
                } else {
                    record_source(job, source.clone(), recovery, false);
                    return Err(unsupported(
                        &source,
                        "Move provider omitted temporary-directory cleanup authority after source removal.",
                    ));
                }
                if state == TransferSourceRemoval::Unlinked && !durability_confirmed {
                    return Err(unsupported(
                        &source,
                        "Verified source was removed, but temporary-directory sync failed; removal durability is unconfirmed. Remaining sources retained.",
                    ));
                }
            }
        }
        Ok(false)
    }
    pub(super) fn cleanup(&mut self, job: &mut Job) -> Vec<FsError> {
        self.failed_phase = None;
        self.retryable_failure = false;
        let mut errors: Vec<_> = self
            .stages
            .values_mut()
            .filter_map(|stage| stage.cleanup().err())
            .collect();
        self.stages.clear();
        self.proofs.clear();
        self.finalizer = None;
        let mut tickets = Vec::new();
        for capture in self.captures.drain(..) {
            let source = capture.entry().item.path.clone();
            let restored = match capture.restore() {
                Ok(()) => true,
                Err(error) => {
                    errors.push(error);
                    capture.restoration_completed()
                }
            };
            if restored {
                if let Some(cleanup) = capture.take_stage_cleanup() {
                    tickets.push((source, cleanup));
                }
            } else if let Some((path, retained)) = &self.active
                && *path == source
            {
                record_source(
                    job,
                    source,
                    retained.clone(),
                    retained.identity == Some(capture.entry().identity),
                );
            }
            // Drops the capture's exact source handle before cleanup tickets run.
        }
        for (source, cleanup) in tickets {
            let path = cleanup.path().to_owned();
            if let Err(error) = cleanup.cleanup() {
                record_source(
                    job,
                    source,
                    TransferRecovery {
                        path,
                        identity: None,
                    },
                    false,
                );
                errors.push(error);
            }
        }
        self.active = None;
        errors
    }
}
fn record_source(
    job: &mut Job,
    source: std::path::PathBuf,
    recovery: TransferRecovery,
    matched_source: bool,
) {
    job.progress.source_residual_count += 1;
    if job.progress.source_residuals.len() < 16 {
        job.progress.source_residuals.push(MoveSourceResidual {
            source,
            temporary: recovery,
            matched_source,
        });
    }
}
pub(super) fn permit(
    cancel: &Cancellation,
    path: &Path,
) -> Result<midnight_work_budget::Permit, FsError> {
    midnight_work_budget::shared_io()
        .acquire(midnight_work_budget::Admission::Exclusive, || {
            cancel.is_cancelled()
        })
        .ok_or_else(|| {
            error(
                path,
                io::ErrorKind::Interrupted,
                "Move cancelled; unfinished sources will be restored without replacement.",
            )
        })
}
fn unsupported(path: &Path, message: &str) -> FsError {
    error(path, io::ErrorKind::Unsupported, message)
}
