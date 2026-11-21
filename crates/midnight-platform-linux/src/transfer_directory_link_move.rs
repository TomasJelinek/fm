//! Protected removal of an original, separately captured directory link.

use super::super::destination::recovery::StageRemovalError;
use super::*;
use midnight_core::{
    TransferDirectoryLinkMoveCapture, TransferDirectoryLinkRemoval, TransferNativeMoveOutcome,
    TransferReplacementPolicy, TransferSourceRemoval,
};
use rustix::fs::{AtFlags, unlinkat};

#[cfg(test)]
#[path = "transfer_directory_link_move_tests.rs"]
mod tests;

pub(super) struct Capture {
    original: Arc<directory_links::OriginalLink>,
    parent: destination::Directory,
    /// Which folder `parent` is: only a folder of the moved tree gets owner write.
    folder: Folder,
    outcome: Mutex<Option<Completed>>,
}

enum Completed {
    Native(TransferNativeMoveOutcome),
    Removed(TransferDirectoryLinkRemoval),
}

impl TransferDirectoryLinkMoveCapture for Capture {
    fn entry(&self) -> &TransferEntry {
        self.original.entry()
    }

    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.original.qualify_removal()?;
        self.parent.check_recovery_parent(cancel)?;
        self.original.verify_original(cancel)
    }

    fn try_native_move(
        &self,
        destination: &dyn TransferDirectory,
        name: &OsStr,
        replace: Option<(Arc<dyn TransferTarget>, TransferReplacementPolicy)>,
        cancel: &Cancellation,
    ) -> Result<TransferNativeMoveOutcome, FsError> {
        let mut terminal = self
            .outcome
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        match terminal.as_ref() {
            Some(Completed::Native(outcome)) => return Ok(outcome.clone()),
            Some(Completed::Removed(_)) => {
                return Err(invalid(
                    &self.entry().item.path,
                    "the original link already entered removal",
                ));
            }
            None => {}
        }
        let outcome = native::attempt_link(
            &self.original,
            (&self.parent, self.folder),
            destination,
            name,
            replace,
            cancel,
        )?;
        if native::terminal(&outcome) {
            *terminal = Some(Completed::Native(outcome.clone()));
        }
        Ok(outcome)
    }

    fn remove_original(
        &self,
        cancel: &Cancellation,
    ) -> Result<TransferDirectoryLinkRemoval, FsError> {
        self.remove_with(cancel, || {}, || {})
    }
}

impl Capture {
    pub(super) fn prepare(
        original: Arc<directory_links::OriginalLink>,
        parent: destination::Directory,
        cancel: &Cancellation,
    ) -> Result<Self, FsError> {
        let capture = Self {
            original,
            parent,
            folder: Folder::ContainsSelection,
            outcome: Mutex::new(None),
        };
        capture.revalidate(cancel)?;
        Ok(capture)
    }

    /// The link leaves `folder` (by default the folder that contains the selection).
    pub(super) fn leaving(mut self, folder: Folder) -> Self {
        self.folder = folder;
        self
    }

    fn remove_with(
        &self,
        cancel: &Cancellation,
        before_capture: impl FnOnce(),
        after_capture: impl FnOnce(),
    ) -> Result<TransferDirectoryLinkRemoval, FsError> {
        let mut terminal = self
            .outcome
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        match terminal.as_ref() {
            Some(Completed::Removed(outcome)) => return Ok(outcome.clone()),
            Some(Completed::Native(_)) => {
                return Err(invalid(
                    &self.entry().item.path,
                    "the original link already entered native Move",
                ));
            }
            None => {}
        }
        let outcome = namespace::change(cancel, |mut change| {
            self.remove_in(cancel, before_capture, after_capture, &mut change)
        })?;
        *terminal = Some(Completed::Removed(outcome.clone()));
        Ok(outcome)
    }

    fn remove_in(
        &self,
        cancel: &Cancellation,
        before_capture: impl FnOnce(),
        after_capture: impl FnOnce(),
        change: &mut namespace::Change<'_>,
    ) -> Result<TransferDirectoryLinkRemoval, FsError> {
        self.revalidate(cancel)?;
        let path = &self.entry().item.path;
        let name = self.original.name();
        self.parent.namespace_announce(name, change, cancel)?;
        // For the stage, the capture, and the restoration or stage removal of a failure;
        // the folder gets its own mode back once the change returns.
        let writable = self.folder.writable(&self.parent);
        let outcome = (|| -> Result<TransferDirectoryLinkRemoval, FsError> {
            let stage = match self.parent.source_stage_in(cancel, change) {
                Ok(stage) => stage,
                Err(failure) => return self.stage_creation_failure(failure),
            };
            let captured = self.revalidate(cancel).and_then(|()| {
                stage.check_recovery_parent(cancel)?;
                before_capture();
                checked(cancel, path)?;
                renameat_with(
                    self.parent.native_file(),
                    name,
                    stage.native_file(),
                    name,
                    RenameFlags::NOREPLACE,
                )
                .map_err(|failure| error(path, failure.into()))
            });
            if let Err(failure) = captured {
                return self.failed_with_empty_stage(stage, failure, change);
            }
            self.parent.namespace_changed(name);
            stage.namespace_changed(name);
            after_capture();
            let staged_path = stage.path().join(name);
            let removed = (|| {
                self.parent.check_recovery_parent(cancel)?;
                stage.check_recovery_parent(cancel)?;
                self.original.qualify_removal()?;
                self.original
                    .verify_staged(stage.native_file(), &staged_path, cancel)?;
                // The owned private stage excludes other unprivileged users. The
                // existing same-UID and privileged namespace limitation still applies.
                unlinkat(stage.native_file(), name, AtFlags::empty())
                    .map_err(|failure| error(&staged_path, failure.into()))
            })();
            if let Err(failure) = removed {
                return self.restore(stage, failure, change);
            }
            stage.namespace_changed(name);
            if let Err(failure) = self.original.verify_unlinked() {
                return Ok(self.keep_recovery(&stage, stage_receipt(&stage), failure));
            }
            let synced = stage
                .native_file()
                .sync_all()
                .map_err(|failure| error(&staged_path, failure));
            let (recovery, cleanup_error) = self.clean_stage(stage, change);
            let mut failure = synced.err();
            if let Some(cleanup) = cleanup_error {
                match &mut failure {
                    Some(failure) => append_error(failure, "Temporary directory cleanup", cleanup),
                    None => failure = Some(cleanup),
                }
            }
            Ok(TransferDirectoryLinkRemoval::Removed {
                state: TransferSourceRemoval::Unlinked,
                durability_confirmed: failure.is_none(),
                recovery: recovery.into_iter().collect(),
                error: failure,
            })
        })();
        writable::restored(outcome, [writable])
    }

    fn restore(
        &self,
        stage: destination::Directory,
        mut failure: FsError,
        change: &mut namespace::Change<'_>,
    ) -> Result<TransferDirectoryLinkRemoval, FsError> {
        let name = self.original.name();
        let cleanup = Cancellation::default();
        let receipt = match stage.inspect(name, &cleanup) {
            Ok(Some(entry)) => TransferRecovery {
                path: stage.path().join(name),
                identity: Some(entry.identity),
            },
            Ok(None) => {
                failure
                    .message
                    .push_str("\nThe staged entry could not be found for restoration.");
                return Ok(self.keep_recovery(&stage, stage_receipt(&stage), failure));
            }
            Err(lookup) => {
                append_error(&mut failure, "Inspecting the staged entry", lookup);
                return Ok(self.keep_recovery(&stage, stage_receipt(&stage), failure));
            }
        };
        let restored = (|| {
            self.parent.check_recovery_parent(&cleanup)?;
            stage.check_recovery_parent(&cleanup)?;
            renameat_with(
                stage.native_file(),
                name,
                self.parent.native_file(),
                name,
                RenameFlags::NOREPLACE,
            )
            .map_err(|issue| error(&self.entry().item.path, issue.into()))
        })();
        if let Err(restore) = restored {
            append_error(&mut failure, "No-replace restoration", restore);
            return Ok(self.keep_recovery(&stage, receipt, failure));
        }
        self.parent.namespace_changed(name);
        stage.namespace_changed(name);
        if let Err(sync) = stage.native_file().sync_all() {
            append_error(
                &mut failure,
                "Restoration durability is unconfirmed",
                error(stage.path(), sync),
            );
        }
        failure
            .message
            .push_str("\nThe source name was restored without replacement.");
        let (recovery, cleanup) = self.clean_stage(stage, change);
        if let Some(cleanup) = cleanup {
            append_error(&mut failure, "Temporary directory cleanup", cleanup);
        }
        // Staging and restoration can advance ctime even for the original inode.
        // Cache the explicit post-mutation outcome with the original witnesses.
        Ok(TransferDirectoryLinkRemoval::RecoveryRequired {
            recovery: recovery.into_iter().collect(),
            error: failure,
        })
    }

    fn keep_recovery(
        &self,
        stage: &destination::Directory,
        receipt: TransferRecovery,
        mut failure: FsError,
    ) -> TransferDirectoryLinkRemoval {
        if let Err(sync) = stage
            .native_file()
            .sync_all()
            .and_then(|()| self.parent.native_file().sync_all())
        {
            append_error(
                &mut failure,
                "Recovery durability is unconfirmed",
                error(&receipt.path, sync),
            );
        }
        TransferDirectoryLinkRemoval::RecoveryRequired {
            recovery: vec![receipt],
            error: failure,
        }
    }

    fn failed_with_empty_stage(
        &self,
        stage: destination::Directory,
        mut failure: FsError,
        change: &mut namespace::Change<'_>,
    ) -> Result<TransferDirectoryLinkRemoval, FsError> {
        let (recovery, cleanup) = self.clean_stage(stage, change);
        if let Some(cleanup) = cleanup {
            append_error(&mut failure, "Temporary directory cleanup", cleanup);
        }
        match recovery {
            Some(receipt) => Ok(TransferDirectoryLinkRemoval::RecoveryRequired {
                recovery: vec![receipt],
                error: failure,
            }),
            None => Err(failure),
        }
    }

    fn clean_stage(
        &self,
        stage: destination::Directory,
        change: &mut namespace::Change<'_>,
    ) -> (Option<TransferRecovery>, Option<FsError>) {
        match self.parent.remove_empty_stage_checked(stage, change) {
            Ok(()) => (None, None),
            Err(StageRemovalError::Unsynced(failure)) => (None, Some(failure)),
            Err(StageRemovalError::Retained(failure, stage)) => {
                (Some(stage_receipt(&stage)), Some(failure))
            }
        }
    }

    fn stage_creation_failure(
        &self,
        mut failure: FsError,
    ) -> Result<TransferDirectoryLinkRemoval, FsError> {
        let Some(receipt) = stage_creation_receipt(&self.parent, &mut failure) else {
            return Err(failure);
        };
        Ok(TransferDirectoryLinkRemoval::RecoveryRequired {
            recovery: vec![receipt],
            error: failure,
        })
    }
}

pub(super) fn stage_creation_receipt(
    parent: &destination::Directory,
    failure: &mut FsError,
) -> Option<TransferRecovery> {
    let path = &failure.path;
    let name = path
        .file_name()
        .filter(|name| name.as_bytes().starts_with(b".midnight-move-"))?;
    if path.parent() != Some(parent.path()) {
        return None;
    }
    // Creation already tried to clean its private name. A failed recheck
    // leaves explicitly uncertain recovery information.
    let identity = match parent.inspect(name, &Cancellation::default()) {
        Ok(Some(entry)) => Some(entry.identity),
        Ok(None) => return None,
        Err(lookup) => {
            append_error(failure, "Rechecking the private recovery name", lookup);
            None
        }
    };
    Some(TransferRecovery {
        path: failure.path.clone(),
        identity,
    })
}

fn stage_receipt(stage: &destination::Directory) -> TransferRecovery {
    TransferRecovery {
        path: stage.path().to_owned(),
        identity: Some(stage.identity()),
    }
}

fn append_error(failure: &mut FsError, context: &str, issue: FsError) {
    failure.message.push_str(&format!("\n{context}. {issue}"));
}
