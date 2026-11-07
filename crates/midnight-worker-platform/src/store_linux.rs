//! Handle-relative private cache paths; no final or intermediate symlink is traversed.
use super::{Record, valid_name};
use rustix::{
    fs::{self, AtFlags, Dir, Mode, OFlags},
    process::geteuid,
};
use std::{
    fs::File,
    io,
    os::unix::{ffi::OsStrExt, fs::MetadataExt},
    path::{Component, Path},
    sync::atomic::{AtomicU64, Ordering},
};

static TEMPORARIES: AtomicU64 = AtomicU64::new(1);
pub(super) struct Root {
    directory: File,
}
pub(super) struct Temporary {
    pub file: File,
    directory: File,
    name: String,
    published: bool,
}
impl Drop for Temporary {
    fn drop(&mut self) {
        if !self.published {
            // Cache roots are mode 0700. Verify the named inode remains our own before cleanup.
            if let (Ok(owned), Ok(named)) = (
                self.file.metadata(),
                fs::statat(&self.directory, &self.name, AtFlags::SYMLINK_NOFOLLOW),
            ) && owned.ino() == named.st_ino
                && owned.dev() == named.st_dev
            {
                let _ = fs::unlinkat(&self.directory, &self.name, AtFlags::empty());
            }
        }
    }
}

fn regular(file: File) -> io::Result<File> {
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.uid() != geteuid().as_raw() || metadata.mode() & 0o077 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Cache record is not private and regular",
        ));
    }
    Ok(file)
}

impl Root {
    pub fn open(path: &Path) -> io::Result<Self> {
        if !path.is_absolute() {
            return Err(io::Error::other("Cache root must be absolute"));
        }
        let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW;
        let mut directory = File::from(fs::open("/", flags, Mode::empty())?);
        for component in path.components() {
            let name = match component {
                Component::RootDir => continue,
                Component::Normal(name) => name,
                _ => {
                    return Err(io::Error::other(
                        "Cache root must not contain traversal components",
                    ));
                }
            };
            directory = File::from(match fs::openat(&directory, name, flags, Mode::empty()) {
                Ok(file) => file,
                Err(rustix::io::Errno::NOENT) => {
                    match fs::mkdirat(&directory, name, Mode::RUSR | Mode::WUSR | Mode::XUSR) {
                        Ok(()) | Err(rustix::io::Errno::EXIST) => {}
                        Err(error) => return Err(error.into()),
                    }
                    fs::openat(&directory, name, flags, Mode::empty())?
                }
                Err(error) => return Err(error.into()),
            });
        }
        let metadata = directory.metadata()?;
        if metadata.uid() != geteuid().as_raw() || metadata.mode() & 0o077 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Cache root must be owned and mode 0700",
            ));
        }
        Ok(Self { directory })
    }
    pub fn read(&self, name: &str) -> io::Result<File> {
        regular(File::from(fs::openat(
            &self.directory,
            name,
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::NOCTTY,
            Mode::empty(),
        )?))
    }
    pub fn temporary(&self) -> io::Result<Temporary> {
        for _ in 0..16 {
            let name = format!(
                "owned-{}-{}.tmp",
                std::process::id(),
                TEMPORARIES.fetch_add(1, Ordering::Relaxed)
            );
            match fs::openat(
                &self.directory,
                &name,
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC | OFlags::NOFOLLOW,
                Mode::RUSR | Mode::WUSR,
            ) {
                Ok(file) => {
                    return Ok(Temporary {
                        file: File::from(file),
                        directory: self.directory.try_clone()?,
                        name,
                        published: false,
                    });
                }
                Err(rustix::io::Errno::EXIST) => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Err(io::Error::other("Cache temporary names exhausted"))
    }
    pub fn publish(&self, mut temporary: Temporary, name: &str) -> io::Result<()> {
        let owned = temporary.file.metadata()?;
        let named = fs::statat(&self.directory, &temporary.name, AtFlags::SYMLINK_NOFOLLOW)?;
        if owned.ino() != named.st_ino || owned.dev() != named.st_dev {
            return Err(io::Error::other("Owned cache temporary was replaced"));
        }
        match self.read(name) {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        fs::renameat(&self.directory, &temporary.name, &self.directory, name)?;
        temporary.published = true;
        self.directory.sync_all()?;
        Ok(())
    }
    pub fn records(&self) -> io::Result<Vec<Record>> {
        let mut records = Vec::new();
        for item in Dir::read_from(&self.directory)?.take(100_000) {
            let item = item?;
            let name = std::ffi::OsStr::from_bytes(item.file_name().to_bytes());
            let Some(name) = name.to_str().filter(|name| valid_name(name)) else {
                continue;
            };
            let Ok(file) = self.read(name) else {
                continue;
            };
            let metadata = file.metadata()?;
            records.push(Record {
                name: name.to_owned(),
                bytes: metadata.len(),
                modified: metadata.modified()?,
            });
        }
        Ok(records)
    }
    pub fn remove(&self, name: &str) -> io::Result<()> {
        let file = self.read(name)?;
        let owned = file.metadata()?;
        let named = fs::statat(&self.directory, name, AtFlags::SYMLINK_NOFOLLOW)?;
        if owned.ino() != named.st_ino || owned.dev() != named.st_dev {
            return Err(io::Error::other("Cache record changed"));
        }
        fs::unlinkat(&self.directory, name, AtFlags::empty())?;
        Ok(())
    }
}
