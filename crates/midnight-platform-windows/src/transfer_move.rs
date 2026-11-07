//! Source-root recovery capture. Only separate explicit finalization may delete.
use super::*;
use midnight_core::{
    PreparedTransferMove, TransferMoveCapture, TransferMoveCaptureOutcome, TransferRecovery,
    TransferTarget,
};
use std::{ffi::OsString, fs::OpenOptions, os::windows::fs::OpenOptionsExt, sync::Mutex};
#[path = "transfer_finalize.rs"]
mod finalization;

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
    // READ_CONTROL supports a handle-bound parent DACL check. Denied delete sharing
    // pins this namespace during the batch; ordinary Copy keeps its existing rights.
    let parent_file = Arc::new(
        OpenOptions::new()
            .access_mode(
                FILE_READ_ATTRIBUTES | FILE_TRAVERSE | READ_CONTROL | FILE_ADD_SUBDIRECTORY,
            )
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(parent_path)
            .map_err(|e| error(path, e))?,
    );
    let parent = destination::Directory::from_file(
        parent_file.try_clone().map_err(|e| error(path, e))?,
        parent_path.to_owned(),
    )?;
    if !parent.capabilities().recoverable_replacement {
        return Err(unsupported(
            path,
            "Recoverable Move requires local fixed NTFS. Choose Copy to retain the source; no fallback is performed.",
        ));
    }
    parent.check_recovery_parent(cancel)?;
    let source = source::Source::open_for_move(parent_file, parent_path.to_owned(), name, cancel)?;
    let expected = parent
        .prepare_target(name, cancel)?
        .ok_or_else(|| invalid(path, "Move source disappeared while preparing"))?;
    if expected.entry().identity != source.entry().identity {
        return Err(invalid(path, "Move source changed while preparing"));
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
    fn preflight_finalization(&self, cancel: &Cancellation) -> Result<(), FsError> {
        checked(cancel, &self.entry.item.path)?;
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
        self.parent.check_recovery_parent(cancel)?;
        self.expected.revalidate(cancel)
    }
    fn capture(&self, cancel: &Cancellation) -> Result<TransferMoveCaptureOutcome, FsError> {
        self.capture_with(cancel, || {}, || {})
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
        let recovery = self.parent.source_recovery(cancel)?;
        self.revalidate(cancel)?;
        before_capture();
        let path = recovery.capture_handle(&self.file, &self.name, cancel)?;
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
