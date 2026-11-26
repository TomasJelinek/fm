//! Descriptor-pinned native configuration storage for command artwork snapshots.
use midnight_core::Cancellation;
use midnight_platform_common::icon_snapshot::SnapshotPixels;
use rustix::fs::{Mode, OFlags, mkdirat, open, openat};
use std::{
    collections::BTreeSet,
    fs::{self, File},
    io,
    os::{fd::AsRawFd, unix::fs::DirBuilderExt},
    path::{Path, PathBuf},
    time::SystemTime,
};

pub(crate) fn save(
    home: Option<&Path>,
    pixels: &SnapshotPixels,
    cancel: &Cancellation,
) -> io::Result<PathBuf> {
    let preferences = crate::settings::preferences_path(std::env::var_os("XDG_CONFIG_HOME"), home)?;
    let parent = preferences
        .parent()
        .ok_or_else(|| io::Error::other("Missing configuration parent"))?;
    save_at(parent, pixels, cancel)
}
fn open_existing(path: &Path) -> io::Result<Option<File>> {
    let descriptor = match open(
        path,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(file) => file,
        Err(error) if error == rustix::io::Errno::NOENT => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let file = File::from(descriptor);
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "A managed icon must be a regular file.",
        ));
    }
    Ok(Some(file))
}
fn create_exclusive(path: &Path) -> io::Result<File> {
    open(
        path,
        OFlags::WRONLY
            | OFlags::CREATE
            | OFlags::EXCL
            | OFlags::NOFOLLOW
            | OFlags::NONBLOCK
            | OFlags::CLOEXEC,
        Mode::RUSR | Mode::WUSR,
    )
    .map(File::from)
    .map_err(io::Error::from)
}
/// The modification time of a regular file; links and special files are `None`.
fn plain_file_modified(path: &Path) -> io::Result<Option<SystemTime>> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() {
        return Ok(None);
    }
    metadata.modified().map(Some)
}

/// After the preferences beside `icons` were saved: removes the snapshots they do not
/// name (`icon_snapshot::collect`). A missing folder has nothing to collect.
pub(crate) fn collect_at(parent: &Path, referenced: &BTreeSet<String>) -> io::Result<usize> {
    let directory_flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let parent_fd = match open(parent, directory_flags, Mode::empty()) {
        Err(error) if error == rustix::io::Errno::NOENT => return Ok(0),
        result => result?,
    };
    private_directory(&parent_fd)?;
    let directory = match openat(&parent_fd, "icons", directory_flags, Mode::empty()) {
        Err(error) if error == rustix::io::Errno::NOENT => return Ok(0),
        result => result?,
    };
    private_directory(&directory)?;
    // As in publication, every entry resolves through the owned directory descriptor;
    // unlink removes the entry itself, so a link that replaced a checked file is removed
    // without touching its target.
    let scope = PathBuf::from(format!("/proc/self/fd/{}", directory.as_raw_fd()));
    midnight_platform_common::icon_snapshot::collect(
        &scope,
        referenced,
        create_exclusive,
        open_existing,
        plain_file_modified,
        |path| fs::remove_file(path),
    )
}
fn save_at(parent: &Path, pixels: &SnapshotPixels, cancel: &Cancellation) -> io::Result<PathBuf> {
    let snapshot = midnight_platform_common::icon_snapshot::encode(pixels, cancel)?;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(parent)?;
    let parent_fd = open(
        parent,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )?;
    private_directory(&parent_fd)?;
    match mkdirat(&parent_fd, "icons", Mode::RWXU) {
        Ok(()) => (),
        Err(error) if error == rustix::io::Errno::EXIST => (),
        Err(error) => return Err(error.into()),
    }
    let directory = openat(
        &parent_fd,
        "icons",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )?;
    private_directory(&directory)?;
    // Publication resolves exclusively through our owned directory descriptor, even
    // if a pathname ancestor is renamed concurrently. This private path is not persisted.
    let scope = PathBuf::from(format!("/proc/self/fd/{}", directory.as_raw_fd()));
    let result = midnight_platform_common::icon_snapshot::publish(
        &scope,
        &snapshot,
        cancel,
        create_exclusive,
        open_existing,
    )?;
    let name = result
        .file_name()
        .ok_or_else(|| io::Error::other("Missing icon snapshot name"))?;
    Ok(parent.join("icons").join(name))
}

fn private_directory(directory: &impl std::os::fd::AsFd) -> io::Result<()> {
    // Group write is accepted only for this user's private group, without an ACL.
    if !crate::ownership::writable_only_by_caller(directory)? {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Managed icon directories must be owned by this user and not writable by other users.",
        ));
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};
    #[test]
    fn managed_snapshot_is_private_and_refuses_linked_storage() {
        let root = tempfile::tempdir().unwrap();
        let application = root.path().join("配置 文档");
        let pixels = SnapshotPixels::new(1, 1, vec![20, 40, 60, 255]).unwrap();
        let path = save_at(&application, &pixels, &Cancellation::default()).unwrap();
        assert!(path.starts_with(application.join("icons")));
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        fs::set_permissions(application.join("icons"), fs::Permissions::from_mode(0o777)).unwrap();
        assert!(save_at(&application, &pixels, &Cancellation::default()).is_err());
        // Group write is accepted only for this user's private group.
        fs::set_permissions(application.join("icons"), fs::Permissions::from_mode(0o775)).unwrap();
        let group =
            std::os::unix::fs::MetadataExt::gid(&fs::metadata(application.join("icons")).unwrap());
        assert_eq!(
            save_at(&application, &pixels, &Cancellation::default()).is_ok(),
            crate::ownership::caller_private_group(group)
        );
        fs::set_permissions(application.join("icons"), fs::Permissions::from_mode(0o700)).unwrap();
        let outside = root.path().join("outside");
        fs::write(&outside, b"untouched").unwrap();
        fs::remove_file(&path).unwrap();
        symlink(&outside, &path).unwrap();
        assert!(save_at(&application, &pixels, &Cancellation::default()).is_err());
        assert_eq!(fs::read(&outside).unwrap(), b"untouched");
        let linked = root.path().join("linked");
        symlink(&application, &linked).unwrap();
        assert!(save_at(&linked, &pixels, &Cancellation::default()).is_err());
    }

    /// Collection removes only old regular snapshot files: a symbolic link under a
    /// snapshot name and its target stay, also when the link itself is old.
    #[test]
    fn collection_leaves_links_and_their_targets_alone() {
        use std::time::{Duration, SystemTime};
        let root = tempfile::tempdir().unwrap();
        let application = root.path().join("配置 文档");
        let pixels = SnapshotPixels::new(1, 1, vec![20, 40, 60, 255]).unwrap();
        save_at(&application, &pixels, &Cancellation::default()).unwrap();
        let icons = application.join("icons");
        let outside = root.path().join("outside.png");
        fs::write(&outside, b"untouched").unwrap();
        let name = |index: u8| icons.join(format!("{index:064x}.png"));
        symlink(&outside, name(1)).unwrap();
        // Ages the link itself (to 1970), never its target.
        let epoch = rustix::fs::Timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        rustix::fs::utimensat(
            rustix::fs::CWD,
            name(1),
            &rustix::fs::Timestamps {
                last_access: epoch,
                last_modification: epoch,
            },
            rustix::fs::AtFlags::SYMLINK_NOFOLLOW,
        )
        .unwrap();
        fs::write(name(2), b"plain").unwrap();
        File::options()
            .write(true)
            .open(name(2))
            .unwrap()
            .set_modified(SystemTime::now() - Duration::from_secs(30 * 24 * 60 * 60))
            .unwrap();

        assert_eq!(collect_at(&application, &BTreeSet::new()).unwrap(), 1);
        assert!(!name(2).exists(), "the old plain snapshot was collected");
        assert!(fs::symlink_metadata(name(1)).unwrap().is_symlink());
        assert_eq!(fs::read(&outside).unwrap(), b"untouched");
        // A profile without an icons folder has nothing to collect and creates none.
        let fresh = root.path().join("fresh");
        fs::create_dir(&fresh).unwrap();
        assert_eq!(collect_at(&fresh, &BTreeSet::new()).unwrap(), 0);
        assert!(!fresh.join("icons").exists());
    }

    /// An access ACL makes the group bits its mask, so a 0775 private-group
    /// folder can still let another user write.
    #[test]
    fn storage_shared_through_an_access_acl_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let application = root.path().join("application");
        let pixels = SnapshotPixels::new(1, 1, vec![20, 40, 60, 255]).unwrap();
        let cancel = Cancellation::default();
        save_at(&application, &pixels, &cancel).unwrap();
        let group = std::os::unix::fs::MetadataExt::gid(&fs::metadata(&application).unwrap());
        if !crate::ownership::caller_private_group(group) {
            eprintln!(
                "SKIP ACL-shared icon storage: group {group} is not this user's private group"
            );
            return;
        }
        if !crate::test_support::share_through_access_acl(&application, "ACL-shared icon storage") {
            return;
        }
        assert_eq!(
            fs::metadata(&application).unwrap().permissions().mode() & 0o7777,
            0o775
        );
        let refused = save_at(&application, &pixels, &cancel).unwrap_err();
        assert_eq!(refused.kind(), io::ErrorKind::PermissionDenied);
        // The same 0775 private-group folder without the ACL is the caller's alone.
        crate::test_support::remove_access_acl(&application);
        assert_eq!(
            fs::metadata(&application).unwrap().permissions().mode() & 0o7777,
            0o775
        );
        save_at(&application, &pixels, &cancel).unwrap();
    }
}
