//! Move of a preserved link: the link itself moves, never what it points to.
//! On one filesystem it is renamed. Across filesystems it is recreated at the
//! destination (staged privately, read back, published without replacement or
//! by a recoverable exchange) and only then is the original link removed.

use super::*;
use midnight_core::{
    StagedTransferLink, TransferDirectoryLinkMoveCapture, TransferDirectoryLinkRemoval,
    TransferLinkRepresentation, TransferNativeMoveOutcome, TransferPublication,
    TransferReplacementPolicy,
};

#[cfg(test)]
#[path = "transfer_link_move_tests.rs"]
mod tests;

pub(super) struct Capture {
    original: Arc<directory_links::OriginalLink>,
    parent: destination::Directory,
    outcome: Mutex<Option<TransferNativeMoveOutcome>>,
}

type Replacement = Option<(Arc<dyn TransferTarget>, TransferReplacementPolicy)>;

impl Capture {
    pub(super) fn prepare(
        original: Arc<directory_links::OriginalLink>,
        parent: destination::Directory,
        cancel: &Cancellation,
    ) -> Result<Self, FsError> {
        let capture = Self {
            original,
            parent,
            outcome: Mutex::new(None),
        };
        capture.revalidate(cancel)?;
        Ok(capture)
    }

    fn path(&self) -> &Path {
        &self.original.entry().item.path
    }

    /// Recreate the link at `destination`, then remove the original. Before the
    /// publication an error leaves both names as they were; afterwards every
    /// failure is a structured outcome that keeps the published link.
    fn recreate(
        &self,
        destination: &dyn TransferDirectory,
        name: &OsStr,
        replace: Replacement,
        cancel: &Cancellation,
    ) -> Result<TransferNativeMoveOutcome, FsError> {
        let representation = TransferLinkRepresentation::linux_symbolic_link(
            self.original.payload(),
        )
        .map_err(|mut failure| {
            failure.path = self.path().to_owned();
            failure
        })?;
        self.revalidate(cancel)?;
        let target =
            native::bind_link_destination(destination, &self.original, &self.parent, cancel)?;
        midnight_core::TransferDirectory::admit_link(&target, &representation, cancel)?;
        let mut stage =
            match midnight_core::TransferDirectory::stage_link(&target, &representation, cancel) {
                Ok(stage) => stage,
                Err(failure) => {
                    return match failure.stage {
                        Some(stage) => abandon(stage, failure.error),
                        None => Err(failure.error),
                    };
                }
            };
        let verified = (|| {
            stage.sync(cancel)?;
            if stage.link_representation(cancel)? != representation {
                return Err(invalid(
                    self.path(),
                    "the recreated link reads back differently from the original",
                ));
            }
            let identity = stage.revalidate_stage(&representation, cancel)?;
            // The original is still the captured link right before publication.
            self.revalidate(cancel)?;
            Ok(identity)
        })();
        let identity = match verified {
            Ok(identity) => identity,
            Err(failure) => return abandon(stage, failure),
        };
        let publication = match replace {
            Some((expected, policy)) => stage
                .prepare_replacement(name, expected, policy, cancel)
                .and_then(|()| stage.publish_replace(name, cancel)),
            None => stage.publish_no_replace(name, cancel),
        };
        let publication = match publication {
            Ok(publication) => publication,
            // No user entry moved; an AlreadyExists asks for a fresh conflict decision.
            Err(failure) => return abandon(stage, failure),
        };
        let path = target.path().join(name);
        let (published, mut recovery) = match publication {
            TransferPublication::Published(published) => (published, Vec::new()),
            TransferPublication::Replaced {
                path: published,
                recovery,
            } => (published, vec![recovery]),
            TransferPublication::PublishedDurabilityUnconfirmed {
                path: published,
                mut error,
            } => {
                error.message = format!(
                    "The link was recreated at {}, but that is not yet durable, so the original \
                     link was kept: {}",
                    published.display(),
                    error.message
                );
                return Ok(finish(stage, Some(published), Vec::new(), error));
            }
            TransferPublication::RecoveryRequired {
                published,
                recovery,
                mut error,
            } => {
                error
                    .message
                    .push_str("; the original link was kept where it was");
                return Ok(finish(stage, published, recovery, error));
            }
        };
        let checked = if published != path {
            Err(invalid(
                &published,
                "the link was moved under an unexpected name",
            ))
        } else {
            stage
                .revalidate_publication(&representation, &Cancellation::default())
                .and_then(|actual| {
                    (actual == identity).then_some(()).ok_or_else(|| {
                        invalid(&published, "the moved link is not the verified stage")
                    })
                })
        };
        if let Err(mut error) = checked {
            error
                .message
                .push_str("; the original link was kept where it was");
            return Ok(finish(stage, Some(published), recovery, error));
        }
        if let Err(failure) = stage.cleanup() {
            let error = failure.error.clone();
            recovery.extend(failure.recovery);
            return Ok(TransferNativeMoveOutcome::RecoveryRequired {
                destination: Some(published),
                recovery,
                error: stage_kept(error),
            });
        }
        drop(stage);
        // The link exists at its destination; only now is the original removed.
        let removal = link_capture::Capture::prepare(
            self.original.clone(),
            self.parent.clone(),
            &Cancellation::default(),
        )
        .and_then(|capture| capture.remove_original(&Cancellation::default()));
        Ok(match removal {
            Ok(TransferDirectoryLinkRemoval::Removed {
                durability_confirmed,
                recovery: residual,
                error: None,
                ..
            }) if residual.is_empty() => TransferNativeMoveOutcome::Moved {
                destination: published,
                recovery,
                durability_confirmed,
            },
            Ok(TransferDirectoryLinkRemoval::Removed {
                recovery: residual,
                error,
                ..
            }) => {
                recovery.extend(residual);
                TransferNativeMoveOutcome::RecoveryRequired {
                    destination: Some(published),
                    recovery,
                    error: error.unwrap_or_else(|| {
                        invalid(
                            self.path(),
                            "the original link was removed, but its temporary folder remains",
                        )
                    }),
                }
            }
            Ok(TransferDirectoryLinkRemoval::RecoveryRequired {
                recovery: residual,
                error,
            }) => {
                recovery.extend(residual);
                TransferNativeMoveOutcome::RecoveryRequired {
                    destination: Some(published),
                    recovery,
                    error: kept_original(error),
                }
            }
            Err(error) => TransferNativeMoveOutcome::RecoveryRequired {
                destination: Some(published),
                recovery,
                error: kept_original(error),
            },
        })
    }
}

/// The private stage of a recreated link could not be cleaned up, so the original link
/// was not removed: removal was not attempted.
fn stage_kept(mut error: FsError) -> FsError {
    error.message = format!(
        "The link was recreated at its destination, but its private stage could not be \
         cleaned up, so the original link was kept: {}",
        error.message
    );
    error
}

fn kept_original(mut error: FsError) -> FsError {
    error.message = format!(
        "The link was recreated at its destination, but the original link could not be \
         removed: {}",
        error.message
    );
    error
}

/// Clean an unpublished stage after a failure before publication. A stage that
/// cannot be cleaned is reported as residue, never silently dropped.
fn abandon(
    mut stage: Box<dyn StagedTransferLink>,
    failure: FsError,
) -> Result<TransferNativeMoveOutcome, FsError> {
    match stage.cleanup() {
        Ok(()) => Err(failure),
        Err(cleanup) => {
            let mut error = failure;
            error.message.push_str(&format!(
                "; the private link stage could not be removed: {}",
                cleanup.error.message
            ));
            Ok(TransferNativeMoveOutcome::RecoveryRequired {
                destination: None,
                recovery: cleanup.recovery,
                error,
            })
        }
    }
}

/// End after an uncertain publication: report it, clean only the empty stage.
fn finish(
    mut stage: Box<dyn StagedTransferLink>,
    destination: Option<PathBuf>,
    mut recovery: Vec<midnight_core::TransferRecovery>,
    mut error: FsError,
) -> TransferNativeMoveOutcome {
    if let Err(cleanup) = stage.cleanup() {
        error.message.push_str(&format!(
            "; the private link stage could not be removed: {}",
            cleanup.error.message
        ));
        recovery.extend(cleanup.recovery);
    }
    TransferNativeMoveOutcome::RecoveryRequired {
        destination,
        recovery,
        error,
    }
}

impl TransferMoveCapture for Capture {
    fn entry(&self) -> &TransferEntry {
        self.original.entry()
    }

    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.original.qualify_removal()?;
        self.parent.check_recovery_parent(cancel)?;
        self.original.verify_original(cancel)
    }

    fn try_native_move(
        &self,
        destination: &dyn TransferDirectory,
        name: &OsStr,
        replace: Replacement,
        cancel: &Cancellation,
    ) -> Result<TransferNativeMoveOutcome, FsError> {
        let mut terminal = self.outcome.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(outcome) = terminal.as_ref() {
            return Ok(outcome.clone());
        }
        let outcome = match native::attempt_link(
            &self.original,
            &self.parent,
            destination,
            name,
            replace.clone(),
            cancel,
        )? {
            // A link has no content to verify: across filesystems it is recreated.
            TransferNativeMoveOutcome::CrossDevice => {
                self.recreate(destination, name, replace, cancel)?
            }
            outcome => outcome,
        };
        if native::terminal(&outcome) {
            *terminal = Some(outcome.clone());
        }
        Ok(outcome)
    }

    fn capture(&self, _cancel: &Cancellation) -> Result<TransferMoveCaptureOutcome, FsError> {
        Err(unsupported(
            self.path(),
            "A link is moved by renaming or recreating it, never by copying its content",
        ))
    }

    /// Nothing is ever staged from the original before its move commits.
    fn restore(&self) -> Result<(), FsError> {
        Ok(())
    }
}
