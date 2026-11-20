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

/// `attempt`, with test hooks run just before the capture and the publication.
pub(super) fn attempt_with(
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
    namespace::change(cancel, |mut change| {
        let mut retained = capture.native.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(outcome) = retained.as_ref() {
            return Ok(outcome.clone());
        }
        let result = execute_in(
            MoveSource::Ordinary(capture),
            (destination, name),
            replace,
            cancel,
            hooks,
            |stage, target| sync(capture, stage, target),
            &mut change,
        )?;
        if terminal(&result) {
            *retained = Some(result.clone());
        }
        Ok(result)
    })
}

pub(super) fn attempt_link(
    original: &directory_links::OriginalLink,
    parent: &destination::Directory,
    destination: &dyn TransferDirectory,
    name: &OsStr,
    replace: Replacement,
    cancel: &Cancellation,
) -> Result<TransferNativeMoveOutcome, FsError> {
    namespace::change(cancel, |mut change| {
        execute_in(
            MoveSource::Link { original, parent },
            (destination, name),
            replace,
            cancel,
            (|| {}, || {}),
            |stage, target| sync_locations(parent, stage, target),
            &mut change,
        )
    })
}

pub(super) fn terminal(outcome: &TransferNativeMoveOutcome) -> bool {
    matches!(
        outcome,
        TransferNativeMoveOutcome::Moved { .. }
            | TransferNativeMoveOutcome::RecoveryRequired { .. }
    )
}

#[derive(Clone, Copy)]
enum MoveSource<'a> {
    Ordinary(&'a Capture),
    Link {
        original: &'a directory_links::OriginalLink,
        parent: &'a destination::Directory,
    },
}

impl<'a> MoveSource<'a> {
    fn entry(self) -> &'a TransferEntry {
        match self {
            Self::Ordinary(capture) => &capture.entry,
            Self::Link { original, .. } => original.entry(),
        }
    }

    fn parent(self) -> &'a destination::Directory {
        match self {
            Self::Ordinary(capture) => &capture.parent,
            Self::Link { parent, .. } => parent,
        }
    }

    fn name(self) -> &'a OsStr {
        match self {
            Self::Ordinary(capture) => &capture.name,
            Self::Link { original, .. } => original.name(),
        }
    }

    fn is_link(self) -> bool {
        matches!(self, Self::Link { .. })
    }

    fn revalidate(self, cancel: &Cancellation) -> Result<(), FsError> {
        match self {
            Self::Ordinary(capture) => capture.revalidate(cancel),
            Self::Link { original, parent } => {
                original.qualify_removal()?;
                parent.check_recovery_parent(cancel)?;
                original.verify_original(cancel)
            }
        }
    }

    fn check_link_stage(
        self,
        stage: &destination::Directory,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        if self.is_link() {
            self.parent().check_recovery_parent(cancel)?;
            stage.check_recovery_parent(cancel)?;
        }
        Ok(())
    }

    fn verify_link(
        self,
        parent: &destination::Directory,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        if let Self::Link { original, .. } = self {
            original.verify_relocated(
                parent.native_file(),
                name,
                &parent.path().join(name),
                cancel,
            )?;
        }
        Ok(())
    }

    fn sync_restored_names(self, stage: &destination::Directory) -> Result<(), FsError> {
        stage
            .native_file()
            .sync_all()
            .and_then(|()| self.parent().native_file().sync_all())
            .map_err(|failure| error(&self.entry().item.path, failure))
    }
}

fn execute_in(
    source: MoveSource<'_>,
    (destination, name): (&dyn TransferDirectory, &OsStr),
    replace: Replacement,
    cancel: &Cancellation,
    hooks: (impl FnOnce(), impl FnOnce()),
    sync: impl FnOnce(&destination::Directory, &destination::Directory) -> Result<(), FsError>,
    change: &mut namespace::Change<'_>,
) -> Result<TransferNativeMoveOutcome, FsError> {
    let (before_capture, before_publish) = hooks;
    let entry = source.entry();
    let parent = source.parent();
    let original_name = source.name();
    validate_name(name, destination.path())?;
    source.revalidate(cancel)?;
    if let MoveSource::Ordinary(capture) = source
        && capture
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .outcome
            .is_some()
    {
        return Err(invalid(
            &entry.item.path,
            "source already entered verified Move staging",
        ));
    }
    let target = bind_destination(destination, source, cancel)?;
    if target.identity() == parent.identity() && name == original_name {
        return Ok(TransferNativeMoveOutcome::SameEntry);
    }
    if entry.identity.device != target.identity().device
        || different_mount(parent.native_file(), target.native_file(), target.path())?
    {
        return Ok(TransferNativeMoveOutcome::CrossDevice);
    }
    qualify(&target)?;
    target.check_recovery_parent(cancel)?;
    // A preserved link (its own entry kind is Symlink) can replace a file or link
    // like a file does; the original of a followed directory link cannot.
    if source.is_link() && replace.is_some() && entry.item.kind != EntryKind::Symlink {
        return Err(unsupported(
            &target.path().join(name),
            "native directory link Move requires a vacant destination name",
        ));
    }
    if entry.item.kind == EntryKind::Directory {
        target.reject_source_ancestor(entry.identity, cancel)?;
    }
    let replaced = validate_target(entry, &target, name, replace.as_ref(), cancel)?;
    source.revalidate(cancel)?;
    before_capture();
    checked(cancel, &entry.item.path)?;
    parent.namespace_announce(original_name, change, cancel)?;
    target.namespace_announce(name, change, cancel)?;
    let stage = match parent.source_stage_in(cancel, change) {
        Ok(stage) => stage,
        Err(mut failure) => {
            if source.is_link()
                && let Some(receipt) = link_capture::stage_creation_receipt(parent, &mut failure)
            {
                return Ok(TransferNativeMoveOutcome::RecoveryRequired {
                    destination: None,
                    recovery: vec![receipt],
                    error: failure,
                });
            }
            return Err(failure);
        }
    };
    // Moving a folder into another folder rewrites its ".." entry, which needs write
    // permission on the folder itself: an owned folder has it until this returns.
    let _writable = writable_for_rename(source);
    let captured = (|| {
        if source.is_link() {
            source.revalidate(cancel)?;
            stage.check_recovery_parent(cancel)?;
        }
        renameat_with(
            parent.native_file(),
            original_name,
            stage.native_file(),
            original_name,
            RenameFlags::NOREPLACE,
        )
        .map_err(|failure| capture_error(source, failure))
    })();
    if let Err(failure) = captured {
        let mut recovery = Vec::new();
        return match clean_stage(source, stage, change, &mut recovery) {
            Ok(()) => Err(failure),
            Err(cleanup) => Ok(TransferNativeMoveOutcome::RecoveryRequired {
                destination: None,
                recovery,
                error: combined(failure, "empty staging cleanup", cleanup),
            }),
        };
    }
    parent.namespace_changed(original_name);
    stage.namespace_changed(original_name);
    // Once captured, every refusal is a terminal namespace fact.
    let validated = (|| {
        let actual = stage.inspect(original_name, &Cancellation::default())?;
        if actual.as_ref().map(|entry| entry.identity) != Some(entry.identity) {
            return Err(invalid(
                &entry.item.path,
                "a different source was captured; its bytes were retained",
            ));
        }
        target.revalidate(cancel)?;
        validate_target(entry, &target, name, replace.as_ref(), cancel)?;
        checked(cancel, &entry.item.path)
    })();
    if let Err(error) = validated {
        return Ok(restore(source, stage, error, change));
    }
    before_publish();
    let ready = checked(cancel, &entry.item.path)
        .and_then(|()| source.check_link_stage(&stage, cancel))
        .and_then(|()| source.verify_link(&stage, original_name, cancel));
    if let Err(error) = ready {
        return Ok(restore(source, stage, error, change));
    }
    let flags = if replaced.is_some() {
        RenameFlags::EXCHANGE
    } else {
        RenameFlags::NOREPLACE
    };
    if let Err(failure) = renameat_with(
        stage.native_file(),
        original_name,
        target.native_file(),
        name,
        flags,
    ) {
        return Ok(restore(
            source,
            stage,
            move_error(&target.path().join(name), failure.into()),
            change,
        ));
    }
    stage.namespace_changed(original_name);
    target.namespace_changed(name);
    Ok(committed(
        source, &target, name, stage, replaced, sync, change,
    ))
}

/// The pinned destination as this provider's directory, for recreating a link that
/// moves to another filesystem.
pub(super) fn bind_link_destination(
    expected: &dyn TransferDirectory,
    original: &directory_links::OriginalLink,
    parent: &destination::Directory,
    cancel: &Cancellation,
) -> Result<destination::Directory, FsError> {
    let target = bind_destination(expected, MoveSource::Link { original, parent }, cancel)?;
    qualify(&target)?;
    Ok(target)
}

fn bind_destination(
    expected: &dyn TransferDirectory,
    source: MoveSource<'_>,
    cancel: &Cancellation,
) -> Result<destination::Directory, FsError> {
    if let MoveSource::Link { parent, .. } = source
        && expected.path() == parent.path()
    {
        if expected.identity() != parent.identity() {
            return Err(invalid(expected.path(), "pinned Move destination changed"));
        }
        // The requested destination is the same retained, mount-checked parent.
        parent.revalidate(cancel)?;
        return Ok(parent.clone());
    }
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
    source: &TransferEntry,
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
            let replaceable = match source.item.kind {
                EntryKind::File => current.entry().item.kind == EntryKind::File,
                // A link replaces a file or another link, never a folder.
                EntryKind::Symlink => matches!(
                    current.entry().item.kind,
                    EntryKind::File | EntryKind::Symlink
                ),
                _ => false,
            };
            if !replaceable
                || expected.entry().item.path != path
                || current.entry().identity != expected.entry().identity
                || current.entry().identity == source.identity
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
    source: MoveSource<'_>,
    stage: destination::Directory,
    mut failure: FsError,
    change: &mut namespace::Change<'_>,
) -> TransferNativeMoveOutcome {
    let parent = source.parent();
    let name = source.name();
    let mut recovery = Vec::new();
    let restored = (|| -> Result<(), String> {
        if source.is_link() {
            source
                .check_link_stage(&stage, &Cancellation::default())
                .map_err(|failure| failure.to_string())?;
        }
        renameat_with(
            stage.native_file(),
            name,
            parent.native_file(),
            name,
            RenameFlags::NOREPLACE,
        )
        .map_err(|failure| failure.to_string())
    })();
    match restored {
        Ok(()) => {
            parent.namespace_changed(name);
            stage.namespace_changed(name);
            failure
                .message
                .push_str("; source name restored without replacement");
            if let Err(sync) = source.sync_restored_names(&stage) {
                failure = combined(failure, "restored-name durability", sync);
            }
            if let Err(cleanup) = clean_stage(source, stage, change, &mut recovery) {
                failure = combined(failure, "empty staging cleanup", cleanup);
            }
        }
        Err(restore) => {
            failure.message.push_str(&format!(
                "; restoration refused without replacement: {restore}"
            ));
            recovery.push(receipt(&stage, name));
            if source.is_link()
                && let Err(sync) = source.sync_restored_names(&stage)
            {
                failure = combined(failure, "recovery durability", sync);
            }
        }
    }
    TransferNativeMoveOutcome::RecoveryRequired {
        destination: None,
        recovery,
        error: failure,
    }
}

fn committed(
    source: MoveSource<'_>,
    target: &destination::Directory,
    name: &OsStr,
    stage: destination::Directory,
    replaced: Option<(TransferIdentity, destination::recovery::Revision)>,
    sync: impl FnOnce(&destination::Directory, &destination::Directory) -> Result<(), FsError>,
    change: &mut namespace::Change<'_>,
) -> TransferNativeMoveOutcome {
    let path = target.path().join(name);
    let cleanup = Cancellation::default();
    let published = target.inspect(name, &cleanup);
    let observed_identity = published
        .as_ref()
        .ok()
        .and_then(|entry| entry.as_ref())
        .map(|entry| entry.identity);
    let destination = (observed_identity == Some(source.entry().identity)).then(|| path.clone());
    let mut recovery = replaced
        .map(|_| receipt(&stage, source.name()))
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
        source.verify_link(target, name, &cleanup)?;
        if let Some((expected, revision)) = replaced {
            let file = File::from(
                openat2(
                    stage.native_file(),
                    source.name(),
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
    if let Err(mut error) = checked {
        if source.is_link() {
            recovery.push(TransferRecovery {
                path,
                identity: observed_identity,
            });
            if let Err(failure) = sync(&stage, target) {
                error = combined(error, "saving the moved link", failure);
            }
            if let Err(failure) = clean_stage(source, stage, change, &mut recovery) {
                error = combined(error, "empty staging cleanup", failure);
            }
            return TransferNativeMoveOutcome::RecoveryRequired {
                destination: None,
                recovery,
                error,
            };
        }
        return TransferNativeMoveOutcome::RecoveryRequired {
            destination,
            recovery,
            error,
        };
    }
    let mut failure = sync(&stage, target).err();
    if replaced.is_none()
        && let Err(cleanup) = clean_stage(source, stage, change, &mut recovery)
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

fn clean_stage(
    source: MoveSource<'_>,
    stage: destination::Directory,
    change: &mut namespace::Change<'_>,
    recovery: &mut Vec<TransferRecovery>,
) -> Result<(), FsError> {
    if !source.is_link() {
        return source.parent().remove_empty_stage_in(stage, change);
    }
    match source.parent().remove_empty_stage_checked(stage, change) {
        Ok(()) => Ok(()),
        Err(destination::recovery::StageRemovalError::Unsynced(error)) => Err(error),
        Err(destination::recovery::StageRemovalError::Retained(error, stage)) => {
            recovery.push(TransferRecovery {
                path: stage.path().to_owned(),
                identity: Some(stage.identity()),
            });
            Err(error)
        }
    }
}

fn sync_native(
    capture: &Capture,
    stage: &destination::Directory,
    target: &destination::Directory,
) -> Result<(), FsError> {
    sync_locations(&capture.parent, stage, target)
}

fn sync_locations(
    parent: &destination::Directory,
    stage: &destination::Directory,
    target: &destination::Directory,
) -> Result<(), FsError> {
    sync_each([stage, parent, target], |directory| {
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

const READ_ONLY_FOLDER: &str = "Permission denied: the folder is read-only. Linux moves a folder \
into another folder only when the folder itself is writable, because its link to its parent \
folder changes. The folder was left in place; make it writable, or copy it instead.";

/// Why the source could not be taken into its private stage. The parent was just
/// writable enough to hold the stage, so a refused folder is usually read-only itself.
fn capture_error(source: MoveSource<'_>, failure: rustix::io::Errno) -> FsError {
    use rustix::fs::{Access, AtFlags, accessat};
    let entry = source.entry();
    let read_only_folder = failure == rustix::io::Errno::ACCESS
        && entry.item.kind == EntryKind::Directory
        && !source.is_link()
        && accessat(
            source.parent().native_file(),
            source.name(),
            Access::WRITE_OK,
            AtFlags::EACCESS,
        ) == Err(rustix::io::Errno::ACCESS);
    let failure = if read_only_folder {
        io::Error::new(io::ErrorKind::PermissionDenied, READ_ONLY_FOLDER)
    } else {
        failure.into()
    };
    move_error(&entry.item.path, failure)
}

/// Owner write added to an owned folder for its renames (see `execute_in`). Dropping
/// it gives the folder its own mode back, wherever the folder is by then.
struct WritableForRename {
    folder: Arc<File>,
    mode: rustix::fs::RawMode,
}

impl Drop for WritableForRename {
    fn drop(&mut self) {
        // The folder is ours and its mode was just changed the same way.
        let _ = chmod_retained(&self.folder, self.mode);
    }
}

fn writable_for_rename(source: MoveSource<'_>) -> Option<WritableForRename> {
    let MoveSource::Ordinary(capture) = source else {
        return None;
    };
    if capture.entry.item.kind != EntryKind::Directory {
        return None;
    }
    // The retained descriptor is the captured folder itself, not its name.
    let folder = capture._file.clone();
    let stat = rustix::fs::fstat(&*folder).ok()?;
    let mode = stat.st_mode & 0o7777;
    if stat.st_uid != rustix::process::geteuid().as_raw() || mode & 0o200 != 0 {
        return None;
    }
    chmod_retained(&folder, mode | 0o200).ok()?;
    Some(WritableForRename { folder, mode })
}

/// `fchmod` refuses the retained O_PATH descriptor. Its /proc entry names exactly
/// that inode, never a path that another process could replace.
fn chmod_retained(file: &File, mode: rustix::fs::RawMode) -> rustix::io::Result<()> {
    use std::os::fd::AsRawFd;
    rustix::fs::chmod(
        format!("/proc/self/fd/{}", file.as_raw_fd()).as_str(),
        Mode::from_raw_mode(mode),
    )
}

#[cfg(test)]
#[path = "transfer_native_move_race_tests.rs"]
mod tests;
