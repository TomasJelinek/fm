//! Metadata-only native relocation. Private capture preserves the selected inode;
//! all failures after capture carry explicit namespace facts, never copy fallback.
use super::*;
use midnight_core::{TransferNativeMoveOutcome, TransferReplacementPolicy};

type Replacement = Option<(Arc<dyn TransferTarget>, TransferReplacementPolicy)>;

pub(super) fn attempt(
    capture: &Capture,
    destination: &dyn TransferDirectory,
    name: &OsStr,
    replace: Replacement,
    cancel: &Cancellation,
) -> Result<TransferNativeMoveOutcome, FsError> {
    attempt_with(capture, destination, name, replace, cancel, || {}, || {})
}

fn attempt_with(
    capture: &Capture,
    destination: &dyn TransferDirectory,
    name: &OsStr,
    replace: Replacement,
    cancel: &Cancellation,
    before_capture: impl FnOnce(),
    before_publish: impl FnOnce(),
) -> Result<TransferNativeMoveOutcome, FsError> {
    execute(
        capture,
        destination,
        name,
        replace,
        cancel,
        (before_capture, before_publish),
        sync_native,
    )
}

fn execute(
    capture: &Capture,
    destination: &dyn TransferDirectory,
    name: &OsStr,
    replace: Replacement,
    cancel: &Cancellation,
    hooks: (impl FnOnce(), impl FnOnce()),
    sync: impl FnOnce(&Capture, &destination::Directory, &destination::Directory) -> Result<(), FsError>,
) -> Result<TransferNativeMoveOutcome, FsError> {
    let (before_capture, before_publish) = hooks;
    let mut retained = capture.native.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(outcome) = retained.as_ref() {
        return Ok(outcome.clone());
    }
    validate_name(name, destination.path())?;
    capture.revalidate(cancel)?;
    if capture
        .state
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .outcome
        .is_some()
    {
        return Err(invalid(
            &capture.entry.item.path,
            "source already entered verified Move staging",
        ));
    }
    let target = bind_destination(destination, cancel)?;
    if target.identity() == capture.parent.identity() && name == capture.name {
        return Ok(TransferNativeMoveOutcome::SameEntry);
    }
    if capture.entry.identity.device != target.identity().device
        || different_mount(
            capture.parent.native_file(),
            target.native_file(),
            target.path(),
        )?
    {
        return Ok(TransferNativeMoveOutcome::CrossDevice);
    }
    qualify(&target)?;
    target.check_recovery_parent(cancel)?;
    if capture.entry.item.kind == EntryKind::Directory {
        target.reject_source_ancestor(capture.entry.identity, cancel)?;
    }
    let replaced = validate_target(capture, &target, name, replace.as_ref(), cancel)?;
    capture.revalidate(cancel)?;
    before_capture();
    checked(cancel, &capture.entry.item.path)?;
    let stage = capture.parent.source_stage(cancel)?;
    let captured = renameat_with(
        capture.parent.native_file(),
        &capture.name,
        stage.native_file(),
        &capture.name,
        RenameFlags::NOREPLACE,
    );
    if let Err(failure) = captured {
        let failure = error(&capture.entry.item.path, failure.into());
        return match capture.parent.remove_empty_stage(stage) {
            Ok(()) => Err(failure),
            Err(cleanup) => {
                let outcome = TransferNativeMoveOutcome::RecoveryRequired {
                    destination: None,
                    recovery: vec![],
                    error: combined(failure, "empty staging cleanup", cleanup),
                };
                *retained = Some(outcome.clone());
                Ok(outcome)
            }
        };
    }
    // Once captured, neither cancellation nor any error may authorize copy fallback.
    let validated = (|| {
        let actual = stage.inspect(&capture.name, &Cancellation::default())?;
        if actual.as_ref().map(|entry| entry.identity) != Some(capture.entry.identity) {
            return Err(invalid(
                &capture.entry.item.path,
                "a different source was captured; its bytes were retained",
            ));
        }
        target.revalidate(cancel)?;
        validate_target(capture, &target, name, replace.as_ref(), cancel)?;
        checked(cancel, &capture.entry.item.path)
    })();
    if let Err(error) = validated {
        let outcome = restore(capture, stage, error);
        *retained = Some(outcome.clone());
        return Ok(outcome);
    }
    before_publish();
    if let Err(error) = checked(cancel, &capture.entry.item.path) {
        let outcome = restore(capture, stage, error);
        *retained = Some(outcome.clone());
        return Ok(outcome);
    }
    let flags = if replaced.is_some() {
        RenameFlags::EXCHANGE
    } else {
        RenameFlags::NOREPLACE
    };
    if let Err(failure) = renameat_with(
        stage.native_file(),
        &capture.name,
        target.native_file(),
        name,
        flags,
    ) {
        let outcome = restore(
            capture,
            stage,
            error(&target.path().join(name), failure.into()),
        );
        *retained = Some(outcome.clone());
        return Ok(outcome);
    }
    let outcome = committed(capture, &target, name, stage, replaced, sync);
    *retained = Some(outcome.clone());
    Ok(outcome)
}

fn bind_destination(
    expected: &dyn TransferDirectory,
    cancel: &Cancellation,
) -> Result<destination::Directory, FsError> {
    expected.revalidate(cancel)?;
    let file = File::from(
        openat(
            CWD,
            expected.path(),
            DIRECTORY | OFlags::NOFOLLOW,
            Mode::empty(),
        )
        .map_err(|e| error(expected.path(), e.into()))?,
    );
    if identity(&file, expected.path())? != expected.identity() {
        return Err(invalid(expected.path(), "pinned Move destination changed"));
    }
    expected.revalidate(cancel)?;
    destination::Directory::from_file(file, expected.path().to_owned())
}

fn different_mount(source: &File, destination: &File, path: &Path) -> Result<bool, FsError> {
    let a = super::super::native_location::mount(source, path)?;
    let b = super::super::native_location::mount(destination, path)?;
    Ok(matches!((a, b), (Some(a), Some(b)) if a != b))
}

pub(super) fn qualify(directory: &destination::Directory) -> Result<(), FsError> {
    // Native no-replace/exchange is qualified independently of the read-lease
    // proof used by cross-device Move. Remote/unknown filesystems are excluded:
    // their failed requests can have committed on a server despite the error.
    if super::super::native_location::local_filesystem(directory.native_file(), directory.path())? {
        Ok(())
    } else {
        Err(unsupported(
            directory.path(),
            "Native Move requires a local ext, XFS, Btrfs or tmpfs filesystem",
        ))
    }
}

fn validate_target(
    capture: &Capture,
    target: &destination::Directory,
    name: &OsStr,
    replace: Option<&(Arc<dyn TransferTarget>, TransferReplacementPolicy)>,
    cancel: &Cancellation,
) -> Result<Option<(TransferIdentity, destination::recovery::Revision)>, FsError> {
    let path = target.path().join(name);
    let current = target.prepare_target(name, cancel)?;
    let conflict = || {
        error(
            &path,
            io::Error::new(
                io::ErrorKind::AlreadyExists,
                "Move destination exists or changed; a fresh conflict decision is required",
            ),
        )
    };
    match (current, replace) {
        (None, None) => Ok(None),
        (Some(current), Some((expected, TransferReplacementPolicy::RecoverableCurrentEntry))) => {
            if capture.entry.item.kind != EntryKind::File
                || current.entry().item.kind != EntryKind::File
                || expected.entry().item.path != path
                || current.entry().identity != expected.entry().identity
                || current.entry().identity == capture.entry.identity
            {
                return Err(conflict());
            }
            expected.revalidate(cancel).map_err(|_| conflict())?;
            let file = File::from(
                openat2(
                    target.native_file(),
                    name,
                    OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                    Mode::empty(),
                    RESOLVE,
                )
                .map_err(|e| error(&path, e.into()))?,
            );
            let revision = destination::recovery::Revision::of(&file, &path)?;
            if identity(&file, &path)? != expected.entry().identity {
                return Err(conflict());
            }
            expected.revalidate(cancel).map_err(|_| conflict())?;
            Ok(Some((expected.entry().identity, revision)))
        }
        (_, Some((_, TransferReplacementPolicy::ConditionalIdentity))) => Err(unsupported(
            &path,
            "Linux native Move does not provide identity-conditional replacement",
        )),
        _ => Err(conflict()),
    }
}

fn receipt(stage: &destination::Directory, name: &OsStr) -> TransferRecovery {
    TransferRecovery {
        path: stage.path().join(name),
        identity: stage
            .inspect(name, &Cancellation::default())
            .ok()
            .flatten()
            .map(|e| e.identity),
    }
}

fn restore(
    capture: &Capture,
    stage: destination::Directory,
    mut failure: FsError,
) -> TransferNativeMoveOutcome {
    let recovery = match renameat_with(
        stage.native_file(),
        &capture.name,
        capture.parent.native_file(),
        &capture.name,
        RenameFlags::NOREPLACE,
    ) {
        Ok(()) => {
            failure
                .message
                .push_str("; source name restored without replacement");
            if let Err(sync) = capture.sync_names(&stage) {
                failure = combined(failure, "restored-name durability", sync);
            }
            if let Err(cleanup) = capture.parent.remove_empty_stage(stage) {
                failure = combined(failure, "empty staging cleanup", cleanup);
            }
            vec![]
        }
        Err(restore) => {
            failure.message.push_str(&format!(
                "; restoration refused without replacement: {restore}"
            ));
            vec![receipt(&stage, &capture.name)]
        }
    };
    TransferNativeMoveOutcome::RecoveryRequired {
        destination: None,
        recovery,
        error: failure,
    }
}

fn committed(
    capture: &Capture,
    target: &destination::Directory,
    name: &OsStr,
    stage: destination::Directory,
    replaced: Option<(TransferIdentity, destination::recovery::Revision)>,
    sync: impl FnOnce(&Capture, &destination::Directory, &destination::Directory) -> Result<(), FsError>,
) -> TransferNativeMoveOutcome {
    let path = target.path().join(name);
    let cleanup = Cancellation::default();
    let published = target.inspect(name, &cleanup);
    let destination = published
        .as_ref()
        .ok()
        .and_then(|entry| entry.as_ref())
        .filter(|entry| entry.identity == capture.entry.identity)
        .map(|_| path.clone());
    let recovery = replaced
        .map(|_| receipt(&stage, &capture.name))
        .into_iter()
        .collect::<Vec<_>>();
    let checked = (|| {
        if destination.is_none() {
            return Err(published.err().unwrap_or_else(|| {
                invalid(
                    &path,
                    "native Move completed but the destination identity changed",
                )
            }));
        }
        if let Some((expected, revision)) = replaced {
            let file = File::from(
                openat2(
                    stage.native_file(),
                    &capture.name,
                    OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                    Mode::empty(),
                    RESOLVE,
                )
                .map_err(|e| error(stage.path(), e.into()))?,
            );
            if identity(&file, stage.path())? != expected
                || !revision
                    .after_rename_matches(destination::recovery::Revision::of(&file, stage.path())?)
            {
                return Err(invalid(
                    &path,
                    "a concurrent destination change was retained in recovery",
                ));
            }
        }
        Ok(())
    })();
    if let Err(error) = checked {
        return TransferNativeMoveOutcome::RecoveryRequired {
            destination,
            recovery,
            error,
        };
    }
    let mut failure = sync(capture, &stage, target).err();
    if replaced.is_none()
        && let Err(cleanup) = capture.parent.remove_empty_stage(stage)
    {
        failure = Some(match failure {
            Some(error) => combined(error, "empty staging cleanup", cleanup),
            None => cleanup,
        });
    }
    if let Some(error) = failure {
        return TransferNativeMoveOutcome::RecoveryRequired {
            destination,
            recovery,
            error,
        };
    }
    TransferNativeMoveOutcome::Moved {
        destination: path,
        recovery,
        durability_confirmed: true,
    }
}

fn sync_native(
    capture: &Capture,
    stage: &destination::Directory,
    target: &destination::Directory,
) -> Result<(), FsError> {
    sync_each([stage, &capture.parent, target], |directory| {
        directory
            .native_file()
            .sync_all()
            .map_err(|failure| error(directory.path(), failure))
    })
}

fn sync_each(
    directories: [&destination::Directory; 3],
    mut synchronize: impl FnMut(&destination::Directory) -> Result<(), FsError>,
) -> Result<(), FsError> {
    let mut first = None;
    for directory in directories {
        let result = synchronize(directory);
        if first.is_none() {
            first = result.err();
        }
    }
    first.map_or(Ok(()), Err)
}

fn combined(mut error: FsError, context: &str, extra: FsError) -> FsError {
    error.message.push_str(&format!("; {context}: {extra}"));
    error
}

#[cfg(test)]
#[path = "transfer_native_move_race_tests.rs"]
mod tests;
