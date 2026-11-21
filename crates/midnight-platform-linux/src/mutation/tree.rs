//! Iterative, handle-relative traversal. openat2 rejects even same-device bind
//! mounts of directories, and O_NOFOLLOW ensures directory links are never
//! traversed. A device number alone does not identify a mount: overlayfs without
//! xino reports a per-layer device for every non-directory. A non-directory on
//! another device than its directory is therefore refused only when the
//! mount-refusing resolver identifies a mount point there; a non-directory mount
//! point on the same device is refused by unlink itself (EBUSY).
//!
//! Removing a folder's entries needs write and search permission on that folder. A
//! folder of the selected tree (the selected folder or a folder below it) that the
//! current user owns and that lacks owner write or search permission gets both,
//! through the descriptor this traversal opened and verified, before its entries are
//! removed: deleting everything in it was confirmed. A folder that stays in the end
//! (it still holds skipped items, its removal failed, or the deletion stopped) gets
//! its original mode back; if that fails (the filesystem turned read-only or failed),
//! the failure names the folder and is offered like any other (`Walk::restore`), or
//! added to the reason the deletion stopped. A folder whose set-group-ID bit the kernel
//! would clear with any change of its mode (the user is not a member of its group) is
//! not changed and is reported as write-protected. The folder that contains the
//! selection, anything outside the selected tree and anything owned by another user is
//! never changed, and links are never followed, so a link to a folder elsewhere never
//! changes that folder.
//!
//! A step that fails is offered to the caller (`Decide`): Retry repeats it, Skip
//! leaves the entry and goes on, Cancel ends the deletion. When the folder itself
//! keeps its entries from leaving (write-protected, or owned by another user), the
//! failure names the folder, once, and Skip leaves the folder with what is still in
//! it. Folders that keep skipped items are not removed and not reported again.
use super::*;
use rustix::{
    fs::{Dir, RawMode, fchmod},
    io::Errno,
};

/// Answers a failure. `Err` ends the deletion with that error: a decision that
/// could not be carried out, or a caller that takes no decisions.
pub(super) type Decide<'a> =
    dyn FnMut(DeleteEntryFailure) -> Result<DeleteFailureChoice, FsError> + 'a;

/// Owner write and search permission, which removing a folder's entries needs.
const OWNER_REMOVAL: RawMode = 0o300;

struct Frame {
    parent: Arc<File>,
    directory: Arc<File>,
    entries: Dir,
    name: OsString,
    path: PathBuf,
    identity: Identity,
    /// The folder's mode before owner write and search permission were added.
    original_mode: Option<RawMode>,
    /// Why owner write and search permission could not be added.
    unprotect_error: Option<String>,
    /// Skipped entries stay in this folder, so the folder stays too.
    keeps_skipped: bool,
}

impl Frame {
    /// Gives a folder that stays its original mode back. A failure leaves the folder
    /// with owner write and search permission and names it; a later call tries again.
    fn restore(&mut self) -> Result<(), FsError> {
        if let Some(mode) = self.original_mode {
            set_mode_back(&self.directory, mode, &self.path)?;
            self.original_mode = None;
        }
        Ok(())
    }
}

#[cfg(test)]
thread_local! {
    /// The failure that the next `set_mode_back` of this thread reports instead of
    /// changing the mode (tests of a filesystem that turned read-only).
    pub(super) static FAIL_RESTORE: std::cell::Cell<Option<Errno>> =
        const { std::cell::Cell::new(None) };
}

/// Sets the mode of the folder `directory`, found at `path`, back to `mode`, which it had
/// before a deletion added owner write and search permission. A failure names the folder
/// and says what it keeps; its kind is Other, whatever the system call reported, so it
/// is never taken for a cancellation or a refusal of the deletion itself.
pub(super) fn set_mode_back(directory: &File, mode: RawMode, path: &Path) -> Result<(), FsError> {
    #[cfg(test)]
    if let Some(failure) = FAIL_RESTORE.take() {
        return Err(restore_error(path, mode, failure));
    }
    fchmod(directory, Mode::from_raw_mode(mode))
        .map_err(|failure| restore_error(path, mode, failure))
}

fn restore_error(path: &Path, mode: RawMode, failure: Errno) -> FsError {
    error(
        path,
        ErrorKind::Other,
        &format!(
            "This folder got owner write and search permission so that its entries could be \
             deleted, and setting its own mode {mode:04o} again failed: {}. It keeps those \
             permissions; remove them yourself if the folder should stay protected.",
            io::Error::from(failure)
        ),
    )
}

/// `failure`, followed by `issue`.
pub(super) fn with_failure(mut failure: FsError, issue: FsError) -> FsError {
    failure.message.push_str(&format!("; {issue}"));
    failure
}

/// Why a folder of the selected tree that lacks owner write or search permission is not
/// given them (`crate::ownership::keeps_set_group_id`).
pub(super) const SET_GROUP_ID_KEPT: &str =
    "a change would clear its set-group-ID bit, because you are not a member of its group";

/// What the walk does after one entry of the current folder.
enum Next {
    Continue,
    Descend(Frame),
    /// The current folder's entries cannot leave it and the caller skipped it.
    KeepFolder,
}

struct Walk<'a, 'b> {
    cancel: &'a Cancellation,
    decide: &'a mut Decide<'b>,
    owner: u32,
    skipped: u64,
}

/// Deletes the staged entry and returns how many failures were skipped: when any
/// were, what they left (with the folders that contain it) stays at the staged name.
pub(super) fn delete(
    staging: &Arc<File>,
    identity: Identity,
    path: &Path,
    cancel: &Cancellation,
    progress: &mut dyn FnMut(&Path),
    decide: &mut Decide<'_>,
) -> Result<u64, FsError> {
    let mut walk = Walk {
        cancel,
        decide,
        owner: rustix::process::geteuid().as_raw(),
        skipped: 0,
    };
    let name = OsStr::new(STAGED_NAME);
    if identity.kind != FileType::Directory {
        loop {
            match unlink(staging, name, identity, path, cancel) {
                Ok(()) => {
                    progress(path);
                    break;
                }
                Err(failure) => {
                    if !walk.retry(failure, true)? {
                        break;
                    }
                }
            }
        }
        return Ok(walk.skipped);
    }
    let root = loop {
        match open_frame(staging, name, identity, path, walk.owner) {
            Ok(frame) => break frame,
            Err(failure) => {
                if !walk.retry(failure, true)? {
                    return Ok(walk.skipped);
                }
            }
        }
    };
    let mut stack = vec![root];
    let result = walk.run(&mut stack, progress);
    // Folders still open when the deletion ended early stay, with their own modes; a
    // failure to give one back is added to the reason it ended. The walk returns Ok only
    // once every folder was handled.
    let unrestored: Vec<_> = stack
        .iter_mut()
        .rev()
        .filter_map(|frame| frame.restore().err())
        .collect();
    result
        .map(|()| walk.skipped)
        .map_err(|failure| unrestored.into_iter().fold(failure, with_failure))
}

impl Walk<'_, '_> {
    fn run(
        &mut self,
        stack: &mut Vec<Frame>,
        progress: &mut dyn FnMut(&Path),
    ) -> Result<(), FsError> {
        while let Some(frame) = stack.last_mut() {
            check_cancel(self.cancel, &frame.path)?;
            let next = match frame.entries.next() {
                Some(Ok(next)) => next,
                // The listing cannot go on: the folder stays with what is left in it.
                Some(Err(e)) => {
                    let failure = native_error(&frame.path, e);
                    self.retry(failure, false)?;
                    let frame = stack.pop().expect("nonempty stack");
                    self.keep(frame, stack)?;
                    continue;
                }
                None => {
                    let frame = stack.pop().expect("nonempty stack");
                    self.close(frame, stack, progress)?;
                    continue;
                }
            };
            let name = OsStr::from_bytes(next.file_name().to_bytes()).to_owned();
            if name == "." || name == ".." {
                continue;
            }
            match self.entry(frame, &name, progress)? {
                Next::Continue => {}
                Next::Descend(child) => stack.push(child),
                Next::KeepFolder => {
                    let frame = stack.pop().expect("nonempty stack");
                    self.keep(frame, stack)?;
                }
            }
        }
        Ok(())
    }

    /// Removes one entry of `frame`, or opens it when it is a folder.
    fn entry(
        &mut self,
        frame: &mut Frame,
        name: &OsStr,
        progress: &mut dyn FnMut(&Path),
    ) -> Result<Next, FsError> {
        let path = frame.path.join(name);
        // One no-follow lookup on the held directory is both the identity and the
        // live check immediately before unlink. Directories are opened below with
        // RESOLVE_NO_XDEV; other entries are probed for a mount only when their
        // device differs from the directory's.
        let identity = loop {
            match statat(&*frame.directory, name, AtFlags::SYMLINK_NOFOLLOW) {
                Ok(current) => break Identity::from_stat(&current),
                // Gone already: nothing is left to delete.
                Err(Errno::NOENT) => return Ok(Next::Continue),
                Err(e) => {
                    if let Some(next) = self.removal_failure(frame, name, &path, e)? {
                        return Ok(next);
                    }
                }
            }
        };
        if identity.kind == FileType::Directory {
            loop {
                let failure = match open_frame(&frame.directory, name, identity, &path, self.owner)
                {
                    Ok(child) => return Ok(Next::Descend(child)),
                    Err(failure) => failure,
                };
                let failure = if failure.kind == ErrorKind::PermissionDenied
                    && statat(&*frame.directory, name, AtFlags::SYMLINK_NOFOLLOW)
                        .is_ok_and(|stat| stat.st_uid != self.owner)
                {
                    error(
                        &path,
                        ErrorKind::PermissionDenied,
                        "This folder is owned by another user, who does not allow reading it",
                    )
                } else {
                    failure
                };
                if !self.retry(failure, true)? {
                    frame.keeps_skipped = true;
                    return Ok(Next::Continue);
                }
            }
        }
        loop {
            match other_mount(
                &frame.directory,
                name,
                &path,
                identity.device,
                frame.identity.device,
            ) {
                Ok(false) => break,
                Ok(true) => {
                    let failure = error(
                        &path,
                        ErrorKind::Unsupported,
                        "deletion does not cross mount boundaries",
                    );
                    self.retry(failure, false)?;
                    frame.keeps_skipped = true;
                    return Ok(Next::Continue);
                }
                Err(failure) => {
                    if !self.retry(failure, true)? {
                        frame.keeps_skipped = true;
                        return Ok(Next::Continue);
                    }
                }
            }
        }
        loop {
            check_cancel(self.cancel, &path)?;
            match unlinkat(&*frame.directory, name, AtFlags::empty()) {
                Ok(()) => {
                    progress(&path);
                    return Ok(Next::Continue);
                }
                Err(Errno::NOENT) => return Ok(Next::Continue),
                Err(e) => {
                    if let Some(next) = self.removal_failure(frame, name, &path, e)? {
                        return Ok(next);
                    }
                }
            }
        }
    }

    /// Removes a folder whose entries were all handled, unless it keeps skipped ones.
    fn close(
        &mut self,
        mut frame: Frame,
        stack: &mut Vec<Frame>,
        progress: &mut dyn FnMut(&Path),
    ) -> Result<(), FsError> {
        if frame.keeps_skipped {
            return self.keep(frame, stack);
        }
        loop {
            let failure = match unlink(
                &frame.parent,
                &frame.name,
                frame.identity,
                &frame.path,
                self.cancel,
            ) {
                Ok(()) => {
                    progress(&frame.path);
                    return Ok(());
                }
                Err(failure) if failure.kind == ErrorKind::Interrupted => return Err(failure),
                // Gone already: nothing is left to delete.
                Err(failure) if failure.kind == ErrorKind::NotFound => return Ok(()),
                Err(failure) => failure,
            };
            // A parent that keeps its entries is named once, for all of them (EACCES
            // and EPERM are both PermissionDenied).
            if failure.kind == ErrorKind::PermissionDenied
                && let Some(parent) = stack.last()
                && let Some(blocked) = self.blocked_folder(parent)
            {
                // The folder has its own mode while the parent's failure waits for a
                // decision; a failure to give it back is offered once the folder stays.
                let restored = frame.restore();
                let retry = self
                    .retry(blocked, true)
                    .map_err(|stop| restored.err().into_iter().fold(stop, with_failure))?;
                if retry {
                    continue;
                }
                self.restore(&mut frame)?;
                let parent = stack.pop().expect("a parent frame");
                return self.keep(parent, stack);
            }
            if self.retry(failure, true)? {
                continue;
            }
            return self.keep(frame, stack);
        }
    }

    /// A folder that stays: its original mode returns (`restore`), and so does its
    /// parent's need to stay.
    fn keep(&mut self, mut frame: Frame, stack: &mut [Frame]) -> Result<(), FsError> {
        if let Some(parent) = stack.last_mut() {
            parent.keeps_skipped = true;
        }
        self.restore(&mut frame)
    }

    /// Gives a folder that stays its original mode back. A failure names the folder and
    /// is offered like any other: Retry sets the mode again, Skip leaves the folder with
    /// owner write and search permission, Cancel ends the deletion.
    fn restore(&mut self, frame: &mut Frame) -> Result<(), FsError> {
        while let Err(failure) = frame.restore() {
            if !self.retry(failure, true)? {
                break;
            }
        }
        Ok(())
    }

    /// A failure to look up or remove `name` in `frame`. `None` to try again, or what
    /// the walk does once the entry, or the whole folder, was skipped.
    fn removal_failure(
        &mut self,
        frame: &mut Frame,
        name: &OsStr,
        path: &Path,
        e: Errno,
    ) -> Result<Option<Next>, FsError> {
        if matches!(e, Errno::ACCESS | Errno::PERM)
            && let Some(blocked) = self.blocked_folder(frame)
        {
            return Ok((!self.retry(blocked, true)?).then_some(Next::KeepFolder));
        }
        let failure = if e == Errno::PERM
            && statat(&*frame.directory, name, AtFlags::SYMLINK_NOFOLLOW)
                .is_ok_and(|stat| stat.st_uid != self.owner)
        {
            error(
                path,
                ErrorKind::PermissionDenied,
                "This item is owned by another user, and its folder lets only an item's \
                 owner delete it",
            )
        } else {
            native_error(path, e)
        };
        if self.retry(failure, true)? {
            return Ok(None);
        }
        frame.keeps_skipped = true;
        Ok(Some(Next::Continue))
    }

    /// The failure that names `folder` when the current user cannot remove entries
    /// from it; `None` while its permissions allow that (an entry's own cause).
    fn blocked_folder(&self, folder: &Frame) -> Option<FsError> {
        accessat(
            &*folder.directory,
            ".",
            Access::WRITE_OK | Access::EXEC_OK,
            AtFlags::EACCESS,
        )
        .is_err()
        .then(|| {
            let owned = fstat(&*folder.directory).is_ok_and(|stat| stat.st_uid == self.owner);
            let message = match (&folder.unprotect_error, owned) {
                (Some(reason), true) => format!(
                    "This folder is write-protected, so nothing in it can be deleted, and its \
                     permissions could not be changed: {reason}"
                ),
                (None, true) => {
                    "This folder is write-protected, so nothing in it can be deleted".into()
                }
                (_, false) => "This folder is owned by another user, who does not allow \
                               deleting anything in it"
                    .into(),
            };
            error(&folder.path, ErrorKind::PermissionDenied, &message)
        })
    }

    /// Offers `failure`: true to try the same step again, false once it was skipped.
    /// Cancel ends the deletion.
    fn retry(&mut self, failure: FsError, can_retry: bool) -> Result<bool, FsError> {
        if failure.kind == ErrorKind::Interrupted {
            return Err(failure);
        }
        let path = failure.path.clone();
        match (self.decide)(DeleteEntryFailure {
            error: failure,
            can_retry,
        })? {
            DeleteFailureChoice::Retry if can_retry => Ok(true),
            DeleteFailureChoice::Retry | DeleteFailureChoice::Skip => {
                self.skipped += 1;
                Ok(false)
            }
            DeleteFailureChoice::Cancel => Err(error(
                &path,
                ErrorKind::Interrupted,
                "operation cancelled; completed deletions remain",
            )),
        }
    }
}

/// Opens and verifies a folder of the selected tree. One that the current user owns
/// but whose entries could not leave it gets owner write and search permission
/// through this descriptor, never by path; failing that, the reason is kept for the
/// failure that removing its entries then reports.
fn open_frame(
    parent: &Arc<File>,
    name: &OsStr,
    identity: Identity,
    path: &Path,
    owner: u32,
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
    let stat = fstat(&directory).map_err(|e| native_error(path, e))?;
    if Identity::from_stat(&stat) != identity {
        return Err(error(
            path,
            ErrorKind::InvalidInput,
            "directory changed before traversal",
        ));
    }
    let mode = stat.st_mode & 0o7777;
    let (original_mode, unprotect_error) =
        if stat.st_uid == owner && mode & OWNER_REMOVAL != OWNER_REMOVAL {
            if !crate::ownership::keeps_set_group_id(
                mode,
                stat.st_gid,
                crate::ownership::caller_in_group,
            ) {
                (None, Some(SET_GROUP_ID_KEPT.into()))
            } else {
                match fchmod(&directory, Mode::from_raw_mode(mode | OWNER_REMOVAL)) {
                    Ok(()) => (Some(mode), None),
                    Err(e) => (None, Some(io::Error::from(e).to_string())),
                }
            }
        } else {
            (None, None)
        };
    let entries = match Dir::read_from(&directory) {
        Ok(entries) => entries,
        Err(e) => {
            let failure = native_error(path, e);
            return Err(match original_mode {
                Some(mode) => match set_mode_back(&directory, mode, path) {
                    Ok(()) => failure,
                    Err(issue) => with_failure(failure, issue),
                },
                None => failure,
            });
        }
    };
    Ok(Frame {
        parent: parent.clone(),
        directory: Arc::new(directory),
        entries,
        name: name.to_owned(),
        path: path.to_owned(),
        identity,
        original_mode,
        unprotect_error,
        keeps_skipped: false,
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
