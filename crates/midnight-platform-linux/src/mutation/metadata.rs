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
                // A different existing hard-link name must still be treated as a collision.
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

pub(super) fn children(
    item: &Prepared,
    cancel: &Cancellation,
    max_targets: usize,
) -> Result<Vec<Arc<dyn PreparedMutation>>, FsError> {
    item.revalidate(cancel)?;
    if item.identity.kind != FileType::Directory {
        return Err(error(
            &item.item.path,
            ErrorKind::InvalidInput,
            "only ordinary directories can be traversed",
        ));
    }
    let directory = Arc::new(usable_handle(item, &item.name, &item.item.path)?);
    let mut entries = Dir::read_from(&*directory).map_err(|e| native_error(&item.item.path, e))?;
    let mut names = Vec::new();
    let mut bytes = 0usize;
    for entry in &mut entries {
        check_cancel(cancel, &item.item.path)?;
        let entry = entry.map_err(|e| native_error(&item.item.path, e))?;
        let raw = entry.file_name().to_bytes();
        if raw == b"." || raw == b".." {
            continue;
        }
        bytes = bytes.saturating_add(raw.len());
        if names.len() >= max_targets.min(MAX_METADATA_TARGETS) || bytes > MAX_METADATA_NAMES_BYTES
        {
            return Err(error(
                &item.item.path,
                ErrorKind::Other,
                "metadata directory snapshot exceeds its resource budget",
            ));
        }
        names.push(OsStr::from_bytes(raw).to_owned());
    }
    let mut result: Vec<Arc<dyn PreparedMutation>> = Vec::with_capacity(names.len());
    for name in names {
        check_cancel(cancel, &item.item.path)?;
        let path = item.item.path.join(&name);
        let source = File::from(
            openat2(
                &*directory,
                &name,
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
        result.push(Arc::new(Prepared {
            item: MutationItem {
                path,
                kind,
                metadata: EntryMetadata {
                    unix_mode: Some(stat.st_mode),
                    modified: modified_time(stat.st_mtime, stat.st_mtime_nsec),
                    ..Default::default()
                },
            },
            parent_path: item.item.path.clone(),
            name,
            parent: directory.clone(),
            parent_identity: item.identity,
            identity,
            _source: source,
            trash_root: None,
        }));
    }
    Ok(result)
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
    if let Some(name) = name.filter(|name| *name != item.name) {
        name_available(item, name, cancel)?;
        rename(item, name, cancel, result)?;
        current_name = name;
    }
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
        if let Err(failure) = utimensat(&item._source, c"", &times, AtFlags::EMPTY_PATH) {
            if matches!(
                failure,
                rustix::io::Errno::INVAL | rustix::io::Errno::NOENT | rustix::io::Errno::NOSYS
            ) {
                let source = usable_handle(item, current_name, &result.current_path)?;
                futimens(&source, &times).map_err(|e| native_error(&result.current_path, e))?;
            } else {
                return Err(native_error(&result.current_path, failure));
            }
        }
        result.changes.modified = patch.modified != TimeEdit::Keep;
        result.changes.accessed = patch.accessed != TimeEdit::Keep;
    }
    Ok(())
}

fn rename(
    item: &Prepared,
    name: &OsStr,
    cancel: &Cancellation,
    result: &mut MetadataOutcome,
) -> Result<(), FsError> {
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
    // Cleanup may fail after publication; the outcome must still describe the completed rename.
    staged.cleanup()
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
