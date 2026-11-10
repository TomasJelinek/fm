//! Temporary source staging, no-replace restoration and exact empty-stage cleanup.
use super::*;
use midnight_core::{
    PreparedTransferMove, TransferMoveCapture, TransferMoveCaptureOutcome, TransferRecovery,
    TransferTarget,
};
use std::{
    ffi::{OsStr, OsString},
    fs::OpenOptions,
    os::windows::fs::OpenOptionsExt,
    sync::Mutex,
};
#[path = "transfer_finalize.rs"]
mod finalization;
#[path = "transfer_native_move.rs"]
mod native_move;

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
    validate_path(path, false)?;
    let parent_path = path
        .parent()
        .ok_or_else(|| invalid(path, "source needs a parent"))?;
    let name = path
        .file_name()
        .ok_or_else(|| invalid(path, "source needs a basename"))?;
    // Denied delete sharing pins this namespace. Recovery-only rights and DACL
    // checks are deferred until a cross-volume proof path actually needs staging.
    let parent_file = Arc::new(
        OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES | FILE_TRAVERSE)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(parent_path)
            .map_err(|e| error(path, e))?,
    );
    prepare_native_child(parent_file, parent_path.to_owned(), name, cancel)
}

pub(super) fn prepare_child(
    parent_file: Arc<File>,
    parent_path: std::path::PathBuf,
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
    parent_path: std::path::PathBuf,
    name: &OsStr,
    cancel: &Cancellation,
) -> Result<(source::Source, Arc<Capture>), FsError> {
    let path = parent_path.join(name);
    let parent = destination::Directory::from_file(
        parent_file.try_clone().map_err(|e| error(&path, e))?,
        parent_path.to_owned(),
    )?;
    let source = source::Source::open_for_move(parent_file, parent_path.to_owned(), name, cancel)?;
    let expected = parent
        .prepare_target(name, cancel)?
        .ok_or_else(|| invalid(&path, "Move source disappeared while preparing"))?;
    if expected.entry().identity != source.entry().identity {
        return Err(invalid(&path, "Move source changed while preparing"));
    }
    let capture = Arc::new(Capture {
        entry: expected.entry().clone(),
        expected,
        file: source.native_file(),
        name: name.to_owned(),
        parent,
        state: Mutex::new(State::default()),
    });
    Ok((source, capture))
}

#[derive(Default)]
struct State {
    outcome: Option<TransferMoveCaptureOutcome>,
    retained: Option<destination::Directory>,
    native_outcome: Option<midnight_core::TransferNativeMoveOutcome>,
}
struct Capture {
    entry: TransferEntry,
    expected: Arc<dyn TransferTarget>,
    file: Arc<File>,
    name: OsString,
    parent: destination::Directory,
    state: Mutex<State>,
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
        native_move::attempt(self, destination, name, replace, cancel)
    }
    fn remove_empty_directory(
        &self,
        cancel: &Cancellation,
    ) -> Result<midnight_core::TransferDirectoryRemoval, FsError> {
        let path = &self.entry.item.path;
        checked(cancel, path)?;
        if self.entry.item.kind != EntryKind::Directory
            || identity(&self.file, path)? != self.entry.identity
            || self
                .parent
                .inspect(&self.name, cancel)?
                .is_none_or(|entry| {
                    entry.identity != self.entry.identity || entry.item.kind != EntryKind::Directory
                })
        {
            return Err(invalid(path, "Move directory identity changed"));
        }
        finalization::qualify_directory(&self.file, path)?;
        checked(cancel, path)?;
        // Exact held DELETE handle; the kernel refuses a nonempty directory.
        native::remove(&self.file).map_err(|e| error(path, e))?;
        Ok(midnight_core::TransferDirectoryRemoval {
            state: midnight_core::TransferSourceRemoval::DeletePending,
            durability_confirmed: false,
        })
    }
    fn preflight_finalization(&self, cancel: &Cancellation) -> Result<(), FsError> {
        checked(cancel, &self.entry.item.path)?;
        self.parent.check_recovery_parent(cancel)?;
        if self.entry.item.kind == EntryKind::Directory {
            finalization::qualify_directory(&self.file, &self.entry.item.path)?;
            return destination::Directory::from_file(
                self.file
                    .try_clone()
                    .map_err(|e| error(&self.entry.item.path, e))?,
                self.entry.item.path.clone(),
            )?
            .check_recovery_parent(cancel);
        }
        finalization::qualify(&self.file, &self.entry.item.path)
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
        self.expected.revalidate(cancel)
    }
    fn capture(&self, cancel: &Cancellation) -> Result<TransferMoveCaptureOutcome, FsError> {
        self.capture_with(cancel, || {}, || {})
    }
    fn restore(&self) -> Result<(), FsError> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.retained.is_none()
            || matches!(
                state.outcome,
                Some(TransferMoveCaptureOutcome::Restored { .. })
            )
        {
            return Ok(());
        }
        let cleanup = Cancellation::default();
        self.parent.check_recovery_parent(&cleanup)?;
        // Rename exactly our selected held object. A new original-name winner is
        // never replaced, even if the winner was created after cancellation.
        self.parent
            .capture_handle(&self.file, &self.name, &cleanup)?;
        state.outcome = Some(TransferMoveCaptureOutcome::Restored {
            source: self.entry.item.path.clone(),
            error: invalid(&self.entry.item.path, "temporary source staging restored"),
        });
        Ok(())
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
            .map(|directory| {
                Box::new(StageCleanup(directory))
                    as Box<dyn midnight_core::TransferMoveStageCleanup>
            })
    }
}
struct StageCleanup(destination::Directory);
impl midnight_core::TransferMoveStageCleanup for StageCleanup {
    fn path(&self) -> &Path {
        self.0.path()
    }
    fn cleanup(self: Box<Self>) -> Result<(), FsError> {
        self.0.remove_empty_stage()
    }
}
impl Capture {
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
        self.parent.check_recovery_parent(cancel)?;
        let recovery = self.parent.source_stage(cancel)?;
        if let Err(mut failure) = self.revalidate(cancel) {
            if let Err(cleanup) = recovery.remove_empty_stage() {
                failure
                    .message
                    .push_str(&format!("; temporary directory cleanup failed: {cleanup}"));
            }
            return Err(failure);
        }
        before_capture();
        let path = match recovery.capture_handle(&self.file, &self.name, cancel) {
            Ok(path) => path,
            Err(mut failure) => {
                if let Err(cleanup) = recovery.remove_empty_stage() {
                    failure
                        .message
                        .push_str(&format!("; temporary directory cleanup failed: {cleanup}"));
                }
                return Err(failure);
            }
        };
        after_capture();
        let cleanup = Cancellation::default();
        let inspected = recovery.inspect(&self.name, &cleanup);
        let receipt = TransferRecovery {
            path,
            identity: Some(self.entry.identity),
        };
        let failure = match inspected {
            Ok(Some(entry)) if entry.identity == self.entry.identity => {
                checked(cancel, &self.entry.item.path).err()
            }
            Ok(_) => Some(invalid(
                &self.entry.item.path,
                "captured Move root identity changed; original handle remains retained",
            )),
            Err(error) => Some(error),
        };
        let outcome = if let Some(error) = failure {
            match self.parent.capture_handle(&self.file, &self.name, &cleanup) {
                Ok(_) => TransferMoveCaptureOutcome::Restored {
                    source: self.entry.item.path.clone(),
                    error,
                },
                Err(restore) => {
                    let mut error = error;
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
        } else {
            TransferMoveCaptureOutcome::Captured {
                source: self.entry.item.path.clone(),
                recovery: receipt,
            }
        };
        // This directory and the root handle remain pinned until the authority is
        // dropped. No Drop implementation unlinks originals or recovery directories.
        state.retained = Some(recovery);
        state.outcome = Some(outcome.clone());
        Ok(outcome)
    }
}

#[cfg(test)]
#[path = "transfer_move_tests.rs"]
mod tests;
