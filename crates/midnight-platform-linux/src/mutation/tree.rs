//! Iterative, handle-relative traversal. openat2 rejects even same-device bind
//! mounts of directories, and O_NOFOLLOW ensures directory links are never
//! traversed. A device number alone does not identify a mount: overlayfs without
//! xino reports a per-layer device for every non-directory. A non-directory on
//! another device than its directory is therefore refused only when the
//! mount-refusing resolver identifies a mount point there; a non-directory mount
//! point on the same device is refused by unlink itself (EBUSY).
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
            // RESOLVE_NO_XDEV; other entries are probed for a mount only when their
            // device differs from the directory's.
            let current = statat(&*frame.directory, name, AtFlags::SYMLINK_NOFOLLOW)
                .map_err(|e| native_error(&path, e))?;
            let identity = Identity::from_stat(&current);
            if identity.kind == FileType::Directory {
                let child = open_frame(&frame.directory, name, identity, &path)?;
                stack.push(child);
            } else {
                if other_mount(
                    &frame.directory,
                    name,
                    &path,
                    identity.device,
                    frame.identity.device,
                )? {
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

/// Whether the non-directory `name`, which reported `device`, lies on another
/// mount than `directory`, which reported `directory_device`. Equal devices need
/// no lookup. Otherwise the beneath-only, no-follow resolution that refuses mount
/// crossings fails with EXDEV exactly at a mount point; an overlayfs layer
/// object resolves normally.
fn other_mount(
    directory: &File,
    name: &OsStr,
    path: &Path,
    device: u64,
    directory_device: u64,
) -> Result<bool, FsError> {
    if device == directory_device {
        return Ok(false);
    }
    match openat2(
        directory,
        name,
        OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
        ResolveFlags::BENEATH | ResolveFlags::NO_XDEV | ResolveFlags::NO_SYMLINKS,
    ) {
        Ok(_) => Ok(false),
        Err(rustix::io::Errno::XDEV) => Ok(true),
        Err(e) => Err(native_error(path, e)),
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::MetadataExt};

    #[test]
    fn a_different_device_is_a_mount_boundary_only_at_a_mount_point() {
        let fixture = tempfile::tempdir().unwrap();
        let path = fixture.path().join("file");
        fs::write(&path, b"layer object").unwrap();
        let directory = File::open(fixture.path()).unwrap();
        let device = fs::metadata(fixture.path()).unwrap().dev();
        let name = OsStr::new("file");
        assert!(!other_mount(&directory, name, &path, device, device).unwrap());
        // Overlayfs without xino reports a per-layer device for ordinary files.
        assert!(!other_mount(&directory, name, &path, device ^ 1, device).unwrap());
        // A real mount point below its directory: /dev/shm on /dev, when separate.
        match (fs::metadata("/dev"), fs::symlink_metadata("/dev/shm")) {
            (Ok(dev), Ok(shm)) if shm.is_dir() && dev.dev() != shm.dev() => {
                let parent = File::open("/dev").unwrap();
                let shm_path = Path::new("/dev/shm");
                let mounted = OsStr::new("shm");
                assert!(other_mount(&parent, mounted, shm_path, shm.dev(), dev.dev()).unwrap());
            }
            _ => eprintln!("SKIP mount-point probe: /dev/shm is not a separate mount below /dev"),
        }
        assert_eq!(fs::read(&path).unwrap(), b"layer object");
    }
}
