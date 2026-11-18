//! Handle-relative native access keeps recursion independent of mutable path traversal.
use super::{cancelled, checked, failure};
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
    ptr, thread,
    time::{Duration, Instant},
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
        Foundation::{ERROR_ACCESS_DENIED, RtlNtStatusToDosError, UNICODE_STRING},
        Security::{SECURITY_QUALITY_OF_SERVICE, SecurityIdentification},
        Storage::FileSystem::*,
        System::IO::IO_STATUS_BLOCK,
    },
};

/// The first wait before a refused directory rename is repeated; each later wait doubles.
const RENAME_RETRY_FIRST: Duration = Duration::from_millis(10);
/// The total wait after which a refused directory rename is reported (see [`rename`]).
const RENAME_RETRY_BUDGET: Duration = Duration::from_secs(1);
/// Cancellation and pause are observed at least this often while a rename waits (the
/// job-pause polling cadence).
const RENAME_RETRY_STEP: Duration = Duration::from_millis(10);

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct Identity {
    pub(crate) volume: u64,
    pub(crate) id: [u8; 16],
}

/// The volume serial number and 128-bit `FileIdInfo` identifier. It survives renames
/// and moves within its volume (NTFS, ReFS). Copy and Move compare identities across
/// their own renames (stage publication, Move capture, namespace pins), so they use only
/// this form and refuse a filesystem without it.
pub(crate) fn identity(file: &File) -> io::Result<Identity> {
    file_id(file).map_err(without_stable_identity)
}

fn without_stable_identity(error: io::Error) -> io::Error {
    if !unsupported(&error) {
        return error;
    }
    io::Error::new(
        io::ErrorKind::Unsupported,
        format!(
            "Copy and Move are not supported on this filesystem: it has no stable file \
             identities (FAT32 and exFAT media do not). Use an NTFS or ReFS volume, or \
             transfer these files with another tool. ({error})"
        ),
    )
}

/// Identity for operations that never compare an identity across their own rename:
/// permanent deletion, deletion discovery, Rename/F7/metadata preparation, Find and
/// advisory queries. Filesystems without `FileIdInfo` (FAT32, exFAT, some redirectors)
/// use the 32-bit volume serial number and the 64-bit file index instead. FAT derives
/// that index from the position of the entry's directory record: it is unique among the
/// existing entries, but renaming the entry to a longer name, moving it to another
/// directory or third-party defragmentation changes it, and a new entry can then take
/// over the old value. A volume consistently uses one of the two forms.
pub(crate) fn local_identity(file: &File) -> io::Result<Identity> {
    match file_id(file) {
        Err(error) if unsupported(&error) => legacy_identity(file),
        result => result,
    }
}

fn file_id(file: &File) -> io::Result<Identity> {
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

/// ERROR_INVALID_FUNCTION, ERROR_NOT_SUPPORTED and ERROR_INVALID_PARAMETER are returned
/// by filesystems (FAT32, exFAT, some redirectors) without an extended information class.
pub(crate) fn unsupported(error: &io::Error) -> bool {
    matches!(error.raw_os_error(), Some(1 | 50 | 87)) || error.kind() == io::ErrorKind::Unsupported
}

/// The 64-bit file index and 32-bit volume serial number, for filesystems without
/// `FileIdInfo` (see [`local_identity`]).
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

/// Reopens a filter-backed file's content (see `reparse::reopen_content`): like
/// `ReOpenFile`, the empty name relative to `pinned` reopens the same object, here
/// without `FILE_OPEN_REPARSE_POINT` so the owning filter supplies the data. Unlike
/// `ReOpenFile`, which rejects the `SECURITY_IDENTIFICATION` flag value, the open
/// carries an identification-level quality of service (see [`open_identifying`]).
pub(crate) fn reopen_mediated(pinned: &File, access: u32, share: u32) -> io::Result<File> {
    open_identifying(pinned, OsStr::new(""), access, share)
}

/// Opens a non-directory relative to `parent` and follows its reparse points, with an
/// identification-level security quality of service: should the open reach a named pipe
/// (a reparse point replaced by a symbolic link to one), its server can identify this
/// process but never impersonate it.
fn open_identifying(parent: &File, name: &OsStr, access: u32, share: u32) -> io::Result<File> {
    let quality = SECURITY_QUALITY_OF_SERVICE {
        Length: size_of::<SECURITY_QUALITY_OF_SERVICE>() as u32,
        ImpersonationLevel: SecurityIdentification,
        ContextTrackingMode: 0, // SECURITY_STATIC_TRACKING
        EffectiveOnly: false,
    };
    create_relative(
        parent,
        name,
        access,
        share,
        FILE_OPEN,
        FILE_NON_DIRECTORY_FILE,
        Some(&quality),
    )
}

pub(crate) fn open_relative(
    parent: &File,
    name: &OsStr,
    access: u32,
    share: u32,
    disposition: u32,
    options: u32,
) -> io::Result<File> {
    create_relative(
        parent,
        name,
        access,
        share,
        disposition,
        options | FILE_OPEN_REPARSE_POINT,
        None,
    )
}

fn create_relative(
    parent: &File,
    name: &OsStr,
    access: u32,
    share: u32,
    disposition: u32,
    options: u32,
    quality: Option<&SECURITY_QUALITY_OF_SERVICE>,
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
        SecurityQualityOfService: quality.map_or(ptr::null(), ptr::from_ref),
        ..Default::default()
    };
    let mut result = ptr::null_mut();
    let mut status = IO_STATUS_BLOCK::default();
    // SAFETY: parent is live; counted UTF-16 name, attributes (and the optional quality of
    // service they point to), result and status outlive the synchronous NtCreateFile call.
    // No caller uses a replace/truncate disposition. Every caller but `open_identifying`
    // (the filter-mediated reopen of the empty name) opens the final reparse point
    // directly; callers accept only one validated basename or the empty name of the parent
    // object itself. Synchronous I/O requires SYNCHRONIZE, which CreateFileW and
    // ReOpenFile add implicitly.
    let code = unsafe {
        NtCreateFile(
            &mut result,
            access | SYNCHRONIZE,
            &attributes,
            &mut status,
            ptr::null(),
            0,
            share,
            disposition,
            options | FILE_SYNCHRONOUS_IO_NONALERT,
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

/// Renames `source` to `name` inside its own directory `parent` (see [`rename_once`]).
///
/// Windows refuses to rename a directory with `ERROR_ACCESS_DENIED` while any entry below
/// it is open, whatever that handle's sharing, and antivirus, indexing and sync programs
/// open files for a moment after they are written (Microsoft Defender: about 10-50 ms per
/// new file on the test host). Such a refusal is waited out: the same rename is repeated
/// after 10 ms, then after twice as long each time, for about one second in all, and
/// then reported. Only a directory is retried, and only while the caller may still add a
/// subdirectory to `parent`: NTFS refuses a caller without that right with the same
/// error, and the DELETE access the rename also needs was granted when `source` was
/// opened, so a refusal by the parent directory's DACL is reported at once. Any other
/// refusal of a directory rename (a program, including this application's watch of a
/// subfolder, keeps an entry below open, or a filter driver refuses the rename) is
/// repeated and reported after about one second, once per refused folder. Meanwhile the
/// handle denies delete sharing, so neither the directory nor (by the same rule) its
/// ancestors can be renamed, and the no-replace rename still fails when another entry
/// takes `name`. A paused job waits paused; a cancelled one stops waiting with
/// `Interrupted`.
pub(crate) fn rename(
    source: &File,
    parent: &File,
    name: &OsStr,
    cancel: &Cancellation,
) -> io::Result<()> {
    rename_with(source, parent, name, &mut |delay| pause(delay, cancel))
}

/// [`rename`] with the wait between attempts supplied by the caller.
fn rename_with(
    source: &File,
    parent: &File,
    name: &OsStr,
    wait: &mut dyn FnMut(Duration) -> io::Result<()>,
) -> io::Result<()> {
    retry_refusal(
        || rename_once(source, name),
        || refused_for_open_entry(source, parent),
        wait,
    )
}

/// Repeats `attempt` while it fails with `ERROR_ACCESS_DENIED` and `transient` does not
/// rule out that the refusal passes, waiting 10 ms, then twice as long each time, for at
/// most [`RENAME_RETRY_BUDGET`] in all. Any other result, and the last refusal, is returned.
fn retry_refusal(
    mut attempt: impl FnMut() -> io::Result<()>,
    mut transient: impl FnMut() -> bool,
    wait: &mut dyn FnMut(Duration) -> io::Result<()>,
) -> io::Result<()> {
    let mut delay = RENAME_RETRY_FIRST;
    let mut waited = Duration::ZERO;
    loop {
        let error = match attempt() {
            Ok(()) => return Ok(()),
            Err(error) => error,
        };
        if error.raw_os_error() != Some(ERROR_ACCESS_DENIED as i32)
            || waited >= RENAME_RETRY_BUDGET
            || !transient()
        {
            return Err(error);
        }
        let step = delay.min(RENAME_RETRY_BUDGET - waited);
        wait(step)?;
        waited += step;
        delay = delay.saturating_mul(2);
    }
}

/// Whether `ERROR_ACCESS_DENIED` from renaming `source` may pass, as it does once no entry
/// below it is open: `source` is a directory, not a link, and the caller may still add a
/// subdirectory to `parent`, the one right besides DELETE that NTFS checks for it. A
/// filter driver that refuses the rename itself cannot be told apart and is waited for
/// too.
fn refused_for_open_entry(source: &File, parent: &File) -> bool {
    Native::query(source).is_ok_and(|native| native.directory() && !native.link())
        && reopen(
            parent,
            FILE_ADD_SUBDIRECTORY,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
        )
        .is_ok()
}

/// Waits `delay` between [`rename`] attempts in steps of at most [`RENAME_RETRY_STEP`].
fn pause(delay: Duration, cancel: &Cancellation) -> io::Result<()> {
    let deadline = Instant::now() + delay;
    loop {
        if !cancel.wait_if_paused() {
            return Err(cancelled());
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Ok(());
        }
        thread::sleep(left.min(RENAME_RETRY_STEP));
    }
}

/// One native rename of `source` to `name` in its own directory.
fn rename_once(source: &File, name: &OsStr) -> io::Result<()> {
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

/// Copy/Move cleanup keeps POSIX disposition only and its stricter attribute policy.
/// Their pinned anchors and pooled directory handles would leave a classic (delete-on-
/// close) disposition pending, and Copy/Move already refuse the filesystems without
/// stable identities, which are also those without POSIX disposition.
pub(crate) fn remove(source: &File) -> io::Result<()> {
    posix_dispose(source, 0)
}

fn remove_confirmed(source: &File) -> io::Result<()> {
    // The user explicitly confirmed permanent deletion. Ignore only the DOS
    // read-only attribute, without changing it on surviving hard links. DELETE
    // access, identity pins and no-follow traversal are still required.
    dispose(
        || posix_dispose(source, FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE),
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
        // The basic class exists on every filesystem, unlike the reparse-tag class.
        || {
            crate::reparse::attributes(source)
                .map(|attributes| attributes & FILE_ATTRIBUTE_READONLY != 0)
        },
    )
}

fn posix_dispose(source: &File, flags: FILE_DISPOSITION_INFO_EX_FLAGS) -> io::Result<()> {
    let information = FILE_DISPOSITION_INFO_EX {
        Flags: FILE_DISPOSITION_FLAG_DELETE | FILE_DISPOSITION_FLAG_POSIX_SEMANTICS | flags,
    };
    // SAFETY: source has DELETE access; the fixed-size structure is valid during this call.
    native_result(unsafe {
        SetFileInformationByHandle(
            source.as_raw_handle(),
            FileDispositionInfoEx,
            (&information as *const FILE_DISPOSITION_INFO_EX).cast(),
            size_of::<FILE_DISPOSITION_INFO_EX>() as u32,
        )
    })
}

fn native_result(succeeded: i32) -> io::Result<()> {
    if succeeded == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Confirmed permanent deletion. POSIX disposition removes the name immediately while
/// identity handles stay open. Filesystems without it (FAT32, exFAT, some redirectors)
/// reject the extended class; they fall back to the classic handle-bound disposition on
/// the same pinned handle, which removes the name once every handle to the entry is
/// closed (the caller closes its own right afterwards). The classic class cannot ignore
/// the DOS read-only attribute, so a read-only entry there is reported instead of
/// changing its attributes.
fn dispose(
    extended: impl FnOnce() -> io::Result<()>,
    classic: impl FnOnce() -> io::Result<()>,
    read_only: impl FnOnce() -> io::Result<bool>,
) -> io::Result<()> {
    let error = match extended() {
        Ok(()) => return Ok(()),
        Err(error) if unsupported(&error) => error,
        Err(error) => return Err(error),
    };
    if read_only()? {
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
    if local_identity(source)? != local_identity(&enumeration)? {
        return Err(directory_changed());
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
        return Err(directory_changed());
    }
    Ok(enumeration)
}

/// A directory replaced (or turned into a link) between its pin and its listing open is a
/// changed selection, reported like the adapter's other revalidation failures: `NotFound`.
/// The engine reserves `ErrorKind::Other` from a metadata listing for its name-budget
/// refusal, so this must not use `Other`.
fn directory_changed() -> io::Error {
    io::Error::new(
        io::ErrorKind::NotFound,
        "the directory changed before enumeration",
    )
}

fn child_names(directory: &File, cancel: &Cancellation) -> io::Result<Vec<OsString>> {
    collect_child_names(
        directory,
        cancel,
        Limits::wide(usize::MAX, usize::MAX),
        None,
    )
}

/// The recursive metadata snapshot: at most `max_names` names whose `OsStr::len` (their
/// WTF-8 bytes) together stay within `max_bytes`, as `PreparedMutation::metadata_children`
/// promises; the excess name is refused with `ErrorKind::Other` before it is retained. Each
/// name and the list hold no spare capacity, so the bytes the engine charges for the
/// snapshot are the bytes it keeps.
pub(super) fn bounded_child_names(
    directory: &File,
    cancel: &Cancellation,
    max_names: usize,
    max_bytes: usize,
) -> io::Result<Vec<OsString>> {
    let limits = Limits {
        names: max_names,
        bytes: max_bytes,
        snapshot: true,
    };
    let mut names = collect_child_names(directory, cancel, limits, None)?;
    names.shrink_to_fit();
    Ok(names)
}

pub(crate) fn has_exact_child_name(
    directory: &File,
    name: &OsStr,
    cancel: &Cancellation,
) -> io::Result<bool> {
    let limits = Limits::wide(1_000_000, 64 * 1024 * 1024);
    Ok(!collect_child_names(directory, cancel, limits, Some(name))?.is_empty())
}

/// How a listing bounds the names it reads.
#[derive(Clone, Copy)]
struct Limits {
    names: usize,
    bytes: usize,
    /// The metadata snapshot measures each name as its `OsStr::len` and keeps it shrunk to
    /// fit. Deletion and exact-name probes (Move shares them) measure UTF-16 bytes.
    snapshot: bool,
}

impl Limits {
    fn wide(names: usize, bytes: usize) -> Self {
        Self {
            names,
            bytes,
            snapshot: false,
        }
    }
}

fn collect_child_names(
    directory: &File,
    cancel: &Cancellation,
    limits: Limits,
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
                    if limits.snapshot {
                        "metadata listing cancelled"
                    } else {
                        "deletion cancelled"
                    },
                ));
            }
            let mut name = OsString::from_wide(record.name);
            let size = if limits.snapshot {
                name.len()
            } else {
                record.name.len() * 2
            };
            name_bytes = name_bytes.saturating_add(size);
            if name_count >= limits.names || name_bytes > limits.bytes {
                return Err(io::Error::other(
                    "metadata directory snapshot exceeds its resource budget",
                ));
            }
            name_count += 1;
            match find {
                Some(find) if name == find => {
                    found = true;
                    names = vec![name];
                }
                Some(_) => {}
                None => {
                    if limits.snapshot {
                        name.shrink_to_fit();
                    }
                    names.push(name);
                }
            }
            Ok(())
        })?;
    }
    Ok(names)
}

#[cfg(test)]
#[path = "mutation_native_tests.rs"]
mod tests;
