//! Same-volume namespace moves. No content reads, staging copy, or checksum work.
use super::*;
use midnight_core::{TransferNativeMoveOutcome as Outcome, TransferReplacementPolicy};
use std::os::windows::io::AsRawHandle;
use windows_sys::{
    Wdk::Storage::FileSystem::{FILE_NON_DIRECTORY_FILE, FILE_OPEN},
    Win32::Foundation::ERROR_NOT_SAME_DEVICE,
};

pub(super) fn attempt(
    capture: &Capture,
    target: &dyn TransferDirectory,
    name: &OsStr,
    replace: Option<(Arc<dyn TransferTarget>, TransferReplacementPolicy)>,
    cancel: &Cancellation,
) -> Result<Outcome, FsError> {
    attempt_with(capture, target, name, replace, cancel, || {})
}

pub(super) fn attempt_with(
    capture: &Capture,
    target: &dyn TransferDirectory,
    name: &OsStr,
    replace: Option<(Arc<dyn TransferTarget>, TransferReplacementPolicy)>,
    cancel: &Cancellation,
    after_capture: impl FnOnce(),
) -> Result<Outcome, FsError> {
    let mut state = capture.state.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(outcome) = &state.native_outcome {
        return Ok(outcome.clone());
    }
    if state.outcome.is_some() {
        return Err(invalid(
            &capture.entry.item.path,
            "source already entered verified Move recovery",
        ));
    }
    checked(cancel, &capture.entry.item.path)?;
    validate_name(name).map_err(|e| error(target.path(), e))?;
    capture.expected.revalidate(cancel)?;
    let directory = pin_destination(target, cancel)?;
    if capture.entry.identity.device != directory.identity().device {
        return Ok(Outcome::CrossDevice);
    }
    let same_parent = capture.parent.identity() == directory.identity();
    if same_parent && name == capture.name {
        return Ok(Outcome::SameEntry);
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
    let outcome = if let Some((expected, policy)) = replace {
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
            expected,
            policy,
            cancel,
            after_capture,
        )?
    } else {
        if existing.is_some() && !same_object {
            return Err(collision(
                &directory.path().join(name),
                "destination already exists",
            ));
        }
        capture.expected.revalidate(cancel)?;
        directory.revalidate(cancel)?;
        checked(cancel, &capture.entry.item.path)?;
        let moved = if same_parent {
            native::rename(&capture.file, name)
        } else {
            destination::recovery::rename_into(&capture.file, directory.native_file(), name)
        };
        match moved {
            Ok(()) => Outcome::Moved {
                destination: directory.path().join(name),
                recovery: Vec::new(),
                durability_confirmed: false,
            },
            Err(failure) if failure.raw_os_error() == Some(ERROR_NOT_SAME_DEVICE as i32) => {
                return Ok(Outcome::CrossDevice);
            }
            Err(failure) => return Err(error(&directory.path().join(name), failure)),
        }
    };
    // Cancellation after a successful rename cannot erase the committed namespace fact.
    state.native_outcome = Some(outcome.clone());
    Ok(outcome)
}

fn pin_destination(
    target: &dyn TransferDirectory,
    cancel: &Cancellation,
) -> Result<destination::Directory, FsError> {
    target.revalidate(cancel)?;
    let path = target.path();
    let file = OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES | FILE_TRAVERSE)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|e| error(path, e))?;
    if identity(&file, path)? != target.identity()
        || file
            .metadata()
            .map_err(|e| error(path, e))?
            .file_attributes()
            & FILE_ATTRIBUTE_REPARSE_POINT
            != 0
    {
        return Err(invalid(path, "destination authority changed while pinning"));
    }
    target.revalidate(cancel)?;
    destination::Directory::from_file(file, path.to_owned())
}

fn exact_alias(
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
    // this ambiguous case needs directory-name enumeration, never file content.
    let held = OpenOptions::new()
        .access_mode(FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(directory.path())
        .map_err(|e| error(directory.path(), e))?;
    if identity(&held, directory.path())? != directory.identity() {
        return Err(invalid(
            directory.path(),
            "case-alias directory changed before enumeration",
        ));
    }
    native::has_exact_child_name(&held, name, cancel).map_err(|e| error(directory.path(), e))
}

fn collision(path: &Path, message: &str) -> FsError {
    error(path, io::Error::new(io::ErrorKind::AlreadyExists, message))
}

fn replace_existing(
    capture: &Capture,
    directory: &destination::Directory,
    name: &OsStr,
    expected: Arc<dyn TransferTarget>,
    policy: TransferReplacementPolicy,
    cancel: &Cancellation,
    after_capture: impl FnOnce(),
) -> Result<Outcome, FsError> {
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
    capture.expected.revalidate(cancel)?;
    let recovery = destination::recovery::native_move_recovery(directory, cancel)?;
    let original_path = recovery.path().join("original");
    let original_receipt = TransferRecovery {
        path: original_path,
        identity: Some(expected.entry().identity),
    };
    let capture_original = checked(cancel, &path).and_then(|()| {
        destination::recovery::rename_into(
            &original,
            recovery.native_file(),
            OsStr::new("original"),
        )
        .map_err(|e| error(&path, e))
    });
    if let Err(failure) = capture_original {
        return cleanup_empty(recovery, failure, false);
    }
    after_capture();
    let publish = capture
        .expected
        .revalidate(cancel)
        .and_then(|()| directory.revalidate(cancel))
        .and_then(|()| checked(cancel, &path))
        .and_then(|()| {
            destination::recovery::rename_into(&capture.file, directory.native_file(), name)
                .map_err(|e| error(&path, e))
        });
    if let Err(mut failure) = publish {
        // Restore only into an absent final name. A newly created winner survives.
        if let Err(restore) =
            destination::recovery::rename_into(&original, directory.native_file(), name)
        {
            failure
                .message
                .push_str(&format!("; original retained for recovery: {restore}"));
            return Ok(Outcome::RecoveryRequired {
                destination: None,
                recovery: vec![original_receipt],
                error: failure,
            });
        }
        drop(original);
        return cleanup_empty(recovery, failure, true);
    }
    Ok(Outcome::Moved {
        destination: path,
        recovery: vec![original_receipt],
        durability_confirmed: false,
    })
}

fn cleanup_empty(
    directory: destination::Directory,
    mut failure: FsError,
    restored: bool,
) -> Result<Outcome, FsError> {
    let receipt = TransferRecovery {
        path: directory.path().to_owned(),
        identity: Some(directory.identity()),
    };
    match directory.remove_empty_stage() {
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
