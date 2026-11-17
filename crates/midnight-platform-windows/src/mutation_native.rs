//! Handle-relative native access keeps recursion independent of mutable path traversal.
use super::{checked, failure};
use crate::{directory_records::Cursor, reparse::Native};
use midnight_core::{Cancellation, FsError};
use std::{
    ffi::{OsStr, OsString},
    fs::{File, OpenOptions},
    io,
    mem::size_of,
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        fs::OpenOptionsExt,
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
        Foundation::{RtlNtStatusToDosError, UNICODE_STRING},
        Storage::FileSystem::*,
        System::IO::IO_STATUS_BLOCK,
    },
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct Identity {
    pub(crate) volume: u64,
    pub(crate) id: [u8; 16],
}

pub(crate) fn identity(file: &File) -> io::Result<Identity> {
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
        let error = io::Error::last_os_error();
        return if unsupported(&error) {
            legacy_identity(file)
        } else {
            Err(error)
        };
    }
    Ok(Identity {
        volume: information.VolumeSerialNumber,
        id: information.FileId.Identifier,
    })
}

/// ERROR_INVALID_FUNCTION, ERROR_NOT_SUPPORTED and ERROR_INVALID_PARAMETER are returned
/// by filesystems (FAT32, exFAT, some redirectors) without an extended information class.
pub(crate) fn unsupported(error: &io::Error) -> bool {
    matches!(error.raw_os_error(), Some(1 | 50 | 87)) || error.kind() == io::ErrorKind::Unsupported
}

/// The 64-bit file index and 32-bit volume serial number, for filesystems without
/// `FileIdInfo`. A volume consistently uses one of the two forms.
fn legacy_identity(file: &File) -> io::Result<Identity> {
    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: file retains the live handle and information is an exact writable native
    // structure for this synchronous metadata-only query.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut information) } == 0 {
        let error = io::Error::last_os_error();
        return Err(if unsupported(&error) {
            io::ErrorKind::Unsupported.into()
        } else {
            error
        });
    }
    let index =
        (u64::from(information.nFileIndexHigh) << 32) | u64::from(information.nFileIndexLow);
    Ok(Identity {
        volume: u64::from(information.dwVolumeSerialNumber),
        id: u128::from(index).to_le_bytes(),
    })
}

pub(crate) fn open_parent(path: &Path) -> io::Result<File> {
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
    // Cloud placeholder and other non-surrogate reparse directories are ordinary
    // directories; only namespace-redirecting links are refused.
    if Native::query(&directory)?.link() {
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

/// Reopens exactly the object `file` refers to with new access, through an empty
/// handle-relative name instead of a pathname. Unlike `ReOpenFile`, this also works for
/// directories, including one reached through a junction or mount point.
pub(crate) fn reopen(file: &File, access: u32, share: u32) -> io::Result<File> {
    open_relative(file, OsStr::new(""), access, share, FILE_OPEN, 0)
}

pub(crate) fn open_relative(
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

pub(crate) fn rename(source: &File, name: &OsStr) -> io::Result<()> {
    let name: Vec<_> = name.encode_wide().chain(Some(0)).collect();
    let length = size_of::<FILE_RENAME_INFORMATION>() + name.len() * 2;
    let mut buffer = vec![0u64; length.div_ceil(size_of::<u64>())];
    let information = buffer.as_mut_ptr().cast::<FILE_RENAME_INFORMATION>();
    let mut status = IO_STATUS_BLOCK::default();
    // SAFETY: aligned allocation covers the header and counted UTF-16 basename. The name
    // pointer derives from the whole allocation, not from the one-element FileName field.
    // Native FileRenameInformation with null RootDirectory and a simple name renames within
    // the source handle's own parent; it never resolves through the process working
    // directory. ReplaceIfExists stays false. The source handle denies concurrent
    // delete/rename access.
    unsafe {
        (*information).FileNameLength = ((name.len() - 1) * 2) as u32;
        ptr::copy_nonoverlapping(
            name.as_ptr(),
            ptr::addr_of_mut!((*information).FileName).cast::<u16>(),
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

pub(crate) fn remove(source: &File) -> io::Result<()> {
    remove_with_flags(source, 0)
}

fn remove_confirmed(source: &File) -> io::Result<()> {
    // The user explicitly confirmed permanent deletion. Ignore only the DOS
    // read-only attribute, without changing it on surviving hard links. DELETE
    // access, identity pins and no-follow traversal are still required. Copy/
    // move cleanup uses remove() and retains its stricter attribute policy.
    remove_with_flags(source, FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE)
}

fn remove_with_flags(source: &File, flags: FILE_DISPOSITION_INFO_EX_FLAGS) -> io::Result<()> {
    dispose(
        flags,
        || {
            let information = FILE_DISPOSITION_INFO_EX {
                Flags: FILE_DISPOSITION_FLAG_DELETE | FILE_DISPOSITION_FLAG_POSIX_SEMANTICS | flags,
            };
            // SAFETY: source has DELETE access; the fixed-size structure is valid during
            // this call.
            native_result(unsafe {
                SetFileInformationByHandle(
                    source.as_raw_handle(),
                    FileDispositionInfoEx,
                    (&information as *const FILE_DISPOSITION_INFO_EX).cast(),
                    size_of::<FILE_DISPOSITION_INFO_EX>() as u32,
                )
            })
        },
        || {
            let information = FILE_DISPOSITION_INFO { DeleteFile: true };
            // SAFETY: source has DELETE access; the fixed-size structure is valid during
            // this call.
            native_result(unsafe {
                SetFileInformationByHandle(
                    source.as_raw_handle(),
                    FileDispositionInfo,
                    (&information as *const FILE_DISPOSITION_INFO).cast(),
                    size_of::<FILE_DISPOSITION_INFO>() as u32,
                )
            })
        },
        || Native::query(source).map(|native| native.attributes & FILE_ATTRIBUTE_READONLY != 0),
    )
}

fn native_result(succeeded: i32) -> io::Result<()> {
    if succeeded == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// POSIX disposition removes the name immediately while identity handles stay open.
/// Filesystems without it (FAT32, exFAT, some redirectors) reject the extended class;
/// they fall back to the classic handle-bound disposition on the same pinned handle,
/// which removes the name when the caller closes its last handle right afterwards.
/// The classic class cannot ignore the DOS read-only attribute, so a confirmed
/// deletion of a read-only entry there is reported instead of changing attributes.
fn dispose(
    flags: FILE_DISPOSITION_INFO_EX_FLAGS,
    extended: impl FnOnce() -> io::Result<()>,
    classic: impl FnOnce() -> io::Result<()>,
    read_only: impl FnOnce() -> io::Result<bool>,
) -> io::Result<()> {
    let error = match extended() {
        Ok(()) => return Ok(()),
        Err(error) if unsupported(&error) => error,
        Err(error) => return Err(error),
    };
    if flags & FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE != 0 && read_only()? {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "this filesystem cannot delete a read-only entry without changing its \
                 attributes; clear Read-only first ({error})"
            ),
        ));
    }
    classic()
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
            let child = match open_child(&frame.enumeration, &name, true) {
                Ok(child) => child,
                // Another process (a compiler, antivirus, a sync client) removed this
                // snapshotted child first; the requested end state is already reached.
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(failure("open delete child", &child_path, error)),
            };
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
            remove_confirmed(&frame.file)
                .map_err(|e| failure("delete directory", &frame.path, e))?;
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
    let native = Native::query(&file).map_err(|e| failure("read delete attributes", &path, e))?;
    // Links are leaves: the no-follow handle removes the link itself, never its target.
    // Cloud placeholder and other non-surrogate reparse directories are recursed.
    if !native.directory() || native.link() {
        remove_confirmed(&file).map_err(|e| failure("delete", &path, e))?;
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

pub(super) fn open_enumeration(parent: &File, source: &File, name: &OsStr) -> io::Result<File> {
    // Listing access shares its bit with file-content read. Only ordinary directories reach
    // here. Keep source's no-delete-sharing guard while opening relative to its pinned parent;
    // the extra handle allows sharing DELETE because source already holds that access.
    let enumeration = open_listing(parent, name)?;
    if identity(source)? != identity(&enumeration)? {
        return Err(io::Error::other("the directory changed before enumeration"));
    }
    Ok(enumeration)
}

/// Opens a child directory for listing without following its final reparse point and
/// refuses a namespace-redirecting link. Non-surrogate reparse directories (cloud
/// placeholders, ...) are ordinary directories.
pub(super) fn open_listing(parent: &File, name: &OsStr) -> io::Result<File> {
    let enumeration = open_relative(
        parent,
        name,
        FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
        FILE_OPEN,
        FILE_DIRECTORY_FILE,
    )?;
    if Native::query(&enumeration)?.link() {
        return Err(io::Error::other("the directory changed before enumeration"));
    }
    Ok(enumeration)
}

fn child_names(directory: &File, cancel: &Cancellation) -> io::Result<Vec<OsString>> {
    bounded_child_names(directory, cancel, usize::MAX, usize::MAX)
}

pub(super) fn bounded_child_names(
    directory: &File,
    cancel: &Cancellation,
    max_names: usize,
    max_bytes: usize,
) -> io::Result<Vec<OsString>> {
    collect_child_names(directory, cancel, max_names, max_bytes, None)
}

pub(crate) fn has_exact_child_name(
    directory: &File,
    name: &OsStr,
    cancel: &Cancellation,
) -> io::Result<bool> {
    Ok(
        !collect_child_names(directory, cancel, 1_000_000, 64 * 1024 * 1024, Some(name))?
            .is_empty(),
    )
}

fn collect_child_names(
    directory: &File,
    cancel: &Cancellation,
    max_names: usize,
    max_bytes: usize,
    find: Option<&OsStr>,
) -> io::Result<Vec<OsString>> {
    let mut names = Vec::new();
    let mut name_bytes = 0usize;
    let mut name_count = 0usize;
    let mut found = false;
    let mut cursor = Cursor::default();
    while !cursor.ended() && !found {
        cursor.next_batch(directory, &mut |record| {
            if found {
                return Ok(());
            }
            if !cancel.wait_if_paused() {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "deletion cancelled",
                ));
            }
            name_bytes = name_bytes.saturating_add(record.name.len() * 2);
            if name_count >= max_names || name_bytes > max_bytes {
                return Err(io::Error::other(
                    "metadata directory snapshot exceeds its resource budget",
                ));
            }
            name_count += 1;
            let name = OsString::from_wide(record.name);
            match find {
                Some(find) if name == find => {
                    found = true;
                    names = vec![name];
                }
                Some(_) => {}
                None => names.push(name),
            }
            Ok(())
        })?;
    }
    Ok(names)
}

#[cfg(test)]
#[path = "mutation_native_tests.rs"]
mod tests;
