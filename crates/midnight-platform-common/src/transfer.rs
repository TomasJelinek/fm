//! Bounded independent SHA-256 readback; native adapters own all filesystem publication.
use midnight_core::{
    Cancellation, EntryKind, FsError, StagedTransferFile, TransferEntry, TransferIdentity,
    TransferModifiedTimePin, TransferPublication, TransferReplacementPolicy, TransferSource,
    TransferTarget, TransferVerification,
};
use sha2::{Digest, Sha256};
use std::{
    ffi::OsStr,
    io::{self, SeekFrom},
    path::Path,
    sync::Arc,
};

#[path = "transfer_proof.rs"]
mod proof;
pub use proof::{FileProofPhase, VerifiedFileProof};
#[path = "transfer_retry.rs"]
mod retry;
use retry::RetryCheckpoint;
#[path = "transfer_edges.rs"]
mod edges;
use edges::Edges;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyPhase {
    Copying,
    Syncing,
    Verifying,
    ReadyToPublish,
    Published,
    RecoveryRequired,
    Failed,
}

pub struct VerifiedFileCopy {
    source: Box<dyn TransferSource>,
    stage: Box<dyn StagedTransferFile>,
    phase: CopyPhase,
    buffer: Vec<u8>,
    source_hash: Sha256,
    edges: Option<Edges>,
    target_hash: Sha256,
    copied: u64,
    verified: u64,
    proof_eligible: bool,
    modified_time: Option<Box<dyn TransferModifiedTimePin>>,
    pending: usize,
    retry_checkpoint: Option<RetryCheckpoint>,
    failure_stage: midnight_core::CopyErrorStage,
}
impl VerifiedFileCopy {
    pub fn new(
        source: Box<dyn TransferSource>,
        stage: Box<dyn StagedTransferFile>,
    ) -> Result<Self, FsError> {
        Self::new_with_verification(source, stage, TransferVerification::default())
    }
    pub fn new_with_verification(
        source: Box<dyn TransferSource>,
        mut stage: Box<dyn StagedTransferFile>,
        verification: TransferVerification,
    ) -> Result<Self, FsError> {
        if source.entry().item.kind != EntryKind::File
            || source.entry().item.metadata.size.is_none()
        {
            let failure = failure(
                &source.entry().item.path,
                "a regular source file with known length is required",
            );
            return Err(match stage.cleanup() {
                Ok(()) => failure,
                Err(mut cleanup) => {
                    cleanup
                        .message
                        .push_str(&format!("; initial operation failed: {failure}"));
                    cleanup
                }
            });
        }
        let edges = (verification == TransferVerification::SizeAndSamples)
            .then(|| Edges::new(source.entry().item.metadata.size.unwrap_or(0)));
        Ok(Self {
            source,
            stage,
            phase: CopyPhase::Copying,
            buffer: vec![0; 256 * 1024],
            source_hash: Sha256::new(),
            edges,
            target_hash: Sha256::new(),
            copied: 0,
            verified: 0,
            proof_eligible: false,
            modified_time: None,
            pending: 0,
            retry_checkpoint: None,
            failure_stage: midnight_core::CopyErrorStage::Reading,
        })
    }
    /// Use a source metadata pin captured before any source data read.
    pub fn new_with_modified_time(
        source: Box<dyn TransferSource>,
        stage: Box<dyn StagedTransferFile>,
        pin: Box<dyn TransferModifiedTimePin>,
    ) -> Result<Self, FsError> {
        Self::new_with_options(source, stage, Some(pin), TransferVerification::default())
    }
    pub fn new_with_options(
        source: Box<dyn TransferSource>,
        stage: Box<dyn StagedTransferFile>,
        pin: Option<Box<dyn TransferModifiedTimePin>>,
        verification: TransferVerification,
    ) -> Result<Self, FsError> {
        let mut copy = Self::new_with_verification(source, stage, verification)?;
        copy.modified_time = pin;
        Ok(copy)
    }
    pub fn source_entry(&self) -> &TransferEntry {
        self.source.entry()
    }
    pub fn source_native_identity(&self) -> Option<TransferIdentity> {
        self.source.native_identity()
    }
    pub fn phase(&self) -> CopyPhase {
        self.phase
    }
    pub fn copied_bytes(&self) -> u64 {
        self.copied
    }
    pub fn verified_bytes(&self) -> u64 {
        self.verified
    }
    /// Explicit worker-side cancellation cleanup lets callers surface native cleanup
    /// errors. Dropping the job also drops the owned staging inode as a final guard.
    pub fn cleanup(&mut self) -> Result<(), FsError> {
        self.retry_checkpoint = None;
        if !matches!(
            self.phase,
            CopyPhase::Published | CopyPhase::RecoveryRequired
        ) {
            self.phase = CopyPhase::Failed;
        }
        self.stage.cleanup()
    }
    /// At most one 256 KiB block, or bounded sync/metadata finalization. Individual
    /// native calls share the process I/O budget; hashing runs after releasing it.
    pub fn step(&mut self, cancel: &Cancellation) -> Result<CopyPhase, FsError> {
        if self.phase == CopyPhase::Failed {
            return Err(failure(
                &self.source.entry().item.path,
                "failed copy cannot resume without an explicit retry",
            ));
        }
        self.retry_checkpoint = None;
        let result = self.advance(cancel);
        if let Err(error) = &result {
            self.record_failure(error, cancel);
        } else {
            self.retry_checkpoint = None;
        }
        result
    }
    fn advance(&mut self, cancel: &Cancellation) -> Result<CopyPhase, FsError> {
        check_cancel(cancel, &self.source.entry().item.path)?;
        let source_path = self.source.entry().item.path.clone();
        let path = source_path.as_path();
        match self.phase {
            CopyPhase::Copying => {
                let count = if self.pending != 0 {
                    self.pending
                } else {
                    self.failure_stage = midnight_core::CopyErrorStage::Reading;
                    self.retry_checkpoint = self
                        .source
                        .supports_read_retry()
                        .then_some(RetryCheckpoint::SourceRead);
                    let _permit = io_permit(cancel, path)?;
                    let count = self.source.read_chunk(&mut self.buffer, cancel)?;
                    if count > self.buffer.len() {
                        return Err(failure(path, "provider returned an invalid read length"));
                    }
                    self.pending = count;
                    count
                };
                if count == 0 {
                    {
                        let _permit = io_permit(cancel, path)?;
                        self.source.revalidate(cancel)?;
                    }
                    if Some(self.copied) != self.source.entry().item.metadata.size {
                        return Err(failure(
                            &self.source.entry().item.path,
                            "source length changed during copy",
                        ));
                    }
                    self.phase = CopyPhase::Syncing;
                } else {
                    self.failure_stage = midnight_core::CopyErrorStage::Writing;
                    self.retry_checkpoint = Some(RetryCheckpoint::PendingWrite);
                    let path = &self.source.entry().item.path;
                    let copied = self
                        .copied
                        .checked_add(count as u64)
                        .ok_or_else(|| failure(path, "copied byte count overflow"))?;
                    if self
                        .source
                        .entry()
                        .item
                        .metadata
                        .size
                        .is_some_and(|size| copied > size)
                    {
                        return Err(failure(path, "source grew during copy"));
                    }
                    {
                        let _permit = io_permit(cancel, path)?;
                        self.stage
                            .write_all(&self.buffer[..count])
                            .map_err(|e| io_error(path, e))?;
                    }
                    if let Some(edges) = &mut self.edges {
                        edges.update(&self.buffer[..count]);
                    } else {
                        self.source_hash.update(&self.buffer[..count]);
                    }
                    self.copied = copied;
                    self.pending = 0;
                }
            }
            CopyPhase::Syncing => {
                self.failure_stage = midnight_core::CopyErrorStage::Syncing;
                self.retry_checkpoint = Some(RetryCheckpoint::Sync);
                let _permit = io_permit(cancel, path)?;
                self.stage.flush().map_err(|e| io_error(path, e))?;
                self.stage.sync(cancel)?;
                self.stage
                    .seek(SeekFrom::Start(0))
                    .map_err(|e| io_error(path, e))?;
                self.phase = CopyPhase::Verifying;
            }
            CopyPhase::Verifying => {
                self.failure_stage = midnight_core::CopyErrorStage::Verifying;
                self.retry_checkpoint = Some(RetryCheckpoint::Verify);
                if self.edges.is_some() {
                    self.verify_edges(cancel)?;
                    return Ok(self.phase);
                }
                let count = {
                    let _permit = io_permit(cancel, path)?;
                    self.stage
                        .read(&mut self.buffer)
                        .map_err(|e| io_error(path, e))?
                };
                if count > self.buffer.len() {
                    return Err(failure(path, "provider returned an invalid read length"));
                }
                if count == 0 {
                    if self.verified != self.copied
                        || {
                            let _permit = io_permit(cancel, path)?;
                            self.stage.len()?
                        } != self.copied
                        || self.source_hash.clone().finalize()
                            != self.target_hash.clone().finalize()
                    {
                        return Err(failure(
                            path,
                            "independent target checksum or length verification failed",
                        ));
                    }
                    self.finish_readback(cancel)?;
                } else {
                    self.verified = self
                        .verified
                        .checked_add(count as u64)
                        .ok_or_else(|| failure(path, "verified byte count overflow"))?;
                    if self.verified > self.copied {
                        return Err(failure(path, "target grew during verification"));
                    }
                    self.target_hash.update(&self.buffer[..count]);
                }
            }
            CopyPhase::ReadyToPublish | CopyPhase::Published => {}
            CopyPhase::RecoveryRequired => {
                return Err(failure(
                    path,
                    "copy has committed recovery data and cannot resume",
                ));
            }
            CopyPhase::Failed => return Err(failure(path, "failed copy cannot resume")),
        }
        Ok(self.phase)
    }
    fn finish_readback(&mut self, cancel: &Cancellation) -> Result<(), FsError> {
        let path = &self.source.entry().item.path;
        let _permit = io_permit(cancel, path)?;
        self.source.revalidate(cancel)?;
        if let Some(pin) = &self.modified_time {
            pin.revalidate(cancel)?;
            self.stage.set_modified_time(pin.modified(), cancel)?;
            check_cancel(cancel, path)?;
            self.stage.sync(cancel)?;
            check_cancel(cancel, path)?;
            if self.stage.modified_time(cancel)? != pin.modified() {
                return Err(failure(
                    path,
                    "required modified time was not retained exactly",
                ));
            }
            pin.revalidate(cancel)?;
        }
        self.phase = CopyPhase::ReadyToPublish;
        Ok(())
    }
    fn verify_edges(&mut self, cancel: &Cancellation) -> Result<(), FsError> {
        let path = &self.source.entry().item.path;
        let (offset, maximum) = Edges::range(self.copied, self.verified, self.buffer.len());
        let count = {
            let _permit = io_permit(cancel, path)?;
            if self.stage.len()? != self.copied {
                return Err(failure(
                    path,
                    "independent target length verification failed",
                ));
            }
            if maximum == 0 {
                0
            } else {
                if self
                    .stage
                    .seek(SeekFrom::Start(offset))
                    .map_err(|e| io_error(path, e))?
                    != offset
                {
                    return Err(failure(path, "stage did not seek to verification range"));
                }
                self.stage
                    .read(&mut self.buffer[..maximum])
                    .map_err(|e| io_error(path, e))?
            }
        };
        if maximum == 0 {
            return self.finish_readback(cancel);
        }
        if count == 0
            || count > maximum
            || !self.edges.as_ref().is_some_and(|edges| {
                edges.matches(self.copied, self.verified, &self.buffer[..count])
            })
        {
            return Err(failure(
                path,
                "independent first/middle/last block verification failed",
            ));
        }
        self.verified += count as u64;
        Ok(())
    }
    /// Publication is separate so a conflict never blocks a worker. A no-replace
    /// collision leaves the verified stage ready for an explicit different name.
    pub fn publish(
        &mut self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<TransferPublication, FsError> {
        self.publish_with(name, cancel, false)
    }

    /// Bind explicit consent to the existing stage, including a late collision
    /// after readback. This never replays or replaces the consumed source stream.
    pub fn prepare_replacement(
        &mut self,
        name: &OsStr,
        expected: Arc<dyn TransferTarget>,
        policy: TransferReplacementPolicy,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        if matches!(
            self.phase,
            CopyPhase::Published | CopyPhase::RecoveryRequired | CopyPhase::Failed
        ) {
            return self.prepare_replacement_attempt(name, expected, policy, cancel);
        }
        self.failure_stage = midnight_core::CopyErrorStage::Preparing;
        self.retry_checkpoint = Some(RetryCheckpoint::Preparation(self.phase));
        let result = self.prepare_replacement_attempt(name, expected, policy, cancel);
        match &result {
            Err(error) if error.kind == io::ErrorKind::AlreadyExists => {
                self.retry_checkpoint = None
            }
            Err(error) => self.record_failure(error, cancel),
            Ok(()) => self.retry_checkpoint = None,
        }
        result
    }

    fn prepare_replacement_attempt(
        &mut self,
        name: &OsStr,
        expected: Arc<dyn TransferTarget>,
        policy: TransferReplacementPolicy,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        let path = &self.source.entry().item.path;
        check_cancel(cancel, path)?;
        if matches!(
            self.phase,
            CopyPhase::Published | CopyPhase::RecoveryRequired | CopyPhase::Failed
        ) {
            return Err(failure(
                path,
                "a terminal copy cannot accept replacement consent",
            ));
        }
        let _permit = io_permit(cancel, path)?;
        if let Err(error) = self.source.revalidate(cancel) {
            self.phase = CopyPhase::Failed;
            return Err(error);
        }
        if let Some(pin) = &self.modified_time
            && let Err(error) = pin.revalidate(cancel)
        {
            self.phase = CopyPhase::Failed;
            return Err(error);
        }
        self.stage
            .prepare_replacement(name, expected, policy, cancel)
    }

    /// Replacement shares the full verification gate with no-replace publication.
    pub fn publish_replace(
        &mut self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<TransferPublication, FsError> {
        self.publish_with(name, cancel, true)
    }

    fn publish_with(
        &mut self,
        name: &OsStr,
        cancel: &Cancellation,
        replacement: bool,
    ) -> Result<TransferPublication, FsError> {
        if self.phase != CopyPhase::ReadyToPublish {
            return self.publish_attempt(name, cancel, replacement);
        }
        self.failure_stage = midnight_core::CopyErrorStage::Publishing;
        self.retry_checkpoint = Some(RetryCheckpoint::Publish);
        let result = self.publish_attempt(name, cancel, replacement);
        match &result {
            Err(error) if error.kind == io::ErrorKind::AlreadyExists => {
                // Existing conflict handling reuses the verified stage directly.
                self.retry_checkpoint = None;
            }
            Err(error) => self.record_failure(error, cancel),
            Ok(_) => self.retry_checkpoint = None,
        }
        result
    }

    fn publish_attempt(
        &mut self,
        name: &OsStr,
        cancel: &Cancellation,
        replacement: bool,
    ) -> Result<TransferPublication, FsError> {
        let path = &self.source.entry().item.path;
        check_cancel(cancel, path)?;
        if self.phase != CopyPhase::ReadyToPublish {
            return Err(failure(
                path,
                "copy must be completely verified before publication",
            ));
        }
        let _permit = io_permit(cancel, path)?;
        if let Err(error) = self.source.revalidate(cancel) {
            self.phase = CopyPhase::Failed;
            return Err(error);
        }
        let metadata = self
            .modified_time
            .as_ref()
            .map(|pin| {
                pin.revalidate(cancel)?;
                if self.stage.modified_time(cancel)? != pin.modified() {
                    return Err(failure(
                        path,
                        "required modified time changed before publication",
                    ));
                }
                Ok::<(), FsError>(())
            })
            .transpose();
        if let Err(error) = metadata {
            self.phase = CopyPhase::Failed;
            return Err(error);
        }
        let outcome = if replacement {
            self.stage.publish_replace(name, cancel)?
        } else {
            self.stage.publish_no_replace(name, cancel)?
        };
        self.proof_eligible = matches!(
            outcome,
            TransferPublication::Published(_) | TransferPublication::Replaced { .. }
        );
        self.phase = if matches!(outcome, TransferPublication::RecoveryRequired { .. }) {
            CopyPhase::RecoveryRequired
        } else {
            CopyPhase::Published
        };
        Ok(outcome)
    }
}
fn io_permit(cancel: &Cancellation, path: &Path) -> Result<midnight_work_budget::Permit, FsError> {
    // Transfer capabilities currently carry no verified storage classification. Conservatively
    // serialize source and target calls; concurrent copies require both to be confirmed SSDs.
    midnight_work_budget::shared_io()
        .acquire(midnight_work_budget::Admission::Exclusive, || {
            cancel.is_cancelled()
        })
        .ok_or_else(|| io_error(path, io::ErrorKind::Interrupted.into()))
}
fn check_cancel(cancel: &Cancellation, path: &Path) -> Result<(), FsError> {
    if cancel.is_cancelled() {
        Err(io_error(path, io::ErrorKind::Interrupted.into()))
    } else {
        Ok(())
    }
}
fn io_error(path: &Path, error: io::Error) -> FsError {
    FsError::from_io("verified copy", path, error)
}
fn failure(path: &Path, message: &str) -> FsError {
    FsError::from_io(
        "verified copy",
        path,
        std::io::Error::new(std::io::ErrorKind::InvalidData, message),
    )
}
#[cfg(test)]
#[path = "transfer_tests.rs"]
mod tests;
