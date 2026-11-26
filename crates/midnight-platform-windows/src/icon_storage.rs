//! Native configuration-root authority for immutable command artwork snapshots.
use midnight_core::Cancellation;
use midnight_platform_common::icon_snapshot::SnapshotPixels;
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io,
    os::windows::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    time::SystemTime,
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_GENERIC_READ, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_WRITE_ATTRIBUTES,
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
    // Attribute access lets a reused snapshot restart its collection grace period.
    let file = match OpenOptions::new()
        .access_mode(FILE_GENERIC_READ | FILE_WRITE_ATTRIBUTES)
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
fn create_exclusive(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}
/// The modification time of a regular file; links and other reparse points are `None`.
fn plain_file_modified(path: &Path) -> io::Result<Option<SystemTime>> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Ok(None);
    }
    metadata.modified().map(Some)
}

/// After the preferences beside `icons` were saved: removes the snapshots they do not
/// name (`icon_snapshot::collect`). A missing folder has nothing to collect.
pub(crate) fn collect_at(parent: &Path, referenced: &BTreeSet<String>) -> io::Result<usize> {
    let pin = |path: &Path| match pin_directory(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        result => result.map(Some),
    };
    let Some(_parent) = pin(parent)? else {
        return Ok(0);
    };
    let directory = parent.join("icons");
    let Some(_directory) = pin(&directory)? else {
        return Ok(0);
    };
    // DeleteFile removes the entry itself: a link that replaced a checked file is
    // removed without touching its target.
    midnight_platform_common::icon_snapshot::collect(
        &directory,
        referenced,
        create_exclusive,
        open_existing,
        plain_file_modified,
        |path| fs::remove_file(path),
    )
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
        create_exclusive,
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
        let root = crate::test_fixtures::build_temp("fixture-");
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

    /// Collection removes only old regular snapshot files: a junction, a file link and a
    /// file with another reparse point under snapshot names, and their targets, stay.
    #[test]
    fn collection_leaves_links_reparse_points_and_their_targets_alone() {
        use crate::test_fixtures::{FILTER_FILE, clear_filter_tag, new_junction, set_filter_tag};
        use std::time::{Duration, SystemTime};
        let root = crate::test_fixtures::build_temp("fixture-");
        let application = root.path().join("配置 文档");
        let icons = application.join("icons");
        fs::create_dir_all(&icons).unwrap();
        let outside = root.path().join("outside");
        fs::create_dir(&outside).unwrap();
        let target = outside.join("target.png");
        fs::write(&target, b"untouched").unwrap();
        let name = |index: u8| icons.join(format!("{index:064x}.png"));
        let old = SystemTime::now() - Duration::from_secs(30 * 24 * 60 * 60);
        // Ages the entry itself, never a link's target.
        let age = |path: &Path| {
            OpenOptions::new()
                .access_mode(FILE_WRITE_ATTRIBUTES)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
                .open(path)
                .unwrap()
                .set_modified(old)
                .unwrap();
            assert_eq!(fs::symlink_metadata(path).unwrap().modified().unwrap(), old);
        };
        new_junction(&name(1), &outside).unwrap();
        age(&name(1));
        fs::write(name(2), b"tagged").unwrap();
        set_filter_tag(&name(2), FILTER_FILE).unwrap();
        age(&name(2));
        let file_link = std::os::windows::fs::symlink_file(&target, name(3)).is_ok();
        if file_link {
            age(&name(3));
        } else {
            eprintln!("file symbolic links need Developer Mode here; the junction stays covered");
        }
        fs::write(name(4), b"plain").unwrap();
        age(&name(4));

        assert_eq!(collect_at(&application, &BTreeSet::new()).unwrap(), 1);
        assert!(!name(4).exists(), "the old plain snapshot was collected");
        assert!(
            fs::symlink_metadata(name(1)).unwrap().is_symlink(),
            "the junction stays"
        );
        assert!(fs::symlink_metadata(name(2)).is_ok());
        assert_eq!(file_link, fs::symlink_metadata(name(3)).is_ok());
        assert_eq!(fs::read(&target).unwrap(), b"untouched");
        assert_eq!(fs::read_dir(&outside).unwrap().count(), 1);
        // A profile without an icons folder has nothing to collect and creates none.
        let fresh = root.path().join("fresh");
        fs::create_dir(&fresh).unwrap();
        assert_eq!(collect_at(&fresh, &BTreeSet::new()).unwrap(), 0);
        assert!(!fresh.join("icons").exists());
        clear_filter_tag(&name(2), FILTER_FILE).unwrap();
    }
}
