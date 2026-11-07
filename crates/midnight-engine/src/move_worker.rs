//! Bounded ordinary-file Move. Verify and remove each source before copying the next.
use super::*;
use midnight_platform_common::transfer::{FileProofPhase, VerifiedFileProof};
use std::collections::HashMap;

const MAX_MOVE_FILES: usize = 64;
const MAX_PROOF_PATH_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy)]
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
    pub(super) stages: HashMap<TransferIdentity, Box<dyn StagedTransferFile>>,
    pub(super) proofs: Vec<VerifiedFileProof>,
    phase: Phase,
    finalizer: Option<Box<dyn TransferMoveFinalizer>>,
    reading: bool,
    buffer: Vec<u8>,
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
    if request.clipboard.paths.len() > MAX_MOVE_FILES {
        return Err(unsupported(
            &request.destination,
            "This verified transfer policy accepts at most 64 regular files. Choose content-only Copy for larger or recursive selections.",
        ));
    }
    // Reserve repeated native source/parent/stage/proof and receipt path storage,
    // including recovery suffixes. Reject the whole input before creating stages.
    let mut bytes = 0usize;
    for source in &request.clipboard.paths {
        bytes = bytes
            .saturating_add(
                source
                    .as_os_str()
                    .as_encoded_bytes()
                    .len()
                    .saturating_mul(32),
            )
            .saturating_add(
                request
                    .destination
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
    let destination = service.prepare_move_destination(&request.destination, &cancel)?;
    if destination.path() != request.destination {
        return Err(unsupported(
            &request.destination,
            "Provider returned a different Move destination.",
        ));
    }
    let mut roots = VecDeque::new();
    let mut captures = Vec::new();
    let mut identities = HashSet::new();
    for path in &request.clipboard.paths {
        let mut prepared = service.prepare_move_source(path, &cancel)?;
        let source = prepared.source.entry();
        if source.item.path != *path
            || source.item.kind != EntryKind::File
            || source.item.metadata.size.is_none()
            || prepared.source.native_identity() != Some(source.identity)
            || prepared.capture.entry().identity != source.identity
            || prepared.capture.entry().item.path != *path
        {
            return Err(unsupported(
                path,
                "Recoverable Move currently accepts native regular-file roots only. The entire selection was retained; choose Copy for directories, links or virtual files.",
            ));
        }
        if !identities.insert(source.identity) {
            return Err(unsupported(
                path,
                "The same native source appears more than once. All source names were retained.",
            ));
        }
        prepared.capture.revalidate(&cancel)?;
        prepared.capture.preflight_finalization(&cancel)?;
        prepared
            .source
            .rewind_for_validation(&cancel)?
            .revalidate(&cancel)?;
        roots.push_back(prepared.source);
        captures.push(prepared.capture);
    }
    let pins = if metadata == TransferMetadataPolicy::RequireModifiedTime {
        Some(super::metadata_job::pins(&roots, &cancel)?)
    } else {
        None
    };
    let stages = super::metadata_job::stages(&roots, &destination, metadata, true, &cancel)?;
    let mut job = Job::from_sources(destination, roots, cancel, status);
    job.metadata = pins.map(|pins| super::metadata_job::MetadataWork {
        pins,
        stages: HashMap::new(),
    });
    job.progress.intent = TransferIntent::RecoverableMove;
    job.move_work = Some(MoveWork {
        captures,
        stages,
        proofs: vec![],
        phase: Phase::Copying,
        finalizer: None,
        reading: false,
        buffer: vec![],
    });
    Ok(job)
}

impl MoveWork {
    pub(super) fn has_pending(&self) -> bool {
        !self.proofs.is_empty()
    }
    pub(super) fn advance(&mut self, job: &mut Job) -> Result<bool, FsError> {
        let result = self.advance_inner(job);
        if result.is_err() {
            self.phase = Phase::Halted;
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
        if let Some(parent) = path.parent()
            && !job
                .progress
                .source_directories
                .iter()
                .any(|old| old == parent)
        {
            job.progress.source_directories.push(parent.to_owned());
        }
        match outcome {
            TransferMoveCaptureOutcome::Captured { source, recovery } => {
                let matched = source == path && recovery.identity == Some(expected);
                record_source(job, source, recovery, matched);
                if !matched {
                    return Err(unsupported(
                        &path,
                        "Source capture returned an unexpected identity. Captured data was retained; later sources remain untouched.",
                    ));
                }
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
                    let matched = source == path && retained.identity == Some(expected);
                    record_source(job, source.clone(), retained, matched);
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
            TransferMoveFinalizeOutcome::Retained { error, .. } => return Err(error),
            TransferMoveFinalizeOutcome::Removed {
                source,
                recovery,
                state,
                durability_confirmed,
            } => {
                job.progress.source_files_removed += 1;
                if state == TransferSourceRemoval::DeletePending {
                    job.progress.source_delete_pending += 1;
                }
                if !durability_confirmed {
                    job.progress.source_removal_durability_unconfirmed += 1;
                }
                job.progress.source_recovery_count -= 1;
                job.progress.recovery_count -= 1;
                job.progress
                    .source_recoveries
                    .retain(|item| item.source != source);
                job.progress
                    .recovery_paths
                    .retain(|path| *path != recovery.path);
                let identity = self.proofs[0].source_entry().identity;
                // No further source queries after Windows normal disposition. Close
                // EVERY application owner before starting the next file's Copy.
                self.proofs.clear();
                self.finalizer = None;
                self.captures
                    .retain(|capture| capture.entry().identity != identity);
                self.buffer = Vec::new();
                self.phase = Phase::Copying;
                if state == TransferSourceRemoval::Unlinked && !durability_confirmed {
                    return Err(unsupported(
                        &source,
                        "Verified source was removed, but recovery-directory sync failed; removal durability is unconfirmed. Remaining sources retained.",
                    ));
                }
            }
        }
        Ok(false)
    }
    pub(super) fn cleanup(&mut self) -> Vec<FsError> {
        let errors = self
            .stages
            .values_mut()
            .filter_map(|stage| stage.cleanup().err())
            .collect();
        self.stages.clear();
        self.proofs.clear();
        self.finalizer = None;
        self.captures.clear();
        errors
    }
}
fn record_source(
    job: &mut Job,
    source: std::path::PathBuf,
    recovery: TransferRecovery,
    matched_source: bool,
) {
    job.progress.source_recovery_count += 1;
    if matched_source {
        job.progress.source_names_captured += 1;
    }
    job.record_recovery(recovery.path.clone());
    if job.progress.source_recoveries.len() < 16 {
        job.progress.source_recoveries.push(MoveSourceRecovery {
            source,
            recovery,
            matched_source,
        });
    }
}
pub(super) fn permit(
    cancel: &Cancellation,
    path: &Path,
) -> Result<midnight_work_budget::Permit, FsError> {
    midnight_work_budget::shared_io()
        .acquire(midnight_work_budget::IO_SLOTS, || cancel.is_cancelled())
        .ok_or_else(|| {
            error(
                path,
                io::ErrorKind::Interrupted,
                "Move cancelled; source and recovery data retained.",
            )
        })
}
fn unsupported(path: &Path, message: &str) -> FsError {
    error(path, io::ErrorKind::Unsupported, message)
}
