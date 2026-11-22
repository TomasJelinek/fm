//! A link that a Move keeps as a link (Preserve, or a link that FollowLinks cannot
//! follow): renamed on one volume, as a moved directory link is; across volumes
//! recreated at the destination (planned, staged, read back and published without
//! replacing anything, as Copy recreates a link) and only then removed from the source
//! with the protected removal of a moved directory link. If that removal fails, both
//! links remain and the outcome says so.
use super::*;
use midnight_core::{
    DeferredNativeMove, TransferDirectoryLinkMoveCapture, TransferDirectoryLinkRemoval,
    TransferNativeMoveOutcome as Outcome, TransferPublication, TransferReplacementPolicy,
};

type Replacement = Option<(Arc<dyn TransferTarget>, TransferReplacementPolicy)>;

pub(super) struct Capture {
    proof: Arc<source_links::OriginalLink>,
    /// The original link's capture: its rename, revalidation and protected removal.
    inner: Arc<dyn TransferDirectoryLinkMoveCapture>,
    /// The outcome of a recreation across volumes, which is never repeated.
    recreated: Mutex<Option<Outcome>>,
}

impl Capture {
    pub(super) fn new(
        proof: Arc<source_links::OriginalLink>,
        inner: Arc<dyn TransferDirectoryLinkMoveCapture>,
    ) -> Self {
        Self {
            proof,
            inner,
            recreated: Mutex::new(None),
        }
    }

    /// Recreates the link as `name` in `destination` on another volume, then removes the
    /// original. `Err` leaves no new link at the destination.
    fn recreate(
        &self,
        destination: &dyn TransferDirectory,
        name: &OsStr,
        replace: Replacement,
        cancel: &Cancellation,
    ) -> Result<Outcome, FsError> {
        let path = destination.path().join(name);
        let mut recreated = self
            .recreated
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(outcome) = recreated.as_ref() {
            return Ok(outcome.clone());
        }
        if replace.is_some() {
            return Err(native_move::collision(
                &path,
                "replacing an existing entry with a moved link is unsupported",
            ));
        }
        validate_name(name).map_err(|e| error(&path, e))?;
        self.inner.revalidate(cancel)?;
        let plan = destination.plan_link(self.proof.representation(), cancel)?;
        let mut stage = destination
            .stage_link(&plan.representation, cancel)
            .map_err(|mut failure| {
                if let Some(mut stage) = failure.stage.take()
                    && let Err(cleanup) = stage.cleanup()
                {
                    failure.error.message.push_str(&format!(
                        "; the private link stage could not be removed: {}",
                        cleanup.error.message
                    ));
                }
                failure.error
            })?;
        let staged = (|| {
            stage.sync(cancel)?;
            if stage.link_representation(cancel)? != plan.representation {
                return Err(invalid(
                    &path,
                    "Independent staged link readback differs from the source.",
                ));
            }
            self.inner.revalidate(cancel)?;
            let identity = stage.revalidate_stage(&plan.representation, cancel)?;
            stage
                .publish_no_replace(name, cancel)
                .map(|publication| (identity, publication))
        })();
        let (identity, publication) = match staged {
            Ok(staged) => staged,
            Err(mut failure) => {
                if let Err(cleanup) = stage.cleanup() {
                    failure.message.push_str(&format!(
                        "; the private link stage could not be removed: {}",
                        cleanup.error.message
                    ));
                }
                return Err(failure);
            }
        };
        // The link is published: from here on only what happened is reported.
        let durable = !matches!(
            publication,
            TransferPublication::PublishedDurabilityUnconfirmed { .. }
        );
        let checked = stage
            .revalidate_publication(&plan.representation, &Cancellation::default())
            .and_then(|actual| {
                if actual == identity {
                    Ok(())
                } else {
                    Err(invalid(
                        &path,
                        "The link at the destination is not the independently verified stage.",
                    ))
                }
            });
        let outcome = match checked {
            Err(error) => Outcome::RecoveryRequired {
                destination: Some(path.clone()),
                recovery: Vec::new(),
                error,
            },
            // The original goes only once its recreation is verified at the destination.
            Ok(()) => match self.inner.remove_original(&Cancellation::default()) {
                Ok(TransferDirectoryLinkRemoval::Removed {
                    durability_confirmed,
                    recovery,
                    error: None,
                    ..
                }) if recovery.is_empty() => Outcome::Moved {
                    destination: path.clone(),
                    recovery,
                    durability_confirmed: durability_confirmed && durable,
                },
                Ok(TransferDirectoryLinkRemoval::Removed {
                    recovery, error, ..
                }) => Outcome::RecoveryRequired {
                    destination: Some(path.clone()),
                    recovery,
                    error: error.unwrap_or_else(|| {
                        invalid(&path, "the original link left recovery data behind")
                    }),
                },
                Ok(TransferDirectoryLinkRemoval::RecoveryRequired { recovery, error }) => {
                    Outcome::RecoveryRequired {
                        destination: Some(path.clone()),
                        recovery,
                        error,
                    }
                }
                Err(error) => Outcome::RecoveryRequired {
                    destination: Some(path.clone()),
                    recovery: Vec::new(),
                    error,
                },
            },
        };
        *recreated = Some(outcome.clone());
        Ok(outcome)
    }
}

impl TransferMoveCapture for Capture {
    fn try_native_move(
        &self,
        destination: &dyn TransferDirectory,
        name: &OsStr,
        replace: Replacement,
        cancel: &Cancellation,
    ) -> Result<Outcome, FsError> {
        match self
            .inner
            .try_native_move(destination, name, replace.clone(), cancel)?
        {
            Outcome::CrossDevice => self.recreate(destination, name, replace, cancel),
            outcome => Ok(outcome),
        }
    }

    fn try_native_move_deferred(
        &self,
        destination: &dyn TransferDirectory,
        name: &OsStr,
        replace: Replacement,
        cancel: &Cancellation,
    ) -> Result<DeferredNativeMove, FsError> {
        match self
            .inner
            .try_native_move_deferred(destination, name, replace.clone(), cancel)?
        {
            (Outcome::CrossDevice, _) => self
                .recreate(destination, name, replace, cancel)
                .map(|outcome| (outcome, None)),
            deferred => Ok(deferred),
        }
    }

    fn entry(&self) -> &TransferEntry {
        &self.proof.entry
    }

    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.inner.revalidate(cancel)
    }

    fn capture(&self, _cancel: &Cancellation) -> Result<TransferMoveCaptureOutcome, FsError> {
        Err(unsupported(
            &self.proof.entry.item.path,
            "a link has no content to capture; it is moved or recreated as a link",
        ))
    }
}
