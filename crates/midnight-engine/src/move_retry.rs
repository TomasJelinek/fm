//! A failed deletion can acquire fresh authority and repeat the complete hash gate.
use super::*;

pub(super) fn retryable(error: &FsError) -> bool {
    !matches!(
        error.kind,
        io::ErrorKind::InvalidData
            | io::ErrorKind::InvalidInput
            | io::ErrorKind::Unsupported
            | io::ErrorKind::AlreadyExists
    )
}

impl MoveWork {
    pub(in super::super) fn failure_stage(&self) -> CopyErrorStage {
        match self.failed_phase {
            Some(Phase::Before | Phase::After) => CopyErrorStage::Verifying,
            _ => CopyErrorStage::RemovingSource,
        }
    }

    fn failed_capture_index(&self) -> Option<usize> {
        if self.phase != Phase::Halted || self.proofs.len() != 1 {
            return None;
        }
        let source = self.proofs[0].source_entry();
        let index = self.captures.iter().position(|capture| {
            capture.entry().identity == source.identity
                && capture.entry().item.path == source.item.path
        })?;
        if let Some((path, recovery)) = &self.active
            && (*path != source.item.path || recovery.identity != Some(source.identity))
        {
            return None;
        }
        Some(index)
    }

    /// Retry only a pre-deletion failure with the successful proof still retained.
    pub(in super::super) fn can_retry(&self) -> bool {
        self.failed_phase == Some(Phase::Finalize)
            && self.retryable_failure
            && self.failed_capture_index().is_some()
            && self.active.is_some()
            && self.finalizer.is_some()
            && self.proofs[0].phase() == FileProofPhase::Complete
    }

    pub(in super::super) fn retry(&mut self, job: &mut Job) -> Result<(), FsError> {
        if !self.can_retry() {
            return Err(unsupported(
                &job.progress.destination_directory,
                "Move has no safe source-removal retry checkpoint.",
            ));
        }
        let result = self.prepare_retry(job);
        if let Err(error) = &result {
            self.retryable_failure = !job.cancel.is_cancelled() && retryable(error);
        }
        result
    }

    fn prepare_retry(&mut self, job: &mut Job) -> Result<(), FsError> {
        let index = self.failed_capture_index().ok_or_else(|| {
            unsupported(
                &job.progress.destination_directory,
                "Move source retry authority missing.",
            )
        })?;
        let capture = &self.captures[index];
        let _permit = permit(&job.cancel, &capture.entry().item.path)?;
        // Native finalizers cache actual Removed/Retained outcomes. A fresh one
        // binds the same captured source; it grants no permission to skip rehashing.
        let finalizer = capture.prepare_finalization(&job.cancel)?;
        finalizer.revalidate(&job.cancel)?;
        self.finalizer = Some(finalizer);
        self.reading = false;
        self.phase = Phase::After;
        self.failed_phase = None;
        self.retryable_failure = false;
        job.progress.phase = CopyOperationPhase::ValidatingAfterMove;
        Ok(())
    }

    /// Skipping a failed file may preserve a published destination, but must restore
    /// its captured source before allowing the next file to proceed.
    pub(in super::super) fn can_skip_failure(&self) -> bool {
        self.failed_capture_index().is_some()
    }

    pub(in super::super) fn skip_failed(&mut self, job: &mut Job) -> Result<(), FsError> {
        let index = self.failed_capture_index().ok_or_else(|| {
            unsupported(
                &job.progress.destination_directory,
                "Move failure cannot be skipped safely.",
            )
        })?;
        let capture = self.captures[index].clone();
        let source = capture.entry().item.path.clone();
        let identity = capture.entry().identity;
        let staged_directory = self
            .active
            .as_ref()
            .and_then(|(_, recovery)| recovery.path.parent().map(Path::to_owned));
        self.proofs.clear();
        self.finalizer = None;
        self.retryable_failure = false;
        self.failed_phase = None;
        self.buffer.clear();
        self.reading = false;
        let restored = capture.restore();
        if restored.is_err() && !capture.restoration_completed() {
            // Preserve the capture/receipt for terminal cleanup and residual reporting.
            return restored;
        }
        let cleanup = capture.take_stage_cleanup();
        self.captures.remove(index);
        drop(capture);
        self.active = None;
        self.record_parents(job, &source);
        if let Some(mut stage) = self.stages.remove(&identity) {
            stage.cleanup()?;
        }
        if let Some(cleanup) = cleanup {
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
                return Err(error);
            }
        } else if let Some(path) = staged_directory {
            record_source(
                job,
                source.clone(),
                TransferRecovery {
                    path,
                    identity: None,
                },
                false,
            );
            return Err(unsupported(
                &source,
                "Move provider omitted temporary-directory cleanup authority after source restoration.",
            ));
        }
        restored?;
        self.phase = Phase::Copying;
        Ok(())
    }
}
