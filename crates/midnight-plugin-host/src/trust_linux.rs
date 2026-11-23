use crate::Error;
use std::{
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

/// Whether other accounts may change an object through its group or other write bits. That
/// alone is not an integrity failure: plugins still load from such a folder, and the
/// application reports the shared installation once (see `installation_sharing`). A symbolic
/// link and an owner other than the user or root stay refusals.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Access {
    Private,
    Shared,
}

/// Validate `path` and every ancestor, accepting objects that only their mode makes shared.
pub(crate) fn check(path: &Path) -> Result<(), Error> {
    inspect(path).map(drop)
}

/// [`check`] that also refuses an object or ancestor other accounts can change.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn check_private(path: &Path) -> Result<(), Error> {
    match inspect(path)? {
        None => Ok(()),
        Some(shared) => Err(shared_error(&shared)),
    }
}

/// The first of `path` and its ancestors that other accounts can change, after validating all
/// of them: an integrity failure anywhere is returned even above a shared folder.
pub(crate) fn inspect(path: &Path) -> Result<Option<PathBuf>, Error> {
    Ok(match inspect_split(path)? {
        (Access::Shared, _) => Some(path.to_owned()),
        (Access::Private, ancestor) => ancestor,
    })
}

/// [`inspect`] that keeps `path`'s own access apart from the first shared ancestor.
pub(crate) fn inspect_split(path: &Path) -> Result<(Access, Option<PathBuf>), Error> {
    let user = effective_user();
    let mut own = Access::Private;
    let mut ancestor = None;
    for (index, component) in path.ancestors().enumerate() {
        let access = inspect_one(component, index, user)?;
        if index == 0 {
            own = access;
        } else if access == Access::Shared && ancestor.is_none() {
            ancestor = Some(component.to_owned());
        }
    }
    Ok((own, ancestor))
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn shared_error(path: &Path) -> Error {
    Error(format!(
        "Plugin path is writable or replaceable by another account: {}",
        path.display()
    ))
}

pub(crate) fn effective_user() -> u32 {
    // SAFETY: get effective UID has no pointer arguments or side effects.
    unsafe { libc::geteuid() }
}

/// Checks the objects inside a folder that [`check`] has just accepted with every folder
/// above it: each object on its own, as [`check`] tests a path's first component, without
/// repeating the ancestors for every object.
pub(crate) struct EntryCheck(u32);

impl EntryCheck {
    pub(crate) fn new() -> Result<Self, Error> {
        Ok(Self(effective_user()))
    }

    pub(crate) fn check(&self, path: &Path) -> Result<(), Error> {
        self.inspect(path).map(drop)
    }

    pub(crate) fn inspect(&self, path: &Path) -> Result<Access, Error> {
        inspect_one(path, 0, self.0)
    }
}

fn inspect_one(component: &Path, index: usize, user: u32) -> Result<Access, Error> {
    let metadata = std::fs::symlink_metadata(component)?;
    if metadata.file_type().is_symlink() || !matches!(metadata.uid(), 0) && metadata.uid() != user {
        return Err(Error(format!(
            "Plugin path is redirected or owned by another account: {}",
            component.display()
        )));
    }
    // A trusted-owned sticky ancestor such as /tmp prevents other users replacing
    // the already validated child. The installation directory itself is never shared.
    let sticky_parent = index != 0 && metadata.is_dir() && metadata.mode() & 0o1000 != 0;
    Ok(if metadata.mode() & 0o022 != 0 && !sticky_parent {
        Access::Shared
    } else {
        Access::Private
    })
}

#[path = "trust_repair_linux.rs"]
pub(crate) mod repair;

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    #[test]
    fn shared_writable_installation_roots_and_files_are_reported_not_refused() {
        let root = tempfile::tempdir().unwrap();
        assert!(check(root.path()).is_ok());
        let child = root.path().join("private");
        std::fs::create_dir(&child).unwrap();
        std::fs::set_permissions(&child, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(check_private(root.path()).is_err());
        assert!(check_private(&child).is_err());
        check(&child).unwrap();
        assert_eq!(inspect(&child).unwrap().as_deref(), Some(root.path()));
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let file = root.path().join("library.so");
        std::fs::write(&file, b"fixture").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o666)).unwrap();
        assert!(check_private(&file).is_err());
        check(&file).unwrap();
        assert_eq!(
            EntryCheck::new().unwrap().inspect(&file).unwrap(),
            Access::Shared
        );
        let link = root.path().join("link.so");
        std::os::unix::fs::symlink(&file, &link).unwrap();
        assert!(check(&link).is_err(), "a redirect stays refused");
    }
}
