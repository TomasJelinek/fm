//! Native namespace Move precedes every content read and proof-only authority.
use super::*;

impl Job {
    pub(super) fn try_native_move(&mut self, work: &mut EntryWork) -> Result<bool, FsError> {
        let Some(identity) = work.removal.native_move_identity(work.native_identity()) else {
            return Ok(false);
        };
        let Some(move_work) = self.move_work.as_ref() else {
            return Ok(false);
        };
        if work.native_fallback || work.consent == Some(Consent::Merge) {
            return Ok(false);
        }
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
        // A provider may leave saving a committed rename to a confirmation, which the job
        // runs with the others it defers, saving each folder once for all of them.
        let (outcome, confirmation) = match work.removal {
            EntryRemoval::Ordinary => {
                let capture = move_work.capture_for(work.entry())?;
                capture.revalidate(&self.cancel)?;
                capture.try_native_move_deferred(
                    work.destination.as_ref(),
                    &work.name,
                    replacement,
                    &self.cancel,
                )?
            }
            EntryRemoval::DirectoryLink(original) => {
                let capture = move_work.link_capture_for(original, &work.entry().item.path)?;
                capture.revalidate(&self.cancel)?;
                capture.try_native_move_deferred(
                    work.destination.as_ref(),
                    &work.name,
                    replacement,
                    &self.cancel,
                )?
            }
            EntryRemoval::Copy => unreachable!("read-only entries have no native Move identity"),
        };
        let mut deferred = None;
        // `failure` breaks the provider's contract and ends the job; `uncertain` is a
        // rename that needs recovery, whose names stay where they are: the job asks
        // whether to go on, and never repeats it.
        let (moved, recovery, failure, uncertain) = match outcome {
            TransferNativeMoveOutcome::CrossDevice | TransferNativeMoveOutcome::Unsupported => {
                work.native_fallback = true;
                return Ok(false);
            }
            TransferNativeMoveOutcome::SameEntry => (false, vec![], None, None),
            TransferNativeMoveOutcome::Moved {
                destination,
                recovery,
                durability_confirmed,
            } => {
                match confirmation {
                    Some(confirmation) => deferred = Some(confirmation),
                    None if !durability_confirmed => {
                        self.progress.source_removal_durability_unconfirmed += 1;
                    }
                    None => {}
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
                (true, recovery, failure, None)
            }
            TransferNativeMoveOutcome::RecoveryRequired {
                destination,
                recovery,
                error,
            } => (destination.is_some(), recovery, None, Some(error)),
        };
        // Every terminal native outcome consumes this attempt. Never run proof
        // finalization, automatic restoration or content-copy Retry after commit.
        // Armed: an unwinding native call must not destroy the remaining captures,
        // stages, proofs or finalizer; they are released only explicitly.
        let mut move_work = ownership::Armed::new(self.move_work.take().expect("Move authority"));
        if moved {
            self.progress.native_entries_moved += 1;
            self.protected_roots.insert(identity);
        }
        if moved || failure.is_some() || uncertain.is_some() {
            // Reconcile the captured source parents after publication or recovery.
            move_work.record_parents(self, &work.entry().item.path);
        }
        let completion = move_work.discard_entry(work.removal, work.entry().identity);
        if let Some(confirmation) = deferred {
            move_work.defer_confirmation(confirmation, &mut self.progress);
        }
        self.move_work = Some(move_work.into_inner());
        for retained in recovery {
            if !moved && retained.identity == Some(identity) {
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
        if let Some(error) = uncertain {
            // The entry's authority was released above.
            let committed = failures::Committed {
                entry: None,
                ..self.committed_entry(work, error, CopyErrorStage::MovingSource)
            };
            self.pause_committed(committed);
        }
        if let Err(failure) = completion {
            self.record(failure);
        }
        self.progress.phase = CopyOperationPhase::Planning;
        Ok(true)
    }

    pub(super) fn prepare_move_copy(&mut self, work: &mut EntryWork) -> Result<(), FsError> {
        if let EntryRemoval::DirectoryLink(original) = work.removal
            && work.native_identity().is_none()
        {
            let path = &work.entry().item.path;
            let _permit = move_job::permit(&self.cancel, path)?;
            let projection = self
                .move_work
                .as_mut()
                .ok_or_else(|| {
                    error(
                        path,
                        io::ErrorKind::InvalidInput,
                        "Original directory link authority is missing.",
                    )
                })?
                .take_link_projection(original, path, &self.cancel)?;
            work.entry = projection.entry().clone();
            work.native_identity = projection.native_identity();
            work.source = Some(projection);
        }
        if work.removal != EntryRemoval::Ordinary {
            return Ok(());
        }
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
