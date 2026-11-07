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
) -> Result<HashMap<TransferIdentity, Box<dyn StagedTransferFile>>, FsError> {
    let mut stages: HashMap<_, Box<dyn StagedTransferFile>> = HashMap::new();
    for source in roots {
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
            if let Err(mut failure) = probe {
                if let Err(cleanup) = stage.cleanup() {
                    failure
                        .message
                        .push_str(&format!("\nStage cleanup requires attention: {cleanup}"));
                }
                return Err(failure);
            }
            Ok(stage)
        })();
        match result {
            Ok(stage) => {
                stages.insert(source.entry().identity, stage);
            }
            Err(mut failure) => {
                for stage in stages.values_mut() {
                    if let Err(cleanup) = stage.cleanup() {
                        failure
                            .message
                            .push_str(&format!("\nStage cleanup requires attention: {cleanup}"));
                    }
                }
                return Err(failure);
            }
        }
    }
    Ok(stages)
}
