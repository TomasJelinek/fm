//! Read-only native Find handles and bounded native directory records.
use crate::mutation::native;
use midnight_core::*;
use std::{
    collections::VecDeque,
    ffi::OsString,
    fs::{File, OpenOptions},
    io::{self, Read},
    mem::{offset_of, size_of},
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawHandle,
    },
    path::{Component, Path, PathBuf, Prefix},
};
use windows_sys::{
    Wdk::Storage::FileSystem::{FILE_DIRECTORY_FILE, FILE_OPEN},
    Win32::{Foundation::ERROR_NO_MORE_FILES, Storage::FileSystem::*},
};

impl FindService for crate::WindowsFileSystem {
    fn open_find_source(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Box<dyn FindSource>, FsError> {
        checked(cancel, path)?;
        native_path(path)?;
        let probe = open_path(path, FILE_READ_ATTRIBUTES)?;
        let entry = snapshot(&probe, path)?;
        ordinary(&entry)?;
        let file = if entry.kind == EntryKind::Directory {
            open_path(
                path,
                FILE_READ_ATTRIBUTES | FILE_LIST_DIRECTORY | FILE_TRAVERSE,
            )?
        } else {
            probe
        };
        if !snapshot(&file, path)?.matches_capture(&entry) {
            return Err(changed(path));
        }
        Ok(Box::new(Source {
            entry: localized(entry),
            file,
            names: VecDeque::new(),
            ended: false,
            reader: None,
        }))
    }
}
struct Source {
    entry: FindEntry,
    file: File,
    names: VecDeque<OsString>,
    ended: bool,
    reader: Option<File>,
}
impl Source {
    fn child(&self, name: &std::ffi::OsStr) -> Result<FindChild, FsError> {
        let path = self.entry.path.join(name);
        let probe = native::open_relative(
            &self.file,
            name,
            FILE_READ_ATTRIBUTES | SYNCHRONIZE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            FILE_OPEN,
            0,
        )
        .map_err(|error| FsError::from_io("find child", &path, error))?;
        let entry = snapshot(&probe, &path)?;
        if matches!(entry.kind, EntryKind::DirectoryLink | EntryKind::Symlink) {
            return Ok(FindChild::Skipped {
                path,
                reason: FindSkipReason::Link,
            });
        }
        if entry.identity.device != self.entry.identity.device {
            return Ok(FindChild::Skipped {
                path,
                reason: FindSkipReason::MountBoundary,
            });
        }
        let file = if entry.kind == EntryKind::Directory {
            native::open_relative(
                &self.file,
                name,
                FILE_READ_ATTRIBUTES | FILE_LIST_DIRECTORY | FILE_TRAVERSE | SYNCHRONIZE,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                FILE_OPEN,
                FILE_DIRECTORY_FILE,
            )
            .map_err(|error| FsError::from_io("find directory", &path, error))?
        } else {
            probe
        };
        if !snapshot(&file, &path)?.matches_capture(&entry) {
            return Err(changed(&path));
        }
        Ok(FindChild::Entry(Box::new(Source {
            entry: localized(entry),
            file,
            names: VecDeque::new(),
            ended: false,
            reader: None,
        })))
    }
    fn fill_names(&mut self) -> Result<(), FsError> {
        let mut buffer = vec![0u64; 8192];
        // SAFETY: the live directory handle and aligned writable 64 KiB buffer
        // remain valid for the entire synchronous native query.
        if unsafe {
            GetFileInformationByHandleEx(
                self.file.as_raw_handle(),
                FileIdBothDirectoryInfo,
                buffer.as_mut_ptr().cast(),
                (buffer.len() * 8) as u32,
            )
        } == 0
        {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(ERROR_NO_MORE_FILES as i32) {
                self.ended = true;
                return Ok(());
            }
            return Err(FsError::from_io("find directory", &self.entry.path, error));
        }
        let mut offset = 0usize;
        loop {
            let header = offset_of!(FILE_ID_BOTH_DIR_INFO, FileName);
            if offset + size_of::<FILE_ID_BOTH_DIR_INFO>() > buffer.len() * 8 {
                return Err(record_error(&self.entry.path));
            }
            // SAFETY: the record header is in bounds and each native offset is
            // checked below before it becomes the next eight-byte aligned offset.
            let record = unsafe {
                &*buffer
                    .as_ptr()
                    .cast::<u8>()
                    .add(offset)
                    .cast::<FILE_ID_BOTH_DIR_INFO>()
            };
            let bytes = record.FileNameLength as usize;
            if !bytes.is_multiple_of(2) || offset + header + bytes > buffer.len() * 8 {
                return Err(record_error(&self.entry.path));
            }
            // SAFETY: the complete counted UTF-16 filename range was checked above.
            let name = OsString::from_wide(unsafe {
                std::slice::from_raw_parts(record.FileName.as_ptr(), bytes / 2)
            });
            if name != "." && name != ".." {
                self.names.push_back(name);
            }
            if record.NextEntryOffset == 0 {
                break;
            }
            let next = record.NextEntryOffset as usize;
            if !next.is_multiple_of(8) || next < header + bytes {
                return Err(record_error(&self.entry.path));
            }
            offset += next;
        }
        Ok(())
    }
}
impl FindSource for Source {
    fn entry(&self) -> &FindEntry {
        &self.entry
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        checked(cancel, &self.entry.path)?;
        let visible = open_path(&self.entry.path, FILE_READ_ATTRIBUTES)?;
        if !snapshot(&visible, &self.entry.path)?.matches_capture(&self.entry)
            || !snapshot(&self.file, &self.entry.path)?.matches_capture(&self.entry)
        {
            return Err(changed(&self.entry.path));
        }
        Ok(())
    }
    fn next_child(&mut self, cancel: &Cancellation) -> Result<Option<FindChild>, FsError> {
        self.revalidate(cancel)?;
        if self.entry.kind != EntryKind::Directory {
            return Err(find_error(
                &self.entry.path,
                io::ErrorKind::InvalidInput,
                "Find source is not a directory.",
            ));
        }
        loop {
            checked(cancel, &self.entry.path)?;
            if let Some(name) = self.names.pop_front() {
                return Ok(Some(match self.child(&name) {
                    Ok(child) => child,
                    Err(error) => FindChild::Failed(error),
                }));
            }
            if self.ended {
                return Ok(None);
            }
            self.fill_names()?;
        }
    }
    fn read_chunk(&mut self, buffer: &mut [u8], cancel: &Cancellation) -> Result<usize, FsError> {
        checked(cancel, &self.entry.path)?;
        if self.entry.kind != EntryKind::File {
            return Err(find_error(
                &self.entry.path,
                io::ErrorKind::InvalidInput,
                "Directory content is not a byte stream.",
            ));
        }
        if self.reader.is_none() {
            // Find is a live read-only scan: do not deny an application's log
            // writer. Identity and ChangeTime are checked before/after reading;
            // concurrent edits produce an explicit incomplete result.
            let reader = OpenOptions::new()
                .access_mode(FILE_READ_DATA | FILE_READ_ATTRIBUTES)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
                .open(native_path(&self.entry.path)?)
                .map_err(|error| FsError::from_io("find content", &self.entry.path, error))?;
            if !snapshot(&reader, &self.entry.path)?.matches_capture(&self.entry) {
                return Err(changed(&self.entry.path));
            }
            self.reader = Some(reader);
        }
        match &mut self.reader {
            Some(reader) => reader
                .read(buffer)
                .map_err(|error| FsError::from_io("find content", &self.entry.path, error)),
            None => Err(find_error(
                &self.entry.path,
                io::ErrorKind::Other,
                "Find content reader is unavailable.",
            )),
        }
    }
}
fn open_path(path: &Path, access: u32) -> Result<File, FsError> {
    OpenOptions::new()
        .access_mode(access)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(native_path(path)?)
        .map_err(|error| FsError::from_io("find source", path, error))
}
fn ordinary(entry: &FindEntry) -> Result<(), FsError> {
    if matches!(entry.kind, EntryKind::File | EntryKind::Directory) {
        Ok(())
    } else {
        Err(find_error(
            &entry.path,
            io::ErrorKind::Unsupported,
            "Find roots/results must be ordinary native files or directories; links are not followed.",
        ))
    }
}
fn localized(mut entry: FindEntry) -> FindEntry {
    entry.metadata.localized_modified = entry.metadata.modified.and_then(crate::locale::modified);
    entry
}
fn snapshot(file: &File, path: &Path) -> Result<FindEntry, FsError> {
    // SAFETY: the owned live handle is only queried for its native type.
    if unsafe { GetFileType(file.as_raw_handle()) } != FILE_TYPE_DISK {
        return Err(find_error(
            path,
            io::ErrorKind::Unsupported,
            "Find does not open device or special-file content.",
        ));
    }
    let metadata = file
        .metadata()
        .map_err(|error| FsError::from_io("find metadata", path, error))?;
    let mut basic = FILE_BASIC_INFO::default();
    // SAFETY: the owned live handle and writable fixed-size FILE_BASIC_INFO
    // remain valid for this synchronous query. ChangeTime catches ordinary edits
    // even when an application restores the file's LastWriteTime afterwards.
    if unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileBasicInfo,
            (&mut basic as *mut FILE_BASIC_INFO).cast(),
            size_of::<FILE_BASIC_INFO>() as u32,
        )
    } == 0
    {
        return Err(FsError::from_io(
            "find revision",
            path,
            io::Error::last_os_error(),
        ));
    }
    let attributes = metadata.file_attributes();
    let directory = attributes & FILE_ATTRIBUTE_DIRECTORY != 0;
    let link = attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0;
    let kind = match (directory, link) {
        (true, true) => EntryKind::DirectoryLink,
        (false, true) => EntryKind::Symlink,
        (true, false) => EntryKind::Directory,
        (false, false) => EntryKind::File,
    };
    let identity =
        native::identity(file).map_err(|error| FsError::from_io("find identity", path, error))?;
    Ok(FindEntry {
        path: path.to_owned(),
        kind,
        identity: TransferIdentity {
            device: identity.volume,
            object: identity.id,
        },
        revision: FindRevision([
            metadata.file_size(),
            metadata.last_write_time(),
            metadata.creation_time(),
            attributes as u64,
            basic.ChangeTime as u64,
            0,
        ]),
        metadata: EntryMetadata {
            size: (!directory).then_some(metadata.file_size()),
            modified: metadata.modified().ok(),
            native_attributes: Some(attributes),
            ..Default::default()
        },
    })
}
fn checked(cancel: &Cancellation, path: &Path) -> Result<(), FsError> {
    if cancel.is_cancelled() {
        Err(find_error(
            path,
            io::ErrorKind::Interrupted,
            "Find cancelled.",
        ))
    } else {
        Ok(())
    }
}
fn changed(path: &Path) -> FsError {
    find_error(
        path,
        io::ErrorKind::InvalidData,
        "Find object identity or revision changed.",
    )
}
fn record_error(path: &Path) -> FsError {
    find_error(
        path,
        io::ErrorKind::InvalidData,
        "Invalid native Find directory record.",
    )
}

// Verbatim native paths retain reserved-looking names and trailing spaces as
// filesystem names instead of allowing the Win32 DOS parser to retarget them.
fn native_path(path: &Path) -> Result<PathBuf, FsError> {
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, Component::CurDir | Component::ParentDir))
    {
        return Err(find_error(
            path,
            io::ErrorKind::InvalidInput,
            "An absolute native Find path is required.",
        ));
    }
    let units: Vec<_> = path.as_os_str().encode_wide().collect();
    if units.contains(&0)
        || units
            .split(|unit| *unit == 47 || *unit == 92)
            .any(|part| part == [46] || part == [46, 46])
    {
        return Err(find_error(
            path,
            io::ErrorKind::InvalidInput,
            "NUL and dot path steps are not accepted.",
        ));
    }
    let prefix = match path.components().next() {
        Some(Component::Prefix(prefix)) => prefix.kind(),
        _ => {
            return Err(find_error(
                path,
                io::ErrorKind::InvalidInput,
                "A native drive or UNC root is required.",
            ));
        }
    };
    let mut result: Vec<u16> = match prefix {
        Prefix::VerbatimDisk(_) | Prefix::VerbatimUNC(_, _) => return Ok(path.to_owned()),
        Prefix::Disk(_) => r"\\?\".encode_utf16().collect(),
        Prefix::UNC(_, _) => r"\\?\UNC\".encode_utf16().collect(),
        _ => {
            return Err(find_error(
                path,
                io::ErrorKind::Unsupported,
                "Find rejects device and special namespaces.",
            ));
        }
    };
    let skip = usize::from(matches!(prefix, Prefix::UNC(_, _))) * 2;
    result.extend(
        units
            .into_iter()
            .skip(skip)
            .map(|unit| if unit == 47 { 92 } else { unit }),
    );
    Ok(PathBuf::from(OsString::from_wide(&result)))
}
