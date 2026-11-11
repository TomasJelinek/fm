//! Bounded required metadata preflight; existing ContentOnly traversal is unchanged.
use super::*;
use std::collections::HashMap;

pub(super) struct MetadataWork {
    pub(super) pins: HashMap<TransferIdentity, Box<dyn TransferModifiedTimePin>>,
    pub(super) stages: HashMap<TransferIdentity, Box<dyn StagedTransferFile>>,
}
impl MetadataWork {
    pub(super) fn cleanup(&mut self) -> Vec<FsError> {
        self.stages
            .values_mut()
            .filter_map(|stage| stage.cleanup().err())
            .collect()
    }
}
/// Preflight may retry only if every abandoned owned stage was cleaned.
/// This is private engine state; native error kinds and paths remain unchanged.
pub(super) struct StageFailure {
    pub(super) error: FsError,
    pub(super) cleanup_failed: bool,
}
impl Job {
    pub(super) fn prepare_metadata(
        &mut self,
        metadata: TransferMetadataPolicy,
    ) -> Result<(), FsError> {
        let pins = pins(&self.roots, &self.cancel)?;
        let stages = match stages(
            &self.roots,
            &self.destination,
            metadata,
            false,
            &self.cancel,
        ) {
            Ok(stages) => stages,
            Err(failure) => {
                if failure.cleanup_failed {
                    self.record_cleanup_error(failure.error.clone());
                    // Preparation has no surviving Job to retain these stages.
                    // Publish the bounded residual diagnostics before its drop;
                    // the async admission loop must not offer another attempt.
                    self.store_progress();
                }
                return Err(failure.error);
            }
        };
        self.metadata = Some(MetadataWork { pins, stages });
        Ok(())
    }
}
pub(super) fn pins(
    roots: &VecDeque<Box<dyn TransferSource>>,
    cancel: &Cancellation,
) -> Result<HashMap<TransferIdentity, Box<dyn TransferModifiedTimePin>>, FsError> {
    let mut pins = HashMap::new();
    for source in roots {
        let entry = source.entry();
        if entry.item.kind != EntryKind::File
            || source.native_identity() != Some(entry.identity)
            || pins.contains_key(&entry.identity)
        {
            return Err(error(
                &entry.item.path,
                io::ErrorKind::Unsupported,
                "Required modified time currently accepts distinct native regular files only; choose content-only Copy for folders, links or virtual files.",
            ));
        }
        source.revalidate(cancel)?;
        let pin = source.prepare_modified_time(cancel)?;
        pin.revalidate(cancel)?;
        pins.insert(entry.identity, pin);
    }
    Ok(pins)
}
pub(super) fn stages(
    roots: &VecDeque<Box<dyn TransferSource>>,
    destination: &Arc<dyn TransferDirectory>,
    metadata: TransferMetadataPolicy,
    proof: bool,
    cancel: &Cancellation,
) -> Result<HashMap<TransferIdentity, Box<dyn StagedTransferFile>>, StageFailure> {
    let mut stages: HashMap<_, Box<dyn StagedTransferFile>> = HashMap::new();
    for source in roots {
        if source.entry().item.kind == EntryKind::Directory
            && metadata == TransferMetadataPolicy::ContentOnly
        {
            continue;
        }
        let mut cleanup_failure: Option<FsError> = None;
        let result = (|| {
            let mut stage = if metadata == TransferMetadataPolicy::ContentOnly {
                destination.stage_file(cancel)?
            } else {
                destination.stage_file_with_metadata(metadata, cancel)?
            };
            let probe = if proof {
                stage
                    .rewind_for_validation(cancel)
                    .and_then(|guard| guard.revalidate(cancel))
            } else {
                Ok(())
            }
            .and_then(|()| {
                if metadata == TransferMetadataPolicy::RequireModifiedTime {
                    stage.modified_time(cancel).map(|_| ())
                } else {
                    Ok(())
                }
            });
            if let Err(failure) = probe {
                cleanup_failure = stage.cleanup().err();
                return Err(failure);
            }
            Ok(stage)
        })();
        match result {
            Ok(stage) => {
                stages.insert(source.entry().identity, stage);
            }
            Err(failure) => {
                for stage in stages.values_mut() {
                    if let Err(cleanup) = stage.cleanup() {
                        if let Some(first) = &mut cleanup_failure {
                            first.message.push_str(&format!("\n{cleanup}"));
                        } else {
                            cleanup_failure = Some(cleanup);
                        }
                    }
                }
                let cleanup_failed = cleanup_failure.is_some();
                let error = match cleanup_failure {
                    Some(mut cleanup) => {
                        // Preserve the first residual's native classification and
                        // path; a real refusal must not look like plain cancellation.
                        cleanup
                            .message
                            .push_str(&format!("\nInitial operation failed: {failure}"));
                        cleanup
                    }
                    None => failure,
                };
                return Err(StageFailure {
                    error,
                    cleanup_failed,
                });
            }
        }
    }
    Ok(stages)
}
