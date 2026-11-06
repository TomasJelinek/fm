use crate::Error;
use std::{os::unix::fs::MetadataExt, path::Path};
pub(crate) fn check(path: &Path) -> Result<(), Error> {
    // SAFETY: get effective UID has no pointer arguments or side effects.
    let user = unsafe { libc::geteuid() };
    for (index, component) in path.ancestors().enumerate() {
        let metadata = std::fs::symlink_metadata(component)?;
        if metadata.file_type().is_symlink()
            || !matches!(metadata.uid(), 0) && metadata.uid() != user
        {
            return Err(Error(format!(
                "Plugin path is redirected or owned by another account: {}",
                component.display()
            )));
        }
        // A trusted-owned sticky ancestor such as /tmp prevents other users replacing
        // the already validated child. The installation directory itself is never shared.
        let sticky_parent = index != 0 && metadata.is_dir() && metadata.mode() & 0o1000 != 0;
        if metadata.mode() & 0o022 != 0 && !sticky_parent {
            return Err(Error(format!(
                "Plugin path is writable or replaceable by another account: {}",
                component.display()
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    #[test]
    fn shared_writable_installation_roots_and_files_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        assert!(check(root.path()).is_ok());
        let child = root.path().join("private");
        std::fs::create_dir(&child).unwrap();
        std::fs::set_permissions(&child, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(check(root.path()).is_err());
        assert!(check(&child).is_err());
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let file = root.path().join("library.so");
        std::fs::write(&file, b"fixture").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o666)).unwrap();
        assert!(check(&file).is_err());
    }
}
