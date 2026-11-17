//! Explicit retry checkpoints retain the source block and both checksum histories.
use super::*;

#[derive(Clone, Copy)]
pub(super) enum RetryCheckpoint {
    SourceRead,
    PendingWrite,
    Sync,
    Verify,
    Publish,
    Preparation(CopyPhase),
}

impl VerifiedFileCopy {
    /// True only for a failed pre-publication operation with a safe checkpoint.
    /// No worker retries automatically; the caller must obtain an explicit choice.
    pub fn can_retry(&self) -> bool {
        self.phase == CopyPhase::Failed && self.retry_checkpoint.is_some()
    }

    /// The failed operation, including terminal integrity and source-change errors.
    pub fn failure_stage(&self) -> midnight_core::CopyErrorStage {
        self.failure_stage
    }

    /// Restore a failed operation on the same retained source/staging objects.
    /// Failed writes replay the entire pending block at the last committed offset;
    /// no hash or progress counter includes a block until its write completes.
    pub fn retry(&mut self, cancel: &Cancellation) -> Result<(), FsError> {
        if !self.can_retry() {
            return Err(failure(
                &self.source.entry().item.path,
                "copy has no safe retry checkpoint",
            ));
        }
        let result = self.restore_checkpoint(cancel);
        if let Err(error) = &result {
            self.record_failure(error, cancel);
        } else {
            self.retry_checkpoint = None;
        }
        result
    }

    fn restore_checkpoint(&mut self, cancel: &Cancellation) -> Result<(), FsError> {
        let path = self.source.entry().item.path.clone();
        check_cancel(cancel, &path)?;
        let _permit = io_permit(cancel, &path)?;
        self.source.revalidate(cancel)?;
        if let Some(pin) = &self.modified_time {
            pin.revalidate(cancel)?;
        }
        let checkpoint = self
            .retry_checkpoint
            .ok_or_else(|| failure(&path, "copy checkpoint missing"))?;
        self.phase = match checkpoint {
            RetryCheckpoint::SourceRead => {
                self.source.retry_read_from(self.copied, cancel)?;
                CopyPhase::Copying
            }
            RetryCheckpoint::PendingWrite => {
                if self
                    .stage
                    .seek(SeekFrom::Start(self.copied))
                    .map_err(|error| io_error(&path, error))?
                    != self.copied
                {
                    return Err(failure(
                        &path,
                        "stage did not restore the committed write offset",
                    ));
                }
                CopyPhase::Copying
            }
            RetryCheckpoint::Sync => CopyPhase::Syncing,
            RetryCheckpoint::Verify => {
                let offset = if self.edges.is_some() {
                    Edges::range(self.copied, self.verified, self.buffer.len()).0
                } else {
                    self.verified
                };
                if self
                    .stage
                    .seek(SeekFrom::Start(offset))
                    .map_err(|error| io_error(&path, error))?
                    != offset
                {
                    return Err(failure(
                        &path,
                        "stage did not restore the verified read offset",
                    ));
                }
                CopyPhase::Verifying
            }
            RetryCheckpoint::Publish => CopyPhase::ReadyToPublish,
            RetryCheckpoint::Preparation(phase) => phase,
        };
        Ok(())
    }

    pub(super) fn record_failure(&mut self, error: &FsError, cancel: &Cancellation) {
        if matches!(
            self.phase,
            CopyPhase::Published | CopyPhase::RecoveryRequired
        ) {
            self.retry_checkpoint = None;
            return;
        }
        self.phase = CopyPhase::Failed;
        if cancel.is_cancelled()
            || matches!(
                error.kind,
                io::ErrorKind::InvalidData
                    | io::ErrorKind::InvalidInput
                    | io::ErrorKind::Unsupported
                    | io::ErrorKind::AlreadyExists
            )
        {
            self.retry_checkpoint = None;
        }
    }
}
