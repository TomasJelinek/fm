//! Preserved times and attributes (`TransferMetadataPolicy::PreserveTimesAndAttributes`).
//! Each source is read once, as the traversal reaches it, before its content or listing
//! is read; a file's copy gets them right after publication, a folder this job created
//! once its frame completes (after its contents and its staging folder), so the
//! traversal's bounded stack is the only state and no item is visited again. What a
//! destination cannot take is a warning, never a failure of the copy.
use super::*;

/// What a file or folder source carried when the job read it; None when the job does not
/// preserve metadata or the folder existed before (Merge).
pub(super) type Preserved = Option<Result<TransferPreservedMetadata, FsError>>;

/// One scope for every warning of this kind, so that "Skip all this warning type" covers
/// all of them, whatever the native error.
pub(super) fn warning_scope() -> failures::Scope {
    failures::Scope::warning(CopyErrorStage::PreservingMetadata)
}

/// A stage for one copied file of the job: one whose handle may set the copy's times and
/// attributes when the job preserves them.
pub(super) fn job_stage(
    preserve: bool,
    destination: &Arc<dyn TransferDirectory>,
    cancel: &Cancellation,
) -> Result<Box<dyn StagedTransferFile>, FsError> {
    if preserve {
        destination
            .stage_file_with_metadata(TransferMetadataPolicy::PreserveTimesAndAttributes, cancel)
    } else {
        destination.stage_job_file(cancel)
    }
}

impl Job {
    /// The job's frozen choice, captured with its request.
    pub(in crate::copy_operations) fn set_preserve_metadata(&mut self, preserve: bool) {
        self.preserve = preserve;
    }

    /// Read `source`'s times and attributes when the job preserves them: a file's after
    /// it was opened for reading, before its first byte; a folder's before its listing.
    /// An archive member's recorded time is read as an instant here, where the job's
    /// locale is known (`ArchiveTimestamp::instant`).
    pub(super) fn read_preserved(&self, source: &dyn TransferSource) -> Preserved {
        if !self.preserve {
            return None;
        }
        Some(source.preserved_metadata(&self.cancel).map(|mut metadata| {
            if let Some(recorded) = source.recorded_modified() {
                let locale = self.locale.as_ref().map(|locale| locale.service());
                metadata.modified = recorded.instant(locale.as_deref());
            }
            metadata
        }))
    }

    /// Give the just published copy of `work` its source's times and attributes, before
    /// a Move's proof reads it.
    pub(super) fn preserve_file(&mut self, work: &mut EntryWork) {
        let Some(preserved) = work.preserved.take() else {
            return;
        };
        let result = preserved.and_then(|metadata| {
            work.copy
                .as_mut()
                .ok_or_else(|| {
                    error(
                        &work.entry.item.path,
                        io::ErrorKind::InvalidData,
                        "The published copy is no longer held.",
                    )
                })?
                .apply_preserved_metadata(&metadata, &self.cancel)
        });
        if let Err(failure) = result {
            let destination = work.destination.path().join(&work.name);
            self.metadata_not_preserved(work.shown.clone(), destination, failure);
        }
    }

    /// Give the top frame's folder, which this job created and whose contents and staging
    /// are finished, its source folder's times and attributes.
    pub(super) fn preserve_folder(&mut self) {
        let Some(frame) = self.stack.last_mut() else {
            return;
        };
        let Some(preserved) = frame.preserved.take() else {
            return;
        };
        let destination = frame.destination.clone();
        let source = frame
            .source
            .display_path()
            .unwrap_or(&frame.source.entry().item.path)
            .to_path_buf();
        let result = preserved
            .and_then(|metadata| destination.apply_preserved_metadata(&metadata, &self.cancel));
        if let Err(failure) = result {
            self.metadata_not_preserved(source, destination.path().to_path_buf(), failure);
        }
    }

    /// Count and name an item whose times or attributes were not all preserved, and ask
    /// about the first one before the next step unless the user chose to skip this
    /// warning for the job. A cancelled job only stops.
    fn metadata_not_preserved(
        &mut self,
        source: std::path::PathBuf,
        destination: std::path::PathBuf,
        failure: FsError,
    ) {
        if self.cancel.is_cancelled() {
            return;
        }
        self.progress.metadata_not_preserved =
            self.progress.metadata_not_preserved.saturating_add(1);
        self.progress.add_note(format!(
            "Times or attributes not preserved: {} ({})",
            destination.display(),
            failure.message
        ));
        if self.warning.is_none() && !self.skipped_errors.contains(&warning_scope()) {
            self.warning = Some(failures::Committed {
                error: failure,
                stage: CopyErrorStage::PreservingMetadata,
                source: Some(source),
                destination,
                entry: None,
                leaves_source: false,
            });
        }
    }
}
