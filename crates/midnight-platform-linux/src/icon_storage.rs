//! Descriptor-pinned native configuration storage for command artwork snapshots.
use midnight_core::Cancellation;
use midnight_platform_common::icon_snapshot::SnapshotPixels;
use rustix::fs::{Mode, OFlags, mkdirat, open, openat};
use std::{
    fs::{self, File},
    io,
    os::{fd::AsRawFd, unix::fs::DirBuilderExt},
    path::{Path, PathBuf},
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
        |path| {
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
        },
        open_existing,
    )?;
    let name = result
        .file_name()
        .ok_or_else(|| io::Error::other("Missing icon snapshot name"))?;
    Ok(parent.join("icons").join(name))
}

fn private_directory(directory: &impl std::os::fd::AsFd) -> io::Result<()> {
    let metadata = rustix::fs::fstat(directory)?;
    if metadata.st_uid != rustix::process::geteuid().as_raw() || metadata.st_mode & 0o022 != 0 {
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
}
