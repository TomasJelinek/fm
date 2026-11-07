//! Retained file receipts. No byte buffer is retained per receipt.
use super::*;
use midnight_core::TransferReadGuard;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileProofPhase {
    Ready,
    Source,
    DestinationStart,
    Destination,
    Complete,
    Failed,
}

pub struct VerifiedFileProof {
    source: Box<dyn TransferSource>,
    stage: Box<dyn StagedTransferFile>,
    identity: TransferIdentity,
    digest: [u8; 32],
    length: u64,
    phase: FileProofPhase,
    guard: Option<Box<dyn TransferReadGuard>>,
    hash: Sha256,
    read: u64,
    validation_bytes: u64,
}

impl VerifiedFileCopy {
    pub fn into_published_proof(self, cancel: &Cancellation) -> Result<VerifiedFileProof, FsError> {
        let path = &self.source.entry().item.path;
        check_cancel(cancel, path)?;
        if self.phase != CopyPhase::Published || !self.proof_eligible {
            return Err(failure(
                path,
                "Move validation requires successful publication and all required durability steps",
            ));
        }
        let identity = {
            let _permit = io_permit(cancel, path)?;
            self.stage.revalidate_publication(cancel)?
        };
        Ok(VerifiedFileProof {
            source: self.source,
            stage: self.stage,
            identity,
            digest: self.source_hash.finalize().into(),
            length: self.copied,
            phase: FileProofPhase::Ready,
            guard: None,
            hash: Sha256::new(),
            read: 0,
            validation_bytes: 0,
        })
    }
}

impl VerifiedFileProof {
    pub fn source_entry(&self) -> &TransferEntry {
        self.source.entry()
    }
    pub fn destination_identity(&self) -> TransferIdentity {
        self.identity
    }
    pub fn phase(&self) -> FileProofPhase {
        self.phase
    }
    pub fn validation_bytes(&self) -> u64 {
        self.validation_bytes
    }
    /// Recheck only the exact final name binding. This supplements, never replaces,
    /// a complete independently hashed validation pass.
    pub fn revalidate_publication(&self, cancel: &Cancellation) -> Result<(), FsError> {
        let path = &self.source.entry().item.path;
        check_cancel(cancel, path)?;
        let _permit = io_permit(cancel, path)?;
        if self.stage.revalidate_publication(cancel)? != self.identity {
            return Err(failure(path, "published destination identity changed"));
        }
        Ok(())
    }
    pub fn begin_validation(&mut self, cancel: &Cancellation) -> Result<(), FsError> {
        let result = self.begin(cancel);
        if result.is_err() {
            self.fail();
        }
        result
    }
    fn begin(&mut self, cancel: &Cancellation) -> Result<(), FsError> {
        let path = &self.source.entry().item.path;
        check_cancel(cancel, path)?;
        if !matches!(self.phase, FileProofPhase::Ready | FileProofPhase::Complete) {
            return Err(failure(
                path,
                "proof is not ready for a new validation pass",
            ));
        }
        let _permit = io_permit(cancel, path)?;
        if self.stage.revalidate_publication(cancel)? != self.identity {
            return Err(failure(path, "published destination identity changed"));
        }
        self.guard = Some(self.source.rewind_for_validation(cancel)?);
        self.hash = Sha256::new();
        self.read = 0;
        self.validation_bytes = 0;
        self.phase = FileProofPhase::Source;
        Ok(())
    }
    pub fn validation_step(
        &mut self,
        buffer: &mut [u8],
        cancel: &Cancellation,
    ) -> Result<FileProofPhase, FsError> {
        let result = self.advance(buffer, cancel);
        if result.is_err() {
            self.fail();
        }
        result
    }
    fn fail(&mut self) {
        self.phase = FileProofPhase::Failed;
        self.guard = None;
    }
    fn advance(
        &mut self,
        buffer: &mut [u8],
        cancel: &Cancellation,
    ) -> Result<FileProofPhase, FsError> {
        let path = self.source.entry().item.path.clone();
        check_cancel(cancel, &path)?;
        if buffer.is_empty() || buffer.len() > 256 * 1024 {
            return Err(failure(
                &path,
                "proof validation requires a buffer of 1 through 256 KiB",
            ));
        }
        if self.phase == FileProofPhase::DestinationStart {
            let _permit = io_permit(cancel, &path)?;
            self.guard = Some(self.stage.rewind_for_validation(cancel)?);
            self.hash = Sha256::new();
            self.read = 0;
            self.phase = FileProofPhase::Destination;
            return Ok(self.phase);
        }
        if !matches!(
            self.phase,
            FileProofPhase::Source | FileProofPhase::Destination
        ) {
            return Err(failure(&path, "proof has no active validation read"));
        }
        let count = {
            let _permit = io_permit(cancel, &path)?;
            if self.phase == FileProofPhase::Source {
                self.source.read_chunk(buffer, cancel)?
            } else {
                self.stage.read(buffer).map_err(|e| io_error(&path, e))?
            }
        };
        if count > buffer.len() {
            return Err(failure(&path, "provider returned an invalid read length"));
        }
        if count != 0 {
            self.read = self
                .read
                .checked_add(count as u64)
                .ok_or_else(|| failure(&path, "proof length overflow"))?;
            self.validation_bytes = self
                .validation_bytes
                .checked_add(count as u64)
                .ok_or_else(|| failure(&path, "proof byte count overflow"))?;
            if self.read > self.length {
                return Err(failure(&path, "retained file grew after publication"));
            }
            self.hash.update(&buffer[..count]);
            return Ok(self.phase);
        }
        {
            let _permit = io_permit(cancel, &path)?;
            self.guard
                .as_ref()
                .expect("active proof read has a guard")
                .revalidate(cancel)?;
            if self.phase == FileProofPhase::Destination
                && self.stage.revalidate_publication(cancel)? != self.identity
            {
                return Err(failure(&path, "published destination identity changed"));
            }
        }
        if self.read != self.length || <[u8; 32]>::from(self.hash.clone().finalize()) != self.digest
        {
            return Err(failure(
                &path,
                "retained file checksum or length changed after publication",
            ));
        }
        self.guard = None;
        self.phase = if self.phase == FileProofPhase::Source {
            FileProofPhase::DestinationStart
        } else {
            FileProofPhase::Complete
        };
        Ok(self.phase)
    }
}

#[cfg(test)]
#[path = "transfer_proof_tests.rs"]
mod tests;
