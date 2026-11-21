//! Bounded Move authority. Native rename is attempted before copy-only proof work.
use super::*;
use midnight_platform_common::transfer::{FileProofPhase, VerifiedFileProof};
use std::collections::HashMap;
#[cfg(test)]
#[path = "copy_finish_seal_tests.rs"]
mod finish_seal_tests;
#[cfg(test)]
#[path = "move_preparation_tests.rs"]
mod preparation_tests;
#[path = "move_retry.rs"]
mod retry;
#[cfg(test)]
#[path = "move_retry_tests.rs"]
mod retry_tests;

const MAX_MOVE_ROOTS: usize = 64;
const MAX_MOVE_ENTRIES: usize = 100_000;
pub(super) const MAX_MOVE_DIRECTORY_DEPTH: usize = 128;
const MAX_PROOF_PATH_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Copying,
    Before,
    Capture,
    After,
    Finalize,
    Halted,
}

pub(super) struct MoveWork {
    captures: Vec<Arc<dyn TransferMoveCapture>>,
    link_captures: HashMap<TransferIdentity, LinkAuthority>,
    /// The job's frozen link policy; Preserve admits links as movable entries.
    links: CopyLinkPolicy,
    pub(super) metadata_policy: TransferMetadataPolicy,
    pub(super) stages: HashMap<TransferIdentity, Box<dyn StagedTransferFile>>,
    pub(super) proofs: Vec<VerifiedFileProof>,
    /// Where the file of the active proof was published.
    pub(super) published: Option<std::path::PathBuf>,
    phase: Phase,
    finalizer: Option<Box<dyn TransferMoveFinalizer>>,
    active: Option<(std::path::PathBuf, TransferRecovery)>,
    reading: bool,
    buffer: Vec<u8>,
    admitted: usize,
    refresh_parents: Vec<std::path::PathBuf>,
    failed_phase: Option<Phase>,
    retryable_failure: bool,
    /// Confirmations of committed native renames, run together ([`Self::confirm_pending`]).
    pending_confirmations: Vec<Box<dyn TransferDurabilityConfirmation>>,
}

struct LinkAuthority {
    projection: Option<Box<dyn TransferSource>>,
    selected_parents: Option<Arc<[Arc<dyn TransferDirectory>]>>,
    capture: Arc<dyn TransferDirectoryLinkMoveCapture>,
}

impl LinkAuthority {
    fn new(capture: Arc<dyn TransferDirectoryLinkMoveCapture>) -> Self {
        Self {
            projection: None,
            selected_parents: None,
            capture,
        }
    }
}

pub(super) fn prepare(
    request: CopyRequest,
    service: &dyn TransferService,
    cancel: Cancellation,
    status: Status,
) -> Result<Job, FsError> {
    prepare_with_metadata(
        request,
        service,
        cancel,
        status,
        TransferMetadataPolicy::ContentOnly,
    )
}
pub(super) fn check_bounds(request: &CopyRequest) -> Result<(), FsError> {
    request.clipboard.validate()?;
    check_root_bounds(
        &request.destination,
        request.clipboard.paths.iter().map(|path| path.as_path()),
    )
}
pub(super) fn check_root_bounds<'a>(
    destination: &Path,
    paths: impl IntoIterator<Item = &'a Path>,
) -> Result<(), FsError> {
    let paths: Vec<_> = paths.into_iter().collect();
    if paths.len() > MAX_MOVE_ROOTS {
        return Err(unsupported(
            destination,
            "Move accepts at most 64 selected roots. Choose a smaller selection.",
        ));
    }
    // Reserve repeated native source/parent/stage/proof and receipt path storage,
    // including recovery suffixes. Reject the whole input before creating stages.
    let mut bytes = 0usize;
    for source in paths {
        bytes = bytes
            .saturating_add(
                source
                    .as_os_str()
                    .as_encoded_bytes()
                    .len()
                    .saturating_mul(32),
            )
            .saturating_add(
                destination
                    .as_os_str()
                    .as_encoded_bytes()
                    .len()
                    .saturating_mul(16),
            )
            .saturating_add(8192);
        if bytes > MAX_PROOF_PATH_BYTES {
            return Err(unsupported(
                source,
                "Verified transfer proof paths exceed the 4 MiB budget. Choose content-only Copy or a smaller batch.",
            ));
        }
    }
    Ok(())
}
pub(super) fn prepare_with_metadata(
    request: CopyRequest,
    service: &dyn TransferService,
    cancel: Cancellation,
    status: Status,
    metadata: TransferMetadataPolicy,
) -> Result<Job, FsError> {
    prepare_with_options(
        request,
        service,
        cancel,
        status,
        metadata,
        CopyLinkPolicy::Reject,
    )
}

pub(super) fn prepare_with_options(
    request: CopyRequest,
    service: &dyn TransferService,
    cancel: Cancellation,
    status: Status,
    metadata: TransferMetadataPolicy,
    links: CopyLinkPolicy,
) -> Result<Job, FsError> {
    check_bounds(&request)?;
    let links = move_links(&request.destination, metadata, links)?;
    let permit = permit(&cancel, &request.destination)?;
    let mut prepared_roots = VecDeque::new();
    for path in &request.clipboard.paths {
        let prepared =
            service.prepare_move_source_with_policy(path, links, TRANSFER_REVISIONS, &cancel)?;
        if prepared.source().entry().item.path != *path {
            return Err(unsupported(
                path,
                "Provider returned a different Move source.",
            ));
        }
        prepared_roots.push_back(prepared);
    }
    let mut job = ownership::Armed::new(prepare_sources_inner(
        request.destination,
        prepared_roots,
        service,
        cancel,
        status,
        metadata,
        links,
    )?);
    job.set_verification(request.verification);
    drop(permit);
    Ok(job.into_inner())
}

pub(super) fn prepare_sources(
    destination_path: std::path::PathBuf,
    prepared_roots: VecDeque<PreparedTransferMove>,
    service: &dyn TransferService,
    cancel: Cancellation,
    status: Status,
    metadata: TransferMetadataPolicy,
) -> Result<Job, FsError> {
    prepare_entries(
        destination_path,
        prepared_roots
            .into_iter()
            .map(PreparedTransferMoveEntry::Ordinary)
            .collect(),
        service,
        cancel,
        status,
        metadata,
        CopyLinkPolicy::Reject,
    )
}

pub(super) fn prepare_entries(
    destination_path: std::path::PathBuf,
    prepared_roots: VecDeque<PreparedTransferMoveEntry>,
    service: &dyn TransferService,
    cancel: Cancellation,
    status: Status,
    metadata: TransferMetadataPolicy,
    links: CopyLinkPolicy,
) -> Result<Job, FsError> {
    let links = move_links(&destination_path, metadata, links)?;
    let permit = permit(&cancel, &destination_path)?;
    let job = ownership::Armed::new(prepare_sources_inner(
        destination_path,
        prepared_roots,
        service,
        cancel,
        status,
        metadata,
        links,
    )?);
    drop(permit);
    Ok(job.into_inner())
}

fn move_links(
    destination: &Path,
    metadata: TransferMetadataPolicy,
    links: CopyLinkPolicy,
) -> Result<CopyLinkPolicy, FsError> {
    // Move moves links themselves (Preserve); it never copies what they point to.
    if links == CopyLinkPolicy::FollowLinks {
        return Err(unsupported(
            destination,
            "Following links is available only for Copy.",
        ));
    }
    Ok(input::source_link_policy(links, metadata))
}

/// Whether `kind` may be a Move entry: ordinary files and folders, and links
/// when the job moves links themselves.
fn movable_kind(kind: EntryKind, links: CopyLinkPolicy) -> bool {
    matches!(kind, EntryKind::File | EntryKind::Directory)
        || (links == CopyLinkPolicy::Preserve && link_job::is_link_kind(kind))
}

fn prepare_sources_inner(
    destination_path: std::path::PathBuf,
    prepared_roots: VecDeque<PreparedTransferMoveEntry>,
    service: &dyn TransferService,
    cancel: Cancellation,
    status: Status,
    metadata: TransferMetadataPolicy,
    links: CopyLinkPolicy,
) -> Result<Job, FsError> {
    check_root_bounds(
        &destination_path,
        prepared_roots
            .iter()
            .map(|root| root.source().entry().item.path.as_path()),
    )?;
    let admitted = prepared_roots.len();
    let refresh_parents = prepared_roots
        .iter()
        .filter_map(|root| root.source().entry().item.path.parent().map(Path::to_owned))
        .collect();
    let destination = service.prepare_move_destination(&destination_path, &cancel)?;
    if destination.path() != destination_path {
        return Err(unsupported(
            &destination_path,
            "Provider returned a different Move destination.",
        ));
    }
    let mut roots = VecDeque::new();
    let mut captures = Vec::new();
    let mut link_captures = HashMap::new();
    let mut identities = HashSet::new();
    for prepared in prepared_roots {
        let path = prepared.source().entry().item.path.clone();
        let deferred = prepared.has_deferred_directory_link();
        let (source, original) = match prepared {
            PreparedTransferMoveEntry::Ordinary(prepared) => {
                let entry = prepared.source.entry();
                if !movable_kind(entry.item.kind, links)
                    || (entry.item.kind == EntryKind::File && entry.item.metadata.size.is_none())
                    || prepared.source.native_identity() != Some(entry.identity)
                    || !same_move_entry(prepared.capture.entry(), entry)
                {
                    return Err(unsupported(
                        &path,
                        "Move requires native ordinary files or directories with matching mutation authority. Links, special and virtual sources are unsupported.",
                    ));
                }
                let original = entry.identity;
                prepared.capture.revalidate(&cancel)?;
                captures.push(prepared.capture);
                (prepared.source, original)
            }
            PreparedTransferMoveEntry::DirectoryLink { source, capture } => {
                if !links.materializes_directory_links() {
                    return Err(unsupported(
                        &path,
                        "Directory link Move requires an explicit policy.",
                    ));
                }
                let original = validate_link_source(&*source, &*capture, deferred, &cancel)?;
                link_captures.insert(original, LinkAuthority::new(capture));
                (source, original)
            }
        };
        if !identities.insert(original) {
            return Err(unsupported(
                &path,
                "The same native source appears more than once. All source names were retained.",
            ));
        }
        let entry = source.entry();
        if entry.item.kind == EntryKind::Directory
            && let Some(identity) = source.native_identity()
        {
            destination.reject_source_ancestor(identity, &cancel)?;
        }
        if metadata == TransferMetadataPolicy::RequireModifiedTime
            && entry.item.kind != EntryKind::File
        {
            return Err(unsupported(
                &path,
                "Required modified time accepts native regular files only.",
            ));
        }
        roots.push_back(source);
    }
    let selected_parents = reject_nested_roots(&roots, service, &cancel)?;
    if !selected_parents.is_empty() {
        let selected_parents: Arc<[_]> = selected_parents.into();
        for source in &roots {
            if source.native_identity().is_none()
                && let Some(original) = source.native_original_identity()
                && let Some(link) = link_captures.get_mut(&original)
            {
                link.selected_parents = Some(selected_parents.clone());
            }
        }
    }
    let mut job = ownership::Armed::new(Job::from_sources(destination, roots, cancel, status));
    job.progress.intent = TransferIntent::Move;
    job.links = links;
    job.move_work = Some(MoveWork {
        captures,
        link_captures,
        links,
        metadata_policy: metadata,
        stages: HashMap::new(),
        proofs: vec![],
        published: None,
        phase: Phase::Copying,
        finalizer: None,
        active: None,
        reading: false,
        buffer: vec![],
        admitted,
        failed_phase: None,
        retryable_failure: false,
        refresh_parents,
        pending_confirmations: Vec::new(),
    });
    Ok(job.into_inner())
}

fn same_move_entry(actual: &TransferEntry, expected: &TransferEntry) -> bool {
    actual.identity == expected.identity
        && actual.item.kind == expected.item.kind
        && actual.item.path == expected.item.path
        && actual.item.metadata == expected.item.metadata
}

fn validate_link_source(
    source: &dyn TransferSource,
    capture: &dyn TransferDirectoryLinkMoveCapture,
    deferred: bool,
    cancel: &Cancellation,
) -> Result<TransferIdentity, FsError> {
    let projected = source.entry();
    let original = capture.entry();
    if projected.item.kind != EntryKind::Directory
        || (!deferred && source.native_identity() != Some(projected.identity))
        || original.item.kind != EntryKind::DirectoryLink
        || source.native_original_identity() != Some(original.identity)
        || projected.item.path != original.item.path
        || source.virtual_identity().is_some()
    {
        return Err(unsupported(
            &projected.item.path,
            "Directory link Move requires both the read-only projection and matching original link authority.",
        ));
    }
    source.revalidate(cancel)?;
    capture.revalidate(cancel)?;
    Ok(original.identity)
}

/// Refuse a selected directory that is an ancestor of any selected root's parent.
/// Identity-based ancestry includes aliases/case variants, not just path prefixes.
/// Selected roots usually share one parent: open and check each parent once.
fn reject_nested_roots(
    roots: &VecDeque<Box<dyn TransferSource>>,
    service: &dyn TransferService,
    cancel: &Cancellation,
) -> Result<Vec<Arc<dyn TransferDirectory>>, FsError> {
    if !roots
        .iter()
        .any(|source| source.entry().item.kind == EntryKind::Directory)
    {
        return Ok(Vec::new());
    }
    let retain_parents = roots
        .iter()
        .any(|source| source.native_identity().is_none());
    let mut retained = Vec::new();
    let mut checked = HashSet::new();
    for source in roots {
        let parent =
            source.entry().item.path.parent().ok_or_else(|| {
                unsupported(&source.entry().item.path, "Move source needs a parent.")
            })?;
        if !checked.insert(parent) {
            continue;
        }
        let directory = service.prepare_move_destination(parent, cancel)?;
        if directory.path() != parent {
            return Err(unsupported(
                parent,
                "Provider returned a different Move source parent.",
            ));
        }
        for other in roots {
            if other.entry().item.kind == EntryKind::Directory
                && let Some(identity) = other.native_identity()
            {
                directory.reject_source_ancestor(identity, cancel)?;
            }
        }
        if retain_parents {
            retained.push(directory);
        }
    }
    Ok(retained)
}

pub(super) fn proof_path_bytes(path: &Path) -> usize {
    path.as_os_str()
        .as_encoded_bytes()
        .len()
        .saturating_mul(32)
        .saturating_add(8192)
}

impl MoveWork {
    pub(super) fn entry_removal(&self, source: &dyn TransferSource) -> EntryRemoval {
        source
            .native_original_identity()
            .filter(|identity| self.link_captures.contains_key(identity))
            .map_or(EntryRemoval::Ordinary, EntryRemoval::DirectoryLink)
    }

    pub(super) fn admit_entry(
        &mut self,
        prepared: PreparedTransferMoveEntry,
        parent: &Path,
        cancel: &Cancellation,
    ) -> Result<(Box<dyn TransferSource>, EntryRemoval), FsError> {
        let deferred = prepared.has_deferred_directory_link();
        match prepared {
            PreparedTransferMoveEntry::Ordinary(prepared) => self
                .admit_child(prepared, parent, cancel)
                .map(|source| (source, EntryRemoval::Ordinary)),
            PreparedTransferMoveEntry::DirectoryLink { source, capture } => {
                let path = &source.entry().item.path;
                self.check_child_budget(path, 0)?;
                let original = validate_link_source(&*source, &*capture, deferred, cancel)?;
                if path.parent() != Some(parent)
                    || self.link_captures.contains_key(&original)
                    || self
                        .captures
                        .iter()
                        .any(|capture| capture.entry().identity == original)
                {
                    return Err(unsupported(
                        path,
                        "Directory link Move overlaps an active original or has a different parent.",
                    ));
                }
                self.link_captures
                    .insert(original, LinkAuthority::new(capture));
                self.admitted += 1;
                Ok((source, EntryRemoval::DirectoryLink(original)))
            }
        }
    }

    pub(super) fn admit_projected_child(
        &mut self,
        child: &dyn TransferSource,
        parent: &Path,
        active_projection_bytes: usize,
    ) -> Result<(), FsError> {
        let path = &child.entry().item.path;
        self.check_child_budget(path, active_projection_bytes)?;
        if path.parent() != Some(parent) || path.file_name().is_none() {
            return Err(unsupported(
                path,
                "The projected child has a different parent.",
            ));
        }
        self.admitted += 1;
        Ok(())
    }

    fn check_child_budget(
        &self,
        path: &Path,
        active_projection_bytes: usize,
    ) -> Result<(), FsError> {
        let retained_bytes = self
            .captures
            .iter()
            .map(|capture| capture.entry())
            .chain(self.link_captures.values().map(|link| link.capture.entry()))
            .fold(active_projection_bytes, |total, entry| {
                total.saturating_add(proof_path_bytes(&entry.item.path))
            });
        if self.admitted >= MAX_MOVE_ENTRIES
            || retained_bytes.saturating_add(proof_path_bytes(path)) > MAX_PROOF_PATH_BYTES
        {
            return Err(unsupported(
                path,
                "Recursive Move reached its entry or active-path budget; completed moves remain.",
            ));
        }
        Ok(())
    }

    pub(super) fn capture_for(
        &self,
        entry: &TransferEntry,
    ) -> Result<Arc<dyn TransferMoveCapture>, FsError> {
        self.captures
            .iter()
            .find(|capture| capture.entry().identity == entry.identity)
            .cloned()
            .ok_or_else(|| unsupported(&entry.item.path, "Move source authority missing."))
    }
    pub(super) fn link_capture_for(
        &self,
        original: TransferIdentity,
        path: &Path,
    ) -> Result<&dyn TransferDirectoryLinkMoveCapture, FsError> {
        self.link_captures
            .get(&original)
            .map(|link| link.capture.as_ref())
            .ok_or_else(|| unsupported(path, "Original directory link authority is missing."))
    }

    fn qualify_link_projection(
        &mut self,
        original: TransferIdentity,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<TransferIdentity, FsError> {
        let link = self
            .link_captures
            .get_mut(&original)
            .ok_or_else(|| unsupported(path, "Original directory link authority is missing."))?;
        if link.projection.is_none() {
            link.capture.revalidate(cancel)?;
            link.projection = Some(link.capture.prepare_copy_projection(cancel)?);
        }
        let projection = link
            .projection
            .as_ref()
            .expect("retained directory link projection");
        validate_link_source(projection.as_ref(), link.capture.as_ref(), false, cancel)?;
        let identity = projection.entry().identity;
        if let Some(parents) = &link.selected_parents {
            for parent in parents.iter() {
                parent.reject_source_ancestor(identity, cancel)?;
            }
        }
        Ok(identity)
    }

    pub(super) fn pending_link_identity(
        &mut self,
        source: &dyn TransferSource,
        cancel: &Cancellation,
    ) -> Result<Option<TransferIdentity>, FsError> {
        let EntryRemoval::DirectoryLink(original) = self.entry_removal(source) else {
            return Ok(None);
        };
        self.qualify_link_projection(original, &source.entry().item.path, cancel)
            .map(Some)
    }

    pub(super) fn take_link_projection(
        &mut self,
        original: TransferIdentity,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        self.qualify_link_projection(original, path, cancel)?;
        Ok(self
            .link_captures
            .get_mut(&original)
            .expect("qualified directory link")
            .projection
            .take()
            .expect("retained directory link projection"))
    }
    pub(super) fn record_parents(&self, job: &mut Job, path: &Path) {
        // Reserve the bounded refresh list for selected-root parents first.
        // Deep trees must not crowd out the panels where the move was requested.
        for parent in self
            .refresh_parents
            .iter()
            .map(std::path::PathBuf::as_path)
            .chain(path.parent())
        {
            if job.progress.source_directories.len() < 64
                && !job
                    .progress
                    .source_directories
                    .iter()
                    .any(|old| old == parent)
            {
                job.progress.source_directories.push(parent.to_owned());
            }
        }
    }
    pub(super) fn admit_child(
        &mut self,
        prepared: midnight_core::PreparedTransferMove,
        parent: &Path,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        let entry = prepared.source.entry();
        self.check_child_budget(&entry.item.path, 0)?;
        if entry.item.path.parent() != Some(parent)
            || entry.item.path.file_name().is_none()
            || prepared.source.native_identity() != Some(entry.identity)
            || prepared.capture.entry().identity != entry.identity
            || prepared.capture.entry().item.path != entry.item.path
            || !movable_kind(entry.item.kind, self.links)
            || self
                .captures
                .iter()
                .any(|capture| capture.entry().identity == entry.identity)
            || self.link_captures.contains_key(&entry.identity)
        {
            return Err(unsupported(
                &entry.item.path,
                "Recursive Move child authority is invalid or overlaps an active source.",
            ));
        }
        prepared.source.revalidate(cancel)?;
        prepared.capture.revalidate(cancel)?;
        self.admitted += 1;
        self.captures.push(prepared.capture);
        Ok(prepared.source)
    }
    pub(super) fn discard(&mut self, identity: TransferIdentity) -> Result<(), FsError> {
        self.captures
            .retain(|capture| capture.entry().identity != identity);
        if let Some(mut stage) = self.stages.remove(&identity) {
            stage.cleanup()?;
        }
        Ok(())
    }
    pub(super) fn discard_entry(
        &mut self,
        removal: EntryRemoval,
        identity: TransferIdentity,
    ) -> Result<(), FsError> {
        match removal {
            EntryRemoval::Copy => Ok(()),
            EntryRemoval::Ordinary => self.discard(identity),
            EntryRemoval::DirectoryLink(original) => {
                self.link_captures.remove(&original);
                Ok(())
            }
        }
    }

    pub(super) fn remove_materialized_link(
        &mut self,
        original: TransferIdentity,
        entry: &TransferEntry,
        job: &mut Job,
    ) -> Result<(), FsError> {
        self.confirm_pending(&mut job.progress);
        let capture = self.link_capture_for(original, &entry.item.path)?;
        let _permit = permit(&job.cancel, &entry.item.path)?;
        capture.revalidate(&job.cancel)?;
        let outcome = capture.remove_original(&job.cancel)?;
        self.link_captures.remove(&original);
        let (recovery, failure, kept) = match outcome {
            TransferDirectoryLinkRemoval::Removed {
                state,
                durability_confirmed,
                recovery,
                error,
            } => {
                if state == TransferSourceRemoval::DeletePending {
                    job.progress.source_delete_pending += 1;
                }
                if !durability_confirmed {
                    job.progress.source_removal_durability_unconfirmed += 1;
                }
                self.record_parents(job, &entry.item.path);
                (recovery, error, false)
            }
            TransferDirectoryLinkRemoval::RecoveryRequired { recovery, error } => {
                self.record_parents(job, &entry.item.path);
                (recovery, Some(error), true)
            }
        };
        // What stays in the source tree: the link, or a stage left with recovery data.
        let leaves_source = kept || !recovery.is_empty();
        for item in recovery {
            let matched = item.identity == Some(original);
            record_source(job, entry.item.path.clone(), item, matched);
        }
        if let Some(error) = failure {
            // The link and its recovery data stay where they are; the job asks
            // whether to go on.
            let destination = job.stack.last().map_or_else(
                || job.progress.destination_directory.clone(),
                |frame| frame.destination.path().to_owned(),
            );
            job.pause_committed(failures::Committed {
                error,
                stage: CopyErrorStage::RemovingSource,
                source: Some(entry.item.path.clone()),
                destination,
                entry: None,
                leaves_source,
            });
        }
        Ok(())
    }

    /// Removes the emptied directory `entry`. The caller counts the returned removal
    /// ([`PendingRemoval::count`]) once it has also dropped the directory's traversal
    /// frame, whose source holds the directory's handle.
    pub(super) fn remove_directory(
        &mut self,
        entry: &TransferEntry,
        job: &mut Job,
    ) -> Result<PendingRemoval, FsError> {
        // The renames out of this folder are saved before it goes.
        self.confirm_pending(&mut job.progress);
        let capture = self
            .captures
            .iter()
            .find(|capture| capture.entry().identity == entry.identity)
            .ok_or_else(|| unsupported(&entry.item.path, "Move directory authority missing."))?;
        let _permit = permit(&job.cancel, &entry.item.path)?;
        let outcome = capture.remove_empty_directory(&job.cancel)?;
        let removal = PendingRemoval {
            state: outcome.state,
            durability_confirmed: outcome.durability_confirmed,
            confirmation: capture.take_removal_confirmation(),
        };
        // Directory handles close with this frame before any subsequent traversal.
        self.captures
            .retain(|capture| capture.entry().identity != entry.identity);
        self.record_parents(job, &entry.item.path);
        Ok(removal)
    }
    /// Logical proof reads retained for the current file, not cumulative retry I/O.
    pub(super) fn active_proof_credit(&self, policy: TransferVerification) -> Option<u64> {
        let Some(proof) = self.proofs.first() else {
            return Some(0);
        };
        if self.proofs.len() != 1 {
            return None;
        }
        let pass = policy
            .readback_bytes(proof.source_entry().item.metadata.size?)
            .checked_mul(2)?;
        let current = if self.reading {
            let read = proof.validation_bytes();
            if read > pass {
                return None;
            }
            read
        } else {
            0
        };
        match self.phase {
            Phase::Copying => Some(0),
            Phase::Before => Some(current),
            Phase::Capture => Some(pass),
            Phase::After => pass.checked_add(current),
            Phase::Finalize => pass.checked_mul(2),
            Phase::Halted => None,
        }
    }
    pub(super) fn has_pending(&self) -> bool {
        !self.proofs.is_empty()
    }
    pub(super) fn advance(&mut self, job: &mut Job) -> Result<bool, FsError> {
        let attempted = self.phase;
        let result = self.advance_inner(job);
        if let Err(error) = &result {
            self.failed_phase = Some(attempted);
            self.retryable_failure = !job.cancel.is_cancelled() && retry::retryable(error);
            self.phase = Phase::Halted;
        } else if self.phase != Phase::Halted {
            self.failed_phase = None;
            self.retryable_failure = false;
        }
        result
    }
    fn advance_inner(&mut self, job: &mut Job) -> Result<bool, FsError> {
        if self.proofs.is_empty() {
            return Ok(true);
        }
        if self.proofs.len() != 1 {
            return Err(unsupported(
                &job.progress.destination_directory,
                "Move requires one active verified file at a time.",
            ));
        }
        match self.phase {
            Phase::Copying => {
                self.buffer = vec![0; 256 * 1024];
                self.phase = Phase::Before;
                self.reading = false;
                job.progress.phase = CopyOperationPhase::ValidatingBeforeMove;
                Ok(false)
            }
            Phase::Before | Phase::After => self.validate(job),
            Phase::Capture => self.capture(job),
            Phase::Finalize => self.finalize(job),
            Phase::Halted => Ok(true),
        }
    }
    fn validate(&mut self, job: &mut Job) -> Result<bool, FsError> {
        let before = matches!(self.phase, Phase::Before);
        job.progress.phase = if before {
            CopyOperationPhase::ValidatingBeforeMove
        } else {
            CopyOperationPhase::ValidatingAfterMove
        };
        let proof = &mut self.proofs[0];
        job.progress.current_path = Some(proof.source_entry().item.path.clone());
        if !self.reading {
            proof.begin_validation(&job.cancel)?;
            self.reading = true;
            return Ok(false);
        }
        let bytes = proof.validation_bytes();
        let result = proof.validation_step(&mut self.buffer, &job.cancel);
        job.progress.validation_bytes = job
            .progress
            .validation_bytes
            .checked_add(proof.validation_bytes() - bytes)
            .ok_or_else(|| {
                unsupported(
                    &job.progress.destination_directory,
                    "Move validation byte count overflow.",
                )
            })?;
        if result? == FileProofPhase::Complete {
            self.reading = false;
            self.phase = if before {
                Phase::Capture
            } else {
                Phase::Finalize
            };
        }
        Ok(false)
    }
    fn capture(&mut self, job: &mut Job) -> Result<bool, FsError> {
        let proof = &self.proofs[0];
        proof.revalidate_finalization(&job.cancel)?;
        let expected = proof.source_entry().identity;
        let capture = self
            .captures
            .iter()
            .find(|capture| capture.entry().identity == expected)
            .ok_or_else(|| {
                unsupported(
                    &proof.source_entry().item.path,
                    "Move source authority missing.",
                )
            })?;
        let path = capture.entry().item.path.clone();
        job.progress.current_path = Some(path.clone());
        job.progress.phase = CopyOperationPhase::CapturingSources;
        let outcome = {
            let _permit = permit(&job.cancel, &path)?;
            capture.revalidate(&job.cancel)?;
            capture.capture(&job.cancel)?
        };
        self.record_parents(job, &path);
        match outcome {
            TransferMoveCaptureOutcome::Captured { source, recovery } => {
                let matched = source == path && recovery.identity == Some(expected);
                self.active = Some((source, recovery));
                if !matched {
                    return Err(unsupported(
                        &path,
                        "Source capture returned an unexpected identity. Captured data was retained; later sources remain untouched.",
                    ));
                }
                job.progress.source_names_staged += 1;
                let _permit = permit(&job.cancel, &path)?;
                self.finalizer = Some(capture.prepare_finalization(&job.cancel)?);
                self.phase = Phase::After;
            }
            TransferMoveCaptureOutcome::Restored { error, .. } => return Err(error),
            TransferMoveCaptureOutcome::RecoveryRequired {
                source,
                recovery,
                error,
            } => {
                for retained in recovery {
                    self.active = Some((source.clone(), retained));
                }
                return Err(error);
            }
        }
        Ok(false)
    }
    fn finalize(&mut self, job: &mut Job) -> Result<bool, FsError> {
        self.proofs[0].revalidate_finalization(&job.cancel)?;
        let finalizer = self.finalizer.as_mut().ok_or_else(|| {
            unsupported(
                &job.progress.destination_directory,
                "Move finalization authority missing.",
            )
        })?;
        let outcome = {
            let _permit = permit(&job.cancel, &job.progress.destination_directory)?;
            finalizer.revalidate(&job.cancel)?;
            finalizer.finalize(&job.cancel)
        };
        match outcome {
            TransferMoveFinalizeOutcome::Retained {
                source,
                recovery,
                error,
            } => {
                if !self.active.as_ref().is_some_and(|(path, expected)| {
                    *path == source
                        && expected.path == recovery.path
                        && expected.identity == recovery.identity
                }) {
                    return Err(unsupported(
                        &source,
                        "Move finalizer returned an unexpected retained source receipt.",
                    ));
                }
                return Err(error);
            }
            TransferMoveFinalizeOutcome::Removed {
                source,
                recovery,
                state,
                durability_confirmed,
            } => {
                job.progress.source_files_removed += 1;
                job.work_budget.removed(
                    self.proofs[0].source_entry().item.metadata.size,
                    job.verification,
                );
                let identity = self.proofs[0].source_entry().identity;
                let destination = self
                    .published
                    .take()
                    .unwrap_or_else(|| job.progress.destination_directory.clone());
                let cleanup = self
                    .captures
                    .iter()
                    .find(|capture| capture.entry().identity == identity)
                    .and_then(|capture| capture.take_stage_cleanup());
                let removal = PendingRemoval {
                    state,
                    durability_confirmed,
                    confirmation: self
                        .finalizer
                        .as_mut()
                        .and_then(|finalizer| finalizer.take_removal_confirmation()),
                };
                // No source queries after disposition. Close EVERY source/DELETE
                // owner before removing the empty transient directory or copying
                // the next file. Destination proof owners also end here.
                self.proofs.clear();
                self.finalizer = None;
                self.captures
                    .retain(|capture| capture.entry().identity != identity);
                self.buffer = Vec::new();
                self.phase = Phase::Copying;
                self.active = None;
                // Every handle on the source is closed: a removal Windows completes only
                // now is confirmed (and counted) before its stage is removed.
                let (state, durability_confirmed) = removal.count(&mut job.progress);
                // The source is gone and its copy verified: what fails now leaves only
                // an empty private folder, or the removal's durability, uncertain. The
                // job asks whether to go on (a provider that omits the cleanup still
                // ends it).
                let uncertain = if let Some(cleanup) = cleanup {
                    let path = cleanup.path().to_owned();
                    match cleanup.cleanup() {
                        Err(error) => {
                            record_source(
                                job,
                                source.clone(),
                                TransferRecovery {
                                    path,
                                    identity: None,
                                },
                                false,
                            );
                            Some((error, CopyErrorStage::Cleanup))
                        }
                        Ok(())
                            if state == TransferSourceRemoval::Unlinked
                                && !durability_confirmed =>
                        {
                            let error = unsupported(
                                &source,
                                "Verified source was removed, but temporary-directory sync failed; removal durability is unconfirmed.",
                            );
                            Some((error, CopyErrorStage::RemovingSource))
                        }
                        Ok(()) => None,
                    }
                } else {
                    record_source(job, source.clone(), recovery, false);
                    return Err(unsupported(
                        &source,
                        "Move provider omitted temporary-directory cleanup authority after source removal.",
                    ));
                };
                if let Some((error, stage)) = uncertain {
                    job.pause_committed(failures::Committed {
                        error,
                        stage,
                        source: Some(source),
                        destination,
                        entry: None,
                        // A stage that could not be removed stays in the source's folder; an
                        // unconfirmed durability leaves nothing there.
                        leaves_source: stage == CopyErrorStage::Cleanup,
                    });
                }
            }
        }
        Ok(false)
    }
    pub(super) fn cleanup(&mut self, job: &mut Job) -> Vec<FsError> {
        self.confirm_pending(&mut job.progress);
        self.failed_phase = None;
        self.retryable_failure = false;
        let mut errors: Vec<_> = self
            .stages
            .values_mut()
            .filter_map(|stage| stage.cleanup().err())
            .collect();
        // Armed drains never destroy their remaining native elements while an
        // earlier cleanup or destructor is unwinding toward the Copy boundary.
        let mut stages = ownership::Armed::new(self.stages.drain());
        for (_, stage) in stages.by_ref() {
            drop(stage);
        }
        drop(stages.into_inner());
        let mut proofs = ownership::Armed::new(self.proofs.drain(..));
        for proof in proofs.by_ref() {
            drop(proof);
        }
        drop(proofs.into_inner());
        drop(self.finalizer.take());
        let mut links = ownership::Armed::new(self.link_captures.drain());
        for (_, capture) in links.by_ref() {
            drop(capture);
        }
        drop(links.into_inner());
        let mut tickets = ownership::Armed::new(VecDeque::new());
        let mut captures = ownership::Armed::new(self.captures.drain(..));
        for capture in captures.by_ref() {
            let capture = ownership::Armed::new(capture);
            let source = capture.entry().item.path.clone();
            let restored = match capture.restore() {
                Ok(()) => true,
                Err(error) => {
                    errors.push(error);
                    capture.restoration_completed()
                }
            };
            if restored {
                if let Some(cleanup) = capture.take_stage_cleanup() {
                    tickets.push_back((source, cleanup));
                }
            } else if let Some((path, retained)) = &self.active
                && *path == source
            {
                record_source(
                    job,
                    source,
                    retained.clone(),
                    retained.identity == Some(capture.entry().identity),
                );
            }
            // Drops the capture's exact source handle before cleanup tickets run.
            drop(capture.into_inner());
        }
        drop(captures.into_inner());
        while let Some((source, cleanup)) = tickets.pop_front() {
            let cleanup = ownership::Armed::new(cleanup);
            let path = cleanup.path().to_owned();
            if let Err(error) = cleanup.into_inner().cleanup() {
                record_source(
                    job,
                    source,
                    TransferRecovery {
                        path,
                        identity: None,
                    },
                    false,
                );
                errors.push(error);
            }
        }
        drop(tickets.into_inner());
        self.active = None;
        errors
    }
}
/// A source removal whose disposition and durability are counted only once the job has
/// dropped every handle it holds on the removed entry: a removal by delete disposition
/// (Windows) leaves its name until then, so its confirmation can only run after that.
pub(super) struct PendingRemoval {
    state: TransferSourceRemoval,
    durability_confirmed: bool,
    confirmation: Option<Box<dyn TransferDurabilityConfirmation>>,
}
impl PendingRemoval {
    /// Runs the provider's confirmation of a delete-pending removal, if it gave one, and
    /// counts the removal as it then stands; returns its final state and durability.
    /// Call only after the removed entry's source, capture and finalizer are dropped.
    pub(super) fn count(self, progress: &mut CopyProgress) -> (TransferSourceRemoval, bool) {
        let confirmed = self.state == TransferSourceRemoval::DeletePending
            && self.confirmation.is_some_and(|confirmation| {
                confirmation.confirm(&mut TransferDurabilityRound::default())
            });
        let (state, durability_confirmed) = if confirmed {
            (TransferSourceRemoval::Unlinked, true)
        } else {
            (self.state, self.durability_confirmed)
        };
        if state == TransferSourceRemoval::DeletePending {
            progress.source_delete_pending += 1;
        }
        if !durability_confirmed {
            progress.source_removal_durability_unconfirmed += 1;
        }
        (state, durability_confirmed)
    }
}

/// At most this many deferred native-rename confirmations wait for their round: each
/// holds the handles of the folders its rename changed.
const MAX_PENDING_CONFIRMATIONS: usize = 64;

impl MoveWork {
    /// Keeps the confirmation of a committed native rename for the next round
    /// ([`Self::confirm_pending`]), which runs at once when the limit is reached.
    pub(super) fn defer_confirmation(
        &mut self,
        confirmation: Box<dyn TransferDurabilityConfirmation>,
        progress: &mut CopyProgress,
    ) {
        self.pending_confirmations.push(confirmation);
        if self.pending_confirmations.len() >= MAX_PENDING_CONFIRMATIONS {
            self.confirm_pending(progress);
        }
    }
    /// Runs every deferred rename confirmation in one round, so that each folder they
    /// changed is saved once for all of them, and counts each rename it cannot confirm.
    /// Runs before the job removes a folder (a deferred confirmation can hold its handle,
    /// and saving it once removed fails) and when the job ends, whatever its outcome:
    /// the renames are committed.
    pub(super) fn confirm_pending(&mut self, progress: &mut CopyProgress) {
        let mut round = TransferDurabilityRound::default();
        for confirmation in std::mem::take(&mut self.pending_confirmations) {
            if !confirmation.confirm(&mut round) {
                progress.source_removal_durability_unconfirmed += 1;
            }
        }
    }
}

fn record_source(
    job: &mut Job,
    source: std::path::PathBuf,
    recovery: TransferRecovery,
    matched_source: bool,
) {
    job.progress.source_residual_count += 1;
    if job.progress.source_residuals.len() < 16 {
        job.progress.source_residuals.push(MoveSourceResidual {
            source,
            temporary: recovery,
            matched_source,
        });
    }
}
pub(super) fn permit(
    cancel: &Cancellation,
    path: &Path,
) -> Result<midnight_platform_common::transfer_io::CallPermit, FsError> {
    midnight_platform_common::transfer_io::acquire_transfer_exclusive_call(cancel, path).map_err(
        |_| {
            error(
                path,
                io::ErrorKind::Interrupted,
                "Move cancelled; unfinished sources will be restored without replacement.",
            )
        },
    )
}
fn unsupported(path: &Path, message: &str) -> FsError {
    error(path, io::ErrorKind::Unsupported, message)
}
