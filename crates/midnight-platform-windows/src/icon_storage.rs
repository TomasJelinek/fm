//! Native configuration-root authority for immutable command artwork snapshots.
use midnight_core::Cancellation;
use midnight_platform_common::icon_snapshot::SnapshotPixels;
use std::{
    fs::{self, File, OpenOptions},
    io,
    os::windows::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_SHARE_READ, FILE_SHARE_WRITE,
};

pub(crate) fn save(pixels: &SnapshotPixels, cancel: &Cancellation) -> io::Result<PathBuf> {
    let preferences = crate::settings::native_preferences_path()?;
    let parent = preferences
        .parent()
        .ok_or_else(|| io::Error::other("Missing configuration parent"))?;
    save_at(parent, pixels, cancel)
}

fn pin_directory(path: &Path) -> io::Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "The icon directory must be an ordinary native directory.",
        ));
    }
    // Denying delete sharing pins each managed directory until publication finishes.
    Ok(file)
}
fn open_existing(path: &Path) -> io::Result<Option<File>> {
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "A managed icon cannot be a link or special file.",
        ));
    }
    Ok(Some(file))
}
fn save_at(parent: &Path, pixels: &SnapshotPixels, cancel: &Cancellation) -> io::Result<PathBuf> {
    let snapshot = midnight_platform_common::icon_snapshot::encode(pixels, cancel)?;
    fs::create_dir_all(parent)?;
    let _parent = pin_directory(parent)?;
    let directory = parent.join("icons");
    match fs::create_dir(&directory) {
        Ok(()) => (),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (),
        Err(error) => return Err(error),
    }
    let _directory = pin_directory(&directory)?;
    midnight_platform_common::icon_snapshot::publish(
        &directory,
        &snapshot,
        cancel,
        |path| {
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
                .open(path)
        },
        open_existing,
    )
}

#[cfg(test)]
pub(crate) fn save_test(
    parent: &Path,
    pixels: &SnapshotPixels,
    cancel: &Cancellation,
) -> io::Result<PathBuf> {
    save_at(parent, pixels, cancel)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn managed_snapshot_is_native_unicode_and_cannot_replace_a_directory() {
        let root = tempfile::tempdir().unwrap();
        let application = root.path().join("配置 文档");
        let pixels = SnapshotPixels::new(1, 1, vec![20, 40, 60, 255]).unwrap();
        let path = save_at(&application, &pixels, &Cancellation::default()).unwrap();
        assert!(path.starts_with(application.join("icons")));
        assert_eq!(path.extension().unwrap(), "png");
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        assert!(save_at(&application, &pixels, &Cancellation::default()).is_err());
        assert!(path.is_dir());
    }
}
