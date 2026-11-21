//! Owner write for one change of a folder of the moved tree. An entry leaves a folder,
//! and a folder moves into another folder (its `..` entry changes), only while that
//! folder is writable. A Move changes this only on folders of the moved tree that the
//! current user owns (the selected folder and the folders below it), never on the
//! folder that contains the selection, a folder another user owns, or a link, as
//! Delete does inside its confirmed tree. The permission is added through a retained
//! descriptor, never a path, for exactly one change, and the folder gets its own mode
//! back as soon as that change returns, wherever the folder is by then (`restored`). If
//! that fails (the filesystem turned read-only or failed), the folder keeps owner write
//! and the change's outcome says so, naming the folder. A folder whose set-group-ID bit
//! the kernel would clear when its mode changes is never changed.
use super::*;
use midnight_core::{
    TransferDirectoryLinkRemoval, TransferDirectoryRemoval, TransferNativeMoveOutcome,
};
use rustix::fs::{FileType, RawMode};
use std::os::fd::AsRawFd;

/// The permission that adding or removing a folder's entries needs, besides search.
const OWNER_WRITE: RawMode = 0o200;

#[cfg(test)]
thread_local! {
    /// The failure that the next `OwnerWrite::restore` of this thread reports instead of
    /// changing the mode (tests of a filesystem that turned read-only).
    pub(super) static FAIL_RESTORE: std::cell::Cell<Option<rustix::io::Errno>> =
        const { std::cell::Cell::new(None) };
}

/// Owner write added to a folder for one change. `restore` gives the folder its own mode
/// back and says whether that worked; one dropped without it (a change that unwound)
/// gets its mode back without a report.
pub(super) struct OwnerWrite<'a> {
    folder: &'a File,
    /// Where the folder was when the change began, to name it in a failure.
    path: &'a Path,
    mode: RawMode,
    /// `restore` gave the mode back, or reported why not.
    restored: bool,
}

impl<'a> OwnerWrite<'a> {
    /// Adds owner write to `folder`, found at `path`, until `restore`, when the current
    /// user owns the folder and it lacks owner write (`lacking_owner_write`). Anything
    /// else, and a folder whose mode cannot be changed, stays as it is, so the change it
    /// was wanted for fails as it would have without it.
    pub(super) fn grant(folder: &'a File, path: &'a Path) -> Option<Self> {
        let mode = lacking_owner_write(folder)?;
        chmod_retained(folder, mode | OWNER_WRITE).ok()?;
        Some(Self {
            folder,
            path,
            mode,
            restored: false,
        })
    }

    /// Gives the folder its own mode back now. A failure leaves the folder with owner
    /// write; the error names the folder, for the outcome of the change to report.
    pub(super) fn restore(mut self) -> Result<(), FsError> {
        self.restored = true;
        #[cfg(test)]
        if let Some(failure) = FAIL_RESTORE.take() {
            return Err(restore_error(self.path, self.mode, failure));
        }
        chmod_retained(self.folder, self.mode)
            .map_err(|failure| restore_error(self.path, self.mode, failure))
    }
}

impl Drop for OwnerWrite<'_> {
    fn drop(&mut self) {
        if !self.restored {
            // The folder is ours and its mode was just changed the same way; nothing is
            // left to report a failure to.
            let _ = chmod_retained(self.folder, self.mode);
        }
    }
}

/// Gives each folder that `grants` made writable its own mode back, the first given
/// first, and adds the failures to `result`, the result of the change they were made
/// writable for (`Outcome`).
pub(super) fn restored<'a, T: Outcome>(
    result: Result<T, FsError>,
    grants: impl IntoIterator<Item = Option<OwnerWrite<'a>>>,
) -> Result<T, FsError> {
    let Some(failure) = restore_all(grants) else {
        return result;
    };
    match result {
        Ok(outcome) => outcome.with_restore_failure(failure),
        Err(error) => Err(with_restore_failure(error, failure)),
    }
}

/// Gives each folder that `grants` made writable its own mode back, the first given
/// first: the failures, one after the other, or None.
pub(super) fn restore_all<'a>(
    grants: impl IntoIterator<Item = Option<OwnerWrite<'a>>>,
) -> Option<FsError> {
    grants
        .into_iter()
        .flatten()
        .filter_map(|grant| grant.restore().err())
        .reduce(with_restore_failure)
}

/// `error`, followed by the failure to give a folder its own mode back.
pub(super) fn with_restore_failure(mut error: FsError, failure: FsError) -> FsError {
    error.message.push_str(&format!("; {failure}"));
    error
}

/// The outcome of a change that made a folder writable, to which a failure to give it
/// its own mode back is added: the same facts with the failure as (part of) their error,
/// or the failure alone where the outcome has no error to carry it. Its message says
/// what stays, the folder's owner write; the change itself was made.
pub(super) trait Outcome: Sized {
    fn with_restore_failure(self, failure: FsError) -> Result<Self, FsError>;
}

/// A stage's removal or a restoration that completed.
impl Outcome for () {
    fn with_restore_failure(self, failure: FsError) -> Result<Self, FsError> {
        Err(failure)
    }
}

/// An emptied folder that was removed.
impl Outcome for TransferDirectoryRemoval {
    fn with_restore_failure(self, failure: FsError) -> Result<Self, FsError> {
        Err(failure)
    }
}

/// A capture: what it staged stays the recovery of the source, as for any failure after
/// a capture, so the source is put back instead of moved further.
pub(super) fn captured_with_restore_failure(
    outcome: TransferMoveCaptureOutcome,
    failure: FsError,
) -> TransferMoveCaptureOutcome {
    match outcome {
        TransferMoveCaptureOutcome::Captured { source, recovery } => {
            TransferMoveCaptureOutcome::RecoveryRequired {
                source,
                recovery: vec![recovery],
                error: failure,
            }
        }
        TransferMoveCaptureOutcome::Restored { source, error } => {
            TransferMoveCaptureOutcome::Restored {
                source,
                error: with_restore_failure(error, failure),
            }
        }
        TransferMoveCaptureOutcome::RecoveryRequired {
            source,
            recovery,
            error,
        } => TransferMoveCaptureOutcome::RecoveryRequired {
            source,
            recovery,
            error: with_restore_failure(error, failure),
        },
    }
}

/// A native Move: one that reached its destination stays there and asks, as one whose
/// names need recovery; one that changed nothing fails.
impl Outcome for TransferNativeMoveOutcome {
    fn with_restore_failure(self, failure: FsError) -> Result<Self, FsError> {
        match self {
            Self::Moved {
                destination,
                recovery,
                durability_confirmed,
            } => {
                let error = if durability_confirmed {
                    failure
                } else {
                    let mut failure = failure;
                    failure
                        .message
                        .push_str("; saving the moved names is unconfirmed too");
                    failure
                };
                Ok(Self::RecoveryRequired {
                    destination: Some(destination),
                    recovery,
                    error,
                })
            }
            Self::RecoveryRequired {
                destination,
                recovery,
                error,
            } => Ok(Self::RecoveryRequired {
                destination,
                recovery,
                error: with_restore_failure(error, failure),
            }),
            Self::SameEntry | Self::CrossDevice | Self::Unsupported => Err(failure),
        }
    }
}

/// The removal of a link's original, which reports its own error.
impl Outcome for TransferDirectoryLinkRemoval {
    fn with_restore_failure(self, failure: FsError) -> Result<Self, FsError> {
        Ok(match self {
            Self::Removed {
                state,
                durability_confirmed,
                recovery,
                error,
            } => Self::Removed {
                state,
                durability_confirmed,
                recovery,
                error: Some(match error {
                    Some(error) => with_restore_failure(error, failure),
                    None => failure,
                }),
            },
            Self::RecoveryRequired { recovery, error } => Self::RecoveryRequired {
                recovery,
                error: with_restore_failure(error, failure),
            },
        })
    }
}

/// Why `path`, made writable with owner write for a change, kept it: its own `mode`
/// could not be set again. Its kind is Other, whatever the system call reported, so it
/// is never taken for a cancellation or a refusal of the change itself.
fn restore_error(path: &Path, mode: RawMode, failure: rustix::io::Errno) -> FsError {
    let failure = io::Error::from(failure);
    move_error(
        path,
        io::Error::other(format!(
            "This folder got owner write permission for a change of the Move, and setting \
             its own mode {mode:04o} again failed: {failure}. It keeps owner write \
             permission; remove it yourself if the folder should stay write-protected."
        )),
    )
}

/// The permission bits of `folder` when it is a folder that the current user owns and
/// that lacks owner write, and whose set-group-ID bit survives a change of its mode
/// (`crate::ownership::keeps_set_group_id`). Never for the superuser, whom permissions
/// do not bind.
pub(super) fn lacking_owner_write(folder: &File) -> Option<RawMode> {
    let caller = rustix::process::geteuid();
    if caller.is_root() {
        return None;
    }
    let stat = rustix::fs::fstat(folder).ok()?;
    let mode = stat.st_mode & 0o7777;
    (FileType::from_raw_mode(stat.st_mode) == FileType::Directory
        && stat.st_uid == caller.as_raw()
        && mode & OWNER_WRITE == 0
        && crate::ownership::keeps_set_group_id(
            mode,
            stat.st_gid,
            crate::ownership::caller_in_group,
        ))
    .then_some(mode)
}

/// `fchmod` refuses the path-only descriptor that a Move retains for a folder it moves.
/// Its /proc entry names exactly that inode, never a path another process could replace.
fn chmod_retained(file: &File, mode: RawMode) -> rustix::io::Result<()> {
    rustix::fs::chmod(
        format!("/proc/self/fd/{}", file.as_raw_fd()).as_str(),
        Mode::from_raw_mode(mode),
    )
}
