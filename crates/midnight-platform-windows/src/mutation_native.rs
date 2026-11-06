//! Handle-relative native access keeps recursion independent of mutable path traversal.
use super::{checked, failure};
use midnight_core::{Cancellation, FsError};
use std::{
    ffi::{OsStr, OsString},
    fs::{File, OpenOptions},
    io,
    mem::{offset_of, size_of},
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        fs::{MetadataExt, OpenOptionsExt},
        io::{AsRawHandle, FromRawHandle},
    },
    path::{Path, PathBuf},
    ptr,
};
use windows_sys::{
    Wdk::{
        Foundation::OBJECT_ATTRIBUTES,
        Storage::FileSystem::{
            FILE_CREATE, FILE_DIRECTORY_FILE, FILE_NON_DIRECTORY_FILE, FILE_OPEN, FILE_OPEN_IF,
            FILE_OPEN_REPARSE_POINT, FILE_RENAME_INFORMATION, FILE_SYNCHRONOUS_IO_NONALERT,
            FileRenameInformation, NtCreateFile, NtSetInformationFile,
        },
    },
    Win32::{
        Foundation::{ERROR_NO_MORE_FILES, RtlNtStatusToDosError, UNICODE_STRING},
        Storage::FileSystem::*,
        System::IO::IO_STATUS_BLOCK,
    },
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct Identity {
    volume: u64,
    id: [u8; 16],
}

pub(super) fn identity(file: &File) -> io::Result<Identity> {
    let mut information = FILE_ID_INFO::default();
    // SAFETY: file owns a live handle; information is writable for the duration of the call.
    if unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileIdInfo,
            (&mut information as *mut FILE_ID_INFO).cast(),
            size_of::<FILE_ID_INFO>() as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(Identity {
        volume: information.VolumeSerialNumber,
        id: information.FileId.Identifier,
    })
}

pub(super) fn open_parent(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES | FILE_TRAVERSE)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
}

pub(super) fn open_child(parent: &File, name: &OsStr, mutate: bool) -> io::Result<File> {
    let access = FILE_READ_ATTRIBUTES | SYNCHRONIZE | if mutate { DELETE } else { 0 };
    let share = FILE_SHARE_READ | FILE_SHARE_WRITE | if mutate { 0 } else { FILE_SHARE_DELETE };
    open_relative(parent, name, access, share, FILE_OPEN, 0)
}

pub(super) fn create_directory(
    parent: &File,
    name: &OsStr,
    final_component: bool,
) -> io::Result<File> {
    let directory = open_relative(
        parent,
        name,
        FILE_READ_ATTRIBUTES | FILE_TRAVERSE | SYNCHRONIZE,
        FILE_SHARE_READ | FILE_SHARE_WRITE,
        if final_component {
            FILE_CREATE
        } else {
            FILE_OPEN_IF
        },
        FILE_DIRECTORY_FILE,
    )?;
    if directory.metadata()?.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "creating through a junction or symbolic link is not allowed",
        ));
    }
    Ok(directory)
}

pub(super) fn create_empty_file(parent: &File, name: &OsStr) -> io::Result<File> {
    // FILE_CREATE is exclusive, including when a directory or reparse point already uses
    // this basename. No existing file is opened and no content write or truncation occurs.
    open_relative(
        parent,
        name,
        FILE_READ_ATTRIBUTES | SYNCHRONIZE,
        FILE_SHARE_READ | FILE_SHARE_WRITE,
        FILE_CREATE,
        FILE_NON_DIRECTORY_FILE,
    )
}

fn open_relative(
    parent: &File,
    name: &OsStr,
    access: u32,
    share: u32,
    disposition: u32,
    options: u32,
) -> io::Result<File> {
    let mut wide: Vec<_> = name.encode_wide().collect();
    let bytes = u16::try_from(
        wide.len()
            .checked_mul(2)
            .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?,
    )
    .map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "basename exceeds the native Unicode string representation",
        )
    })?;
    let mut unicode = UNICODE_STRING {
        Length: bytes,
        MaximumLength: bytes,
        Buffer: wide.as_mut_ptr(),
    };
    let attributes = OBJECT_ATTRIBUTES {
        Length: size_of::<OBJECT_ATTRIBUTES>() as u32,
        RootDirectory: parent.as_raw_handle(),
        ObjectName: &mut unicode,
        ..Default::default()
    };
    let mut result = ptr::null_mut();
    let mut status = IO_STATUS_BLOCK::default();
    // SAFETY: parent is live; counted UTF-16 name, attributes, result and status outlive the
    // synchronous NtCreateFile call. No caller uses a replace/truncate disposition; the final
    // reparse point is opened directly. Only one validated basename is accepted by callers.
    let code = unsafe {
        NtCreateFile(
            &mut result,
            access,
            &attributes,
            &mut status,
            ptr::null(),
            0,
            share,
            disposition,
            options | FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT,
            ptr::null(),
            0,
        )
    };
    if code < 0 {
        // SAFETY: pure conversion of an NTSTATUS from the failed call.
        return Err(io::Error::from_raw_os_error(
            unsafe { RtlNtStatusToDosError(code) } as i32,
        ));
    }
    // SAFETY: successful NtCreateFile transferred a unique owned kernel handle to result.
    Ok(unsafe { File::from_raw_handle(result) })
}

pub(super) fn rename(source: &File, name: &OsStr) -> io::Result<()> {
    let name: Vec<_> = name.encode_wide().chain(Some(0)).collect();
    let length = size_of::<FILE_RENAME_INFORMATION>() + name.len() * 2;
    let mut buffer = vec![0u64; length.div_ceil(size_of::<u64>())];
    let information = buffer.as_mut_ptr().cast::<FILE_RENAME_INFORMATION>();
    let mut status = IO_STATUS_BLOCK::default();
    // SAFETY: aligned allocation covers the header and counted UTF-16 basename. Native
    // FileRenameInformation with null RootDirectory and a simple name renames within the
    // source handle's own parent; it never resolves through the process working directory.
    // ReplaceIfExists stays false. The source handle denies concurrent delete/rename access.
    unsafe {
        (*information).FileNameLength = ((name.len() - 1) * 2) as u32;
        ptr::copy_nonoverlapping(
            name.as_ptr(),
            (*information).FileName.as_mut_ptr(),
            name.len(),
        );
        let code = NtSetInformationFile(
            source.as_raw_handle(),
            &mut status,
            information.cast(),
            length as u32,
            FileRenameInformation,
        );
        if code < 0 {
            return Err(io::Error::from_raw_os_error(
                RtlNtStatusToDosError(code) as i32
            ));
        }
    }
    Ok(())
}

fn remove(source: &File) -> io::Result<()> {
    let information = FILE_DISPOSITION_INFO_EX {
        Flags: FILE_DISPOSITION_FLAG_DELETE | FILE_DISPOSITION_FLAG_POSIX_SEMANTICS,
    };
    // SAFETY: source has DELETE access; the fixed-size structure is valid during this call.
    if unsafe {
        SetFileInformationByHandle(
            source.as_raw_handle(),
            FileDispositionInfoEx,
            (&information as *const FILE_DISPOSITION_INFO_EX).cast(),
            size_of::<FILE_DISPOSITION_INFO_EX>() as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

struct Frame {
    file: File,
    enumeration: File,
    path: PathBuf,
    names: Vec<OsString>,
}

pub(super) fn delete_tree(
    source: File,
    parent: &File,
    path: PathBuf,
    cancel: &Cancellation,
    progress: &mut dyn FnMut(&Path),
) -> Result<(), FsError> {
    let mut stack = Vec::new();
    if let Some(frame) = visit_delete_entry(source, parent, path, cancel, progress)? {
        stack.push(frame);
    }
    loop {
        let Some(frame) = stack.last_mut() else {
            return Ok(());
        };
        checked(cancel, &frame.path)?;
        if let Some(name) = frame.names.pop() {
            let child_path = frame.path.join(&name);
            let child = open_child(&frame.enumeration, &name, true)
                .map_err(|e| failure("open delete child", &child_path, e))?;
            if let Some(next) =
                visit_delete_entry(child, &frame.enumeration, child_path, cancel, progress)?
            {
                stack.push(next);
            }
        } else {
            let frame = stack.pop().ok_or_else(|| {
                failure(
                    "delete",
                    Path::new(""),
                    io::Error::other("missing traversal frame"),
                )
            })?;
            remove(&frame.file).map_err(|e| failure("delete directory", &frame.path, e))?;
            drop(frame.file);
            progress(&frame.path);
        }
    }
}

fn visit_delete_entry(
    file: File,
    parent: &File,
    path: PathBuf,
    cancel: &Cancellation,
    progress: &mut dyn FnMut(&Path),
) -> Result<Option<Frame>, FsError> {
    checked(cancel, &path)?;
    let attrs = file
        .metadata()
        .map_err(|e| failure("read delete attributes", &path, e))?
        .file_attributes();
    if attrs & FILE_ATTRIBUTE_DIRECTORY == 0 || attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        remove(&file).map_err(|e| failure("delete", &path, e))?;
        drop(file);
        progress(&path);
        return Ok(None);
    }
    let name = path.file_name().ok_or_else(|| {
        failure(
            "enumerate deletion",
            &path,
            io::Error::other("missing directory basename"),
        )
    })?;
    let enumeration = open_enumeration(parent, &file, name)
        .map_err(|e| failure("open directory for enumeration", &path, e))?;
    let names =
        child_names(&enumeration, cancel).map_err(|e| failure("enumerate deletion", &path, e))?;
    // Two directory handles per active depth: DELETE/identity guard and enumeration. OS
    // resource exhaustion is returned; there is no depth cap or handle per descendant.
    Ok(Some(Frame {
        file,
        enumeration,
        path,
        names,
    }))
}

fn open_enumeration(parent: &File, source: &File, name: &OsStr) -> io::Result<File> {
    // Listing access shares its bit with file-content read. Only ordinary directories reach
    // here. Keep source's no-delete-sharing guard while opening relative to its pinned parent;
    // the extra handle allows sharing DELETE because source already holds that access.
    let enumeration = open_relative(
        parent,
        name,
        FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
        FILE_OPEN,
        FILE_DIRECTORY_FILE,
    )?;
    let attributes = enumeration.metadata()?.file_attributes();
    if attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || identity(source)? != identity(&enumeration)?
    {
        return Err(io::Error::other("the directory changed before enumeration"));
    }
    Ok(enumeration)
}

fn child_names(directory: &File, cancel: &Cancellation) -> io::Result<Vec<OsString>> {
    let mut names = Vec::new();
    let mut buffer = vec![0u64; 8192];
    loop {
        // SAFETY: aligned 64 KiB buffer is writable; directory owns a synchronous directory
        // handle. The returned offsets and UTF-16 lengths are checked before dereferencing.
        if unsafe {
            GetFileInformationByHandleEx(
                directory.as_raw_handle(),
                FileIdBothDirectoryInfo,
                buffer.as_mut_ptr().cast(),
                (buffer.len() * 8) as u32,
            )
        } == 0
        {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(ERROR_NO_MORE_FILES as i32) {
                return Ok(names);
            }
            return Err(error);
        }
        let mut offset = 0usize;
        loop {
            if cancel.is_cancelled() {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "deletion cancelled",
                ));
            }
            let header = offset_of!(FILE_ID_BOTH_DIR_INFO, FileName);
            if offset + size_of::<FILE_ID_BOTH_DIR_INFO>() > buffer.len() * 8 {
                return Err(io::Error::other("invalid directory record"));
            }
            // SAFETY: checked record lies inside aligned buffer; Windows offsets are 8-byte aligned.
            let record = unsafe {
                &*buffer
                    .as_ptr()
                    .cast::<u8>()
                    .add(offset)
                    .cast::<FILE_ID_BOTH_DIR_INFO>()
            };
            let bytes = record.FileNameLength as usize;
            if !bytes.is_multiple_of(2) || offset + header + bytes > buffer.len() * 8 {
                return Err(io::Error::other("invalid directory filename"));
            }
            // SAFETY: the preceding byte bounds and even length cover the entire UTF-16 slice.
            let units = unsafe { std::slice::from_raw_parts(record.FileName.as_ptr(), bytes / 2) };
            let name = OsString::from_wide(units);
            if name != "." && name != ".." {
                names.push(name);
            }
            if record.NextEntryOffset == 0 {
                break;
            }
            if !(record.NextEntryOffset as usize).is_multiple_of(8)
                || (record.NextEntryOffset as usize) < header + bytes
            {
                return Err(io::Error::other("invalid next directory record"));
            }
            offset += record.NextEntryOffset as usize;
        }
    }
}
