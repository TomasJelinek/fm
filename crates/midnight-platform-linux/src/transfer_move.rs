//! Source-root recovery capture. Only separate explicit finalization may unlink.
use super::*;
use midnight_core::{
    PreparedTransferMove, TransferMoveCapture, TransferMoveCaptureOutcome, TransferRecovery,
    TransferTarget,
};
use rustix::fs::{RenameFlags, renameat_with};
use std::{ffi::OsString, sync::Mutex};

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
    let parent = destination::Directory::from_file(
        parent_file.try_clone().map_err(|e| error(&path, e))?,
        parent_path.to_owned(),
    )?;
    if !parent.capabilities().recoverable_replacement {
        return Err(unsupported(
            &path,
            "Recoverable Move requires a local ext filesystem. Choose Copy to retain the source; no fallback is performed.",
        ));
    }
    parent.check_recovery_parent(cancel)?;
    rustix::fs::accessat(parent.native_file(), ".",
        rustix::fs::Access::WRITE_OK | rustix::fs::Access::EXEC_OK, rustix::fs::AtFlags::EACCESS)
        .map_err(|failure| unsupported(&path,
            &format!("The source parent cannot create recovery storage: {failure}. Choose Copy to retain the source; no fallback is performed.")))?;
    let source = source::Source::open(parent_file, parent_path.to_owned(), name, cancel)?;
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
}
impl TransferMoveCapture for Capture {
    fn preflight_finalization(&self, cancel: &Cancellation) -> Result<(), FsError> {
        checked(cancel, &self.entry.item.path)?;
        Err(unsupported(
            &self.entry.item.path,
            "Verified Move source removal is not supported on Linux: writer exclusion cannot be guaranteed for an in-flight source open. Choose Copy to retain the source; no fallback is performed.",
        ))
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
        checked(cancel, &self.entry.item.path)?;
        renameat_with(
            self.parent.native_file(),
            &self.name,
            recovery.native_file(),
            &self.name,
            RenameFlags::NOREPLACE,
        )
        .map_err(|e| error(&self.entry.item.path, e.into()))?;
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
