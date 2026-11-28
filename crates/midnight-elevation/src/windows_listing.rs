//! The helper's folder access on Windows. A requested folder is opened by its verbatim path
//! (no `..`, trailing-dot or device-name processing), following links as a navigation would;
//! its location is the normalized final path of that open handle in the NT namespace, which
//! starts with the volume's device; and it is listed through the same handle, so nothing can
//! exchange the folder between the containment check and the listing.
use crate::{
    helper::Entries,
    protocol::{self, WireEntry},
};
use midnight_core::Cancellation;
use std::{
    io,
    mem::{offset_of, size_of},
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::{Component, Path, Prefix},
    ptr,
};
use windows_sys::{
    Wdk::{
        Foundation::OBJECT_ATTRIBUTES,
        Storage::FileSystem::{
            FILE_DIRECTORY_FILE, FILE_OPEN, FILE_OPEN_FOR_BACKUP_INTENT,
            FILE_SYNCHRONOUS_IO_NONALERT, NtCreateFile,
        },
    },
    Win32::{
        Foundation::{
            ERROR_FILE_NOT_FOUND, ERROR_NO_MORE_FILES, OBJ_CASE_INSENSITIVE, RtlNtStatusToDosError,
            STATUS_NOT_A_DIRECTORY, UNICODE_STRING,
        },
        Security::{SECURITY_QUALITY_OF_SERVICE, SECURITY_STATIC_TRACKING, SecurityIdentification},
        Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
            FILE_FULL_DIR_INFO, FILE_LIST_DIRECTORY, FILE_NAME_NORMALIZED, FILE_READ_ATTRIBUTES,
            FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_TYPE_DISK,
            FileFullDirectoryInfo, FileFullDirectoryRestartInfo, GetFileInformationByHandle,
            GetFileInformationByHandleEx, GetFileType, GetFinalPathNameByHandleW, SYNCHRONIZE,
            VOLUME_NAME_NT,
        },
        System::IO::IO_STATUS_BLOCK,
    },
};

const BACKSLASH: u16 = b'\\' as u16;
/// A reparse tag whose reparse point names another object (a junction, a symbolic link).
const NAME_SURROGATE: u32 = 0x2000_0000;
/// 100-nanosecond intervals between 1601-01-01 and 1970-01-01.
const UNIX_EPOCH_INTERVALS: i64 = 116_444_736_000_000_000;
/// Longest final path read; NT paths are far shorter.
const MAX_FINAL_PATH: usize = 64 * 1024;

/// A folder opened for listing.
pub(crate) struct Directory(OwnedHandle);

/// Where an opened folder is: its normalized final NT path, such as
/// `\Device\HarddiskVolume3\Users\name`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Location(Vec<u16>);

impl Location {
    /// Whether `other` is this folder or below it on the same volume.
    pub(crate) fn contains(&self, other: &Self) -> bool {
        let root = self.0.strip_suffix(&[BACKSLASH]).unwrap_or(&self.0);
        other.0 == self.0
            || other
                .0
                .strip_prefix(root)
                .is_some_and(|rest| rest.first() == Some(&BACKSLASH))
    }
}

/// The verbatim spelling of an absolute path whose components after its root are ordinary
/// names, terminated for Windows.
fn verbatim(path: &Path) -> io::Result<Vec<u16>> {
    let refused = || protocol::invalid("Listing path must be an absolute folder path");
    let mut components = path.components();
    let mut wide: Vec<u16> = match components.next() {
        Some(Component::Prefix(prefix)) => match prefix.kind() {
            Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => {
                format!(r"\\?\{}:", char::from(letter))
                    .encode_utf16()
                    .collect()
            }
            Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => {
                let mut wide: Vec<u16> = r"\\?\UNC\".encode_utf16().collect();
                wide.extend(server.encode_wide());
                wide.push(BACKSLASH);
                wide.extend(share.encode_wide());
                wide
            }
            Prefix::Verbatim(name) => {
                let mut wide: Vec<u16> = r"\\?\".encode_utf16().collect();
                wide.extend(name.encode_wide());
                wide
            }
            Prefix::DeviceNS(_) => return Err(refused()),
        },
        _ => return Err(refused()),
    };
    if components.next() != Some(Component::RootDir) {
        return Err(refused());
    }
    wide.push(BACKSLASH);
    for (index, component) in components.enumerate() {
        let Component::Normal(name) = component else {
            return Err(refused());
        };
        if index > 0 {
            wide.push(BACKSLASH);
        }
        wide.extend(name.encode_wide());
    }
    if wide.contains(&0) {
        return Err(protocol::invalid("NUL in native path"));
    }
    wide.push(0);
    Ok(wide)
}

fn not_a_folder() -> io::Error {
    io::Error::new(io::ErrorKind::NotADirectory, "Listing path is not a folder")
}

/// Open the folder at `path` for listing. A pipe server reached through a link may only
/// identify this process, never impersonate it; anything but a folder on a disk is refused.
/// The open asks for a directory (`FILE_DIRECTORY_FILE`, as `O_DIRECTORY` on Linux): a file
/// that a request names is refused by its file system without being opened, so it is never
/// opened for its content, nor does the attempt meet another process's sharing.
pub(crate) fn open_directory(path: &Path) -> io::Result<Directory> {
    let mut name = verbatim(path)?;
    // `\\?\` becomes the NT namespace's `\??\`, which is what CreateFileW would open.
    name[1] = u16::from(b'?');
    name.pop();
    let bytes =
        u16::try_from(name.len() * 2).map_err(|_| protocol::invalid("Listing path is too long"))?;
    let unicode = UNICODE_STRING {
        Length: bytes,
        MaximumLength: bytes,
        Buffer: name.as_mut_ptr(),
    };
    let quality = SECURITY_QUALITY_OF_SERVICE {
        Length: size_of::<SECURITY_QUALITY_OF_SERVICE>() as u32,
        ImpersonationLevel: SecurityIdentification,
        ContextTrackingMode: u8::from(SECURITY_STATIC_TRACKING),
        EffectiveOnly: false,
    };
    let attributes = OBJECT_ATTRIBUTES {
        Length: size_of::<OBJECT_ATTRIBUTES>() as u32,
        ObjectName: &unicode,
        Attributes: OBJ_CASE_INSENSITIVE,
        SecurityQualityOfService: &quality,
        ..Default::default()
    };
    let mut raw = ptr::null_mut();
    let mut status = IO_STATUS_BLOCK::default();
    // SAFETY: the counted name, quality of service, attributes and outputs outlive this
    // synchronous open; only success transfers ownership of `raw`.
    let code = unsafe {
        NtCreateFile(
            &mut raw,
            FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
            &attributes,
            &mut status,
            ptr::null(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            FILE_OPEN,
            FILE_DIRECTORY_FILE | FILE_OPEN_FOR_BACKUP_INTENT | FILE_SYNCHRONOUS_IO_NONALERT,
            ptr::null(),
            0,
        )
    };
    if code == STATUS_NOT_A_DIRECTORY {
        return Err(not_a_folder());
    }
    if code < 0 {
        // SAFETY: pure conversion of the status the failed call returned.
        return Err(io::Error::from_raw_os_error(
            unsafe { RtlNtStatusToDosError(code) } as i32,
        ));
    }
    // SAFETY: NtCreateFile returned a new owned handle.
    let directory = Directory(unsafe { OwnedHandle::from_raw_handle(raw) });
    // SAFETY: a live handle; the query has no other effect.
    if unsafe { GetFileType(directory.0.as_raw_handle()) } != FILE_TYPE_DISK {
        return Err(protocol::invalid("Listing path is not a folder on a disk"));
    }
    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: a live handle and initialized output storage of the documented type.
    if unsafe { GetFileInformationByHandle(directory.0.as_raw_handle(), &mut information) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if information.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0 {
        return Err(not_a_folder());
    }
    Ok(directory)
}

impl Directory {
    /// The folder's location now, wherever links led the open.
    pub(crate) fn location(&self) -> io::Result<Location> {
        let mut buffer = vec![0u16; 512];
        loop {
            // SAFETY: a live handle and a writable buffer of the stated length.
            let length = unsafe {
                GetFinalPathNameByHandleW(
                    self.0.as_raw_handle(),
                    buffer.as_mut_ptr(),
                    buffer.len() as u32,
                    FILE_NAME_NORMALIZED | VOLUME_NAME_NT,
                )
            } as usize;
            if length == 0 {
                return Err(io::Error::last_os_error());
            }
            if length < buffer.len() {
                buffer.truncate(length);
                if buffer.first() != Some(&BACKSLASH) {
                    return Err(protocol::invalid("Unexpected final folder path"));
                }
                return Ok(Location(buffer));
            }
            // Too small: `length` is the size needed, with its terminator.
            if length > MAX_FINAL_PATH {
                return Err(protocol::invalid("Final folder path is too long"));
            }
            buffer = vec![0; length];
        }
    }
}

fn field<const N: usize>(record: &[u8], offset: usize) -> io::Result<[u8; N]> {
    record
        .get(offset..offset + N)
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or_else(|| protocol::invalid("Truncated folder enumeration record"))
}
fn u32_at(record: &[u8], offset: usize) -> io::Result<u32> {
    field(record, offset).map(u32::from_le_bytes)
}
fn i64_at(record: &[u8], offset: usize) -> io::Result<i64> {
    field(record, offset).map(i64::from_le_bytes)
}

/// A FILETIME count as seconds and nanoseconds since the Unix epoch, rounded down.
fn unix_time(intervals: i64) -> (i64, u32) {
    let since = i128::from(intervals) - i128::from(UNIX_EPOCH_INTERVALS);
    (
        since.div_euclid(10_000_000) as i64,
        (since.rem_euclid(10_000_000) * 100) as u32,
    )
}

/// One enumeration record as the entry it describes, from the record alone: links are
/// classified by their own attributes and reparse tag, never by their targets. `None` for
/// `.` and `..`.
fn entry(record: &[u8]) -> io::Result<Option<WireEntry>> {
    let attributes = u32_at(record, offset_of!(FILE_FULL_DIR_INFO, FileAttributes))?;
    let length = u32_at(record, offset_of!(FILE_FULL_DIR_INFO, FileNameLength))? as usize;
    // For a reparse point the extended-attribute size field holds its reparse tag.
    let tag = u32_at(record, offset_of!(FILE_FULL_DIR_INFO, EaSize))?;
    let start = offset_of!(FILE_FULL_DIR_INFO, FileName);
    let name = record
        .get(start..start + length)
        .filter(|name| name.len() % 2 == 0 && !name.is_empty())
        .ok_or_else(|| protocol::invalid("Invalid folder enumeration name"))?;
    if name == [b'.', 0] || name == [b'.', 0, b'.', 0] {
        return Ok(None);
    }
    let directory = attributes & FILE_ATTRIBUTE_DIRECTORY != 0;
    let link = attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 && tag & NAME_SURROGATE != 0;
    let kind = match (link, directory) {
        (true, true) => 1,
        (true, false) => 3,
        (false, true) => 0,
        (false, false) => 2,
    };
    let written = i64_at(record, offset_of!(FILE_FULL_DIR_INFO, LastWriteTime))?;
    let size = i64_at(record, offset_of!(FILE_FULL_DIR_INFO, EndOfFile))?;
    Ok(Some(WireEntry {
        name: name.to_vec(),
        kind,
        size: (kind == 2).then_some(size as u64),
        modified: Some(unix_time(written)),
        native_attributes: Some(attributes),
        unix_mode: None,
    }))
}

/// List the opened folder from its enumeration records; no entry is opened.
pub(crate) fn list_directory(
    directory: &Directory,
    cancel: &Cancellation,
) -> io::Result<Vec<WireEntry>> {
    // Eight-byte units keep every record aligned as the system writes it.
    let mut buffer = vec![0u64; 64 * 1024 / size_of::<u64>()];
    let mut class = FileFullDirectoryRestartInfo;
    let mut entries = Entries::default();
    loop {
        if cancel.is_cancelled() {
            return Err(crate::cancelled());
        }
        // SAFETY: a live handle opened for listing and a writable buffer of the stated size.
        let listed = unsafe {
            GetFileInformationByHandleEx(
                directory.0.as_raw_handle(),
                class,
                buffer.as_mut_ptr().cast(),
                (buffer.len() * size_of::<u64>()) as u32,
            )
        };
        let first = class == FileFullDirectoryRestartInfo;
        class = FileFullDirectoryInfo;
        if listed == 0 {
            let error = io::Error::last_os_error();
            // A folder without even `.` and `..`, such as an empty FAT root, has no first
            // record.
            if error.raw_os_error() == Some(ERROR_NO_MORE_FILES as i32)
                || (first && error.raw_os_error() == Some(ERROR_FILE_NOT_FOUND as i32))
            {
                return Ok(entries.into_vec());
            }
            return Err(error);
        }
        // SAFETY: the buffer is initialized memory of exactly this many bytes.
        let bytes = unsafe {
            std::slice::from_raw_parts(
                buffer.as_ptr().cast::<u8>(),
                buffer.len() * size_of::<u64>(),
            )
        };
        let mut offset = 0;
        loop {
            let record = bytes
                .get(offset..)
                .ok_or_else(|| protocol::invalid("Invalid folder enumeration offset"))?;
            if let Some(entry) = entry(record)? {
                entries.push(entry)?;
            }
            let next = u32_at(record, offset_of!(FILE_FULL_DIR_INFO, NextEntryOffset))? as usize;
            if next == 0 {
                break;
            }
            offset += next;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verbatim_paths_keep_ordinary_names_and_refuse_everything_else() {
        let text = |path: &str| {
            let wide = verbatim(Path::new(path)).unwrap();
            String::from_utf16(&wide[..wide.len() - 1]).unwrap()
        };
        assert_eq!(text(r"C:\Users\name"), r"\\?\C:\Users\name");
        assert_eq!(text(r"C:\"), r"\\?\C:\");
        assert_eq!(text(r"C:/Users//name/"), r"\\?\C:\Users\name");
        assert_eq!(text(r"\\?\C:\a b."), r"\\?\C:\a b.");
        assert_eq!(text(r"\\server\share\x"), r"\\?\UNC\server\share\x");
        assert_eq!(text(r"\\?\Volume{1}\x"), r"\\?\Volume{1}\x");
        for refused in [
            r"C:\Users\..\x",
            r"C:Users",
            r"\Users",
            r"\\.\COM1",
            "relative",
        ] {
            assert!(verbatim(Path::new(refused)).is_err(), "{refused}");
        }
    }

    #[test]
    fn locations_contain_their_own_subtree_only() {
        let location = |text: &str| Location(text.encode_utf16().collect());
        let root = location(r"\Device\HarddiskVolume3\Users\name");
        assert!(root.contains(&root));
        assert!(root.contains(&location(r"\Device\HarddiskVolume3\Users\name\x\y")));
        for outside in [
            r"\Device\HarddiskVolume3\Users",
            r"\Device\HarddiskVolume3\Users\name2",
            r"\Device\HarddiskVolume3\Users\nam",
            r"\Device\HarddiskVolume4\Users\name\x",
            r"\Device\HarddiskVolume33\Users\name",
        ] {
            assert!(!root.contains(&location(outside)), "{outside}");
        }
        let drive = location(r"\Device\HarddiskVolume3\");
        assert!(drive.contains(&location(r"\Device\HarddiskVolume3\Users")));
        assert!(!drive.contains(&location(r"\Device\HarddiskVolume30\Users")));
    }

    #[test]
    fn file_times_convert_around_the_unix_epoch() {
        assert_eq!(unix_time(UNIX_EPOCH_INTERVALS), (0, 0));
        assert_eq!(unix_time(UNIX_EPOCH_INTERVALS + 10_000_001), (1, 100));
        assert_eq!(unix_time(UNIX_EPOCH_INTERVALS - 1), (-1, 999_999_900));
        assert_eq!(unix_time(0), (-11_644_473_600, 0));
    }
}
