//! Query the installed OS, preserving raw UTF-16 and dynamically sized paths.
use crate::WindowsFileSystem;
use midnight_core::{FsError, NavigationFallbackService};
use std::path::{Path, PathBuf};

impl NavigationFallbackService for WindowsFileSystem {
    fn navigation_fallback_root(&self) -> Result<PathBuf, FsError> {
        let directory = crate::desktop::system_directory().map_err(|error| {
            FsError::from_io("locate Windows system directory", Path::new(""), error)
        })?;
        directory
            .ancestors()
            .last()
            .filter(|root| root.is_absolute())
            .map(Path::to_owned)
            .ok_or_else(|| {
                FsError::from_io(
                    "locate Windows drive root",
                    &directory,
                    std::io::ErrorKind::InvalidData.into(),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_is_the_absolute_drive_containing_the_os() {
        let root = WindowsFileSystem::new(None)
            .navigation_fallback_root()
            .unwrap();
        let system = crate::desktop::system_directory().unwrap();
        assert!(root.is_absolute());
        assert!(root.parent().is_none());
        assert!(system.starts_with(&root));
    }
}
