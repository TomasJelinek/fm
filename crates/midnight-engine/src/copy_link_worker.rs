//! Raw link proofs share Copy ownership, never regular-file content or Move authority.
use super::*;
use std::ffi::OsStr;

pub(super) fn is_link_kind(kind: EntryKind) -> bool {
    matches!(kind, EntryKind::Symlink | EntryKind::DirectoryLink)
}

pub(super) fn validate_options(
    path: &Path,
    status: &Status,
    metadata: TransferMetadataPolicy,
) -> Result<(), FsError> {
    if snapshot(status).intent != TransferIntent::Copy
        || metadata != TransferMetadataPolicy::ContentOnly
    {
        return Err(error(
            path,
            io::ErrorKind::Unsupported,
            "Link preservation requires Copy with content-only metadata.",
        ));
    }
    Ok(())
}

pub(super) fn admit_roots(
    roots: &VecDeque<Box<dyn TransferSource>>,
    destination: &dyn TransferDirectory,
    links: CopyLinkPolicy,
    cancel: &Cancellation,
) -> Result<(), FsError> {
    if links == CopyLinkPolicy::Reject {
        return Ok(());
    }
    for source in roots {
        validate_native_source(source.as_ref())?;
        source.revalidate(cancel)?;
        match source.entry().item.kind {
            EntryKind::File | EntryKind::Directory => {}
            EntryKind::Symlink | EntryKind::DirectoryLink => {
                let representation = source.link_representation(cancel)?;
                destination.admit_link(&representation, cancel)?;
            }
            EntryKind::Other => {
                return Err(error(
                    &source.entry().item.path,
                    io::ErrorKind::Unsupported,
                    "Preserving this native source kind is unsupported.",
                ));
            }
        }
    }
    Ok(())
}

fn validate_native_source(source: &dyn TransferSource) -> Result<(), FsError> {
    if source.native_identity() != Some(source.entry().identity)
        || source.virtual_identity().is_some()
    {
        return Err(error(
            &source.entry().item.path,
            io::ErrorKind::Unsupported,
            "Link preservation requires a retained native source.",
        ));
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum Phase {
    Staging,
    Syncing,
    Verifying,
    ReadyToPublish,
    Published,
    ConstructionFailed,
}

#[derive(Default)]
struct RetirementIo {
    permit: Option<OwnedSemaphorePermit>,
    cleanup_owned: bool,
}

pub(super) struct LinkRetirement {
    status: Status,
    cancel: Cancellation,
    io: Mutex<RetirementIo>,
}

impl LinkRetirement {
    pub(super) fn new(status: Status, cancel: Cancellation) -> Self {
        Self {
            status,
            cancel,
            io: Mutex::default(),
        }
    }

    fn begin(&self, permit: Option<OwnedSemaphorePermit>) {
        let mut io = self.io.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(permit) = permit {
            debug_assert!(io.permit.is_none(), "a retained permit is reused");
            io.permit = Some(permit);
        }
        debug_assert!(!io.cleanup_owned || io.permit.is_some());
    }

    fn holds_io(&self) -> bool {
        self.io
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .permit
            .is_some()
    }

    fn retain_cleanup_io(&self) {
        let mut io = self.io.lock().unwrap_or_else(|error| error.into_inner());
        debug_assert!(
            io.permit.is_some(),
            "link staging runs with Copy I/O ownership"
        );
        // One serial Copy job owns this dedicated permit. Keep it until Job's
        // last field drops, after every stage, source and provider registration.
        io.cleanup_owned = true;
    }

    fn accepted(&self) {
        let permit = {
            let mut io = self.io.lock().unwrap_or_else(|error| error.into_inner());
            if io.cleanup_owned {
                None
            } else {
                io.permit.take()
            }
        };
        drop(permit);
    }

    fn report(&self, failure: TransferLinkCleanupError) {
        let mut progress = self
            .status
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        retain_link_cleanup(&mut progress, failure);
        progress.pause_requested = false;
        progress.paused = false;
        progress.state = if self.cancel.is_cancelled() {
            CopyOperationState::Cancelled
        } else if progress.files_completed > 0 || progress.directories_created > 0 {
            CopyOperationState::CompletedWithErrors
        } else {
            CopyOperationState::Failed
        };
        progress.message = progress
            .errors
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
    }
}

pub(super) struct LinkCopy {
    // Field order is intentional: the stage and its cleanup authority die before
    // the source, which may own the captured provider registration and permit.
    stage: Option<Box<dyn StagedTransferLink>>,
    source: Box<dyn TransferSource>,
    representation: TransferLinkRepresentation,
    phase: Phase,
    stage_identity: Option<TransferIdentity>,
    cleanup_attempted: bool,
    // Retains the I/O guard after stage/source destruction on forced retirement.
    retirement: Arc<LinkRetirement>,
}

impl LinkCopy {
    fn new(
        source: Box<dyn TransferSource>,
        representation: TransferLinkRepresentation,
        retirement: Arc<LinkRetirement>,
    ) -> Self {
        Self {
            stage: None,
            source,
            representation,
            phase: Phase::Staging,
            stage_identity: None,
            cleanup_attempted: false,
            retirement,
        }
    }

    fn needs_stage(&self) -> bool {
        matches!(self.phase, Phase::Staging)
    }

    pub(super) fn can_retry(&self) -> bool {
        !self.cleanup_attempted
            && !matches!(self.phase, Phase::Published | Phase::ConstructionFailed)
    }

    pub(super) fn failure_stage(&self) -> CopyErrorStage {
        match self.phase {
            Phase::Staging | Phase::ConstructionFailed => CopyErrorStage::Preparing,
            Phase::Syncing => CopyErrorStage::Syncing,
            Phase::Verifying => CopyErrorStage::Verifying,
            Phase::ReadyToPublish | Phase::Published => CopyErrorStage::Publishing,
        }
    }

    fn progress_phase(&self) -> CopyOperationPhase {
        match self.phase {
            Phase::Staging | Phase::ConstructionFailed => CopyOperationPhase::Planning,
            Phase::Syncing => CopyOperationPhase::Syncing,
            Phase::Verifying => CopyOperationPhase::Verifying,
            Phase::ReadyToPublish | Phase::Published => CopyOperationPhase::Publishing,
        }
    }

    fn revalidate_source(&self, cancel: &Cancellation) -> Result<(), FsError> {
        if cancel.is_cancelled() {
            return Err(error(
                &self.source.entry().item.path,
                io::ErrorKind::Interrupted,
                "Copy cancelled.",
            ));
        }
        self.source.revalidate(cancel)?;
        if self.source.link_representation(cancel)? != self.representation {
            return Err(error(
                &self.source.entry().item.path,
                io::ErrorKind::InvalidData,
                "The retained source link representation changed.",
            ));
        }
        Ok(())
    }

    fn step(
        &mut self,
        destination: &dyn TransferDirectory,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Option<TransferPublication>, FsError> {
        if !self.can_retry() {
            return Err(error(
                &self.source.entry().item.path,
                io::ErrorKind::InvalidInput,
                "This link stage cannot be resumed or published.",
            ));
        }
        self.revalidate_source(cancel)?;
        destination.revalidate(cancel)?;
        match self.phase {
            Phase::Staging => {
                if !self.retirement.holds_io() {
                    return Err(error(
                        &self.source.entry().item.path,
                        io::ErrorKind::InvalidInput,
                        "Link staging requires the existing Copy I/O permit.",
                    ));
                }
                destination.admit_link(&self.representation, cancel)?;
                match destination.stage_link(&self.representation, cancel) {
                    Ok(stage) => {
                        self.stage = Some(stage);
                        self.retirement.retain_cleanup_io();
                        self.phase = Phase::Syncing;
                    }
                    Err(failure) => {
                        // An incomplete native construction is cleanup authority,
                        // never a resumable proof. Keep its source/provider alive.
                        if failure.stage.is_some() {
                            self.retirement.retain_cleanup_io();
                            self.phase = Phase::ConstructionFailed;
                        }
                        self.stage = failure.stage;
                        return Err(failure.error);
                    }
                }
            }
            Phase::Syncing => {
                self.stage
                    .as_mut()
                    .expect("created link stage")
                    .sync(cancel)?;
                self.phase = Phase::Verifying;
            }
            Phase::Verifying => {
                let stage = self.stage.as_ref().expect("created link stage");
                if stage.link_representation(cancel)? != self.representation {
                    return Err(error(
                        &self.source.entry().item.path,
                        io::ErrorKind::InvalidData,
                        "Independent staged link readback differs from the source.",
                    ));
                }
                self.stage_identity = Some(stage.revalidate_stage(&self.representation, cancel)?);
                self.phase = Phase::ReadyToPublish;
            }
            Phase::ReadyToPublish => {
                let stage = self.stage.as_mut().expect("verified link stage");
                let identity = stage.revalidate_stage(&self.representation, cancel)?;
                if Some(identity) != self.stage_identity {
                    return Err(error(
                        &self.source.entry().item.path,
                        io::ErrorKind::InvalidData,
                        "The owned link stage identity changed before publication.",
                    ));
                }
                let publication = stage.publish_no_replace(name, cancel)?;
                // Set this before any post-mutation observation. Late cancellation
                // or proof failure can never turn a commit into an unpublished retry.
                self.phase = Phase::Published;
                let expected_path = destination.path().join(name);
                return Ok(Some(validate_publication(
                    publication,
                    stage.as_ref(),
                    &self.representation,
                    identity,
                    &expected_path,
                )));
            }
            Phase::Published | Phase::ConstructionFailed => {
                unreachable!("nonresumable link work was checked before native calls")
            }
        }
        Ok(None)
    }

    pub(super) fn cleanup(&mut self) -> Result<(), TransferLinkCleanupError> {
        if self.cleanup_attempted {
            return Ok(());
        }
        self.cleanup_attempted = true;
        self.stage.as_mut().map_or(Ok(()), |stage| stage.cleanup())
    }
}

impl Drop for LinkCopy {
    fn drop(&mut self) {
        if self.cleanup_attempted || self.stage.is_none() {
            return;
        }
        let retirement = self.retirement.clone();
        debug_assert!(
            retirement.holds_io(),
            "owned link stages retain their I/O permit"
        );
        // No acquisition or capacity-based abandonment is needed in Drop. The
        // already-held permit outlives actual cleanup and all native pin drops.
        if let Err(failure) = self.cleanup() {
            retirement.report(failure);
        }
    }
}

fn validate_publication(
    publication: TransferPublication,
    stage: &dyn StagedTransferLink,
    representation: &TransferLinkRepresentation,
    identity: TransferIdentity,
    expected_path: &Path,
) -> TransferPublication {
    let validate = |path: &Path| {
        if path != expected_path {
            return Err(error(
                path,
                io::ErrorKind::InvalidData,
                "The link provider reported a different publication destination.",
            ));
        }
        // Publication already happened. Finish the owned read-only observation
        // under the existing worker/I/O lease even after the user's cancellation.
        let actual = stage.revalidate_publication(representation, &Cancellation::default())?;
        if actual != identity {
            return Err(error(
                path,
                io::ErrorKind::InvalidData,
                "The published link is not the independently verified stage.",
            ));
        }
        Ok(())
    };
    match publication {
        TransferPublication::Published(path) => match validate(&path) {
            Ok(()) => TransferPublication::Published(path),
            Err(error) => TransferPublication::RecoveryRequired {
                published: Some(path),
                recovery: vec![],
                error,
            },
        },
        TransferPublication::PublishedDurabilityUnconfirmed { path, error } => {
            match validate(&path) {
                Ok(()) => TransferPublication::PublishedDurabilityUnconfirmed { path, error },
                Err(mut observation) => {
                    observation
                        .message
                        .push_str(&format!("; publication durability: {error}"));
                    TransferPublication::RecoveryRequired {
                        published: Some(path),
                        recovery: vec![],
                        error: observation,
                    }
                }
            }
        }
        TransferPublication::Replaced { path, recovery } => TransferPublication::RecoveryRequired {
            published: Some(path),
            recovery: vec![recovery],
            error: error(
                expected_path,
                io::ErrorKind::InvalidData,
                "The link provider violated exclusive no-replace publication.",
            ),
        },
        recovery @ TransferPublication::RecoveryRequired { .. } => recovery,
    }
}

impl Job {
    pub(in crate::copy_operations) fn begin_io_step(&self, permit: Option<OwnedSemaphorePermit>) {
        self.link_retirement.begin(permit);
    }

    pub(in crate::copy_operations) fn holds_io_permit(&self) -> bool {
        self.link_retirement.holds_io()
    }

    pub(in crate::copy_operations) fn complete_io_step(&self) {
        self.link_retirement.accepted();
    }

    pub(super) fn prepare_link(&mut self, work: &mut EntryWork) -> Result<Action, FsError> {
        if self.links != CopyLinkPolicy::Preserve
            || self.move_work.is_some()
            || self.metadata.is_some()
        {
            return Err(error(
                &work.entry.item.path,
                io::ErrorKind::Unsupported,
                "Copying links requires explicit Copy-only link preservation.",
            ));
        }
        let source = work.source.as_ref().expect("unstarted link source");
        validate_native_source(source.as_ref())?;
        let representation = source.link_representation(&self.cancel)?;
        work.destination.admit_link(&representation, &self.cancel)?;
        work.link = Some(LinkCopy::new(
            work.source.take().expect("retained link source"),
            representation,
            self.link_retirement.clone(),
        ));
        self.link_step(work)
    }

    pub(super) fn link_step(&mut self, work: &mut EntryWork) -> Result<Action, FsError> {
        if work.link.as_ref().expect("started link copy").needs_stage()
            && let Some(target) = self.observe_target(work)?
        {
            return Ok(Action::Conflict(target));
        }
        let copy = work.link.as_mut().expect("started link copy");
        let publication = copy.step(work.destination.as_ref(), &work.name, &self.cancel)?;
        self.progress.phase = copy.progress_phase();
        let Some(publication) = publication else {
            return Ok(Action::Keep);
        };
        // Link payloads are representation proofs, never referent content bytes.
        // Reuse publication facts/counts without touching the content byte meters.
        match publication {
            TransferPublication::Published(_) => self.progress.files_completed += 1,
            TransferPublication::RecoveryRequired {
                published,
                recovery,
                error,
            } => {
                if published.is_some() {
                    self.progress.files_completed += 1;
                }
                for residual in recovery {
                    self.record_recovery(residual.path);
                }
                self.halt = true;
                self.record(error);
            }
            TransferPublication::PublishedDurabilityUnconfirmed { path, mut error } => {
                self.progress.files_completed += 1;
                error.message = format!(
                    "Link published at {}; durability is unconfirmed: {}",
                    path.display(),
                    error.message
                );
                self.halt = true;
                self.record(error);
            }
            TransferPublication::Replaced { .. } => {
                unreachable!("link publication converts unauthorized replacement to recovery")
            }
        }
        if let Err(failure) = copy.cleanup() {
            self.record_link_cleanup(failure);
        }
        Ok(Action::Done)
    }

    pub(super) fn record_link_cleanup(&mut self, failure: TransferLinkCleanupError) {
        self.halt = true;
        retain_link_cleanup(&mut self.progress, failure);
    }
}

fn retain_link_cleanup(progress: &mut CopyProgress, mut failure: TransferLinkCleanupError) {
    if failure.cleanup_unconfirmed || failure.recovery.is_empty() {
        failure
            .error
            .message
            .push_str("; owned link stage cleanup is unconfirmed");
    }
    for residual in failure.recovery {
        progress.recovery_count += 1;
        if progress.recovery_paths.len() < 16 {
            progress.recovery_paths.push(residual.path);
        }
    }
    retain_error(progress, failure.error, true);
}
