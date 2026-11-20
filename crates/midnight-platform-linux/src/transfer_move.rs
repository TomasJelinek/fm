//! Temporary source staging, no-replace restoration and empty-stage-only cleanup.
use super::*;
use midnight_core::{
    PreparedTransferMove, PreparedTransferMoveEntry, TransferMoveCapture,
    TransferMoveCaptureOutcome, TransferRecovery, TransferRevisionPolicy, TransferTarget,
};
use rustix::fs::{RenameFlags, renameat_with};
use std::{ffi::OsString, sync::Mutex};
#[path = "transfer_finalize.rs"]
mod finalization;
#[allow(unsafe_code)]
#[path = "transfer_lease.rs"]
mod lease;
#[path = "transfer_directory_link_move.rs"]
mod link_capture;
#[path = "transfer_link_move.rs"]
mod link_move;
#[path = "transfer_native_move.rs"]
mod native;

pub(super) fn supports_native_replacement(
    destination: &destination::Directory,
    source: TransferIdentity,
) -> bool {
    source.device == destination.identity().device
        && native::qualify(destination).is_ok()
        && destination
            .check_recovery_parent(&Cancellation::default())
            .is_ok()
}

/// `revisions` is how the source and its Move capture compare their pinned revisions: a
/// Move accepts a moved ctime (VIEW-05); Rename's replacement keeps them exact.
pub(super) fn prepare(
    path: &Path,
    revisions: TransferRevisionPolicy,
    cancel: &Cancellation,
) -> Result<PreparedTransferMove, FsError> {
    let (source, capture) = prepare_native_with(path, revisions, cancel)?;
    Ok(PreparedTransferMove {
        source: Box::new(source),
        capture,
    })
}

pub(super) fn prepare_with_links(
    path: &Path,
    links: CopyLinkPolicy,
    revisions: TransferRevisionPolicy,
    cancel: &Cancellation,
) -> Result<PreparedTransferMoveEntry, FsError> {
    if links == CopyLinkPolicy::FollowLinks {
        // Move never copies what a link points to and then removes the link.
        return Err(FsError::from_io(
            "transfer capability",
            path,
            io::Error::new(
                io::ErrorKind::Unsupported,
                "Move does not follow links; it moves each link itself",
            ),
        ));
    }
    let Some(traversal) = directory_links::Traversal::for_policy(links) else {
        return prepare(path, revisions, cancel).map(PreparedTransferMoveEntry::Ordinary);
    };
    let (parent_file, parent_path, name) = move_parent(path, cancel)?;
    prepare_child_with_links(
        parent_file,
        parent_path,
        &name,
        None,
        traversal,
        revisions,
        cancel,
    )
}

/// The exact Move capture, as Rename's replacement uses it.
#[cfg(test)]
fn prepare_native(
    path: &Path,
    cancel: &Cancellation,
) -> Result<(source::Source, Arc<Capture>), FsError> {
    prepare_native_with(path, TransferRevisionPolicy::Exact, cancel)
}

fn prepare_native_with(
    path: &Path,
    revisions: TransferRevisionPolicy,
    cancel: &Cancellation,
) -> Result<(source::Source, Arc<Capture>), FsError> {
    let (parent_file, parent_path, name) = move_parent(path, cancel)?;
    prepare_native_child(parent_file, parent_path, &name, None, revisions, cancel)
}

fn move_parent(
    path: &Path,
    cancel: &Cancellation,
) -> Result<(Arc<File>, PathBuf, OsString), FsError> {
    checked(cancel, path)?;
    let path = absolute(path)?;
    let parent_path = path
        .parent()
        .ok_or_else(|| invalid(&path, "source needs a parent"))?;
    let name = path
        .file_name()
        .ok_or_else(|| invalid(&path, "filesystem roots are not Move sources"))?;
    let parent_file = Arc::new(File::from(
        openat(CWD, parent_path, DIRECTORY, Mode::empty()).map_err(|e| error(&path, e.into()))?,
    ));
    Ok((parent_file, parent_path.to_owned(), name.to_owned()))
}

/// `namespace` is the parent directory's bindings, shared by all of its children.
pub(super) fn prepare_child(
    parent_file: Arc<File>,
    parent_path: PathBuf,
    name: &OsStr,
    namespace: Arc<namespace::Bindings>,
    revisions: TransferRevisionPolicy,
    cancel: &Cancellation,
) -> Result<PreparedTransferMove, FsError> {
    let (source, capture) = prepare_native_child(
        parent_file,
        parent_path,
        name,
        Some(namespace),
        revisions,
        cancel,
    )?;
    Ok(PreparedTransferMove {
        source: Box::new(source),
        capture,
    })
}

pub(super) fn prepare_child_with_links(
    parent_file: Arc<File>,
    parent_path: PathBuf,
    name: &OsStr,
    namespace: Option<Arc<namespace::Bindings>>,
    traversal: directory_links::Traversal,
    revisions: TransferRevisionPolicy,
    cancel: &Cancellation,
) -> Result<PreparedTransferMoveEntry, FsError> {
    let (source, parent) = prepare_candidate(
        parent_file,
        parent_path,
        name,
        namespace,
        Some(traversal),
        cancel,
    )?;
    let source = source.with_revisions(revisions);
    if let Some(original) = source.original_link() {
        let capture = Arc::new(link_capture::Capture::prepare(original, parent, cancel)?);
        return Ok(PreparedTransferMoveEntry::DirectoryLink {
            source: Box::new(source),
            capture,
        });
    }
    if let Some(original) = source.link_leaf() {
        // A preserved link is moved itself: its own rename or recreation.
        let capture = Arc::new(link_move::Capture::prepare(original, parent, cancel)?);
        return Ok(PreparedTransferMoveEntry::Ordinary(PreparedTransferMove {
            source: Box::new(source),
            capture,
        }));
    }
    let (source, capture) = prepare_ordinary(source, parent, name, revisions, cancel)?;
    Ok(PreparedTransferMoveEntry::Ordinary(PreparedTransferMove {
        source: Box::new(source),
        capture,
    }))
}

fn prepare_native_child(
    parent_file: Arc<File>,
    parent_path: PathBuf,
    name: &OsStr,
    namespace: Option<Arc<namespace::Bindings>>,
    revisions: TransferRevisionPolicy,
    cancel: &Cancellation,
) -> Result<(source::Source, Arc<Capture>), FsError> {
    let (source, parent) =
        prepare_candidate(parent_file, parent_path, name, namespace, None, cancel)?;
    prepare_ordinary(
        source.with_revisions(revisions),
        parent,
        name,
        revisions,
        cancel,
    )
}

fn prepare_candidate(
    parent_file: Arc<File>,
    parent_path: PathBuf,
    name: &OsStr,
    namespace: Option<Arc<namespace::Bindings>>,
    traversal: Option<directory_links::Traversal>,
    cancel: &Cancellation,
) -> Result<(source::Source, destination::Directory), FsError> {
    let path = parent_path.join(name);
    // The parent directory and the source share one computation of its bindings.
    let namespace = match namespace {
        Some(namespace) => namespace,
        None => namespace::Bindings::for_parent(&parent_file, &parent_path)?,
    };
    let parent = destination::Directory::from_file_with(
        parent_file.try_clone().map_err(|e| error(&path, e))?,
        parent_path.to_owned(),
        Some(namespace.clone()),
    )?;
    native::qualify(&parent)?;
    let check_parent = |parent: &destination::Directory| {
        parent.check_recovery_parent(cancel)?;
        rustix::fs::accessat(parent.native_file(), ".",
        rustix::fs::Access::WRITE_OK | rustix::fs::Access::EXEC_OK, rustix::fs::AtFlags::EACCESS)
            .map_err(|failure| move_error(&path, io::Error::new(io::ErrorKind::Unsupported,
                format!("The source parent cannot create temporary staging: {failure}. Choose Copy to retain the source; no fallback is performed."))))
    };
    let projects_links = traversal.is_some();
    if !projects_links {
        check_parent(&parent)?;
    }
    let source = source::Source::open_below(
        parent_file,
        parent_path.to_owned(),
        name,
        Some(namespace),
        traversal,
        cancel,
    )?;
    let original_link = source.original_link();
    let parent = match &original_link {
        Some(original) => parent.with_link_source(original)?,
        None => parent,
    };
    if projects_links {
        check_parent(&parent)?;
    }
    let parent_metadata = parent
        .native_file()
        .metadata()
        .map_err(|e| error(&path, e))?;
    let original_file =
        original_link.map_or_else(|| source.native_file(), |original| original.native_file());
    let source_metadata = original_file.metadata().map_err(|e| error(&path, e))?;
    let caller = rustix::process::geteuid().as_raw();
    if parent_metadata.mode() & 0o1000 != 0
        && caller != 0
        && parent_metadata.uid() != caller
        && source_metadata.uid() != caller
    {
        return Err(unsupported(
            &path,
            "The sticky source parent does not permit this identity to relocate the source. Choose Copy to retain it; no fallback is performed.",
        ));
    }
    Ok((source, parent))
}

fn prepare_ordinary(
    mut source: source::Source,
    parent: destination::Directory,
    name: &OsStr,
    revisions: TransferRevisionPolicy,
    cancel: &Cancellation,
) -> Result<(source::Source, Arc<Capture>), FsError> {
    let path = &source.entry().item.path;
    // The source's own binding pin: identity, length and mtime must hold. Under the
    // transfer policy its permissions (mode and ctime) may change before a Retry; Exact
    // (Rename's capture) keeps them too.
    let expected = destination::recovery::SourcePin::prepare(&parent, name, revisions, cancel)?
        .ok_or_else(|| invalid(path, "Move source disappeared while preparing"))?;
    if expected.entry().identity != source.entry().identity {
        return Err(invalid(path, "Move source changed while preparing"));
    }
    source.enable_move();
    let capture = Arc::new(Capture {
        entry: expected.entry().clone(),
        expected,
        _file: source.native_file(),
        name: name.to_owned(),
        parent,
        state: Mutex::new(State::default()),
        lease: Mutex::new(None),
        native: Mutex::new(None),
        namespace: source.namespace_bindings(),
    });
    Ok((source, capture))
}

#[derive(Default)]
struct State {
    outcome: Option<TransferMoveCaptureOutcome>,
    retained: Option<destination::Directory>,
    /// The empty private stage of a failed capture whose removal failed (see
    /// `Capture::keep_or_remove_stage_in`). The next capture reuses it; without one,
    /// `take_stage_cleanup` hands it to the engine's cleanup. A capture that moves the
    /// source consumes it, so it is never set together with `retained`.
    unused: Option<destination::Directory>,
}
struct Capture {
    namespace: Arc<namespace::Bindings>,
    entry: TransferEntry,
    expected: destination::recovery::SourcePin,
    _file: Arc<File>,
    name: OsString,
    parent: destination::Directory,
    state: Mutex<State>,
    lease: Mutex<Option<Arc<lease::ReadLease>>>,
    native: Mutex<Option<midnight_core::TransferNativeMoveOutcome>>,
}
impl TransferMoveCapture for Capture {
    fn try_native_move(
        &self,
        destination: &dyn TransferDirectory,
        name: &OsStr,
        replace: Option<(
            Arc<dyn TransferTarget>,
            midnight_core::TransferReplacementPolicy,
        )>,
        cancel: &Cancellation,
    ) -> Result<midnight_core::TransferNativeMoveOutcome, FsError> {
        native::attempt(self, destination, name, replace, cancel)
    }
    fn remove_empty_directory(
        &self,
        cancel: &Cancellation,
    ) -> Result<midnight_core::TransferDirectoryRemoval, FsError> {
        namespace::change(cancel, |mut change| {
            self.remove_empty_directory_in(cancel, &mut change)
        })
    }
    fn preflight_finalization(&self, cancel: &Cancellation) -> Result<(), FsError> {
        if self.entry.item.kind == EntryKind::Directory {
            return destination::Directory::from_file(
                self._file
                    .try_clone()
                    .map_err(|e| error(&self.entry.item.path, e))?,
                self.entry.item.path.clone(),
            )?
            .check_recovery_parent(cancel);
        }
        self.read_lease(cancel).map(|_| ())
    }
    fn prepare_finalization(
        &self,
        cancel: &Cancellation,
    ) -> Result<Box<dyn midnight_core::TransferMoveFinalizer>, FsError> {
        finalization::prepare(self, cancel)
    }
    fn entry(&self) -> &TransferEntry {
        &self.entry
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.parent.check_recovery_parent(cancel)?;
        self.expected.revalidate(cancel)?;
        if let Some(lease) = self
            .lease
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            lease.revalidate(&self.entry.item.path)?;
        }
        Ok(())
    }
    fn capture(&self, cancel: &Cancellation) -> Result<TransferMoveCaptureOutcome, FsError> {
        self.capture_with(cancel, || {}, || {})
    }
    fn restore(&self) -> Result<(), FsError> {
        self.restore_with(|stage| self.sync_names(stage))
    }
    fn restoration_completed(&self) -> bool {
        matches!(
            self.state.lock().unwrap_or_else(|e| e.into_inner()).outcome,
            Some(TransferMoveCaptureOutcome::Restored { .. })
        )
    }
    fn take_stage_cleanup(&self) -> Option<Box<dyn midnight_core::TransferMoveStageCleanup>> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let stage = state.retained.take().or_else(|| state.unused.take())?;
        Some(Box::new(StageCleanup {
            parent: self.parent.clone(),
            stage,
        }))
    }
}
struct StageCleanup {
    parent: destination::Directory,
    stage: destination::Directory,
}
impl midnight_core::TransferMoveStageCleanup for StageCleanup {
    fn path(&self) -> &Path {
        self.stage.path()
    }
    fn cleanup(self: Box<Self>) -> Result<(), FsError> {
        self.parent.remove_empty_stage(self.stage)
    }
}
impl Capture {
    fn restore_with(
        &self,
        sync: impl FnOnce(&destination::Directory) -> Result<(), FsError>,
    ) -> Result<(), FsError> {
        namespace::change(&Cancellation::default(), |mut change| {
            self.restore_in(sync, &mut change)
        })
    }
    fn read_lease(&self, cancel: &Cancellation) -> Result<Arc<lease::ReadLease>, FsError> {
        checked(cancel, &self.entry.item.path)?;
        if !destination::recovery::supported(&self.parent) {
            return Err(unsupported(
                &self.entry.item.path,
                "Verified cross-device Move requires a qualified local ext source",
            ));
        }
        finalization::qualify(&self._file, &self.entry.item.path)?;
        let mut lease = self.lease.lock().unwrap_or_else(|e| e.into_inner());
        if lease.is_none() {
            *lease = Some(lease::ReadLease::acquire(
                Arc::new(source::open_retained(&self._file, &self.entry.item.path)?),
                &self.entry.item.path,
            )?);
        }
        let lease = lease
            .as_ref()
            .ok_or_else(|| invalid(&self.entry.item.path, "source lease missing"))?;
        lease.revalidate(&self.entry.item.path)?;
        Ok(lease.clone())
    }
    fn capture_with(
        &self,
        cancel: &Cancellation,
        before_capture: impl FnOnce(),
        after_capture: impl FnOnce(),
    ) -> Result<TransferMoveCaptureOutcome, FsError> {
        namespace::change(cancel, |mut change| {
            self.capture_in(cancel, before_capture, after_capture, &mut change)
        })
    }
    fn sync_names(&self, recovery: &destination::Directory) -> Result<(), FsError> {
        recovery
            .native_file()
            .sync_all()
            .and_then(|()| self.parent.native_file().sync_all())
            .map_err(|e| error(&self.entry.item.path, e))
    }
}

#[cfg(test)]
#[path = "transfer_move_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "transfer_move_cleanup_tests.rs"]
mod cleanup_tests;

#[cfg(test)]
#[path = "transfer_move_refusal_tests.rs"]
mod peer_refusal_tests;

#[cfg(test)]
#[path = "transfer_finalize_tests.rs"]
mod finalization_tests;

#[cfg(test)]
#[path = "transfer_native_move_tests.rs"]
mod native_tests;

#[cfg(test)]
#[path = "transfer_move_permission_tests.rs"]
mod permission_tests;

impl Capture {
    fn remove_empty_directory_in(
        &self,
        cancel: &Cancellation,
        change: &mut namespace::Change<'_>,
    ) -> Result<midnight_core::TransferDirectoryRemoval, FsError> {
        let path = &self.entry.item.path;
        self.parent.check_recovery_parent(cancel)?;
        if self.entry.item.kind != EntryKind::Directory
            || identity(&self._file, path)? != self.entry.identity
            || self
                .parent
                .inspect(&self.name, cancel)?
                .is_none_or(|entry| {
                    entry.identity != self.entry.identity || entry.item.kind != EntryKind::Directory
                })
        {
            return Err(invalid(path, "Move directory identity changed"));
        }
        checked(cancel, path)?;
        self.namespace.announce(
            self.parent.native_file(),
            self.parent.path(),
            &self.name,
            None,
            change,
            cancel,
        )?;
        // Empty-only, directory-relative removal. Linux has no inode-CAS rmdir;
        // retain the documented live-validation namespace threat boundary.
        rustix::fs::unlinkat(
            self.parent.native_file(),
            &self.name,
            rustix::fs::AtFlags::REMOVEDIR,
        )
        .map_err(|e| error(path, e.into()))?;
        self.namespace
            .changed(self.parent.native_file(), self.parent.path(), &self.name);
        Ok(midnight_core::TransferDirectoryRemoval {
            state: midnight_core::TransferSourceRemoval::Unlinked,
            durability_confirmed: self.parent.native_file().sync_all().is_ok(),
        })
    }
}

impl Capture {
    fn restore_in(
        &self,
        sync: impl FnOnce(&destination::Directory) -> Result<(), FsError>,
        change: &mut namespace::Change<'_>,
    ) -> Result<(), FsError> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(stage) = &state.retained else {
            return Ok(());
        };
        let receipt = match &state.outcome {
            Some(TransferMoveCaptureOutcome::Restored { .. }) => return Ok(()),
            Some(TransferMoveCaptureOutcome::Captured { recovery, .. }) => recovery,
            Some(TransferMoveCaptureOutcome::RecoveryRequired { recovery, .. }) => recovery
                .first()
                .ok_or_else(|| invalid(&self.entry.item.path, "staged source receipt missing"))?,
            None => {
                return Err(invalid(
                    &self.entry.item.path,
                    "staged source outcome missing",
                ));
            }
        };
        let cleanup = Cancellation::default();
        self.parent.check_recovery_parent(&cleanup)?;
        let visible = stage.inspect(&self.name, &cleanup)?;
        if receipt.identity.is_none() || visible.map(|entry| entry.identity) != receipt.identity {
            return Err(invalid(
                &receipt.path,
                "staged source identity changed before restoration",
            ));
        }
        self.namespace.announce(
            self.parent.native_file(),
            self.parent.path(),
            &self.name,
            None,
            change,
            &cleanup,
        )?;
        stage.namespace_announce(&self.name, change, &cleanup)?;
        renameat_with(
            stage.native_file(),
            &self.name,
            self.parent.native_file(),
            &self.name,
            RenameFlags::NOREPLACE,
        )
        .map_err(|e| error(&self.entry.item.path, e.into()))?;
        self.namespace
            .changed(self.parent.native_file(), self.parent.path(), &self.name);
        stage.namespace_changed(&self.name);
        let synced = sync(stage);
        // Record the namespace fact before reporting any failed durability check.
        state.outcome = Some(TransferMoveCaptureOutcome::Restored {
            source: self.entry.item.path.clone(),
            error: invalid(&self.entry.item.path, "temporary source staging restored"),
        });
        synced.map_err(|mut failure| {
            failure.message = format!(
                "Source was restored; directory durability is unconfirmed: {}",
                failure.message
            );
            failure
        })
    }
}

impl Capture {
    fn capture_in(
        &self,
        cancel: &Cancellation,
        before_capture: impl FnOnce(),
        after_capture: impl FnOnce(),
        change: &mut namespace::Change<'_>,
    ) -> Result<TransferMoveCaptureOutcome, FsError> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(outcome) = &state.outcome {
            return Ok(outcome.clone());
        }
        self.revalidate(cancel)?;
        // This original binding and any selected descendants remain announced
        // throughout capture and any same-turn no-replace restoration.
        self.namespace.announce(
            self.parent.native_file(),
            self.parent.path(),
            &self.name,
            None,
            change,
            cancel,
        )?;
        let recovery = match state.unused.take() {
            // Kept by an earlier attempt: announce it in this turn, as its creation
            // did, before the source is renamed into it. Nothing follows a refusal.
            Some(stage) => match self.parent.reuse_stage_in(&stage, cancel, change) {
                Ok(()) => stage,
                Err(failure) => {
                    state.unused = Some(stage);
                    return Err(failure);
                }
            },
            None => self.parent.source_stage_in(cancel, change)?,
        };
        let captured = (|| {
            self.revalidate(cancel)?;
            before_capture();
            checked(cancel, &self.entry.item.path)?;
            renameat_with(
                self.parent.native_file(),
                &self.name,
                recovery.native_file(),
                &self.name,
                RenameFlags::NOREPLACE,
            )
            .map_err(|e| error(&self.entry.item.path, e.into()))
        })();
        if let Err(mut failure) = captured {
            self.keep_or_remove_stage_in(&mut state, recovery, &mut failure, change);
            return Err(failure);
        }
        self.namespace
            .changed(self.parent.native_file(), self.parent.path(), &self.name);
        recovery.namespace_changed(&self.name);
        // Name capture is recoverable, not expected-inode CAS. Inspect only metadata;
        // even a raced-in link/device/tree is retained and never opened for contents.
        after_capture();
        let cleanup = Cancellation::default();
        let path = recovery.path().join(&self.name);
        let (receipt, mut failure) = match recovery.inspect(&self.name, &cleanup) {
            Ok(Some(entry)) => {
                let failure = if entry.identity == self.entry.identity {
                    checked(cancel, &self.entry.item.path).err()
                } else {
                    Some(invalid(
                        &self.entry.item.path,
                        "a different source root was captured; its bytes were preserved",
                    ))
                };
                (
                    TransferRecovery {
                        path,
                        identity: Some(entry.identity),
                    },
                    failure,
                )
            }
            Ok(None) => (
                TransferRecovery {
                    path,
                    identity: None,
                },
                Some(invalid(
                    &self.entry.item.path,
                    "captured source root is no longer visible in recovery",
                )),
            ),
            Err(error) => (
                TransferRecovery {
                    path,
                    identity: None,
                },
                Some(error),
            ),
        };
        let outcome = if let Some(mut error) = failure.take() {
            match renameat_with(
                recovery.native_file(),
                &self.name,
                self.parent.native_file(),
                &self.name,
                RenameFlags::NOREPLACE,
            ) {
                Ok(()) => {
                    self.namespace.changed(
                        self.parent.native_file(),
                        self.parent.path(),
                        &self.name,
                    );
                    recovery.namespace_changed(&self.name);
                    if let Err(sync) = self.sync_names(&recovery) {
                        error
                            .message
                            .push_str(&format!("; restored names, durability unconfirmed: {sync}"));
                    }
                    TransferMoveCaptureOutcome::Restored {
                        source: self.entry.item.path.clone(),
                        error,
                    }
                }
                Err(restore) => {
                    error.message.push_str(&format!(
                        "; source restoration failed without replacement: {restore}"
                    ));
                    TransferMoveCaptureOutcome::RecoveryRequired {
                        source: self.entry.item.path.clone(),
                        recovery: vec![receipt],
                        error,
                    }
                }
            }
        } else if let Err(error) = self.sync_names(&recovery) {
            TransferMoveCaptureOutcome::RecoveryRequired {
                source: self.entry.item.path.clone(),
                recovery: vec![receipt],
                error,
            }
        } else {
            TransferMoveCaptureOutcome::Captured {
                source: self.entry.item.path.clone(),
                recovery: receipt,
            }
        };
        state.retained = Some(recovery);
        state.outcome = Some(outcome.clone());
        Ok(outcome)
    }

    /// Removes, in the capture's own change, the empty private stage of a capture that
    /// moved nothing. If this change created the stage, the reservation made at its
    /// creation covers the removal, so peers cannot refuse it. A stage reused from an
    /// earlier attempt is announced again, and peers can refuse its removal (nothing
    /// moved); the refusal is then the change's last reservation, so the engine
    /// repeats the capture without a prompt once a refusing peer changes, and drops
    /// `failure`. A stage the removal cannot remove (refused, another entry in it, an
    /// I/O error) is kept instead of dropped unreported: the next attempt reuses it,
    /// and otherwise `take_stage_cleanup` returns it for the engine to remove or report.
    fn keep_or_remove_stage_in(
        &self,
        state: &mut State,
        stage: destination::Directory,
        failure: &mut FsError,
        change: &mut namespace::Change<'_>,
    ) {
        match self.parent.remove_empty_stage_checked(stage, change) {
            Ok(()) => {}
            Err(destination::recovery::StageRemovalError::Retained(_, stage)) => {
                state.unused = Some(*stage);
            }
            Err(destination::recovery::StageRemovalError::Unsynced(sync)) => {
                failure.message.push_str(&format!(
                    "; the empty temporary directory was removed, but that removal may \
                     not be durable: {sync}"
                ));
            }
        }
    }
}
