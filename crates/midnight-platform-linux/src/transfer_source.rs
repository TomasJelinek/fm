use super::*;
use rustix::fs::Dir;
use std::{ffi::OsString, io::Read, os::fd::AsRawFd};

pub(super) struct Source {
    entry: TransferEntry,
    parent: Arc<File>,
    parent_path: PathBuf,
    parent_identity: TransferIdentity,
    name: OsString,
    file: Arc<File>,
    stamp: (u64, i64, i64, i64, i64),
    entries: Option<Dir>,
}
impl Source {
    pub(super) fn native_file(&self) -> Arc<File> {
        self.file.clone()
    }
    pub(super) fn open(
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
        let file = match snapshot.item.kind {
            EntryKind::File => {
                // Reopen the retained O_PATH inode itself. Reopening its original
                // name could open a substituted device before an identity check.
                // This is our live proc descriptor, not an untrusted source link.
                let descriptor = format!("/proc/self/fd/{}", probe.as_raw_fd());
                File::from(
                    openat(
                        CWD,
                        descriptor.as_str(),
                        OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC,
                        Mode::empty(),
                    )
                    .map_err(|e| error(&path, e.into()))?,
                )
            }
            EntryKind::Directory => File::from(
                openat2(
                    &*parent,
                    name,
                    DIRECTORY | OFlags::NOFOLLOW,
                    Mode::empty(),
                    RESOLVE,
                )
                .map_err(|e| error(&path, e.into()))?,
            ),
            _ => {
                return Err(unsupported(
                    &path,
                    "copying links and special entries is not supported",
                ));
            }
        };
        if identity(&file, &path)? != snapshot.identity {
            return Err(invalid(&path, "source changed while opening"));
        }
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
            stamp,
            entries: None,
        })
    }
}
impl TransferSource for Source {
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
        super::validation::rewind(&self.file, &self.entry.item.path, cancel)
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
        checked(cancel, &self.entry.item.path)?;
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
    fn next_child(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Option<Box<dyn TransferSource>>, FsError> {
        self.revalidate(cancel)?;
        if self.entry.item.kind != EntryKind::Directory {
            return Err(invalid(&self.entry.item.path, "source is not a directory"));
        }
        if self.entries.is_none() {
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
            return Self::open(
                self.file.clone(),
                self.entry.item.path.clone(),
                name,
                cancel,
            )
            .map(|source| Some(Box::new(source) as Box<dyn TransferSource>));
        }
    }
}
