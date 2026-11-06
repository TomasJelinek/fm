use super::*;
use std::{
    collections::VecDeque,
    ffi::{OsStr, OsString},
    io::Read,
    mem::{offset_of, size_of},
    os::windows::{ffi::OsStringExt, io::AsRawHandle},
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
    stamp: (u64, u64, u64),
    names: VecDeque<OsString>,
    ended: bool,
}
impl Source {
    pub(super) fn open(
        parent: Arc<File>,
        parent_path: PathBuf,
        name: &OsStr,
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
        let access = FILE_READ_DATA
            | FILE_READ_ATTRIBUTES
            | SYNCHRONIZE
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
            stamp,
            names: VecDeque::new(),
            ended: false,
        })
    }
    fn fill_names(&mut self) -> Result<(), FsError> {
        let path = &self.entry.item.path;
        let mut buffer = vec![0u64; 8192];
        // SAFETY: an aligned, writable 64 KiB buffer and a live directory handle.
        if unsafe {
            GetFileInformationByHandleEx(
                self.file.as_raw_handle(),
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
        loop {
            if let Some(name) = self.names.pop_front() {
                return Self::open(
                    self.file.clone(),
                    self.entry.item.path.clone(),
                    &name,
                    cancel,
                )
                .map(|child| Some(Box::new(child) as Box<dyn TransferSource>));
            }
            if self.ended {
                return Ok(None);
            }
            self.fill_names()?;
        }
    }
}
