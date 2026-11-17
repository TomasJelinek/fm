//! Iterative, handle-relative traversal. openat2 rejects even same-device bind
//! mounts of directories, and O_NOFOLLOW ensures directory links are never
//! traversed. A non-directory mount point is refused by its device or by unlink
//! itself (EBUSY).
use super::*;
use rustix::fs::Dir;

struct Frame {
    parent: Arc<File>,
    directory: Arc<File>,
    entries: Dir,
    name: OsString,
    path: PathBuf,
    identity: Identity,
}

pub(super) fn delete(
    staging: &Arc<File>,
    identity: Identity,
    path: &Path,
    cancel: &Cancellation,
    progress: &mut dyn FnMut(&Path),
) -> Result<(), FsError> {
    let name = OsStr::new(STAGED_NAME);
    if identity.kind != FileType::Directory {
        unlink(staging, name, identity, path, cancel)?;
        progress(path);
        return Ok(());
    }
    let mut stack = vec![open_frame(staging, name, identity, path)?];
    while let Some(frame) = stack.last_mut() {
        check_cancel(cancel, &frame.path)?;
        if let Some(next) = frame.entries.next() {
            let next = next.map_err(|e| native_error(&frame.path, e))?;
            let name = OsStr::from_bytes(next.file_name().to_bytes());
            if name == "." || name == ".." {
                continue;
            }
            let path = frame.path.join(name);
            // One no-follow lookup on the held directory is both the identity and the
            // live check immediately before unlink. Directories are opened below with
            // RESOLVE_NO_XDEV; a non-directory from another device can only be a mount.
            let current = statat(&*frame.directory, name, AtFlags::SYMLINK_NOFOLLOW)
                .map_err(|e| native_error(&path, e))?;
            let identity = Identity::from_stat(&current);
            if identity.kind == FileType::Directory {
                let child = open_frame(&frame.directory, name, identity, &path)?;
                stack.push(child);
            } else {
                if identity.device != frame.identity.device {
                    return Err(error(
                        &path,
                        ErrorKind::Unsupported,
                        "deletion does not cross mount boundaries",
                    ));
                }
                check_cancel(cancel, &path)?;
                unlinkat(&*frame.directory, name, AtFlags::empty())
                    .map_err(|e| native_error(&path, e))?;
                progress(&path);
            }
        } else {
            let frame = stack.pop().expect("nonempty stack");
            unlink(
                &frame.parent,
                &frame.name,
                frame.identity,
                &frame.path,
                cancel,
            )?;
            progress(&frame.path);
        }
    }
    Ok(())
}

fn open_frame(
    parent: &Arc<File>,
    name: &OsStr,
    identity: Identity,
    path: &Path,
) -> Result<Frame, FsError> {
    let directory = File::from(
        openat2(
            &**parent,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
            ResolveFlags::BENEATH | ResolveFlags::NO_XDEV | ResolveFlags::NO_SYMLINKS,
        )
        .map_err(|e| native_error(path, e))?,
    );
    if Identity::of(&directory, path)? != identity {
        return Err(error(
            path,
            ErrorKind::InvalidInput,
            "directory changed before traversal",
        ));
    }
    let entries = Dir::read_from(&directory).map_err(|e| native_error(path, e))?;
    Ok(Frame {
        parent: parent.clone(),
        directory: Arc::new(directory),
        entries,
        name: name.to_owned(),
        path: path.to_owned(),
        identity,
    })
}

fn unlink(
    parent: &File,
    name: &OsStr,
    identity: Identity,
    path: &Path,
    cancel: &Cancellation,
) -> Result<(), FsError> {
    check_cancel(cancel, path)?;
    identity.verify(parent, name, path)?;
    let flags = if identity.kind == FileType::Directory {
        AtFlags::REMOVEDIR
    } else {
        AtFlags::empty()
    };
    unlinkat(parent, name, flags).map_err(|e| native_error(path, e))
}
