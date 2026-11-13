//! Retained tree roots and occurrence feedback; all provider work runs on workers.
use crate::{
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
struct Sources {
    lease: Arc<Lease>,
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
                lease: Arc::new(Lease {
                    capture,
                    _permit: permit,
                }),
                metadata,
                intent: request.intent,
            }),
        })
    }
}
impl CapturedTransferSources for Sources {
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
        if links == CopyLinkPolicy::Preserve && self.intent != TransferIntent::Copy {
            return Err(FsError {
                kind: io::ErrorKind::Unsupported,
                ..failure("Tree link preservation is available only for Copy.")
            });
        }
        if self.intent == TransferIntent::Move {
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
                if links == CopyLinkPolicy::Preserve {
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
        for (id, kind, _) in &archive_roots {
            if *kind == EntryKind::Directory
                && archive_roots.iter().any(|(other, _, trail)| {
                    other != id && other.mount == id.mount && trail.contains(&id.entry)
                })
            {
                return Err(failure("Overlapping archive Copy roots."));
            }
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
                    let source = service.prepare_transfer_source_with_links(path, links, cancel)?;
                    if source.entry().item.path != *path || source.native_identity().is_none() {
                        return Err(failure("Provider returned a different native source."));
                    }
                    source
                }
                ProviderTarget::Archive(target) => {
                    crate::copy_operations::archive_source::captured_root(
                        target,
                        traversal.clone(),
                        cancel,
                    )?
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
    fn prepare_move(
        &self,
        service: &dyn TransferService,
        cancel: &Cancellation,
    ) -> Result<VecDeque<PreparedTransferMove>, FsError> {
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
            let prepared = service.prepare_move_source(path, cancel)?;
            if prepared.source.entry().item.path != path
                || prepared.source.native_identity() != Some(prepared.source.entry().identity)
                || prepared.capture.entry().item.path != path
            {
                return Err(failure("Provider returned a different native Move source."));
            }
            roots.push_back(PreparedTransferMove {
                source: Box::new(LeasedSource {
                    source: prepared.source,
                    lease: self.lease.clone(),
                    occurrence: Some(TreeCopyOccurrence {
                        stamp: self.lease.capture.view.stamp,
                        row: target.row,
                    }),
                }),
                capture: leased_capture(prepared.capture, &self.lease),
            });
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
struct LeasedSource {
    source: Box<dyn TransferSource>,
    lease: Arc<Lease>,
    occurrence: Option<TreeCopyOccurrence>,
}
impl LeasedSource {
    fn child(&self, source: Box<dyn TransferSource>) -> Box<dyn TransferSource> {
        Box::new(Self {
            source,
            lease: self.lease.clone(),
            occurrence: None,
        })
    }
}
impl TransferSource for LeasedSource {
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
}

fn leased_capture(
    inner: Arc<dyn TransferMoveCapture>,
    lease: &Arc<Lease>,
) -> Arc<dyn TransferMoveCapture> {
    Arc::new(LeasedMoveCapture {
        inner,
        lease: lease.clone(),
    })
}
// Authority fields precede the lease: providers remain live through authority destruction.
struct LeasedMoveCapture {
    inner: Arc<dyn TransferMoveCapture>,
    lease: Arc<Lease>,
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
            inner: self.inner.prepare_finalization(cancel)?,
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
                inner,
                _lease: self.lease.clone(),
            }) as Box<dyn TransferMoveStageCleanup>
        })
    }
}
struct LeasedFinalizer {
    inner: Box<dyn TransferMoveFinalizer>,
    _lease: Arc<Lease>,
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
    inner: Box<dyn TransferMoveStageCleanup>,
    _lease: Arc<Lease>,
}
impl TransferMoveStageCleanup for LeasedCleanup {
    fn path(&self) -> &Path {
        self.inner.path()
    }
    fn cleanup(self: Box<Self>) -> Result<(), FsError> {
        self.inner.cleanup()
    }
}

#[cfg(test)]
#[path = "tree_move_capture_tests.rs"]
mod tests;
