//! Temporary source staging, no-replace restoration and empty-stage-only cleanup.
use super::*;
use midnight_core::{
    PreparedTransferMove, TransferMoveCapture, TransferMoveCaptureOutcome, TransferRecovery,
    TransferTarget,
};
use rustix::fs::{RenameFlags, renameat_with};
use std::{ffi::OsString, sync::Mutex};
#[path = "transfer_finalize.rs"]
mod finalization;
#[allow(unsafe_code)]
#[path = "transfer_lease.rs"]
mod lease;
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

pub(super) fn prepare(path: &Path, cancel: &Cancellation) -> Result<PreparedTransferMove, FsError> {
    let (source, capture) = prepare_native(path, cancel)?;
    Ok(PreparedTransferMove {
        source: Box::new(source),
        capture,
    })
}

fn prepare_native(
    path: &Path,
    cancel: &Cancellation,
) -> Result<(source::Source, Arc<Capture>), FsError> {
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
    prepare_native_child(parent_file, parent_path.to_owned(), name, cancel)
}

pub(super) fn prepare_child(
    parent_file: Arc<File>,
    parent_path: PathBuf,
    name: &OsStr,
    cancel: &Cancellation,
) -> Result<PreparedTransferMove, FsError> {
    let (source, capture) = prepare_native_child(parent_file, parent_path, name, cancel)?;
    Ok(PreparedTransferMove {
        source: Box::new(source),
        capture,
    })
}

fn prepare_native_child(
    parent_file: Arc<File>,
    parent_path: PathBuf,
    name: &OsStr,
    cancel: &Cancellation,
) -> Result<(source::Source, Arc<Capture>), FsError> {
    let path = parent_path.join(name);
    let parent = destination::Directory::from_file(
        parent_file.try_clone().map_err(|e| error(&path, e))?,
        parent_path.to_owned(),
    )?;
    native::qualify(&parent)?;
    parent.check_recovery_parent(cancel)?;
    rustix::fs::accessat(parent.native_file(), ".",
        rustix::fs::Access::WRITE_OK | rustix::fs::Access::EXEC_OK, rustix::fs::AtFlags::EACCESS)
        .map_err(|failure| unsupported(&path,
            &format!("The source parent cannot create temporary staging: {failure}. Choose Copy to retain the source; no fallback is performed.")))?;
    let mut source = source::Source::open_move(parent_file, parent_path.to_owned(), name, cancel)?;
    source.enable_move();
    let parent_metadata = parent
        .native_file()
        .metadata()
        .map_err(|e| error(&path, e))?;
    let source_metadata = source
        .native_file()
        .metadata()
        .map_err(|e| error(&path, e))?;
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
    let expected = parent
        .prepare_target(name, cancel)?
        .ok_or_else(|| invalid(&path, "Move source disappeared while preparing"))?;
    if expected.entry().identity != source.entry().identity {
        return Err(invalid(&path, "Move source changed while preparing"));
    }
    let capture = Arc::new(Capture {
        entry: expected.entry().clone(),
        expected,
        _file: source.native_file(),
        name: name.to_owned(),
        parent,
        state: Mutex::new(State::default()),
        lease: Mutex::new(None),
        native: Mutex::new(None),
    });
    Ok((source, capture))
}

#[derive(Default)]
struct State {
    outcome: Option<TransferMoveCaptureOutcome>,
    retained: Option<destination::Directory>,
}
struct Capture {
    entry: TransferEntry,
    expected: Arc<dyn TransferTarget>,
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
        // Empty-only, directory-relative removal. Linux has no inode-CAS rmdir;
        // retain the documented live-validation namespace threat boundary.
        rustix::fs::unlinkat(
            self.parent.native_file(),
            &self.name,
            rustix::fs::AtFlags::REMOVEDIR,
        )
        .map_err(|e| error(path, e.into()))?;
        Ok(midnight_core::TransferDirectoryRemoval {
            state: midnight_core::TransferSourceRemoval::Unlinked,
            durability_confirmed: self.parent.native_file().sync_all().is_ok(),
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
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retained
            .take()
            .map(|stage| {
                Box::new(StageCleanup {
                    parent: self.parent.clone(),
                    stage,
                }) as Box<dyn midnight_core::TransferMoveStageCleanup>
            })
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
        renameat_with(
            stage.native_file(),
            &self.name,
            self.parent.native_file(),
            &self.name,
            RenameFlags::NOREPLACE,
        )
        .map_err(|e| error(&self.entry.item.path, e.into()))?;
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
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(outcome) = &state.outcome {
            return Ok(outcome.clone());
        }
        self.revalidate(cancel)?;
        let recovery = self.parent.source_stage(cancel)?;
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
            if let Err(cleanup) = self.parent.remove_empty_stage(recovery) {
                failure
                    .message
                    .push_str(&format!("; temporary directory cleanup failed: {cleanup}"));
            }
            return Err(failure);
        }
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
#[path = "transfer_finalize_tests.rs"]
mod finalization_tests;

#[cfg(test)]
#[path = "transfer_native_move_tests.rs"]
mod native_tests;
