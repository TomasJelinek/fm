//! Retained file receipts. Quick mode retains at most192 KiB of source evidence.
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
    edges: Option<Edges>,
    length: u64,
    phase: FileProofPhase,
    guard: Option<Box<dyn TransferReadGuard>>,
    source_guard: Option<Box<dyn TransferReadGuard>>,
    destination_guard: Option<Box<dyn TransferReadGuard>>,
    hash: Sha256,
    read: u64,
    validation_bytes: u64,
    modified_time: Option<Box<dyn TransferModifiedTimePin>>,
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
            edges: self.edges,
            length: self.copied,
            phase: FileProofPhase::Ready,
            guard: None,
            source_guard: None,
            destination_guard: None,
            hash: Sha256::new(),
            read: 0,
            validation_bytes: 0,
            modified_time: self.modified_time,
        })
    }
}

impl VerifiedFileProof {
    /// Fresh source and destination revisions from the completed selected-policy
    /// pass, plus exact published binding and required metadata. This is a per-file
    /// removal gate, not a snapshot or permission to remove another file.
    pub fn revalidate_finalization(&self, cancel: &Cancellation) -> Result<(), FsError> {
        let path = &self.source.entry().item.path;
        if self.phase != FileProofPhase::Complete {
            return Err(failure(
                path,
                "source removal requires a completed verification pass",
            ));
        }
        let _permit = io_permit(cancel, path)?;
        for guard in [&self.source_guard, &self.destination_guard] {
            guard
                .as_ref()
                .ok_or_else(|| failure(path, "final read guard missing"))?
                .revalidate(cancel)?;
        }
        drop(_permit);
        self.revalidate_publication(cancel)
    }
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
    /// a completed independent validation pass under the immutable job policy.
    pub fn revalidate_publication(&self, cancel: &Cancellation) -> Result<(), FsError> {
        let path = &self.source.entry().item.path;
        check_cancel(cancel, path)?;
        let _permit = io_permit(cancel, path)?;
        if self.stage.revalidate_publication(cancel)? != self.identity {
            return Err(failure(path, "published destination identity changed"));
        }
        self.check_modified_time(cancel)?;
        Ok(())
    }
    fn check_modified_time(&self, cancel: &Cancellation) -> Result<(), FsError> {
        if let Some(pin) = &self.modified_time
            && (pin.current_modified(cancel)? != pin.modified()
                || self.stage.modified_time(cancel)? != pin.modified())
        {
            return Err(failure(
                &self.source.entry().item.path,
                "required modified time changed after publication",
            ));
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
        self.check_modified_time(cancel)?;
        if self.stage.revalidate_publication(cancel)? != self.identity {
            return Err(failure(path, "published destination identity changed"));
        }
        self.guard = Some(self.source.rewind_for_validation(cancel)?);
        self.source_guard = None;
        self.destination_guard = None;
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
        self.source_guard = None;
        self.destination_guard = None;
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
        let sampled = self.edges.is_some();
        let (offset, maximum) = if sampled {
            Edges::range(self.length, self.read, buffer.len())
        } else {
            (self.read, buffer.len())
        };
        let count = {
            let _permit = io_permit(cancel, &path)?;
            if sampled {
                let length = if self.phase == FileProofPhase::Source {
                    self.source.seek_for_validation(offset, cancel)?
                } else {
                    if self
                        .stage
                        .seek(SeekFrom::Start(offset))
                        .map_err(|e| io_error(&path, e))?
                        != offset
                    {
                        return Err(failure(&path, "published file verification seek failed"));
                    }
                    self.stage.len()?
                };
                if length != self.length {
                    return Err(failure(
                        &path,
                        "retained file length changed after publication",
                    ));
                }
            }
            if maximum == 0 {
                0
            } else if self.phase == FileProofPhase::Source {
                self.source.read_chunk(&mut buffer[..maximum], cancel)?
            } else {
                self.stage
                    .read(&mut buffer[..maximum])
                    .map_err(|e| io_error(&path, e))?
            }
        };
        if count > maximum {
            return Err(failure(&path, "provider returned an invalid read length"));
        }
        if let Some(edges) = &self.edges
            && ((count == 0 && maximum != 0)
                || !edges.matches(self.length, self.read, &buffer[..count]))
        {
            return Err(failure(
                &path,
                "retained first/middle/last block changed after publication",
            ));
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
            if !sampled {
                self.hash.update(&buffer[..count]);
            }
            return Ok(self.phase);
        }
        {
            let _permit = io_permit(cancel, &path)?;
            self.guard
                .as_ref()
                .expect("active proof read has a guard")
                .revalidate(cancel)?;
            self.check_modified_time(cancel)?;
            if self.phase == FileProofPhase::Destination
                && self.stage.revalidate_publication(cancel)? != self.identity
            {
                return Err(failure(&path, "published destination identity changed"));
            }
        }
        if self.read
            != if sampled {
                Edges::checked_length(self.length)
            } else {
                self.length
            }
            || (!sampled && <[u8; 32]>::from(self.hash.clone().finalize()) != self.digest)
        {
            return Err(failure(
                &path,
                "retained file checksum or length changed after publication",
            ));
        }
        if self.phase == FileProofPhase::Source {
            self.source_guard = self.guard.take();
        } else {
            self.destination_guard = self.guard.take();
        }
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
