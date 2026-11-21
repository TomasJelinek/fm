//! Same-volume namespace moves. No content reads, staging copy, or checksum work.
use super::*;
use midnight_core::{
    DeferredNativeMove, TransferNativeMoveOutcome as Outcome, TransferReplacementPolicy,
};
use std::os::windows::io::AsRawHandle;
use windows_sys::{
    Wdk::Storage::FileSystem::{FILE_NON_DIRECTORY_FILE, FILE_OPEN},
    Win32::Foundation::ERROR_NOT_SAME_DEVICE,
};

/// The native rename, its folders saved before it returns.
pub(super) fn attempt(
    capture: &Capture,
    target: &dyn TransferDirectory,
    name: &OsStr,
    replace: Option<(Arc<dyn TransferTarget>, TransferReplacementPolicy)>,
    cancel: &Cancellation,
) -> Result<Outcome, FsError> {
    attempt_with(capture, target, name, replace, cancel, || {})
}

/// The native rename, its folders saved by the returned confirmation, which the engine
/// runs together with the job's other renames (`folder_flush::settle`).
pub(super) fn attempt_deferred(
    capture: &Capture,
    target: &dyn TransferDirectory,
    name: &OsStr,
    replace: Option<(Arc<dyn TransferTarget>, TransferReplacementPolicy)>,
    cancel: &Cancellation,
) -> Result<DeferredNativeMove, FsError> {
    rename(capture, target, name, replace, cancel, || {}, true)
}

pub(super) fn attempt_with(
    capture: &Capture,
    target: &dyn TransferDirectory,
    name: &OsStr,
    replace: Option<(Arc<dyn TransferTarget>, TransferReplacementPolicy)>,
    cancel: &Cancellation,
    after_capture: impl FnOnce(),
) -> Result<Outcome, FsError> {
    rename(capture, target, name, replace, cancel, after_capture, false).map(|(outcome, _)| outcome)
}

fn rename(
    capture: &Capture,
    target: &dyn TransferDirectory,
    name: &OsStr,
    replace: Option<(Arc<dyn TransferTarget>, TransferReplacementPolicy)>,
    cancel: &Cancellation,
    after_capture: impl FnOnce(),
    deferred: bool,
) -> Result<DeferredNativeMove, FsError> {
    let mut state = capture.state.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(outcome) = &state.native_outcome {
        return Ok((outcome.clone(), None));
    }
    if state.outcome.is_some() {
        return Err(invalid(
            &capture.entry.item.path,
            "source already entered verified Move recovery",
        ));
    }
    checked(cancel, &capture.entry.item.path)?;
    validate_name(name).map_err(|e| error(target.path(), e))?;
    capture.revalidate_source(cancel)?;
    // A destination reached through a volume mount point or a junction to another volume
    // is decided before pinning, so it enters the verified cross-device route.
    target.revalidate(cancel)?;
    if capture.entry.identity.device != target.identity().device {
        return Ok((Outcome::CrossDevice, None));
    }
    let directory = pin_destination(target, cancel)?;
    let same_parent = capture.parent.identity() == directory.identity();
    if same_parent && name == capture.name {
        return Ok((Outcome::SameEntry, None));
    }
    directory.reject_source_ancestor(capture.entry.identity, cancel)?;
    let existing = directory.inspect(name, cancel)?;
    let same_object = existing
        .as_ref()
        .is_some_and(|entry| entry.identity == capture.entry.identity);
    if same_object && (!same_parent || exact_alias(&directory, &capture.file, name, cancel)?) {
        return Err(collision(
            &directory.path().join(name),
            "destination is a distinct hard-link alias",
        ));
    }
    // The private folder that keeps a replaced original, saved with the others below.
    let mut kept = None;
    let outcome = super::super::namespace::change(cancel, |mutation| {
        mutation.prepare_rename(&capture.file, directory.native_file(), name, cancel)?;
        if let Some((expected, policy)) = replace {
            if same_object {
                return Err(collision(
                    &directory.path().join(name),
                    "cannot replace the source itself",
                ));
            }
            replace_existing(
                capture,
                &directory,
                name,
                (expected, policy),
                cancel,
                after_capture,
                mutation,
            )
            .map(|(outcome, recovery)| {
                kept = recovery;
                outcome
            })
        } else {
            if existing.is_some() && !same_object {
                return Err(collision(
                    &directory.path().join(name),
                    "destination already exists",
                ));
            }
            capture.revalidate_source(cancel)?;
            directory.revalidate(cancel)?;
            checked(cancel, &capture.entry.item.path)?;
            let moved = mutation.rename(&capture.file, directory.native_file(), name, cancel);
            match moved {
                Ok(()) => Ok(Outcome::Moved {
                    destination: directory.path().join(name),
                    recovery: Vec::new(),
                    // Unless saving a folder it changed fails (`folder_flush::settle`).
                    durability_confirmed: true,
                }),
                Err(failure) if failure.raw_os_error() == Some(ERROR_NOT_SAME_DEVICE as i32) => {
                    Ok(Outcome::CrossDevice)
                }
                Err(failure) => Err(error(&directory.path().join(name), failure)),
            }
        }
    })?;
    if matches!(outcome, Outcome::CrossDevice) {
        return Ok((outcome, None));
    }
    // The rename removed the name from the source folder and added it to the destination
    // (a replacement also put the original in its private folder): save them, outside the
    // namespace turn, before the rename is reported or when the engine confirms it.
    let folders = kept
        .into_iter()
        .chain([directory, capture.parent.clone()])
        .map(super::folder_flush::Held::Directory)
        .collect();
    let (outcome, confirmation) = super::folder_flush::settle(outcome, folders, deferred);
    // Cancellation after a successful rename cannot erase the committed namespace fact.
    state.native_outcome = Some(outcome.clone());
    Ok((outcome, confirmation))
}

pub(super) fn pin_destination(
    target: &dyn TransferDirectory,
    cancel: &Cancellation,
) -> Result<destination::Directory, FsError> {
    target.revalidate(cancel)?;
    let path = target.path();
    // The prepared destination identity was captured through a following open (a panel
    // may sit inside a junction or mount point). Open the same way; the identity match
    // proves this is exactly the prepared directory object, never a substituted target.
    let file = OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES | FILE_TRAVERSE)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .map_err(|e| error(path, e))?;
    if identity(&file, path)? != target.identity() {
        return Err(invalid(path, "destination authority changed while pinning"));
    }
    target.revalidate(cancel)?;
    destination::Directory::from_file(file, path.to_owned())
}

pub(super) fn exact_alias(
    directory: &destination::Directory,
    source: &File,
    name: &OsStr,
    cancel: &Cancellation,
) -> Result<bool, FsError> {
    let mut information = FILE_STANDARD_INFO::default();
    // SAFETY: live source handle and correctly sized writable metadata structure.
    if unsafe {
        GetFileInformationByHandleEx(
            source.as_raw_handle(),
            FileStandardInfo,
            (&mut information as *mut FILE_STANDARD_INFO).cast(),
            size_of::<FILE_STANDARD_INFO>() as u32,
        )
    } == 0
    {
        return Err(error(directory.path(), io::Error::last_os_error()));
    }
    if information.NumberOfLinks <= 1 {
        return Ok(false);
    }
    // A file ID identifies the object, not its individual hard-link names. Only
    // this ambiguous case needs directory-name enumeration, never file content. Reopen
    // the pinned directory object itself, never its (possibly junction) display path.
    let held = native::reopen(
        directory.native_file(),
        FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
    )
    .map_err(|e| error(directory.path(), e))?;
    if identity(&held, directory.path())? != directory.identity() {
        return Err(invalid(
            directory.path(),
            "case-alias directory changed before enumeration",
        ));
    }
    native::has_exact_child_name(&held, name, cancel).map_err(|e| error(directory.path(), e))
}

pub(super) fn collision(path: &Path, message: &str) -> FsError {
    error(path, io::Error::new(io::ErrorKind::AlreadyExists, message))
}

/// A committed replacement also returns the private folder now holding the displaced
/// original, to be saved after the namespace turn with the other folders it changed.
fn replace_existing(
    capture: &Capture,
    directory: &destination::Directory,
    name: &OsStr,
    replacement: (Arc<dyn TransferTarget>, TransferReplacementPolicy),
    cancel: &Cancellation,
    after_capture: impl FnOnce(),
    mutation: &mut super::super::namespace::Mutation<'_>,
) -> Result<(Outcome, Option<destination::Directory>), FsError> {
    let (expected, policy) = replacement;
    let path = directory.path().join(name);
    if policy != TransferReplacementPolicy::RecoverableCurrentEntry {
        return Err(unsupported(
            &path,
            "native replacement requires explicit recoverable-current-entry consent",
        ));
    }
    if capture.entry.item.kind != EntryKind::File
        || expected.entry().item.kind != EntryKind::File
        || expected.entry().item.path != path
    {
        return Err(collision(
            &path,
            "replacement requires the exact consented regular file",
        ));
    }
    // Match the existing recoverable destination-publication capability. The
    // retained original goes into a newly created private, pinned directory.
    if !destination::recovery::supported(directory) {
        return Err(unsupported(
            &path,
            "recoverable native replacement requires local fixed NTFS",
        ));
    }
    checked(cancel, &path)?;
    expected
        .revalidate(cancel)
        .map_err(|e| collision(&path, &format!("replacement consent changed: {e}")))?;
    let original = native::open_relative(
        directory.native_file(),
        name,
        FILE_READ_ATTRIBUTES | DELETE | SYNCHRONIZE,
        FILE_SHARE_READ,
        FILE_OPEN,
        FILE_NON_DIRECTORY_FILE,
    )
    .map_err(|e| error(&path, e))?;
    if identity(&original, &path)? != expected.entry().identity {
        return Err(collision(
            &path,
            "replacement destination changed before commit",
        ));
    }
    expected
        .revalidate(cancel)
        .map_err(|e| collision(&path, &format!("replacement consent changed: {e}")))?;
    capture.revalidate_source(cancel)?;
    let recovery = destination::recovery::native_move_recovery_in(directory, cancel, mutation)?;
    let original_path = recovery.path().join("original");
    let original_receipt = TransferRecovery {
        path: original_path,
        identity: Some(expected.entry().identity),
    };
    let capture_original = checked(cancel, &path).and_then(|()| {
        mutation
            .rename(
                &original,
                recovery.native_file(),
                OsStr::new("original"),
                cancel,
            )
            .map_err(|e| error(&path, e))
    });
    if let Err(failure) = capture_original {
        return cleanup_empty(recovery, failure, false, mutation).map(|outcome| (outcome, None));
    }
    after_capture();
    let publish = capture
        .expected
        .revalidate(cancel)
        .and_then(|()| directory.revalidate(cancel))
        .and_then(|()| checked(cancel, &path))
        .and_then(|()| {
            mutation
                .rename(&capture.file, directory.native_file(), name, cancel)
                .map_err(|e| error(&path, e))
        });
    if let Err(mut failure) = publish {
        // Restore only into an absent final name. A newly created winner survives.
        if let Err(restore) = mutation.rename(
            &original,
            directory.native_file(),
            name,
            &Cancellation::default(),
        ) {
            failure
                .message
                .push_str(&format!("; original retained for recovery: {restore}"));
            let outcome = Outcome::RecoveryRequired {
                destination: None,
                recovery: vec![original_receipt],
                error: failure,
            };
            return Ok((outcome, None));
        }
        drop(original);
        return cleanup_empty(recovery, failure, true, mutation).map(|outcome| (outcome, None));
    }
    let outcome = Outcome::Moved {
        destination: path,
        recovery: vec![original_receipt],
        // Unless saving a folder it changed fails (`folder_flush::settle`).
        durability_confirmed: true,
    };
    Ok((outcome, Some(recovery)))
}

fn cleanup_empty(
    directory: destination::Directory,
    mut failure: FsError,
    restored: bool,
    mutation: &mut super::super::namespace::Mutation<'_>,
) -> Result<Outcome, FsError> {
    let receipt = TransferRecovery {
        path: directory.path().to_owned(),
        identity: Some(directory.identity()),
    };
    match directory.remove_empty_stage_in(mutation) {
        Ok(()) if restored => Ok(Outcome::RecoveryRequired {
            destination: None,
            recovery: Vec::new(),
            error: failure,
        }),
        Ok(()) => Err(failure),
        Err(cleanup) => {
            failure.message.push_str(&format!(
                "; empty recovery directory cleanup failed: {cleanup}"
            ));
            Ok(Outcome::RecoveryRequired {
                destination: None,
                recovery: vec![receipt],
                error: failure,
            })
        }
    }
}
