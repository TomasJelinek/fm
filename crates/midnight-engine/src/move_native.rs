//! Native namespace Move precedes every content read and proof-only authority.
use super::*;

impl Job {
    pub(super) fn try_native_move(&mut self, work: &mut EntryWork) -> Result<bool, FsError> {
        let Some(move_work) = self.move_work.as_ref() else {
            return Ok(false);
        };
        if work.native_fallback || work.consent == Some(Consent::Merge) {
            return Ok(false);
        }
        let capture = move_work.capture_for(work.entry())?;
        let replacement = if work.consent == Some(Consent::Replace) {
            let target = work.target.as_ref().expect("consented target pin");
            target.revalidate(&self.cancel)?;
            Some((
                target.clone(),
                TransferReplacementPolicy::RecoverableCurrentEntry,
            ))
        } else {
            None
        };
        let _permit = move_job::permit(&self.cancel, &work.entry().item.path)?;
        work.destination.revalidate(&self.cancel)?;
        capture.revalidate(&self.cancel)?;
        let outcome = capture.try_native_move(
            work.destination.as_ref(),
            &work.name,
            replacement,
            &self.cancel,
        )?;
        let (moved, recovery, failure) = match outcome {
            TransferNativeMoveOutcome::CrossDevice | TransferNativeMoveOutcome::Unsupported => {
                work.native_fallback = true;
                return Ok(false);
            }
            TransferNativeMoveOutcome::SameEntry => (false, vec![], None),
            TransferNativeMoveOutcome::Moved {
                destination,
                recovery,
                durability_confirmed,
            } => {
                if !durability_confirmed {
                    self.progress.source_removal_durability_unconfirmed += 1;
                }
                let failure = if destination != work.destination.path().join(&work.name) {
                    Some(error(
                        &destination,
                        io::ErrorKind::InvalidData,
                        "Native Move reported an unexpected committed destination.",
                    ))
                } else {
                    None
                };
                (true, recovery, failure)
            }
            TransferNativeMoveOutcome::RecoveryRequired {
                destination,
                recovery,
                error,
            } => (destination.is_some(), recovery, Some(error)),
        };
        // Every terminal native outcome consumes this attempt. Never run proof
        // finalization, automatic restoration or content-copy Retry after commit.
        let mut move_work = self.move_work.take().expect("Move authority");
        let completion = if moved {
            self.progress.native_entries_moved += 1;
            self.protected_roots.insert(work.entry().identity);
            move_work.native_completed(work.entry(), self)
        } else {
            if failure.is_some() {
                // Recovery may have moved/restored a source or displaced target.
                // Reconcile the captured source parents even without publication.
                move_work.record_parents(self, &work.entry().item.path);
            }
            move_work.discard(work.entry().identity)
        };
        self.move_work = Some(move_work);
        for retained in recovery {
            if !moved && retained.identity == Some(work.entry().identity) {
                self.progress.source_residual_count += 1;
                if self.progress.source_residuals.len() < 16 {
                    self.progress.source_residuals.push(MoveSourceResidual {
                        source: work.entry().item.path.clone(),
                        temporary: retained,
                        matched_source: true,
                    });
                }
            } else {
                self.record_recovery(retained.path);
            }
        }
        if let Some(failure) = failure {
            self.record(failure);
        }
        if let Err(failure) = completion {
            self.record(failure);
        }
        self.progress.phase = CopyOperationPhase::Planning;
        Ok(true)
    }

    pub(super) fn prepare_move_copy(&mut self, work: &mut EntryWork) -> Result<(), FsError> {
        let Some(move_work) = self.move_work.as_ref() else {
            return Ok(());
        };
        if work.native_fallback && !work.destination.capabilities().recoverable_replacement {
            return Err(error(
                work.destination.path(),
                io::ErrorKind::Unsupported,
                "Cross-volume Move requires a qualified verified-copy destination; the source was retained.",
            ));
        }
        if work.move_preflight_done {
            return Ok(());
        }
        if work.entry().item.kind == EntryKind::Directory {
            if work.native_fallback {
                let _permit = move_job::permit(&self.cancel, &work.entry().item.path)?;
                move_work
                    .capture_for(work.entry())?
                    .preflight_finalization(&self.cancel)?;
            }
            work.move_preflight_done = true;
            return Ok(());
        }
        // Only an explicitly unchanged cross-device/legacy result reaches here.
        // Failure is offered through the usual identity-retaining Retry/Skip path.
        let capture = move_work.capture_for(work.entry())?;
        let metadata_policy = move_work.metadata_policy;
        let _permit = move_job::permit(&self.cancel, &work.entry().item.path)?;
        capture.preflight_finalization(&self.cancel)?;
        let source = work.source.as_mut().expect("unstarted Move source");
        source
            .rewind_for_validation(&self.cancel)?
            .revalidate(&self.cancel)?;
        if metadata_policy == TransferMetadataPolicy::RequireModifiedTime {
            let pin = source.prepare_modified_time(&self.cancel)?;
            pin.revalidate(&self.cancel)?;
            let metadata = self
                .metadata
                .get_or_insert_with(|| metadata_job::MetadataWork {
                    pins: Default::default(),
                    stages: Default::default(),
                });
            metadata.pins.insert(source.entry().identity, pin);
        }
        work.move_preflight_done = true;
        Ok(())
    }
}
