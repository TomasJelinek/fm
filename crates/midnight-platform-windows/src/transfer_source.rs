use super::*;
use std::{
    collections::VecDeque,
    ffi::{OsStr, OsString},
    io::{Read, Seek, SeekFrom},
    mem::{offset_of, size_of},
    os::windows::{
        ffi::OsStringExt,
        io::{AsRawHandle, FromRawHandle},
    },
    path::PathBuf,
};
use windows_sys::{
    Wdk::Storage::FileSystem::{FILE_DIRECTORY_FILE, FILE_NON_DIRECTORY_FILE, FILE_OPEN},
    Win32::Foundation::ERROR_NO_MORE_FILES,
};

pub(super) struct Source {
    entry: TransferEntry,
    parent: Arc<File>,
    parent_path: PathBuf,
    parent_identity: TransferIdentity,
    name: OsString,
    file: Arc<File>,
    readable: Option<File>,
    stamp: (u64, u64, u64),
    names: VecDeque<OsString>,
    ended: bool,
    move_enabled: bool,
}
impl Source {
    pub(super) fn open(
        parent: Arc<File>,
        parent_path: PathBuf,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Self, FsError> {
        Self::open_with_access(parent, parent_path, name, false, cancel)
    }
    pub(super) fn open_for_move(
        parent: Arc<File>,
        parent_path: PathBuf,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Self, FsError> {
        Self::open_with_access(parent, parent_path, name, true, cancel)
    }
    pub(super) fn native_file(&self) -> Arc<File> {
        self.file.clone()
    }
    fn open_with_access(
        parent: Arc<File>,
        parent_path: PathBuf,
        name: &OsStr,
        move_root: bool,
        cancel: &Cancellation,
    ) -> Result<Self, FsError> {
        let path = parent_path.join(name);
        checked(cancel, &path)?;
        validate_name(name).map_err(|e| error(&path, e))?;
        let probe = native::open_relative(
            &parent,
            name,
            FILE_READ_ATTRIBUTES | SYNCHRONIZE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            FILE_OPEN,
            0,
        )
        .map_err(|e| error(&path, e))?;
        let snapshot = entry(&probe, &path)?;
        let options = match snapshot.item.kind {
            EntryKind::Directory => FILE_DIRECTORY_FILE,
            EntryKind::File => FILE_NON_DIRECTORY_FILE,
            _ => {
                return Err(unsupported(
                    &path,
                    "copying links and special entries is not supported",
                ));
            }
        };
        // Denying write and delete sharing makes ordinary concurrent writers/renamers
        // fail rather than produce a mixed stream. Existing incompatible handles fail here.
        let access = FILE_READ_ATTRIBUTES
            | SYNCHRONIZE
            | if move_root { DELETE } else { FILE_READ_DATA }
            | if snapshot.item.kind == EntryKind::Directory {
                FILE_TRAVERSE
            } else {
                0
            };
        let file =
            native::open_relative(&parent, name, access, FILE_SHARE_READ, FILE_OPEN, options)
                .map_err(|e| error(&path, e))?;
        if identity(&file, &path)? != snapshot.identity {
            return Err(invalid(&path, "source changed while opening"));
        }
        let meta = file.metadata().map_err(|e| error(&path, e))?;
        let stamp = (
            meta.file_size(),
            meta.last_write_time(),
            meta.creation_time(),
        );
        let parent_identity = identity(&parent, &parent_path)?;
        Ok(Self {
            entry: entry(&file, &path)?,
            parent,
            parent_path,
            parent_identity,
            name: name.to_owned(),
            file: Arc::new(file),
            readable: None,
            stamp,
            names: VecDeque::new(),
            ended: false,
            move_enabled: move_root,
        })
    }
    fn fill_names(&mut self) -> Result<(), FsError> {
        self.ensure_readable()?;
        let path = &self.entry.item.path;
        let mut buffer = vec![0u64; 8192];
        // SAFETY: an aligned, writable 64 KiB buffer and a live directory handle.
        if unsafe {
            GetFileInformationByHandleEx(
                self.read_file().as_raw_handle(),
                FileIdBothDirectoryInfo,
                buffer.as_mut_ptr().cast(),
                (buffer.len() * 8) as u32,
            )
        } == 0
        {
            let failure = io::Error::last_os_error();
            if failure.raw_os_error() == Some(ERROR_NO_MORE_FILES as i32) {
                self.ended = true;
                return Ok(());
            }
            return Err(error(path, failure));
        }
        let mut offset = 0usize;
        loop {
            let header = offset_of!(FILE_ID_BOTH_DIR_INFO, FileName);
            if offset + size_of::<FILE_ID_BOTH_DIR_INFO>() > buffer.len() * 8 {
                return Err(invalid(path, "invalid directory record"));
            }
            // SAFETY: offset bounds checked above; native records are eight-byte aligned.
            let record = unsafe {
                &*buffer
                    .as_ptr()
                    .cast::<u8>()
                    .add(offset)
                    .cast::<FILE_ID_BOTH_DIR_INFO>()
            };
            let bytes = record.FileNameLength as usize;
            if !bytes.is_multiple_of(2) || offset + header + bytes > buffer.len() * 8 {
                return Err(invalid(path, "invalid directory filename"));
            }
            // SAFETY: the checked byte range covers this UTF-16 filename.
            let units = unsafe { std::slice::from_raw_parts(record.FileName.as_ptr(), bytes / 2) };
            let name = OsString::from_wide(units);
            if name != "." && name != ".." {
                self.names.push_back(name);
            }
            if record.NextEntryOffset == 0 {
                break;
            }
            let next = record.NextEntryOffset as usize;
            if !next.is_multiple_of(8) || next < header + bytes {
                return Err(invalid(path, "invalid directory offset"));
            }
            offset += next;
        }
        Ok(())
    }
}
impl TransferSource for Source {
    fn discover(
        &self,
        cancel: &Cancellation,
        visit: &mut dyn FnMut(&MutationItem),
    ) -> Result<(), FsError> {
        self.revalidate(cancel)?;
        crate::mutation::discovery::walk(
            &self.parent,
            &self.file,
            &self.name,
            &self.entry.item.path,
            cancel,
            visit,
        )?;
        self.revalidate(cancel)
    }
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
        checked(cancel, &self.entry.item.path)?;
        self.ensure_readable()?;
        super::validation::rewind(self.read_file(), &self.entry.item.path, cancel)
    }
    fn seek_for_validation(&mut self, offset: u64, cancel: &Cancellation) -> Result<u64, FsError> {
        checked(cancel, &self.entry.item.path)?;
        self.ensure_readable()?;
        let file = self.read_file();
        let length = file
            .metadata()
            .map_err(|e| error(&self.entry.item.path, e))?
            .len();
        if offset > length {
            return Err(invalid(
                &self.entry.item.path,
                "validation offset exceeds retained file length",
            ));
        }
        let reached = (&*file)
            .seek(SeekFrom::Start(offset))
            .map_err(|e| error(&self.entry.item.path, e))?;
        if reached != offset {
            return Err(invalid(
                &self.entry.item.path,
                "validation seek did not reach requested offset",
            ));
        }
        Ok(length)
    }
    fn entry(&self) -> &TransferEntry {
        &self.entry
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        let path = &self.entry.item.path;
        checked(cancel, path)?;
        let visible = native::open_parent(&self.parent_path).map_err(|e| error(path, e))?;
        if identity(&visible, path)? != self.parent_identity {
            return Err(invalid(path, "source parent changed"));
        }
        let visible = native::open_relative(
            &self.parent,
            &self.name,
            FILE_READ_ATTRIBUTES | SYNCHRONIZE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            FILE_OPEN,
            0,
        )
        .map_err(|e| error(path, e))?;
        if identity(&visible, path)? != self.entry.identity {
            return Err(invalid(path, "source identity changed"));
        }
        let meta = self.file.metadata().map_err(|e| error(path, e))?;
        if self.entry.item.kind == EntryKind::File
            && (
                meta.file_size(),
                meta.last_write_time(),
                meta.creation_time(),
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
        self.ensure_readable()?;
        self.read_file()
            .read(buffer)
            .map_err(|e| error(&self.entry.item.path, e))
    }
    fn supports_read_retry(&self) -> bool {
        self.entry.item.kind == EntryKind::File
    }
    fn retry_read_from(&mut self, offset: u64, cancel: &Cancellation) -> Result<(), FsError> {
        self.revalidate(cancel)?;
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
        self.ensure_readable()?;
        self.read_file()
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
    fn read_file(&self) -> &File {
        self.readable.as_ref().unwrap_or(&self.file)
    }

    fn ensure_readable(&mut self) -> Result<(), FsError> {
        if !self.move_enabled || self.readable.is_some() {
            return Ok(());
        }
        if self.entry.item.kind == EntryKind::Directory {
            let held = native::open_relative(
                &self.parent,
                &self.name,
                FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES | FILE_TRAVERSE | SYNCHRONIZE,
                FILE_SHARE_READ | FILE_SHARE_DELETE,
                FILE_OPEN,
                FILE_DIRECTORY_FILE,
            )
            .map_err(|e| error(&self.entry.item.path, e))?;
            if identity(&held, &self.entry.item.path)? != self.entry.identity {
                return Err(invalid(
                    &self.entry.item.path,
                    "directory changed before listing upgrade",
                ));
            }
            self.readable = Some(held);
            return Ok(());
        }
        // Upgrade the retained object only when copying or enumerating a merge.
        // The original DELETE pin excludes writers/renamers; this read handle
        // permits that existing DELETE access without requesting it again.
        // SAFETY: retained live file; the OS performs new access/sharing checks.
        let handle = unsafe {
            ReOpenFile(
                self.file.as_raw_handle(),
                FILE_READ_DATA | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
                FILE_SHARE_READ | FILE_SHARE_DELETE,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            )
        };
        if handle == windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE {
            return Err(error(&self.entry.item.path, io::Error::last_os_error()));
        }
        // SAFETY: successful ReOpenFile transfers one owned handle to the exact retained object.
        self.readable = Some(unsafe { File::from_raw_handle(handle) });
        Ok(())
    }

    fn next_name(&mut self, cancel: &Cancellation) -> Result<Option<OsString>, FsError> {
        self.revalidate(cancel)?;
        if self.entry.item.kind != EntryKind::Directory {
            return Err(invalid(&self.entry.item.path, "source is not a directory"));
        }
        loop {
            if let Some(name) = self.names.pop_front() {
                return Ok(Some(name));
            }
            if self.ended {
                return Ok(None);
            }
            self.fill_names()?;
        }
    }
}
