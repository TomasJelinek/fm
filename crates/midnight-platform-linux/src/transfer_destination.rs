use super::*;
use midnight_core::{StagedTransferFile, TransferPublication};
use rustix::fs::{AtFlags, linkat, mkdirat};
use std::{
    io::{Read, Seek, SeekFrom, Write},
    os::fd::AsRawFd,
};

#[derive(Clone)]
pub(super) struct Directory {
    file: Arc<File>,
    path: PathBuf,
    identity: TransferIdentity,
}
impl Directory {
    pub(super) fn from_file(file: File, path: PathBuf) -> Result<Self, FsError> {
        if !file.metadata().map_err(|e| error(&path, e))?.is_dir() {
            return Err(invalid(&path, "destination is not a directory"));
        }
        Ok(Self {
            identity: identity(&file, &path)?,
            file: Arc::new(file),
            path,
        })
    }
}
impl TransferDirectory for Directory {
    fn path(&self) -> &Path {
        &self.path
    }
    fn identity(&self) -> TransferIdentity {
        self.identity
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        checked(cancel, &self.path)?;
        let visible = File::from(
            openat(CWD, &self.path, DIRECTORY, Mode::empty())
                .map_err(|e| error(&self.path, e.into()))?,
        );
        if identity(&visible, &self.path)? != self.identity {
            return Err(invalid(&self.path, "destination directory changed"));
        }
        Ok(())
    }
    fn inspect(
        &self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Option<TransferEntry>, FsError> {
        self.revalidate(cancel)?;
        validate_name(name, &self.path)?;
        let path = self.path.join(name);
        match openat2(
            &*self.file,
            name,
            OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
            RESOLVE,
        ) {
            Ok(fd) => entry(&File::from(fd), &path).map(Some),
            Err(rustix::io::Errno::NOENT) => Ok(None),
            Err(e) => Err(error(&path, e.into())),
        }
    }
    fn reject_source_ancestor(
        &self,
        source: TransferIdentity,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        self.revalidate(cancel)?;
        let mut current = self.file.try_clone().map_err(|e| error(&self.path, e))?;
        loop {
            checked(cancel, &self.path)?;
            let id = identity(&current, &self.path)?;
            if id == source {
                return Err(invalid(
                    &self.path,
                    "destination is inside the source directory",
                ));
            }
            let parent = File::from(
                openat(&current, "..", DIRECTORY, Mode::empty())
                    .map_err(|e| error(&self.path, e.into()))?,
            );
            if identity(&parent, &self.path)? == id {
                break;
            }
            current = parent;
        }
        self.revalidate(cancel)
    }
    fn create_directory(
        &self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.revalidate(cancel)?;
        validate_name(name, &self.path)?;
        // mkdirat does not return an inode handle. Until openat2 completes, protect
        // the new name against a different unprivileged UID replacing it. A sticky
        // directory is safe only when its owner is us or trusted root. Same-UID and
        // privileged namespace mutation remain outside this provider's contract.
        let parent = self.file.metadata().map_err(|e| error(&self.path, e))?;
        let owner = rustix::process::geteuid().as_raw();
        if (parent.uid() != owner && parent.uid() != 0)
            || (parent.mode() & 0o022 != 0 && parent.mode() & 0o1000 == 0)
        {
            return Err(unsupported(
                &self.path,
                "Copy directories require a parent protected against other users renaming the newly created entry (an owned private or trusted sticky directory). File-only copying is still supported.",
            ));
        }
        let path = self.path.join(name);
        mkdirat(&*self.file, name, Mode::RWXU).map_err(|e| error(&path, e.into()))?;
        let file = File::from(
            openat2(
                &*self.file,
                name,
                DIRECTORY | OFlags::NOFOLLOW,
                Mode::empty(),
                RESOLVE,
            )
            .map_err(|e| error(&path, e.into()))?,
        );
        let created = file.metadata().map_err(|e| error(&path, e))?;
        if created.uid() != owner || created.mode() & 0o077 != 0 {
            return Err(unsupported(
                &path,
                "The newly created copy directory is not private and owned; it was left untouched.",
            ));
        }
        Self::from_file(file, path)
            .map(|directory| Arc::new(directory) as Arc<dyn TransferDirectory>)
    }
    fn stage_file(&self, cancel: &Cancellation) -> Result<Box<dyn StagedTransferFile>, FsError> {
        self.revalidate(cancel)?;
        // O_TMPFILE creates an exclusive unnamed inode on the destination filesystem.
        // The file can never be substituted by a staged-path rename. No EXCL here:
        // O_TMPFILE|EXCL deliberately disables the later linkat publication operation.
        let file = File::from(
            openat(
                &*self.file,
                ".",
                OFlags::RDWR | OFlags::TMPFILE | OFlags::CLOEXEC,
                Mode::RUSR | Mode::WUSR,
            )
            .map_err(|e| error(&self.path, e.into()))?,
        );
        Ok(Box::new(Stage {
            file: Some(file),
            directory: self.clone(),
            published: false,
        }))
    }
}

struct Stage {
    file: Option<File>,
    directory: Directory,
    published: bool,
}
impl Stage {
    fn file(&self) -> io::Result<&File> {
        self.file
            .as_ref()
            .ok_or_else(|| io::Error::other("stage was cleaned up"))
    }
    fn file_mut(&mut self) -> io::Result<&mut File> {
        self.file
            .as_mut()
            .ok_or_else(|| io::Error::other("stage was cleaned up"))
    }
}
impl Read for Stage {
    fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
        self.file_mut()?.read(b)
    }
}
impl Write for Stage {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.file_mut()?.write(b)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.file_mut()?.flush()
    }
}
impl Seek for Stage {
    fn seek(&mut self, p: SeekFrom) -> io::Result<u64> {
        self.file_mut()?.seek(p)
    }
}
impl StagedTransferFile for Stage {
    fn sync(&mut self, cancel: &Cancellation) -> Result<(), FsError> {
        checked(cancel, &self.directory.path)?;
        self.file()
            .and_then(File::sync_all)
            .map_err(|e| error(&self.directory.path, e))
    }
    fn len(&self) -> Result<u64, FsError> {
        self.file()
            .and_then(File::metadata)
            .map(|m| m.len())
            .map_err(|e| error(&self.directory.path, e))
    }
    fn publish_no_replace(
        &mut self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<TransferPublication, FsError> {
        if self.published {
            return Err(invalid(&self.directory.path, "stage was already published"));
        }
        self.directory.revalidate(cancel)?;
        validate_name(name, &self.directory.path)?;
        let path = self.directory.path.join(name);
        let file = self.file().map_err(|e| error(&path, e))?;
        // /proc/self/fd identifies our live descriptor, not an untrusted filesystem
        // symlink. This documented O_TMPFILE publication form works without CAP_DAC_READ_SEARCH.
        let descriptor = format!("/proc/self/fd/{}", file.as_raw_fd());
        checked(cancel, &path)?;
        linkat(
            CWD,
            descriptor.as_str(),
            &*self.directory.file,
            name,
            AtFlags::SYMLINK_FOLLOW,
        )
        .map_err(|e| error(&path, e.into()))?;
        self.published = true;
        match self.directory.file.sync_all() {
            Ok(()) => Ok(TransferPublication::Published(path)),
            Err(failure) => Ok(TransferPublication::PublishedDurabilityUnconfirmed {
                error: error(&path, failure),
                path,
            }),
        }
    }
    fn cleanup(&mut self) -> Result<(), FsError> {
        self.file.take();
        Ok(())
    }
}
