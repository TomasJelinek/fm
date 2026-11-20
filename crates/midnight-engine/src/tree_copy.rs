//! Retained tree roots and occurrence feedback; all provider work runs on workers.
use crate::{
    copy_nested_retirement::{self as nested, Retained},
    providers::{ProviderError, ProviderTarget},
    tree_actor::{CapturedSelection, Session},
};
use midnight_core::*;
use std::{
    collections::{HashSet, VecDeque},
    ffi::OsStr,
    io,
    path::Path,
    sync::Arc,
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

struct Lease {
    capture: CapturedSelection,
    _permit: OwnedSemaphorePermit,
}
impl Lease {
    fn release_on_worker(self: Arc<Self>) -> Result<(), ProviderError> {
        nested::provider_release(
            self,
            |lease| {
                let Some(owner) = Arc::into_inner(lease) else {
                    return Ok(());
                };
                let mut permit = Retained::new(owner._permit);
                owner.capture.release_on_worker()?;
                drop(permit.take());
                Ok(())
            },
            Self::give_up,
        )
    }
    /// Decrement-only: the last reference gives its capture up and keeps the capture
    /// permit retained, which returns only after the targets and their registrations
    /// are destroyed.
    fn give_up(self: Arc<Self>) {
        if let Some(owner) = Arc::into_inner(self) {
            let _permit = Retained::new(owner._permit);
            owner.capture.give_up();
        }
    }
}

#[derive(Clone)]
struct LeaseReference(Option<Arc<Lease>>);
impl std::ops::Deref for LeaseReference {
    type Target = Arc<Lease>;
    fn deref(&self) -> &Self::Target {
        self.0.as_ref().expect("tree capture remains owned")
    }
}
impl LeaseReference {
    fn give_up(mut self) {
        if let Some(lease) = self.0.take() {
            lease.give_up();
        }
    }
}
impl Drop for LeaseReference {
    fn drop(&mut self) {
        if let Some(context) = crate::copy_nested_retirement::current()
            && let Some(lease) = self.0.take()
            && let Err(error) = lease.release_on_worker()
        {
            context.record(crate::copy_nested_retirement::fs_error(error));
        }
    }
}
// Used only for the four native authority fields below, created by the service
// supplied to this known capture. The independently audited service witness is
// checked by Preparation before calling prepare; this wrapper mints no witness.
struct NativeAuthority<T>(Option<T>);
impl<T> std::ops::Deref for NativeAuthority<T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.0.as_ref().expect("native authority remains owned")
    }
}
impl<T> std::ops::DerefMut for NativeAuthority<T> {
    fn deref_mut(&mut self) -> &mut T {
        self.0.as_mut().expect("native authority remains owned")
    }
}
impl<T> Drop for NativeAuthority<T> {
    fn drop(&mut self) {
        if crate::copy_nested_retirement::current().is_some()
            && let Some(owner) = self.0.take()
        {
            crate::copy_nested_retirement::checked_step(|| drop(owner));
        }
    }
}
struct Sources {
    lease: LeaseReference,
    metadata: bool,
    intent: TransferIntent,
}
impl std::fmt::Debug for Sources {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TreeTransferSources")
            .field("roots", &self.lease.capture.targets.len())
            .finish_non_exhaustive()
    }
}
impl Session {
    pub(crate) fn capture_transfer(
        &mut self,
        request: &TreeTransferRequest,
        capacity: &Arc<Semaphore>,
        cancel: &Cancellation,
    ) -> Result<TreeTransferCapture, ProviderError> {
        if request.request.0 == 0
            || request.request.0 <= self.last_request.0
            || request.cancel.is_cancelled()
            || cancel.is_cancelled()
        {
            return Err(ProviderError::StaleReference);
        }
        let permit = capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| ProviderError::ResourceLimited)?;
        let capture = self.capture_selection(
            &request.view,
            request.selection_revision,
            &request.selection,
            request.fallback,
            &request.cancel,
        )?;
        if cancel.is_cancelled() {
            return Err(ProviderError::Cancelled);
        }
        if request.intent == TransferIntent::Move {
            for target in &capture.targets {
                if !matches!(target.captured.target.value()?, ProviderTarget::Native(_)) {
                    return Err(ProviderError::Unsupported);
                }
            }
        }
        let metadata = capture.targets.iter().all(|target| {
            matches!(
                target.captured.target.value(),
                Ok(ProviderTarget::Native(_))
            ) && capture
                .view
                .index
                .row_positions
                .get(&target.row)
                .is_some_and(|position| capture.view.rows[*position].entry.kind == EntryKind::File)
        }) && capture.targets.len() <= 64;
        self.last_request = request.request;
        let root_count = capture.targets.len();
        Ok(TreeTransferCapture {
            intent: request.intent,
            root_count,
            metadata_available: metadata,
            sources: Arc::new(Sources {
                lease: LeaseReference(Some(Arc::new(Lease {
                    capture,
                    _permit: permit,
                }))),
                metadata,
                intent: request.intent,
            }),
        })
    }
}
impl CapturedTransferSources for Sources {
    fn prepares_audited_native_children(&self) -> bool {
        true
    }
    fn release_on_worker(self: Arc<Self>) -> Result<(), FsError> {
        nested::provider_release(
            self,
            |sources| {
                if let Some(owner) = Arc::into_inner(sources) {
                    drop(owner);
                }
                Ok(())
            },
            Sources::give_up,
        )
        .map_err(nested::fs_error)
    }
    fn release_quarantined(self: Arc<Self>) {
        self.give_up();
    }
    fn intent(&self) -> TransferIntent {
        self.intent
    }
    fn root_count(&self) -> usize {
        self.lease.capture.targets.len()
    }
    fn metadata_available(&self) -> bool {
        self.metadata
    }
    fn prepare(
        &self,
        service: &dyn TransferService,
        cancel: &Cancellation,
    ) -> Result<PreparedCapturedTransfer, FsError> {
        self.prepare_with_links(service, CopyLinkPolicy::Reject, cancel)
    }
    fn prepare_with_links(
        &self,
        service: &dyn TransferService,
        links: CopyLinkPolicy,
        cancel: &Cancellation,
    ) -> Result<PreparedCapturedTransfer, FsError> {
        if links == CopyLinkPolicy::FollowLinks && self.intent != TransferIntent::Copy {
            return Err(FsError {
                kind: io::ErrorKind::Unsupported,
                ..failure("Following links is available only for Copy.")
            });
        }
        if self.intent == TransferIntent::Move {
            // Any explicit policy carries per-entry link authority; Preserve moves
            // each link itself.
            if links != CopyLinkPolicy::Reject {
                return self
                    .prepare_move_entries(service, links, cancel)
                    .map(PreparedCapturedTransfer::MoveWithDirectoryLinks);
            }
            return self
                .prepare_move(service, cancel)
                .map(PreparedCapturedTransfer::Move);
        }
        let mut virtuals = HashSet::new();
        let mut archive_roots = Vec::new();
        // Provider identities and trails, never captions or paths, reject archive overlaps.
        for target in &self.lease.capture.targets {
            if cancel.is_cancelled() {
                return Err(failure("Tree Copy cancelled."));
            }
            if let ProviderTarget::Archive(target) = target
                .captured
                .target
                .value()
                .map_err(|error| failure(&error.to_string()))?
            {
                if links.recreates_links() {
                    return Err(FsError {
                        kind: io::ErrorKind::Unsupported,
                        ..failure("Archive sources do not support link preservation.")
                    });
                }
                let node = target
                    .original
                    .as_ref()
                    .or(target.member.as_ref())
                    .ok_or_else(|| failure("Archive root has no original record."))?;
                if !virtuals.insert(node.id) {
                    return Err(failure("Duplicate archive Copy root."));
                }
                archive_roots.push((
                    node.id,
                    node.kind,
                    target
                        .location
                        .trail
                        .iter()
                        .map(|(id, _)| *id)
                        .collect::<Vec<_>>(),
                ));
            }
        }
        if overlapping_archive_roots(&archive_roots) {
            return Err(failure("Overlapping archive Copy roots."));
        }
        let traversal = crate::copy_operations::archive_source::traversal();
        let mut roots = VecDeque::new();
        for target in &self.lease.capture.targets {
            let source = match target
                .captured
                .target
                .value()
                .map_err(|error| failure(&error.to_string()))?
            {
                ProviderTarget::Native(path) => {
                    // The captured occurrence is the selected entry, including a
                    // link itself; a browsed referent is separate authority.
                    let source = service.prepare_transfer_source_with_policy(
                        path,
                        links,
                        crate::copy_operations::TRANSFER_REVISIONS,
                        cancel,
                    )?;
                    if source.entry().item.path != *path || source.native_identity().is_none() {
                        return Err(failure("Provider returned a different native source."));
                    }
                    Authority::Native(NativeAuthority(Some(source)))
                }
                ProviderTarget::Archive(target) => {
                    Authority::Archive(crate::copy_operations::archive_source::captured_root(
                        target,
                        traversal.clone(),
                        cancel,
                    )?)
                }
            };
            roots.push_back(Box::new(LeasedSource {
                source,
                lease: self.lease.clone(),
                occurrence: Some(TreeCopyOccurrence {
                    stamp: self.lease.capture.view.stamp,
                    row: target.row,
                }),
            }) as Box<dyn TransferSource>);
        }
        Ok(PreparedCapturedTransfer::Copy(roots))
    }
}
impl Sources {
    /// Decrement-only: the last reference gives its capture lease up.
    fn give_up(self: Arc<Self>) {
        if let Some(owner) = Arc::into_inner(self) {
            owner.lease.give_up();
        }
    }
    fn prepare_move(
        &self,
        service: &dyn TransferService,
        cancel: &Cancellation,
    ) -> Result<VecDeque<PreparedTransferMove>, FsError> {
        self.prepare_move_entries(service, CopyLinkPolicy::Reject, cancel)?
            .into_iter()
            .map(|prepared| match prepared {
                PreparedTransferMoveEntry::Ordinary(prepared) => Ok(prepared),
                PreparedTransferMoveEntry::DirectoryLink { .. } => Err(failure(
                    "Legacy Tree Move cannot carry directory link authority.",
                )),
            })
            .collect()
    }

    fn prepare_move_entries(
        &self,
        service: &dyn TransferService,
        links: CopyLinkPolicy,
        cancel: &Cancellation,
    ) -> Result<VecDeque<PreparedTransferMoveEntry>, FsError> {
        // Preflight every retained provider before any native source is opened.
        let mut paths = Vec::new();
        for target in &self.lease.capture.targets {
            if cancel.is_cancelled() {
                return Err(failure("Tree Move cancelled."));
            }
            match target
                .captured
                .target
                .value()
                .map_err(|e| failure(&e.to_string()))?
            {
                ProviderTarget::Native(path) => paths.push(path.as_path()),
                ProviderTarget::Archive(_) => {
                    return Err(FsError {
                        kind: io::ErrorKind::Unsupported,
                        ..failure("Tree Move requires native sources.")
                    });
                }
            }
        }
        crate::copy_operations::check_move_bounds(Path::new(""), paths.iter().copied())?;
        let mut roots = VecDeque::new();
        for (target, path) in self.lease.capture.targets.iter().zip(paths) {
            let prepared = if links == CopyLinkPolicy::Reject {
                PreparedTransferMoveEntry::Ordinary(
                    crate::copy_operations::prepare_ordinary_move_source(service, path, cancel)?,
                )
            } else {
                service.prepare_move_source_with_policy(
                    path,
                    links,
                    crate::copy_operations::TRANSFER_REVISIONS,
                    cancel,
                )?
            };
            let source = prepared.source();
            let capture_entry = match &prepared {
                PreparedTransferMoveEntry::Ordinary(prepared) => prepared.capture.entry(),
                PreparedTransferMoveEntry::DirectoryLink { capture, .. } => capture.entry(),
            };
            if source.entry().item.path != path
                || (source.native_identity() != Some(source.entry().identity)
                    && !prepared.has_deferred_directory_link())
                || capture_entry.item.path != path
            {
                return Err(failure("Provider returned a different native Move source."));
            }
            roots.push_back(lease_move_entry(prepared, &self.lease, |source| {
                Box::new(LeasedSource {
                    source: Authority::Native(NativeAuthority(Some(source))),
                    lease: self.lease.clone(),
                    occurrence: Some(TreeCopyOccurrence {
                        stamp: self.lease.capture.view.stamp,
                        row: target.row,
                    }),
                })
            }));
        }
        Ok(roots)
    }
}
fn failure(message: &str) -> FsError {
    FsError {
        operation: "tree transfer",
        path: Path::new("").into(),
        kind: io::ErrorKind::InvalidData,
        message: message.into(),
    }
}

/// A root whose captured provider trail passes through another selected directory
/// root lies inside it. One set lookup per trail step: O(roots x depth).
fn overlapping_archive_roots(roots: &[(VirtualEntryId, EntryKind, Vec<u64>)]) -> bool {
    let directories = roots
        .iter()
        .filter(|(_, kind, _)| *kind == EntryKind::Directory)
        .map(|(id, _, _)| *id)
        .collect::<HashSet<_>>();
    roots.iter().any(|(id, _, trail)| {
        trail.iter().any(|entry| {
            *entry != id.entry
                && directories.contains(&VirtualEntryId {
                    mount: id.mount,
                    entry: *entry,
                })
        })
    })
}

#[cfg(test)]
#[test]
fn archive_root_overlap_uses_provider_trails_within_one_mount() {
    // Trails run from the archive root directory to the occurrence itself.
    let root =
        |mount, entry, kind, trail: &[u64]| (VirtualEntryId { mount, entry }, kind, trail.to_vec());
    let (directory, file) = (EntryKind::Directory, EntryKind::File);
    assert!(!overlapping_archive_roots(&[root(
        1,
        2,
        directory,
        &[1, 2]
    )]));
    assert!(!overlapping_archive_roots(&[
        root(1, 2, directory, &[1, 2]),
        root(1, 3, directory, &[1, 3]),
    ]));
    assert!(overlapping_archive_roots(&[
        root(1, 2, directory, &[1, 2]),
        root(1, 3, directory, &[1, 3]),
        root(1, 4, file, &[1, 3, 4]),
    ]));
    assert!(
        overlapping_archive_roots(&[
            root(1, 9, directory, &[1, 2, 7, 9]),
            root(1, 2, directory, &[1, 2]),
        ]),
        "a deep descendant is found whatever the selection order"
    );
    assert!(
        !overlapping_archive_roots(
            &[root(1, 2, directory, &[1, 2]), root(9, 5, file, &[1, 2, 5]),]
        ),
        "equal entry numbers in another mount are unrelated"
    );
    assert!(
        !overlapping_archive_roots(&[root(1, 2, file, &[1, 2]), root(1, 5, file, &[1, 2, 5])]),
        "only a directory root can contain another root"
    );
}

/// What a prepared root or child owns. Children keep their root's kind.
enum Authority {
    /// Native authority created by the audited service.
    Native(NativeAuthority<Box<dyn TransferSource>>),
    /// An archive member source (`archive_source::captured_root` and its children):
    /// plain memory plus archive references, whose drop releases them through
    /// `release_archive`, which gives them up inside a quarantined Copy. It therefore
    /// drops directly, also there, instead of being forgotten with those references.
    Archive(Box<dyn TransferSource>),
}
impl Authority {
    fn child(&self, source: Box<dyn TransferSource>) -> Self {
        match self {
            Self::Native(_) => Self::Native(NativeAuthority(Some(source))),
            Self::Archive(_) => Self::Archive(source),
        }
    }
}
impl std::ops::Deref for Authority {
    type Target = dyn TransferSource;
    fn deref(&self) -> &Self::Target {
        match self {
            Self::Native(source) => &***source,
            Self::Archive(source) => &**source,
        }
    }
}
impl std::ops::DerefMut for Authority {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match self {
            Self::Native(source) => &mut ***source,
            Self::Archive(source) => &mut **source,
        }
    }
}

struct LeasedSource {
    source: Authority,
    lease: LeaseReference,
    occurrence: Option<TreeCopyOccurrence>,
}
impl LeasedSource {
    fn child(&self, source: Box<dyn TransferSource>) -> Box<dyn TransferSource> {
        Box::new(Self {
            source: self.source.child(source),
            lease: self.lease.clone(),
            occurrence: None,
        })
    }
}
impl TransferSource for LeasedSource {
    fn native_namespace_scope(
        &self,
        cancel: &Cancellation,
    ) -> Result<Option<Arc<dyn TransferNamespacePin>>, FsError> {
        self.source.native_namespace_scope(cancel)
    }
    fn discover(
        &self,
        cancel: &Cancellation,
        visit: &mut dyn FnMut(&MutationItem),
    ) -> Result<(), FsError> {
        self.source.discover(cancel, visit)
    }
    fn entry(&self) -> &TransferEntry {
        self.source.entry()
    }
    fn tree_copy_occurrence(&self) -> Option<TreeCopyOccurrence> {
        self.occurrence
    }
    fn native_identity(&self) -> Option<TransferIdentity> {
        self.source.native_identity()
    }
    fn native_original_identity(&self) -> Option<TransferIdentity> {
        self.source.native_original_identity()
    }
    fn virtual_identity(&self) -> Option<VirtualEntryId> {
        self.source.virtual_identity()
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.source.revalidate(cancel)
    }
    fn link_representation(
        &self,
        cancel: &Cancellation,
    ) -> Result<TransferLinkRepresentation, FsError> {
        self.source.link_representation(cancel)
    }
    fn link_not_followed(&self) -> Option<String> {
        self.source.link_not_followed()
    }
    fn open_child(
        &self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        self.source
            .open_child(name, cancel)
            .map(|source| self.child(source))
    }
    fn next_move_child(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Option<PreparedTransferMove>, FsError> {
        self.source.next_move_child(cancel).map(|child| {
            child.map(|mut child| {
                child.source = self.child(child.source);
                child.capture = leased_capture(child.capture, &self.lease);
                child
            })
        })
    }
    fn next_move_child_with_links(
        &mut self,
        links: CopyLinkPolicy,
        cancel: &Cancellation,
    ) -> Result<Option<PreparedTransferMoveEntry>, FsError> {
        if links == CopyLinkPolicy::Reject {
            return self
                .next_move_child(cancel)
                .map(|child| child.map(PreparedTransferMoveEntry::Ordinary));
        }
        self.source
            .next_move_child_with_links(links, cancel)
            .map(|child| {
                child.map(|entry| lease_move_entry(entry, &self.lease, |source| self.child(source)))
            })
    }
    fn prepare_modified_time(
        &self,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferModifiedTimePin>, FsError> {
        self.source.prepare_modified_time(cancel)
    }
    fn rewind_for_validation(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferReadGuard>, FsError> {
        self.source.rewind_for_validation(cancel)
    }
    fn seek_for_validation(&mut self, offset: u64, cancel: &Cancellation) -> Result<u64, FsError> {
        self.source.seek_for_validation(offset, cancel)
    }
    fn read_chunk(&mut self, buffer: &mut [u8], cancel: &Cancellation) -> Result<usize, FsError> {
        self.source.read_chunk(buffer, cancel)
    }
    fn supports_read_retry(&self) -> bool {
        self.source.supports_read_retry()
    }
    fn retry_read_from(&mut self, offset: u64, cancel: &Cancellation) -> Result<(), FsError> {
        self.source.retry_read_from(offset, cancel)
    }
    fn next_child(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Option<Box<dyn TransferSource>>, FsError> {
        self.source
            .next_child(cancel)
            .map(|child| child.map(|source| self.child(source)))
    }
    fn next_child_or_failure(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Option<TransferChild>, FsError> {
        self.source
            .next_child_or_failure(cancel)
            .map(|child| child.map(|child| child.map(|source| self.child(source))))
    }
}

fn lease_move_entry(
    prepared: PreparedTransferMoveEntry,
    lease: &LeaseReference,
    wrap_source: impl FnOnce(Box<dyn TransferSource>) -> Box<dyn TransferSource>,
) -> PreparedTransferMoveEntry {
    match prepared {
        PreparedTransferMoveEntry::Ordinary(prepared) => {
            PreparedTransferMoveEntry::Ordinary(PreparedTransferMove {
                source: wrap_source(prepared.source),
                capture: leased_capture(prepared.capture, lease),
            })
        }
        PreparedTransferMoveEntry::DirectoryLink { source, capture } => {
            PreparedTransferMoveEntry::DirectoryLink {
                source: wrap_source(source),
                capture: Arc::new(LeasedDirectoryLinkCapture {
                    inner: NativeAuthority(Some(capture)),
                    _lease: lease.clone(),
                }),
            }
        }
    }
}

// Close native authority before its retained provider registration.
struct LeasedDirectoryLinkCapture {
    inner: NativeAuthority<Arc<dyn TransferDirectoryLinkMoveCapture>>,
    _lease: LeaseReference,
}

impl TransferDirectoryLinkMoveCapture for LeasedDirectoryLinkCapture {
    fn prepare_copy_projection(
        &self,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        let source = self.inner.prepare_copy_projection(cancel)?;
        Ok(Box::new(LeasedSource {
            source: Authority::Native(NativeAuthority(Some(source))),
            lease: self._lease.clone(),
            occurrence: None,
        }))
    }

    fn try_native_move(
        &self,
        destination: &dyn TransferDirectory,
        name: &OsStr,
        replace: Option<(Arc<dyn TransferTarget>, TransferReplacementPolicy)>,
        cancel: &Cancellation,
    ) -> Result<TransferNativeMoveOutcome, FsError> {
        self.inner
            .try_native_move(destination, name, replace, cancel)
    }

    fn entry(&self) -> &TransferEntry {
        self.inner.entry()
    }

    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.inner.revalidate(cancel)
    }

    fn remove_original(
        &self,
        cancel: &Cancellation,
    ) -> Result<TransferDirectoryLinkRemoval, FsError> {
        self.inner.remove_original(cancel)
    }
}

fn leased_capture(
    inner: Arc<dyn TransferMoveCapture>,
    lease: &LeaseReference,
) -> Arc<dyn TransferMoveCapture> {
    Arc::new(LeasedMoveCapture {
        inner: NativeAuthority(Some(inner)),
        lease: lease.clone(),
    })
}
// Authority fields precede the lease: providers remain live through authority destruction.
struct LeasedMoveCapture {
    inner: NativeAuthority<Arc<dyn TransferMoveCapture>>,
    lease: LeaseReference,
}
impl TransferMoveCapture for LeasedMoveCapture {
    fn try_native_move(
        &self,
        destination: &dyn TransferDirectory,
        name: &OsStr,
        replace: Option<(Arc<dyn TransferTarget>, TransferReplacementPolicy)>,
        cancel: &Cancellation,
    ) -> Result<TransferNativeMoveOutcome, FsError> {
        self.inner
            .try_native_move(destination, name, replace, cancel)
    }
    fn entry(&self) -> &TransferEntry {
        self.inner.entry()
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.inner.revalidate(cancel)
    }
    fn preflight_finalization(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.inner.preflight_finalization(cancel)
    }
    fn remove_empty_directory(
        &self,
        cancel: &Cancellation,
    ) -> Result<TransferDirectoryRemoval, FsError> {
        self.inner.remove_empty_directory(cancel)
    }
    fn prepare_finalization(
        &self,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferMoveFinalizer>, FsError> {
        Ok(Box::new(LeasedFinalizer {
            inner: NativeAuthority(Some(self.inner.prepare_finalization(cancel)?)),
            _lease: self.lease.clone(),
        }))
    }
    fn capture(&self, cancel: &Cancellation) -> Result<TransferMoveCaptureOutcome, FsError> {
        self.inner.capture(cancel)
    }
    fn restore(&self) -> Result<(), FsError> {
        self.inner.restore()
    }
    fn restoration_completed(&self) -> bool {
        self.inner.restoration_completed()
    }
    fn take_stage_cleanup(&self) -> Option<Box<dyn TransferMoveStageCleanup>> {
        self.inner.take_stage_cleanup().map(|inner| {
            Box::new(LeasedCleanup {
                inner: NativeAuthority(Some(inner)),
                _lease: self.lease.clone(),
            }) as Box<dyn TransferMoveStageCleanup>
        })
    }
}
struct LeasedFinalizer {
    inner: NativeAuthority<Box<dyn TransferMoveFinalizer>>,
    _lease: LeaseReference,
}
impl TransferMoveFinalizer for LeasedFinalizer {
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.inner.revalidate(cancel)
    }
    fn finalize(&mut self, cancel: &Cancellation) -> TransferMoveFinalizeOutcome {
        self.inner.finalize(cancel)
    }
}
// Own no source, capture or finalizer: native DELETE owners must close before cleanup.
struct LeasedCleanup {
    inner: NativeAuthority<Box<dyn TransferMoveStageCleanup>>,
    _lease: LeaseReference,
}
impl TransferMoveStageCleanup for LeasedCleanup {
    fn path(&self) -> &Path {
        self.inner.path()
    }
    fn cleanup(mut self: Box<Self>) -> Result<(), FsError> {
        let inner = self.inner.0.take().expect("cleanup ticket remains owned");
        if crate::copy_nested_retirement::current().is_none() {
            return inner.cleanup();
        }
        let mut result = Ok(());
        if !crate::copy_nested_retirement::checked_step(|| result = inner.cleanup()) {
            return Err(crate::copy_nested_retirement::unconfirmed());
        }
        result
    }
}

#[cfg(test)]
#[path = "tree_move_capture_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tree_copy_taint_tests.rs"]
mod taint_tests;
