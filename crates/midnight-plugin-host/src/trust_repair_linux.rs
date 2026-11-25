//! Make an installation the user owns private again: remove group and other write bits.
//!
//! Objects are inspected and changed through a descriptor opened without following links
//! (`O_PATH | O_NOFOLLOW`), and only when the descriptor's real location (the target of
//! `/proc/self/fd/<fd>`) is the expected one: another account that can still write in a folder
//! below the installation could otherwise swap an entry, or a folder on the way, for a
//! symbolic link during the repair and redirect the change to a file of the user's outside the
//! installation. The mode is changed through that `/proc/self/fd` link, which refers to the
//! opened object itself rather than to a name (`fchmod` refuses `O_PATH` descriptors).
use super::effective_user;
use crate::Error;
use std::{
    fs::{self, File, Metadata, OpenOptions},
    os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    },
    path::{Path, PathBuf},
};

/// Open the object at `path` itself, never a link's target, without reading or writing it.
fn open_path(path: &Path) -> Result<File, Error> {
    Ok(OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_PATH | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?)
}

/// The `/proc` link of an open descriptor, which leads to the opened object itself.
fn descriptor_link(file: &File) -> PathBuf {
    PathBuf::from(format!("/proc/self/fd/{}", file.as_raw_fd()))
}

/// Open the object at `path` itself (see [`open_path`]), and only when it is no link and its
/// real location is `expected`: no folder on the way was swapped for a link that leads out of
/// the installation. Without `/proc` the location cannot be verified, so it is refused.
fn open_entry(path: &Path, expected: &Path) -> Result<(File, Metadata), Error> {
    let file = open_path(path)?;
    let metadata = file.metadata()?;
    if metadata.file_type().is_symlink() {
        return Err(Error(format!("{} is a link", path.display())));
    }
    let location = fs::read_link(descriptor_link(&file)).map_err(|error| {
        Error(format!(
            "{}: its location cannot be verified without /proc ({error})",
            path.display()
        ))
    })?;
    if location != expected {
        return Err(Error(format!(
            "{} leads out of the installation; a folder on its way was replaced",
            path.display()
        )));
    }
    Ok((file, metadata))
}

/// Remove group and other write bits from an object the user owns. Links are never followed:
/// callers skip them, and a link swapped in meanwhile, as the object or on its way, is refused
/// by [`open_entry`]. Ownership and mode come from the opened object, and the change is made
/// through its descriptor, never by name again.
fn remove_shared_writes(path: &Path, expected: &Path) -> Result<(), Error> {
    let (file, metadata) = open_entry(path, expected)?;
    if metadata.mode() & 0o022 == 0 {
        return Ok(());
    }
    if metadata.uid() != effective_user() {
        return Err(Error(format!(
            "{} belongs to another account, so only its owner or root can change it",
            path.display()
        )));
    }
    let mode = metadata.mode() & 0o7777 & !0o022;
    fs::set_permissions(descriptor_link(&file), fs::Permissions::from_mode(mode))?;
    Ok(())
}

/// Only the owner may secure a folder; the folder itself is inspected, never a link's target.
pub(crate) fn require_owner(folder: &Path) -> Result<(), Error> {
    let metadata = open_path(folder)?.metadata()?;
    if metadata.file_type().is_symlink() || metadata.uid() != effective_user() {
        return Err(Error(format!(
            "{} belongs to another account, so only its owner or root can change who may \
             modify it",
            folder.display()
        )));
    }
    Ok(())
}

/// See [`crate::secure_installation`]. The folder is changed only while its real location is
/// its canonical path (see [`open_entry`]).
pub(crate) fn secure_folder(folder: &Path) -> Result<(), Error> {
    remove_shared_writes(folder, &folder.canonicalize()?)
}

/// `expected` is the object's real location: the installation's canonical path joined with
/// the object's path inside it (see [`remove_shared_writes`]).
pub(crate) fn secure_entry(path: &Path, expected: &Path) -> Result<(), Error> {
    remove_shared_writes(path, expected)
}

/// Keep a copied file's execute and read bits, without group or other write.
pub(crate) fn copy_permissions(source: &Path, target: &Path) -> Result<(), Error> {
    let mode = fs::symlink_metadata(source)?.mode() & 0o755;
    fs::set_permissions(target, fs::Permissions::from_mode(mode))?;
    Ok(())
}
