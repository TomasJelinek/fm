//! Metadata writes address retained descriptors, never a checked-then-written pathname.
use super::*;
use midnight_core::*;
use midnight_platform_common::metadata::validate_patch;
use rustix::fs::{Dir, Timespec, Timestamps, UTIME_OMIT, fchmod, futimens, utimensat};
use std::os::fd::AsRawFd;

pub(super) fn snapshot(
    item: &Prepared,
    cancel: &Cancellation,
) -> Result<MetadataSnapshot, FsError> {
    item.revalidate(cancel)?;
    let stat = fstat(&item._source).map_err(|e| native_error(&item.item.path, e))?;
    let ordinary = matches!(
        item.identity.kind,
        FileType::RegularFile | FileType::Directory
    );
    Ok(MetadataSnapshot {
        capabilities: MetadataCapabilities {
            unix_permissions: if ordinary { 0o7777 } else { 0 },
            modified: ordinary,
            accessed: ordinary,
            rename: true,
            ..Default::default()
        },
        unix_mode: Some(stat.st_mode),
        modified: modified_time(stat.st_mtime, stat.st_mtime_nsec),
        accessed: modified_time(stat.st_atime, stat.st_atime_nsec),
        hard_links: stat.st_nlink,
        ..Default::default()
    })
}

pub(super) fn name_available(
    item: &Prepared,
    name: &OsStr,
    cancel: &Cancellation,
) -> Result<(), FsError> {
    check_cancel(cancel, &item.item.path)?;
    validate_name(name, &item.item.path)?;
    if name == item.name {
        return Ok(());
    }
    match statat(&*item.parent, name, AtFlags::SYMLINK_NOFOLLOW) {
        Err(rustix::io::Errno::NOENT) => Ok(()),
        Err(error) => Err(native_error(&item.item.path, error)),
        Ok(stat) => {
            if Identity::from_stat(&stat) == item.identity {
                // On casefold filesystems an alternate spelling may address the same entry.
                // A directory has exactly one name and a file with one link has no other, so
                // the spelling found the selected entry itself; the no-replace rename stays
                // authoritative. Only another hard-link name needs the exact-name scan, which
                // would otherwise read the whole directory for every case-only rename.
                if item.identity.kind == FileType::Directory || stat.st_nlink <= 1 {
                    return Ok(());
                }
                let directory = File::from(
                    openat(
                        &*item.parent,
                        c".",
                        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
                        Mode::empty(),
                    )
                    .map_err(|e| native_error(&item.parent_path, e))?,
                );
                let mut entries =
                    Dir::read_from(&directory).map_err(|e| native_error(&item.parent_path, e))?;
                let mut count = 0;
                for entry in &mut entries {
                    check_cancel(cancel, &item.item.path)?;
                    count += 1;
                    if count > 1_000_000 {
                        return Err(error(
                            &item.item.path,
                            ErrorKind::Other,
                            "case-alias inspection exceeds its entry budget",
                        ));
                    }
                    let entry = entry.map_err(|e| native_error(&item.parent_path, e))?;
                    if entry.file_name().to_bytes() == name.as_bytes() {
                        return Err(error(
                            &item.item.path.with_file_name(name),
                            ErrorKind::AlreadyExists,
                            "destination already exists",
                        ));
                    }
                }
                return Ok(());
            }
            Err(error(
                &item.item.path.with_file_name(name),
                ErrorKind::AlreadyExists,
                "destination already exists",
            ))
        }
    }
}

/// A directory's captured child names and the identity-checked descriptor that each child
/// is opened relative to, one at a time (see [`MetadataChildren`]).
struct Children {
    path: PathBuf,
    identity: Identity,
    directory: Arc<File>,
    names: CapturedNames,
}

/// Listing and pinning change nothing, so their cancellation has its own text.
fn listing_continues(cancel: &Cancellation, path: &Path) -> Result<(), FsError> {
    if cancel.wait_if_paused() {
        Ok(())
    } else {
        Err(error(
            path,
            ErrorKind::Interrupted,
            "metadata listing cancelled",
        ))
    }
}

pub(super) fn children(
    item: &Prepared,
    cancel: &Cancellation,
    max_names: usize,
    max_bytes: usize,
) -> Result<Box<dyn MetadataChildren>, FsError> {
    item.revalidate(cancel)?;
    if item.identity.kind != FileType::Directory {
        return Err(error(
            &item.item.path,
            ErrorKind::InvalidInput,
            "only ordinary directories can be traversed",
        ));
    }
    let directory = Arc::new(usable_handle(item, &item.name, &item.item.path)?);
    let (max_names, max_bytes) = (
        max_names.min(MAX_METADATA_NAMES),
        max_bytes.min(MAX_METADATA_NAMES_BYTES),
    );
    let mut entries = Dir::read_from(&*directory).map_err(|e| native_error(&item.item.path, e))?;
    let mut names = Vec::new();
    let mut bytes = 0usize;
    for entry in &mut entries {
        listing_continues(cancel, &item.item.path)?;
        let entry = entry.map_err(|e| native_error(&item.item.path, e))?;
        let raw = entry.file_name().to_bytes();
        if raw == b"." || raw == b".." {
            continue;
        }
        // Raw byte lengths are the names' `OsStr::len`.
        bytes = bytes.saturating_add(raw.len());
        if names.len() >= max_names || bytes > max_bytes {
            return Err(error(
                &item.item.path,
                ErrorKind::Other,
                "metadata directory snapshot exceeds its resource budget",
            ));
        }
        names.push(OsStr::from_bytes(raw).to_owned());
    }
    drop(entries);
    Ok(Box::new(Children {
        path: item.item.path.clone(),
        identity: item.identity,
        directory,
        names: CapturedNames::new(names),
    }))
}

impl MetadataChildren for Children {
    fn names(&self) -> &[OsString] {
        self.names.remaining()
    }

    fn pin_next(
        &mut self,
        cancel: &Cancellation,
    ) -> Option<Result<Arc<dyn PreparedMutation>, (OsString, FsError)>> {
        let name = self.names.take_next()?;
        Some(match self.open(&name, cancel) {
            Ok(mut child) => {
                child.name = name;
                Ok(Arc::new(child))
            }
            Err(error) => Err((name, error)),
        })
    }
}

impl Children {
    /// Opens and pins the entry now named `name` relative to the retained directory
    /// descriptor; the caller hands the pinned entry its name.
    fn open(&self, name: &OsStr, cancel: &Cancellation) -> Result<Prepared, FsError> {
        let path = self.path.join(name);
        listing_continues(cancel, &path)?;
        let source = File::from(
            openat2(
                &*self.directory,
                name,
                OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
                ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_XDEV,
            )
            .map_err(|e| native_error(&path, e))?,
        );
        let stat = fstat(&source).map_err(|e| native_error(&path, e))?;
        let identity = Identity::from_stat(&stat);
        let kind = match identity.kind {
            FileType::Directory => EntryKind::Directory,
            FileType::RegularFile => EntryKind::File,
            FileType::Symlink => EntryKind::Symlink,
            _ => EntryKind::Other,
        };
        Ok(Prepared {
            item: MutationItem {
                path,
                kind,
                metadata: EntryMetadata {
                    unix_mode: Some(stat.st_mode),
                    modified: modified_time(stat.st_mtime, stat.st_mtime_nsec),
                    ..Default::default()
                },
            },
            parent_path: self.path.clone(),
            name: OsString::new(),
            parent: self.directory.clone(),
            parent_identity: self.identity,
            identity,
            _source: source,
            trash_root: None,
        })
    }
}

pub(super) fn edit(
    item: &Prepared,
    patch: &MetadataPatch,
    name: Option<&OsStr>,
    cancel: &Cancellation,
) -> MetadataOutcome {
    let mut result = MetadataOutcome::unchanged(item.item.path.clone());
    if let Err(error) = apply(item, patch, name, cancel, &mut result) {
        result.error = Some(error);
    }
    result
}

fn apply(
    item: &Prepared,
    patch: &MetadataPatch,
    name: Option<&OsStr>,
    cancel: &Cancellation,
    result: &mut MetadataOutcome,
) -> Result<(), FsError> {
    item.revalidate(cancel)?;
    let snap = snapshot(item, cancel)?;
    validate_patch(patch, snap.capabilities, &item.item.path)?;
    let times = Timestamps {
        last_access: timestamp(patch.accessed, &item.item.path)?,
        last_modification: timestamp(patch.modified, &item.item.path)?,
    };
    let mut current_name = item.name.as_os_str();
    let mut residue = None;
    if let Some(name) = name.filter(|name| *name != item.name) {
        name_available(item, name, cancel)?;
        residue = rename(item, name, cancel, result)?;
        current_name = name;
    }
    // Staging left behind by the completed rename never stops the remaining fields;
    // it is reported once they were applied, or with the step that failed.
    let applied = apply_fields(item, patch, &times, current_name, cancel, result);
    match (applied, residue) {
        (applied, None) => applied,
        (Ok(()), Some(residue)) => Err(completed_with_residue("The entry was renamed", residue)),
        (Err(mut failure), Some(residue)) => {
            failure
                .message
                .push_str(&format!("; the entry was renamed, but {residue}"));
            // Never a name conflict or a cancellation that hides the completed rename.
            failure.kind = ErrorKind::Other;
            Err(failure)
        }
    }
}

/// A completed metadata rename whose staging directory could not be removed, as the
/// outcome error: kind Other, so that it never reads as a name conflict or a
/// cancellation of the completed rename. The metadata job counts its item as failed.
fn completed_with_residue(completed: &str, residue: MutationResidue) -> FsError {
    FsError::from_io(
        OPERATION,
        &residue.path,
        io::Error::other(format!("{completed}, but {residue}")),
    )
}

/// Permission and time changes after any rename, through the pinned descriptor.
fn apply_fields(
    item: &Prepared,
    patch: &MetadataPatch,
    times: &Timestamps,
    current_name: &OsStr,
    cancel: &Cancellation,
    result: &mut MetadataOutcome,
) -> Result<(), FsError> {
    if patch.unix_mask != 0 {
        check_cancel(cancel, &result.current_path)?;
        let stat = fstat(&item._source).map_err(|e| native_error(&result.current_path, e))?;
        let mode = (stat.st_mode & !patch.unix_mask) | (patch.unix_value & patch.unix_mask);
        if let Err(failure) = chmod_pinned(&item._source, mode) {
            if matches!(
                failure.raw_os_error(),
                Some(libc::ENOSYS | libc::EINVAL | libc::EOPNOTSUPP)
            ) {
                let source = usable_handle(item, current_name, &result.current_path)?;
                fchmod(&source, Mode::from_raw_mode(mode))
                    .map_err(|e| native_error(&result.current_path, e))?;
            } else {
                return Err(FsError::from_io(
                    "change permissions",
                    &result.current_path,
                    failure,
                ));
            }
        }
        let actual = fstat(&item._source).map_err(|e| native_error(&result.current_path, e))?;
        result.changes.permissions = stat.st_mode != actual.st_mode;
        if actual.st_mode & 0o7777 != mode & 0o7777 {
            return Err(error(
                &result.current_path,
                ErrorKind::Other,
                "the filesystem did not retain the requested permissions, including unchanged special bits",
            ));
        }
    }
    if patch.modified != TimeEdit::Keep || patch.accessed != TimeEdit::Keep {
        check_cancel(cancel, &result.current_path)?;
        if let Err(failure) = utimensat(&item._source, c"", times, AtFlags::EMPTY_PATH) {
            if matches!(
                failure,
                rustix::io::Errno::INVAL | rustix::io::Errno::NOENT | rustix::io::Errno::NOSYS
            ) {
                let source = usable_handle(item, current_name, &result.current_path)?;
                futimens(&source, times).map_err(|e| native_error(&result.current_path, e))?;
            } else {
                return Err(native_error(&result.current_path, failure));
            }
        }
        result.changes.modified = patch.modified != TimeEdit::Keep;
        result.changes.accessed = patch.accessed != TimeEdit::Keep;
    }
    Ok(())
}

/// Renames through private staging. Once published the rename has completed: a
/// staging directory that then cannot be removed is returned, never an error.
fn rename(
    item: &Prepared,
    name: &OsStr,
    cancel: &Cancellation,
    result: &mut MetadataOutcome,
) -> Result<Option<MutationResidue>, FsError> {
    let staged = Staged::create(item)?;
    if let Err(failure) = staged.capture(item, cancel) {
        record_staged_path(item, &staged, result);
        return Err(failure);
    }
    let publish = check_cancel(cancel, &item.item.path).and_then(|()| {
        item.identity
            .verify(&staged.directory, OsStr::new(STAGED_NAME), &item.item.path)?;
        renameat_with(
            &*staged.directory,
            STAGED_NAME,
            &*item.parent,
            name,
            RenameFlags::NOREPLACE,
        )
        .map_err(|e| native_error(&item.item.path.with_file_name(name), e))
    });
    if let Err(failure) = publish {
        let failure = staged.restore(item, failure);
        record_staged_path(item, &staged, result);
        return Err(failure);
    }
    result.current_path = item.item.path.with_file_name(name);
    result.changes.renamed = true;
    Ok(staged.cleanup().err())
}

fn record_staged_path(item: &Prepared, staged: &Staged, result: &mut MetadataOutcome) {
    if item
        .identity
        .verify(&staged.directory, OsStr::new(STAGED_NAME), &item.item.path)
        .is_ok()
    {
        result.current_path = staged.path.join(STAGED_NAME);
        result.changes.renamed = true;
    }
}

#[allow(unsafe_code)] // rustix 1.1 has no fchmodat2 wrapper accepting AT_EMPTY_PATH.
fn chmod_pinned(file: &File, mode: u32) -> io::Result<()> {
    // SAFETY: the O_PATH descriptor remains owned and pinned; empty path requests that exact inode.
    let code = unsafe {
        libc::syscall(
            libc::SYS_fchmodat2,
            file.as_raw_fd(),
            c"".as_ptr(),
            mode as libc::mode_t,
            libc::AT_EMPTY_PATH,
        )
    };
    if code == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

fn usable_handle(item: &Prepared, name: &OsStr, path: &Path) -> Result<File, FsError> {
    let file = File::from(
        openat2(
            &*item.parent,
            name,
            OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
            ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_XDEV,
        )
        .map_err(|e| native_error(path, e))?,
    );
    if Identity::of(&file, path)? != item.identity {
        return Err(error(
            path,
            ErrorKind::InvalidInput,
            "selected identity changed before opening metadata handle",
        ));
    }
    Ok(file)
}

fn timestamp(edit: TimeEdit, path: &Path) -> Result<Timespec, FsError> {
    let TimeEdit::Set(time) = edit else {
        return Ok(Timespec {
            tv_sec: 0,
            tv_nsec: UTIME_OMIT,
        });
    };
    let nanos = match time.duration_since(UNIX_EPOCH) {
        Ok(delta) => delta.as_nanos() as i128,
        Err(error) => -(error.duration().as_nanos() as i128),
    };
    let seconds = i64::try_from(nanos.div_euclid(1_000_000_000)).map_err(|_| {
        error(
            path,
            ErrorKind::InvalidInput,
            "timestamp is outside the native range",
        )
    })?;
    Ok(Timespec {
        tv_sec: seconds,
        tv_nsec: nanos.rem_euclid(1_000_000_000) as _,
    })
}
