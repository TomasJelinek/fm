//! Bounded ordinary-file Move. Capture only follows the complete batch proof pass.
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
    Complete,
    Halted,
}

pub(super) struct MoveWork {
    captures: Vec<Arc<dyn TransferMoveCapture>>,
    pub(super) stages: HashMap<TransferIdentity, Box<dyn StagedTransferFile>>,
    pub(super) proofs: Vec<VerifiedFileProof>,
    phase: Phase,
    cursor: usize,
    reading: bool,
    buffer: Vec<u8>,
}

pub(super) fn prepare(
    request: CopyRequest,
    service: &dyn TransferService,
    cancel: Cancellation,
    status: Status,
) -> Result<Job, FsError> {
    request.clipboard.validate()?;
    if request.clipboard.paths.len() > MAX_MOVE_FILES {
        return Err(unsupported(
            &request.destination,
            "Recoverable Move currently accepts at most 64 regular files. Choose Copy for larger or recursive selections.",
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
                "Recoverable Move proof paths exceed the 4 MiB budget. Choose Copy or a smaller batch.",
            ));
        }
    }
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
        prepared
            .source
            .rewind_for_validation(&cancel)?
            .revalidate(&cancel)?;
        roots.push_back(prepared.source);
        captures.push(prepared.capture);
    }
    let mut stages = HashMap::new();
    for source in &roots {
        let result = (|| {
            let mut stage = destination.stage_file(&cancel)?;
            if let Err(mut failure) = stage
                .rewind_for_validation(&cancel)
                .and_then(|guard| guard.revalidate(&cancel))
            {
                if let Err(cleanup) = stage.cleanup() {
                    failure
                        .message
                        .push_str(&format!("\nStage cleanup requires attention: {cleanup}"));
                }
                return Err(failure);
            }
            Ok::<_, FsError>(stage)
        })();
        match result {
            Ok(stage) => {
                stages.insert(source.entry().identity, stage);
            }
            Err(mut failure) => {
                for stage in stages.values_mut() {
                    if let Err(cleanup) = stage.cleanup() {
                        failure
                            .message
                            .push_str(&format!("\nStage cleanup requires attention: {cleanup}"));
                    }
                }
                return Err(failure);
            }
        }
    }
    let mut job = Job::from_sources(destination, roots, cancel, status);
    job.progress.intent = TransferIntent::RecoverableMove;
    job.move_work = Some(MoveWork {
        captures,
        stages,
        proofs: vec![],
        phase: Phase::Copying,
        cursor: 0,
        reading: false,
        buffer: vec![],
    });
    Ok(job)
}

impl MoveWork {
    pub(super) fn advance(&mut self, job: &mut Job) -> Result<bool, FsError> {
        let result = self.advance_inner(job);
        if result.is_err() {
            self.phase = Phase::Halted;
        }
        result
    }
    fn advance_inner(&mut self, job: &mut Job) -> Result<bool, FsError> {
        match self.phase {
            Phase::Copying => {
                if job.progress.failed != 0 || job.progress.skipped != 0 {
                    self.phase = Phase::Halted;
                    return Ok(true);
                }
                if self.proofs.len() != self.captures.len() {
                    return Err(unsupported(
                        &job.progress.destination_directory,
                        "A selected file has no complete publication proof; every source was retained.",
                    ));
                }
                for (proof, capture) in self.proofs.iter().zip(&self.captures) {
                    if proof.source_entry().identity != capture.entry().identity {
                        return Err(unsupported(
                            &capture.entry().item.path,
                            "Move source/proof correspondence changed; sources retained.",
                        ));
                    }
                }
                self.buffer = vec![0; 256 * 1024];
                self.phase = Phase::Before;
                job.progress.phase = CopyOperationPhase::ValidatingBeforeMove;
                Ok(false)
            }
            Phase::Before | Phase::After => self.validate(job),
            Phase::Capture => self.capture(job),
            Phase::Complete | Phase::Halted => Ok(true),
        }
    }
    fn validate(&mut self, job: &mut Job) -> Result<bool, FsError> {
        let before = matches!(self.phase, Phase::Before);
        job.progress.phase = if before {
            CopyOperationPhase::ValidatingBeforeMove
        } else {
            CopyOperationPhase::ValidatingAfterMove
        };
        if let Some(proof) = self.proofs.get_mut(self.cursor) {
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
                self.cursor += 1;
                self.reading = false;
            }
            return Ok(false);
        }
        if before {
            self.check_capture_gate(job)?;
            self.phase = Phase::Capture;
            self.cursor = 0;
            job.progress.phase = CopyOperationPhase::CapturingSources;
            Ok(false)
        } else {
            self.phase = Phase::Complete;
            Ok(true)
        }
    }
    fn check_capture_gate(&self, job: &Job) -> Result<(), FsError> {
        let _permit = permit(&job.cancel, &job.progress.destination_directory)?;
        if job.progress.failed != 0 || job.progress.skipped != 0 {
            return Err(unsupported(
                &job.progress.destination_directory,
                "A failed or skipped file blocks every remaining source capture.",
            ));
        }
        for capture in self
            .captures
            .iter()
            .skip(if matches!(self.phase, Phase::Capture) {
                self.cursor
            } else {
                0
            })
        {
            capture.revalidate(&job.cancel)?;
        }
        drop(_permit);
        // Full bytes were checked for the entire batch. Recheck exact namespace
        // bindings before each capture; this is not content isolation or CAS.
        for proof in &self.proofs {
            proof.revalidate_publication(&job.cancel)?;
        }
        Ok(())
    }
    fn capture(&mut self, job: &mut Job) -> Result<bool, FsError> {
        if self.cursor == self.captures.len() {
            self.phase = Phase::After;
            self.cursor = 0;
            job.progress.phase = CopyOperationPhase::ValidatingAfterMove;
            return Ok(false);
        }
        self.check_capture_gate(job)?;
        let capture = &self.captures[self.cursor];
        let expected = capture.entry().identity;
        let path = capture.entry().item.path.clone();
        job.progress.current_path = Some(path.clone());
        let outcome = {
            let _permit = permit(&job.cancel, &path)?;
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
                        "Source capture returned an unexpected identity. All retained data requires recovery; no further sources will move.",
                    ));
                }
                self.cursor += 1;
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
    pub(super) fn cleanup(&mut self) -> Vec<FsError> {
        let errors = self
            .stages
            .values_mut()
            .filter_map(|stage| stage.cleanup().err())
            .collect();
        self.stages.clear();
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
fn permit(cancel: &Cancellation, path: &Path) -> Result<midnight_work_budget::Permit, FsError> {
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
