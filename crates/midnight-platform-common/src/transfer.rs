//! Bounded independent SHA-256 readback; native adapters own all filesystem publication.
use midnight_core::{
    Cancellation, EntryKind, FsError, StagedTransferFile, TransferEntry, TransferIdentity,
    TransferPublication, TransferReplacementPolicy, TransferSource, TransferTarget,
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
    target_hash: Sha256,
    copied: u64,
    verified: u64,
    proof_eligible: bool,
}
impl VerifiedFileCopy {
    pub fn new(
        source: Box<dyn TransferSource>,
        stage: Box<dyn StagedTransferFile>,
    ) -> Result<Self, FsError> {
        if source.entry().item.kind != EntryKind::File
            || source.entry().item.metadata.size.is_none()
        {
            return Err(failure(
                &source.entry().item.path,
                "a regular source file with known length is required",
            ));
        }
        Ok(Self {
            source,
            stage,
            phase: CopyPhase::Copying,
            buffer: vec![0; 256 * 1024],
            source_hash: Sha256::new(),
            target_hash: Sha256::new(),
            copied: 0,
            verified: 0,
            proof_eligible: false,
        })
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
        if !matches!(
            self.phase,
            CopyPhase::Published | CopyPhase::RecoveryRequired
        ) {
            self.phase = CopyPhase::Failed;
        }
        self.stage.cleanup()
    }
    /// At most one 256 KiB source/readback block, or one sync phase. Individual
    /// native calls share the process I/O budget; hashing runs after releasing it.
    pub fn step(&mut self, cancel: &Cancellation) -> Result<CopyPhase, FsError> {
        let result = self.advance(cancel);
        if result.is_err() && self.phase != CopyPhase::RecoveryRequired {
            self.phase = CopyPhase::Failed;
        }
        result
    }
    fn advance(&mut self, cancel: &Cancellation) -> Result<CopyPhase, FsError> {
        check_cancel(cancel, &self.source.entry().item.path)?;
        let source_path = self.source.entry().item.path.clone();
        let path = source_path.as_path();
        match self.phase {
            CopyPhase::Copying => {
                let count = {
                    let _permit = io_permit(cancel, path)?;
                    self.source.read_chunk(&mut self.buffer, cancel)?
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
                    self.source_hash.update(&self.buffer[..count]);
                    self.copied = copied;
                }
            }
            CopyPhase::Syncing => {
                let _permit = io_permit(cancel, path)?;
                self.stage.flush().map_err(|e| io_error(path, e))?;
                self.stage.sync(cancel)?;
                self.stage
                    .seek(SeekFrom::Start(0))
                    .map_err(|e| io_error(path, e))?;
                self.phase = CopyPhase::Verifying;
            }
            CopyPhase::Verifying => {
                let count = {
                    let _permit = io_permit(cancel, path)?;
                    self.stage
                        .read(&mut self.buffer)
                        .map_err(|e| io_error(path, e))?
                };
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
                    {
                        let _permit = io_permit(cancel, path)?;
                        self.source.revalidate(cancel)?;
                    }
                    self.phase = CopyPhase::ReadyToPublish;
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
        .acquire(midnight_work_budget::IO_SLOTS, || cancel.is_cancelled())
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
