//! Short, exclusively owned Chromium socket storage, separate from the persistent profile path.
use rustix::{
    fs::{self, AtFlags, Mode, OFlags},
    process::geteuid,
};
use std::{
    fs::File,
    io,
    os::unix::fs::MetadataExt,
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::SystemTime,
};

static NEXT: AtomicU64 = AtomicU64::new(1);
const FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::CLOEXEC)
    .union(OFlags::NOFOLLOW);

pub(super) struct Directory {
    path: PathBuf,
    directory: File,
    parent: File,
    name: String,
    cleanup: bool,
}

fn open_directory(path: &Path) -> io::Result<File> {
    let mut directory = File::from(fs::open("/", FLAGS, Mode::empty())?);
    let mut components = path.components().peekable();
    while let Some(component) = components.next() {
        match component {
            Component::RootDir => {}
            Component::Normal(name) => {
                directory = File::from(fs::openat(&directory, name, FLAGS, Mode::empty())?);
            }
            _ => return Err(io::Error::other("Invalid browser temporary parent")),
        }
        if components.peek().is_some() {
            let metadata = directory.metadata()?;
            if metadata.uid() != 0 || metadata.mode() & 0o022 != 0 {
                return Err(io::Error::other(
                    "Browser temporary ancestry must be root-owned and protected",
                ));
            }
        }
    }
    Ok(directory)
}

fn parent() -> io::Result<(PathBuf, File)> {
    let runtime = PathBuf::from(format!("/run/user/{}", geteuid().as_raw()));
    match open_directory(&runtime) {
        Ok(directory) => {
            let metadata = directory.metadata()?;
            if metadata.uid() != geteuid().as_raw() || metadata.mode() & 0o077 != 0 {
                return Err(io::Error::other(
                    "Browser runtime directory must be owned and mode 0700",
                ));
            }
            Ok((runtime, directory))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let path = PathBuf::from("/tmp");
            let directory = open_directory(&path)?;
            let metadata = directory.metadata()?;
            if metadata.uid() != 0 || metadata.mode() & 0o1000 == 0 {
                return Err(io::Error::other(
                    "Browser temporary fallback needs root-owned sticky /tmp",
                ));
            }
            Ok((path, directory))
        }
        Err(error) => Err(error),
    }
}

impl Directory {
    pub(super) fn new() -> io::Result<Self> {
        let (path, parent) = parent()?;
        for _ in 0..64 {
            let stamp = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map_err(io::Error::other)?
                .as_nanos() as u64;
            let nonce = stamp
                ^ u64::from(std::process::id()).rotate_left(32)
                ^ NEXT.fetch_add(1, Ordering::Relaxed);
            let name = format!("mwv-{nonce:016x}");
            match fs::mkdirat(&parent, &name, Mode::RUSR | Mode::WUSR | Mode::XUSR) {
                Ok(()) => {}
                Err(rustix::io::Errno::EXIST) => continue,
                Err(error) => return Err(error.into()),
            }
            let directory = match fs::openat(&parent, &name, FLAGS, Mode::empty()) {
                Ok(directory) => File::from(directory),
                Err(error) => {
                    let _ = fs::unlinkat(&parent, &name, AtFlags::REMOVEDIR);
                    return Err(error.into());
                }
            };
            let metadata = directory.metadata()?;
            if metadata.uid() != geteuid().as_raw() || metadata.mode() & 0o777 != 0o700 {
                let _ = fs::unlinkat(&parent, &name, AtFlags::REMOVEDIR);
                return Err(io::Error::other(
                    "New browser temporary directory is not private",
                ));
            }
            return Ok(Self {
                path: path.join(&name),
                directory,
                parent,
                name,
                cleanup: true,
            });
        }
        Err(io::Error::other(
            "Cannot exclusively create browser temporary directory",
        ))
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    pub(super) fn retain_until_reaped(&mut self) {
        self.cleanup = false;
    }

    pub(super) fn tree_reaped(&mut self) {
        self.cleanup = true;
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        // A failed reap retains state for possibly surviving processes. Never remove another inode.
        let visible_parent = self
            .path
            .parent()
            .and_then(|path| open_directory(path).ok());
        if self.cleanup
            && let Some(visible_parent) = visible_parent
            && let (Ok(pinned), Ok(visible)) = (self.parent.metadata(), visible_parent.metadata())
            && pinned.dev() == visible.dev()
            && pinned.ino() == visible.ino()
            && let (Ok(owned), Ok(named)) = (
                self.directory.metadata(),
                fs::statat(&self.parent, &self.name, AtFlags::SYMLINK_NOFOLLOW),
            )
            && owned.dev() == named.st_dev
            && owned.ino() == named.st_ino
        {
            // Protected ancestry excludes other-account replacement. Same UID/root remain outside
            // this boundary, as for the private profile. Never follow child symlinks during cleanup.
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}
