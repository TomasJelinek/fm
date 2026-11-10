use super::*;
use rustix::fs::Dir;
use std::{
    ffi::OsString,
    io::{Read, Seek, SeekFrom},
    os::fd::AsRawFd,
};

pub(super) struct Source {
    entry: TransferEntry,
    parent: Arc<File>,
    parent_path: PathBuf,
    parent_identity: TransferIdentity,
    name: OsString,
    file: Arc<File>,
    readable: bool,
    stamp: (u64, i64, i64, i64, i64),
    entries: Option<Dir>,
    move_enabled: bool,
}
impl Source {
    pub(super) fn enable_move(&mut self) {
        self.move_enabled = true;
    }
    pub(super) fn native_file(&self) -> Arc<File> {
        self.file.clone()
    }
    pub(super) fn open(
        parent: Arc<File>,
        parent_path: PathBuf,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Self, FsError> {
        let mut source = Self::open_move(parent, parent_path, name, cancel)?;
        source.make_readable(cancel)?;
        Ok(source)
    }
    /// Retain only metadata authority; native Move never opens file contents.
    pub(super) fn open_move(
        parent: Arc<File>,
        parent_path: PathBuf,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Self, FsError> {
        let path = parent_path.join(name);
        checked(cancel, &path)?;
        validate_name(name, &path)?;
        let probe = File::from(
            openat2(
                &*parent,
                name,
                OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
                RESOLVE,
            )
            .map_err(|e| error(&path, e.into()))?,
        );
        let snapshot = entry(&probe, &path)?;
        if !matches!(snapshot.item.kind, EntryKind::File | EntryKind::Directory) {
            return Err(unsupported(
                &path,
                "copying links and special entries is not supported",
            ));
        }
        let file = probe;
        let metadata = file.metadata().map_err(|e| error(&path, e))?;
        let stamp = (
            metadata.len(),
            metadata.mtime(),
            metadata.mtime_nsec(),
            metadata.ctime(),
            metadata.ctime_nsec(),
        );
        let parent_identity = identity(&parent, &parent_path)?;
        Ok(Self {
            entry: entry(&file, &path)?,
            parent,
            parent_path,
            parent_identity,
            name: name.to_owned(),
            file: Arc::new(file),
            readable: false,
            stamp,
            entries: None,
            move_enabled: false,
        })
    }
    fn make_readable(&mut self, cancel: &Cancellation) -> Result<(), FsError> {
        checked(cancel, &self.entry.item.path)?;
        if !self.readable {
            self.file = Arc::new(open_retained(&self.file, &self.entry.item.path)?);
            self.readable = true;
        }
        Ok(())
    }
}

/// Open the retained inode, never its former pathname, only when content access is needed.
pub(super) fn open_retained(file: &File, path: &Path) -> Result<File, FsError> {
    let expected = identity(file, path)?;
    let descriptor = format!("/proc/self/fd/{}", file.as_raw_fd());
    let opened = File::from(
        openat(
            CWD,
            descriptor.as_str(),
            OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|e| error(path, e.into()))?,
    );
    if identity(&opened, path)? != expected {
        return Err(invalid(path, "retained source identity changed"));
    }
    Ok(opened)
}
impl TransferSource for Source {
    fn open_child(
        &self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        self.revalidate(cancel)?;
        if self.entry.item.kind != EntryKind::Directory {
            return Err(invalid(
                &self.entry.item.path,
                "named child requires a directory",
            ));
        }
        Self::open(
            self.file.clone(),
            self.entry.item.path.clone(),
            name,
            cancel,
        )
        .map(|source| Box::new(source) as Box<dyn TransferSource>)
    }
    fn next_move_child(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Option<midnight_core::PreparedTransferMove>, FsError> {
        if !self.move_enabled {
            return Err(unsupported(
                &self.entry.item.path,
                "Copy sources have no Move authority",
            ));
        }
        self.next_name(cancel)?
            .map(|name| {
                super::move_source::prepare_child(
                    self.file.clone(),
                    self.entry.item.path.clone(),
                    &name,
                    cancel,
                )
            })
            .transpose()
    }
    fn prepare_modified_time(
        &self,
        cancel: &Cancellation,
    ) -> Result<Box<dyn midnight_core::TransferModifiedTimePin>, FsError> {
        if self.entry.item.kind != EntryKind::File {
            return Err(unsupported(
                &self.entry.item.path,
                "required modified time supports regular files only",
            ));
        }
        self.revalidate(cancel)?;
        super::metadata::prepare(self.file.clone(), &self.entry.item.path, cancel)
    }
    fn rewind_for_validation(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Box<dyn midnight_core::TransferReadGuard>, FsError> {
        if self.entry.item.kind != EntryKind::File {
            return Err(unsupported(
                &self.entry.item.path,
                "only regular files support retained validation",
            ));
        }
        self.make_readable(cancel)?;
        super::validation::rewind(&self.file, &self.entry.item.path, cancel)
    }
    fn seek_for_validation(&mut self, offset: u64, cancel: &Cancellation) -> Result<u64, FsError> {
        self.make_readable(cancel)?;
        let path = &self.entry.item.path;
        if self.entry.item.kind != EntryKind::File {
            return Err(invalid(path, "range validation requires a regular file"));
        }
        let length = self.file.metadata().map_err(|e| error(path, e))?.len();
        if offset > length {
            return Err(invalid(
                path,
                "validation offset exceeds retained file length",
            ));
        }
        let actual = (&*self.file)
            .seek(SeekFrom::Start(offset))
            .map_err(|e| error(path, e))?;
        if actual != offset {
            return Err(invalid(
                path,
                "retained file seek returned a different offset",
            ));
        }
        checked(cancel, path)?;
        Ok(length)
    }
    fn entry(&self) -> &TransferEntry {
        &self.entry
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        let path = &self.entry.item.path;
        checked(cancel, path)?;
        let visible_parent = File::from(
            openat(CWD, &self.parent_path, DIRECTORY, Mode::empty())
                .map_err(|e| error(path, e.into()))?,
        );
        if identity(&visible_parent, path)? != self.parent_identity {
            return Err(invalid(path, "source parent changed"));
        }
        let visible = File::from(
            openat2(
                &*self.parent,
                &self.name,
                OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
                RESOLVE,
            )
            .map_err(|e| error(path, e.into()))?,
        );
        if identity(&visible, path)? != self.entry.identity {
            return Err(invalid(path, "source identity changed"));
        }
        let metadata = self.file.metadata().map_err(|e| error(path, e))?;
        if self.entry.item.kind == EntryKind::File
            && (
                metadata.len(),
                metadata.mtime(),
                metadata.mtime_nsec(),
                metadata.ctime(),
                metadata.ctime_nsec(),
            ) != self.stamp
        {
            return Err(invalid(path, "source content changed"));
        }
        Ok(())
    }
    fn read_chunk(&mut self, buffer: &mut [u8], cancel: &Cancellation) -> Result<usize, FsError> {
        self.make_readable(cancel)?;
        if self.entry.item.kind != EntryKind::File {
            return Err(invalid(
                &self.entry.item.path,
                "directory content is not a byte stream",
            ));
        }
        (&*self.file)
            .read(buffer)
            .map_err(|e| error(&self.entry.item.path, e))
    }
    fn supports_read_retry(&self) -> bool {
        self.entry.item.kind == EntryKind::File
    }
    fn retry_read_from(&mut self, offset: u64, cancel: &Cancellation) -> Result<(), FsError> {
        self.revalidate(cancel)?;
        self.make_readable(cancel)?;
        if !self.supports_read_retry()
            || self
                .entry
                .item
                .metadata
                .size
                .is_none_or(|size| offset > size)
        {
            return Err(invalid(
                &self.entry.item.path,
                "invalid source retry offset",
            ));
        }
        (&*self.file)
            .seek(SeekFrom::Start(offset))
            .map_err(|e| error(&self.entry.item.path, e))?;
        self.revalidate(cancel)
    }
    fn next_child(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Option<Box<dyn TransferSource>>, FsError> {
        self.next_name(cancel)?
            .map(|name| {
                Self::open(
                    self.file.clone(),
                    self.entry.item.path.clone(),
                    &name,
                    cancel,
                )
                .map(|source| Box::new(source) as Box<dyn TransferSource>)
            })
            .transpose()
    }
}
impl Source {
    fn next_name(&mut self, cancel: &Cancellation) -> Result<Option<OsString>, FsError> {
        self.revalidate(cancel)?;
        if self.entry.item.kind != EntryKind::Directory {
            return Err(invalid(&self.entry.item.path, "source is not a directory"));
        }
        if self.entries.is_none() {
            self.make_readable(cancel)?;
            self.entries = Some(
                Dir::read_from(&*self.file).map_err(|e| error(&self.entry.item.path, e.into()))?,
            );
        }
        loop {
            checked(cancel, &self.entry.item.path)?;
            let next = self.entries.as_mut().and_then(Iterator::next);
            let Some(next) = next else {
                return Ok(None);
            };
            let next = next.map_err(|e| error(&self.entry.item.path, e.into()))?;
            let name = OsStr::from_bytes(next.file_name().to_bytes());
            if name == "." || name == ".." {
                continue;
            }
            return Ok(Some(name.to_owned()));
        }
    }
}
