//! Make an installation the user owns private again: remove group and other write bits.
use super::effective_user;
use crate::Error;
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::Path,
};

/// Remove group and other write bits from an object the user owns. Links are never followed:
/// callers skip them, and a link swapped in meanwhile is refused here.
fn remove_shared_writes(path: &Path) -> Result<(), Error> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Err(Error(format!("{} is a link", path.display())));
    }
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
    fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    Ok(())
}

/// Only the owner may secure a folder.
pub(crate) fn require_owner(folder: &Path) -> Result<(), Error> {
    let metadata = fs::symlink_metadata(folder)?;
    if metadata.file_type().is_symlink() || metadata.uid() != effective_user() {
        return Err(Error(format!(
            "{} belongs to another account, so only its owner or root can change who may \
             modify it",
            folder.display()
        )));
    }
    Ok(())
}

/// See [`crate::secure_installation`].
pub(crate) fn secure_folder(folder: &Path) -> Result<(), Error> {
    remove_shared_writes(folder)
}

/// `_expected` (the canonical path) is not checked here yet; see SECURITY.md.
pub(crate) fn secure_entry(path: &Path, _expected: &Path) -> Result<(), Error> {
    remove_shared_writes(path)
}

/// Keep a copied file's execute and read bits, without group or other write.
pub(crate) fn copy_permissions(source: &Path, target: &Path) -> Result<(), Error> {
    let mode = fs::symlink_metadata(source)?.mode() & 0o755;
    fs::set_permissions(target, fs::Permissions::from_mode(mode))?;
    Ok(())
}
