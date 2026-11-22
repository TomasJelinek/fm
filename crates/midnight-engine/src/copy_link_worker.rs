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
    links: CopyLinkPolicy,
) -> Result<(), FsError> {
    if snapshot(status).intent != TransferIntent::Copy {
        return Err(error(
            path,
            io::ErrorKind::Unsupported,
            "This source preparation requires Copy authority.",
        ));
    }
    if links.recreates_links() && metadata != TransferMetadataPolicy::ContentOnly {
        return Err(error(
            path,
            io::ErrorKind::Unsupported,
            "Link preservation requires Copy with content-only metadata.",
        ));
    }
    Ok(())
}

/// Checks the selected roots of a job that recreates links: each is a native file,
/// folder or link whose representation reads. The destination is not asked here: a link
/// it cannot create is reported when the job reaches it, with its own question, and the
/// other selected items are still copied.
pub(super) fn admit_roots(
    roots: &VecDeque<Box<dyn TransferSource>>,
    links: CopyLinkPolicy,
    cancel: &Cancellation,
) -> Result<(), FsError> {
    if !links.recreates_links() {
        return Ok(());
    }
    for source in roots {
        validate_native_source(source.as_ref())?;
        source.revalidate(cancel)?;
        match source.entry().item.kind {
            EntryKind::File | EntryKind::Directory => {}
            EntryKind::Symlink | EntryKind::DirectoryLink => {
                source.link_representation(cancel)?;
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
    /// Direct Job fixtures, which have no admission envelope, lend a plain permit.
    #[cfg(test)]
    permit: Option<OwnedSemaphorePermit>,
    admitted: Option<Arc<midnight_work_budget::Permit>>,
}
impl RetirementIo {
    fn held(&self) -> bool {
        #[cfg(test)]
        if self.permit.is_some() {
            return true;
        }
        self.admitted.is_some()
    }
}

pub(super) struct LinkRetirement {
    diagnostics: Arc<Diagnostics>,
    io: Mutex<RetirementIo>,
}

#[derive(Default)]
struct PendingDiagnostics {
    failures: VecDeque<TransferLinkCleanupError>,
    omitted: u64,
    omitted_recovery: u64,
}
/// Bounded diagnostics carry no native owner or execution permit.
#[derive(Default)]
pub(super) struct Diagnostics(Mutex<PendingDiagnostics>);
impl Diagnostics {
    pub(super) fn record(&self, mut failure: TransferLinkCleanupError) {
        let mut pending = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if failure.recovery.len() > MAX_ERROR_DETAILS {
            pending.omitted_recovery += (failure.recovery.len() - MAX_ERROR_DETAILS) as u64;
            failure.recovery.truncate(MAX_ERROR_DETAILS);
        }
        if pending.failures.len() == MAX_ERROR_DETAILS
            && let Some(omitted) = pending.failures.pop_front()
        {
            pending.omitted += 1;
            pending.omitted_recovery += omitted.recovery.len() as u64;
        }
        pending.failures.push_back(failure);
    }
    pub(super) fn drain(&self, progress: &mut CopyProgress) {
        let mut pending = self.0.lock().unwrap_or_else(|error| error.into_inner());
        progress.failed = progress
            .failed
            .saturating_add(std::mem::take(&mut pending.omitted));
        progress.recovery_count = progress
            .recovery_count
            .saturating_add(std::mem::take(&mut pending.omitted_recovery));
        for failure in pending.failures.drain(..) {
            let error = retain_link_recovery(progress, failure);
            retain_error(progress, error, true);
        }
    }
}

impl LinkRetirement {
    pub(super) fn new() -> Self {
        Self {
            diagnostics: Arc::default(),
            io: Mutex::default(),
        }
    }

    pub(super) fn diagnostics(&self) -> Arc<Diagnostics> {
        self.diagnostics.clone()
    }

    #[cfg(test)]
    fn begin(&self, permit: Option<OwnedSemaphorePermit>) {
        let mut io = self.io.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(permit) = permit {
            debug_assert!(io.permit.is_none(), "a retained permit is reused");
            io.permit = Some(permit);
        }
    }

    fn attach(&self, permit: Arc<midnight_work_budget::Permit>) {
        let mut io = self.io.lock().unwrap_or_else(|error| error.into_inner());
        assert!(!io.held(), "Copy I/O is attached once");
        io.admitted = Some(permit);
    }

    fn holds_io(&self) -> bool {
        self.io
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .held()
    }

    /// The job's actual admitted slot survives stage/source/provider Drop; checked
    /// teardown reclaims it before any cleanup runs.
    fn retain_cleanup_io(&self) {
        debug_assert!(self.holds_io(), "link staging runs with Copy I/O ownership");
    }

    fn report(&self, failure: TransferLinkCleanupError) {
        self.diagnostics.record(failure);
        // The sealed owner derives terminal status only after actual native
        // destruction. This callback records diagnostics, never completion.
    }
}

pub(super) struct LinkCopy {
    // Field order is intentional: the stage and its cleanup authority die before
    // the source, which may own the captured provider registration and permit.
    stage: Option<Box<dyn StagedTransferLink>>,
    source: Box<dyn TransferSource>,
    /// The source link's exact representation, checked again before each step.
    representation: TransferLinkRepresentation,
    /// What the destination creates for it (`TransferDirectory::plan_link`): the stage
    /// is verified against this.
    planned: TransferLinkRepresentation,
    phase: Phase,
    stage_identity: Option<TransferIdentity>,
    cleanup_attempted: bool,
    /// What the job details say about this link once it is published: why FollowLinks
    /// copied it as a link, another form the destination created, a relative target
    /// that leads outside what was copied.
    notes: Vec<String>,
    // Retains the I/O guard after stage/source destruction on forced retirement.
    retirement: Arc<LinkRetirement>,
}

impl LinkCopy {
    fn new(
        source: Box<dyn TransferSource>,
        representation: TransferLinkRepresentation,
        plan: TransferLinkPlan,
        notes: Vec<String>,
        retirement: Arc<LinkRetirement>,
    ) -> Self {
        Self {
            stage: None,
            source,
            representation,
            planned: plan.representation,
            phase: Phase::Staging,
            stage_identity: None,
            cleanup_attempted: false,
            notes,
            retirement,
        }
    }

    fn needs_stage(&self) -> bool {
        matches!(self.phase, Phase::Staging)
    }

    /// The next step publishes the verified stage at the final name.
    fn publishes_next(&self) -> bool {
        matches!(self.phase, Phase::ReadyToPublish)
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

    /// `replacement` is the consented target when the next step publishes over it.
    fn step(
        &mut self,
        destination: &dyn TransferDirectory,
        name: &OsStr,
        replacement: Option<Arc<dyn TransferTarget>>,
        cancel: &Cancellation,
    ) -> Result<Option<TransferPublication>, FsError> {
        if !self.can_retry() {
            return Err(error(
                &self.source.entry().item.path,
                io::ErrorKind::InvalidInput,
                "This link copy cannot be resumed or finished.",
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
                destination.admit_link(&self.planned, cancel)?;
                match destination.stage_link(&self.planned, cancel) {
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
                if stage.link_representation(cancel)? != self.planned {
                    return Err(error(
                        &self.source.entry().item.path,
                        io::ErrorKind::InvalidData,
                        "Independent staged link readback differs from the source.",
                    ));
                }
                self.stage_identity = Some(stage.revalidate_stage(&self.planned, cancel)?);
                self.phase = Phase::ReadyToPublish;
            }
            Phase::ReadyToPublish => {
                let stage = self.stage.as_mut().expect("verified link stage");
                let identity = stage.revalidate_stage(&self.planned, cancel)?;
                if Some(identity) != self.stage_identity {
                    return Err(error(
                        &self.source.entry().item.path,
                        io::ErrorKind::InvalidData,
                        "The owned link stage identity changed before the link got its final name.",
                    ));
                }
                let replacing = replacement.is_some();
                let publication = match replacement {
                    // Binding changes no name; a refused binding publishes nothing.
                    Some(target) => {
                        stage.prepare_replacement(
                            name,
                            target,
                            TransferReplacementPolicy::RecoverableCurrentEntry,
                            cancel,
                        )?;
                        stage.publish_replace(name, cancel)?
                    }
                    None => stage.publish_no_replace(name, cancel)?,
                };
                // Set this before any post-mutation observation. Late cancellation
                // or proof failure can never turn a commit into an unpublished retry.
                self.phase = Phase::Published;
                let expected_path = destination.path().join(name);
                return Ok(Some(validate_publication(
                    publication,
                    stage.as_ref(),
                    &self.planned,
                    identity,
                    &expected_path,
                    replacing,
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
    replacing: bool,
) -> TransferPublication {
    let validate = |path: &Path| {
        if path != expected_path {
            return Err(error(
                path,
                io::ErrorKind::InvalidData,
                "The link provider reported a different destination for the link.",
            ));
        }
        // Publication already happened. Finish the owned read-only observation
        // under the existing worker/I/O lease even after the user's cancellation.
        let actual = stage.revalidate_publication(representation, &Cancellation::default())?;
        if actual != identity {
            return Err(error(
                path,
                io::ErrorKind::InvalidData,
                "The link at the destination is not the independently verified stage.",
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
                        .push_str(&format!("; durability is also unconfirmed: {error}"));
                    TransferPublication::RecoveryRequired {
                        published: Some(path),
                        recovery: vec![],
                        error: observation,
                    }
                }
            }
        }
        // A consented replacement keeps the displaced entry in recovery.
        TransferPublication::Replaced { path, recovery } if replacing => match validate(&path) {
            Ok(()) => TransferPublication::Replaced { path, recovery },
            Err(error) => TransferPublication::RecoveryRequired {
                published: Some(path),
                recovery: vec![recovery],
                error,
            },
        },
        TransferPublication::Replaced { path, recovery } => TransferPublication::RecoveryRequired {
            published: Some(path),
            recovery: vec![recovery],
            error: error(
                expected_path,
                io::ErrorKind::InvalidData,
                "The link provider replaced an existing entry although no replacement was chosen.",
            ),
        },
        recovery @ TransferPublication::RecoveryRequired { .. } => recovery,
    }
}

impl Job {
    pub(in crate::copy_operations) fn attach_io_permit(
        &self,
        permit: Arc<midnight_work_budget::Permit>,
    ) {
        self.link_retirement.attach(permit);
    }

    #[cfg(test)]
    pub(in crate::copy_operations) fn begin_io_step(&self, permit: Option<OwnedSemaphorePermit>) {
        self.link_retirement.begin(permit);
    }

    #[cfg(test)]
    pub(in crate::copy_operations) fn holds_io_permit(&self) -> bool {
        self.link_retirement.holds_io()
    }

    pub(super) fn prepare_link(&mut self, work: &mut EntryWork) -> Result<Action, FsError> {
        if !self.links.recreates_links() || self.move_work.is_some() || self.metadata.is_some() {
            return Err(error(
                &work.entry.item.path,
                io::ErrorKind::Unsupported,
                "Copying links requires explicit Copy-only link preservation.",
            ));
        }
        let source = work.source.as_ref().expect("unstarted link source");
        validate_native_source(source.as_ref())?;
        let representation = source.link_representation(&self.cancel)?;
        let plan = work.destination.plan_link(&representation, &self.cancel)?;
        let notes = self.link_notes(work, source.as_ref(), &representation, plan.note.as_deref());
        work.link = Some(LinkCopy::new(
            work.source.take().expect("retained link source"),
            representation,
            plan,
            notes,
            self.link_retirement.clone(),
        ));
        self.link_step(work)
    }

    /// What the job details say about the link `work` that the job keeps as a link: why
    /// FollowLinks did not follow it, another form the destination creates for it
    /// (`planned`), and a relative target that leads outside what is copied or moved.
    pub(super) fn link_notes(
        &self,
        work: &EntryWork,
        source: &dyn TransferSource,
        representation: &TransferLinkRepresentation,
        planned: Option<&str>,
    ) -> Vec<String> {
        let path = &work.entry.item.path;
        let (moved, items) = if self.move_work.is_some() {
            ("moved", "the moved items")
        } else {
            ("copied", "the copied items")
        };
        let mut notes = Vec::new();
        // Following links, only a link that cannot be followed is kept as a link.
        if self.links == CopyLinkPolicy::FollowLinks {
            let reason = source
                .link_not_followed()
                .unwrap_or_else(|| "it could not be followed".into());
            notes.push(format!(
                "{} was {moved} as a link because {reason}.",
                path.display()
            ));
        }
        if let Some(note) = planned {
            notes.push(format!("{} {note}.", path.display()));
        }
        if let Some(root) = self.active_root_path()
            && relative_target_leaves(representation, path, root)
        {
            notes.push(format!(
                "{} keeps its relative target \u{201c}{}\u{201d}, which leads outside {items}; \
                 at the destination it points somewhere else or nowhere.",
                path.display(),
                link_text(representation)
            ));
        }
        notes
    }

    pub(super) fn link_step(&mut self, work: &mut EntryWork) -> Result<Action, FsError> {
        let copy = work.link.as_ref().expect("started link copy");
        if work.consent.is_none()
            && copy.needs_stage()
            && let Some(target) = self.observe_target(work)?
        {
            return Ok(Action::Conflict(target));
        }
        // A consented replacement publishes over the pinned target, which must be
        // unchanged; a changed or new target asks again.
        let replacement = if work.consent == Some(Consent::Replace)
            && work.link.as_ref().is_some_and(LinkCopy::publishes_next)
        {
            let target = work.target.clone().ok_or_else(|| {
                error(
                    &work.entry.item.path,
                    io::ErrorKind::InvalidData,
                    "The consented replacement target is missing.",
                )
            })?;
            if target.revalidate(&self.cancel).is_err() {
                work.consent = None;
                work.force_prompt = true;
                if let Some(target) = self.observe_target(work)? {
                    return Ok(Action::Conflict(target));
                }
                None
            } else {
                Some(target)
            }
        } else {
            None
        };
        let copy = work.link.as_mut().expect("started link copy");
        let publishes = copy.publishes_next();
        let publication = copy.step(
            work.destination.as_ref(),
            &work.name,
            replacement,
            &self.cancel,
        );
        // Only publication uses the final name; a staging AlreadyExists is no collision.
        let publication = if publishes {
            self.at_final_name(publication)?
        } else {
            publication?
        };
        self.progress.phase = copy.progress_phase();
        let Some(publication) = publication else {
            return Ok(Action::Keep);
        };
        // Link payloads are representation proofs, never referent content bytes.
        // Reuse publication facts/counts without touching the content byte meters.
        let published = !matches!(
            &publication,
            TransferPublication::RecoveryRequired {
                published: None,
                ..
            }
        );
        if published {
            for note in std::mem::take(&mut copy.notes) {
                self.progress.add_note(note);
            }
        }
        match publication {
            TransferPublication::Published(_) => self.progress.files_completed += 1,
            TransferPublication::Replaced { recovery, .. } => {
                self.progress.files_completed += 1;
                self.record_recovery(recovery.path);
            }
            // An uncertain commit is never repeated: the job asks whether to go on, and
            // the link and its recovery data stay where they are.
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
                let committed = self.committed_entry(work, error, CopyErrorStage::Publishing);
                self.pause_committed(committed);
            }
            TransferPublication::PublishedDurabilityUnconfirmed { path, mut error } => {
                self.progress.files_completed += 1;
                error.message = format!(
                    "Link copied to {}; durability is unconfirmed: {}",
                    path.display(),
                    error.message
                );
                let committed = self.committed_entry(work, error, CopyErrorStage::Publishing);
                self.pause_committed(committed);
            }
        }
        let copy = work.link.as_mut().expect("started link copy");
        if let Err(failure) = copy.cleanup() {
            // The link is in place; only its private stage remains.
            let error = retain_link_recovery(&mut self.progress, failure);
            let committed = self.committed_entry(work, error, CopyErrorStage::Cleanup);
            self.pause_committed(committed);
        }
        Ok(Action::Done)
    }

    pub(super) fn record_link_cleanup(&mut self, failure: TransferLinkCleanupError) {
        self.halt = true;
        let error = retain_link_recovery(&mut self.progress, failure);
        retain_error(&mut self.progress, error, true);
    }
}

/// Whether the relative text of the link at `path` leads outside the selected item
/// `root` that holds it: from the link's folder, counted from `root` (or, for a selected
/// link, from the folder it is in), its `..` names climb above that folder.
pub(super) fn relative_target_leaves(
    representation: &TransferLinkRepresentation,
    path: &Path,
    root: &Path,
) -> bool {
    let depth = if path == root {
        Some(0)
    } else {
        path.parent()
            .and_then(|parent| parent.strip_prefix(root).ok())
            .map(|below| below.components().count())
    };
    depth.is_some_and(|depth| representation.relative_text_leaves(depth))
}

/// A link's text for the job details: its exact bytes on Linux, its print name (or
/// substitute name) on Windows, with what cannot be shown replaced.
pub(super) fn link_text(representation: &TransferLinkRepresentation) -> String {
    if let Some(target) = representation.linux_target() {
        return String::from_utf8_lossy(target).into_owned();
    }
    representation
        .windows_names()
        .map(|(substitute, print)| {
            String::from_utf16_lossy(if print.is_empty() { substitute } else { print })
        })
        .unwrap_or_default()
}

/// Records the residue of a link stage that could not be cleaned up and returns its error.
pub(super) fn retain_link_recovery(
    progress: &mut CopyProgress,
    mut failure: TransferLinkCleanupError,
) -> FsError {
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
    failure.error
}
