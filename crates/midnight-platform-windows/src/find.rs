//! Read-only native Find handles and bounded native directory records.
use crate::{directory_records::Cursor, mutation::native, reparse::Native};
use midnight_core::*;
use std::{
    collections::VecDeque,
    ffi::OsString,
    fs::{File, OpenOptions},
    io::{self, Read},
    mem::size_of,
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawHandle,
    },
    path::{Component, Path, PathBuf, Prefix},
};
use windows_sys::{
    Wdk::Storage::FileSystem::{FILE_DIRECTORY_FILE, FILE_OPEN},
    Win32::Storage::FileSystem::*,
};

/// Cloud-state attributes change when a placeholder hydrates, dehydrates or is pinned,
/// without any change to its content. They are excluded from the content revision.
const VOLATILE_ATTRIBUTES: u32 = FILE_ATTRIBUTE_OFFLINE
    | FILE_ATTRIBUTE_PINNED
    | FILE_ATTRIBUTE_UNPINNED
    | FILE_ATTRIBUTE_RECALL_ON_OPEN
    | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS;

impl FindService for crate::WindowsFileSystem {
    fn open_find_source(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Box<dyn FindSource>, FsError> {
        checked(cancel, path)?;
        native_path(path)?;
        let probe = open_path(path, FILE_READ_ATTRIBUTES)?;
        let (entry, native) = snapshot(&probe, path)?;
        ordinary(&entry)?;
        let file = if entry.kind == EntryKind::Directory {
            let file = open_path(
                path,
                FILE_READ_ATTRIBUTES | FILE_LIST_DIRECTORY | FILE_TRAVERSE,
            )?;
            if !snapshot(&file, path)?.0.matches_capture(&entry) {
                return Err(changed(path));
            }
            file
        } else {
            probe
        };
        Ok(Box::new(Source::new(localized(entry), native, file)))
    }
}
struct Pending {
    name: OsString,
    native: Native,
}
struct Source {
    entry: FindEntry,
    native: Native,
    file: File,
    pending: VecDeque<Pending>,
    cursor: Cursor,
    reader: Option<File>,
}
impl Source {
    fn new(entry: FindEntry, native: Native, file: File) -> Self {
        Self {
            entry,
            native,
            file,
            pending: VecDeque::new(),
            cursor: Cursor::default(),
            reader: None,
        }
    }
    fn child(&self, pending: Pending) -> Result<FindChild, FsError> {
        let path = self.entry.path.join(&pending.name);
        // The directory record already carries the reparse tag: a link is skipped
        // without being opened. Cloud placeholders are ordinary files/directories.
        if pending.native.link() {
            return Ok(FindChild::Skipped {
                path,
                reason: FindSkipReason::Link,
            });
        }
        // One no-follow open per child, with listing access only for a directory.
        let listing = pending.native.directory();
        let open = |access, options| {
            native::open_relative(
                &self.file,
                &pending.name,
                access,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                FILE_OPEN,
                options,
            )
            .map_err(|error| FsError::from_io("find child", &path, error))
        };
        let directory_access =
            FILE_READ_ATTRIBUTES | FILE_LIST_DIRECTORY | FILE_TRAVERSE | SYNCHRONIZE;
        let file = if listing {
            open(directory_access, 0)?
        } else {
            open(FILE_READ_ATTRIBUTES | SYNCHRONIZE, 0)?
        };
        let (entry, native) = snapshot(&file, &path)?;
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
        let file = if entry.kind == EntryKind::Directory && !listing {
            // Replaced by a directory after enumeration: listing needs another open.
            let directory = open(directory_access, FILE_DIRECTORY_FILE)?;
            if !snapshot(&directory, &path)?.0.matches_capture(&entry) {
                return Err(changed(&path));
            }
            directory
        } else {
            file
        };
        Ok(FindChild::Entry(Box::new(Source::new(
            localized(entry),
            native,
            file,
        ))))
    }
    fn fill_names(&mut self) -> Result<(), FsError> {
        let pending = &mut self.pending;
        self.cursor
            .next_batch(&self.file, &mut |record| {
                pending.push_back(Pending {
                    name: OsString::from_wide(record.name),
                    native: record.native,
                });
                Ok(())
            })
            .map_err(|error| FsError::from_io("find directory", &self.entry.path, error))
    }
}
impl FindSource for Source {
    fn entry(&self) -> &FindEntry {
        &self.entry
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        checked(cancel, &self.entry.path)?;
        let visible = open_path(&self.entry.path, FILE_READ_ATTRIBUTES)?;
        if !snapshot(&visible, &self.entry.path)?
            .0
            .matches_capture(&self.entry)
            || !snapshot(&self.file, &self.entry.path)?
                .0
                .matches_capture(&self.entry)
        {
            return Err(changed(&self.entry.path));
        }
        Ok(())
    }
    fn next_child(&mut self, cancel: &Cancellation) -> Result<Option<FindChild>, FsError> {
        if self.entry.kind != EntryKind::Directory {
            return Err(find_error(
                &self.entry.path,
                io::ErrorKind::InvalidInput,
                "Find source is not a directory.",
            ));
        }
        loop {
            checked(cancel, &self.entry.path)?;
            if let Some(pending) = self.pending.pop_front() {
                return Ok(Some(match self.child(pending) {
                    Ok(child) => child,
                    Err(error) => FindChild::Failed(error),
                }));
            }
            // The directory's path binding and revision are revalidated once per native
            // batch and once at its end, not once per child: every child of a batch is
            // opened relative to the retained handle, and every result is revalidated
            // by path again before publication.
            self.revalidate(cancel)?;
            if self.cursor.ended() {
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
            // concurrent edits produce an explicit incomplete result. The pinned object
            // is reopened, never its path; a cloud placeholder or deduplicated file is
            // read through its filter instead of returning the raw reparse stub.
            let reader = crate::reparse::reopen_content(
                &self.file,
                self.native,
                FILE_READ_DATA | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            )
            .map_err(|error| FsError::from_io("find content", &self.entry.path, error))?;
            if !snapshot(&reader, &self.entry.path)?
                .0
                .matches_capture(&self.entry)
            {
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
fn snapshot(file: &File, path: &Path) -> Result<(FindEntry, Native), FsError> {
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
    // Only name-surrogate reparse points are links; cloud placeholders, deduplicated and
    // WOF-compressed entries are ordinary files and directories.
    let native = Native::of(file, metadata.file_attributes())
        .map_err(|error| FsError::from_io("find metadata", path, error))?;
    let attributes = native.attributes;
    let directory = native.directory();
    let identity =
        native::identity(file).map_err(|error| FsError::from_io("find identity", path, error))?;
    let entry = FindEntry {
        path: path.to_owned(),
        kind: native.kind(),
        identity: TransferIdentity {
            device: identity.volume,
            object: identity.id,
        },
        revision: FindRevision([
            metadata.file_size(),
            metadata.last_write_time(),
            metadata.creation_time(),
            u64::from(attributes & !VOLATILE_ATTRIBUTES),
            basic.ChangeTime as u64,
            0,
        ]),
        metadata: EntryMetadata {
            size: (!directory).then_some(metadata.file_size()),
            modified: metadata.modified().ok(),
            native_attributes: Some(attributes),
            ..Default::default()
        },
    };
    Ok((entry, native))
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
