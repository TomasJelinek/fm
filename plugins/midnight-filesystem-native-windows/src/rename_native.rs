//! Audited handle-relative Windows Rename primitives. This module reads metadata,
//! never file contents. Policy owns retained pins, alias checks and recovery facts;
//! native no-replace rename is not an atomic compare-and-rename primitive.
use midnight_filesystem_plugin_api::{
    ErrorCode, Failure, Kind, MAX_RENAME_NAME_BYTES, MAX_RENAME_RECOVERY_ADDRESS_BYTES, Metadata,
};
use std::{
    ffi::{OsStr, OsString},
    fs::{File, OpenOptions},
    io,
    mem::{align_of, offset_of, size_of},
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        fs::{MetadataExt, OpenOptionsExt},
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::{Path, PathBuf},
    ptr,
};
use windows_sys::{
    Wdk::{
        Foundation::OBJECT_ATTRIBUTES,
        Storage::FileSystem::{
            FILE_CREATE, FILE_DIRECTORY_FILE, FILE_NON_DIRECTORY_FILE, FILE_OPEN,
            FILE_OPEN_REPARSE_POINT, FILE_RENAME_INFORMATION, FILE_SYNCHRONOUS_IO_NONALERT,
            FileRenameInformation, NtCreateFile, NtSetInformationFile,
        },
    },
    Win32::{
        Foundation::{
            ERROR_INSUFFICIENT_BUFFER, ERROR_NO_MORE_FILES, RtlNtStatusToDosError, UNICODE_STRING,
        },
        Globalization::{CSTR_EQUAL, CompareStringOrdinal},
        Security::{
            ACCESS_ALLOWED_ACE, ACL, ACL_REVISION, AddAccessAllowedAce,
            Cryptography::{BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom},
            GetLengthSid, GetTokenInformation, InitializeAcl, InitializeSecurityDescriptor,
            IsValidSid, PSID, SE_DACL_PROTECTED, SECURITY_DESCRIPTOR, SECURITY_MAX_SID_SIZE, SID,
            SetSecurityDescriptorControl, SetSecurityDescriptorDacl, SetSecurityDescriptorOwner,
            TOKEN_QUERY, TOKEN_USER, TokenUser,
        },
        Storage::FileSystem::*,
        System::{
            IO::IO_STATUS_BLOCK,
            Threading::{GetCurrentProcess, OpenProcessToken},
        },
    },
};

const IDENTITY_PREFIX: &[u8] = b"native.windows.object.v1";
const ENUMERATION_BYTES: usize = 64 * 1024;
const MAX_ENUMERATED_NAMES: usize = 1_000_000;
const MAX_ENUMERATED_NAME_BYTES: usize = 64 * 1024 * 1024;
const MAX_SECURITY_BYTES: usize = 64 * 1024;
const MAX_FINAL_PATH_UNITS: usize = MAX_RENAME_RECOVERY_ADDRESS_BYTES / 2 + 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Identity {
    pub volume: u64,
    pub id: [u8; 16],
}
impl Identity {
    /// The same opaque object encoding is used for sources, parents and targets.
    pub(super) fn bytes(&self) -> Vec<u8> {
        let mut bytes =
            Vec::with_capacity(IDENTITY_PREFIX.len() + size_of::<u64>() + self.id.len());
        self.append_bytes(&mut bytes);
        bytes
    }

    /// Callers that publish post-create facts reserve MAX_RENAME_IDENTITY_BYTES
    /// in an empty output before creating the stage; appending then allocates nothing.
    pub(super) fn append_bytes(&self, output: &mut Vec<u8>) {
        output.extend_from_slice(IDENTITY_PREFIX);
        output.extend_from_slice(&self.volume.to_le_bytes());
        output.extend_from_slice(&self.id);
    }
}

#[derive(Clone, Debug)]
pub(super) struct Snapshot {
    pub identity: Identity,
    pub kind: Kind,
    pub metadata: Metadata,
    /// A target-consent revision, not a source binding or Browse identity.
    pub revision: Vec<u8>,
    pub links: u32,
}

pub(super) fn identity(file: &File) -> io::Result<Identity> {
    let mut info = FILE_ID_INFO::default();
    // SAFETY: file retains its live handle and info is an aligned, exact-sized
    // writable FILE_ID_INFO output through this synchronous metadata query.
    if unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileIdInfo,
            (&mut info as *mut FILE_ID_INFO).cast(),
            size_of::<FILE_ID_INFO>() as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(Identity {
        volume: info.VolumeSerialNumber,
        id: info.FileId.Identifier,
    })
}

fn basic(file: &File) -> io::Result<FILE_BASIC_INFO> {
    let mut info = FILE_BASIC_INFO::default();
    // SAFETY: file owns the live handle and the writable output has exactly the
    // layout and capacity required by FileBasicInfo.
    if unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileBasicInfo,
            (&mut info as *mut FILE_BASIC_INFO).cast(),
            size_of::<FILE_BASIC_INFO>() as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(info)
}

fn standard(file: &File) -> io::Result<FILE_STANDARD_INFO> {
    let mut info = FILE_STANDARD_INFO::default();
    // SAFETY: file owns the live handle and the writable output has exactly the
    // layout and capacity required by FileStandardInfo.
    if unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileStandardInfo,
            (&mut info as *mut FILE_STANDARD_INFO).cast(),
            size_of::<FILE_STANDARD_INFO>() as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(info)
}

pub(super) fn snapshot(file: &File) -> Result<Snapshot, Failure> {
    // SAFETY: GetFileType only inspects this retained handle; it cannot read data.
    if unsafe { GetFileType(file.as_raw_handle()) } != FILE_TYPE_DISK {
        return Err(Failure::new(
            ErrorCode::Unsupported,
            "Rename requires a disk object.",
        ));
    }
    let before = basic(file).map_err(Failure::from_io)?;
    let standard_before = standard(file).map_err(Failure::from_io)?;
    let identity = identity(file).map_err(Failure::from_io)?;
    let metadata = file.metadata().map_err(Failure::from_io)?;
    let info = crate::adapter::info(metadata.file_type(), Some(&metadata));
    let standard_after = standard(file).map_err(Failure::from_io)?;
    let after = basic(file).map_err(Failure::from_io)?;
    let size = u64::try_from(standard_after.EndOfFile)
        .map_err(|_| Failure::new(ErrorCode::Invalid, "Invalid native Rename object length."))?;
    if before.CreationTime != after.CreationTime
        || before.LastWriteTime != after.LastWriteTime
        || before.ChangeTime != after.ChangeTime
        || before.FileAttributes != after.FileAttributes
        || standard_before.EndOfFile != standard_after.EndOfFile
        || standard_before.NumberOfLinks != standard_after.NumberOfLinks
        || standard_before.Directory != standard_after.Directory
        || standard_before.DeletePending != standard_after.DeletePending
        || standard_after.DeletePending
        || metadata.file_size() != size
        || metadata.file_attributes() != after.FileAttributes
        || metadata.last_write_time() != after.LastWriteTime as u64
        || metadata.creation_time() != after.CreationTime as u64
        || standard_after.Directory != (after.FileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0)
    {
        return Err(Failure::new(
            ErrorCode::SourceChanged,
            "Object changed while capturing Rename metadata.",
        ));
    }
    let mut revision = b"native.windows.rename.target.v1".to_vec();
    revision.extend(identity.volume.to_le_bytes());
    revision.extend(identity.id);
    revision.push(info.kind as u8);
    revision.extend(size.to_le_bytes());
    revision.extend(after.CreationTime.to_le_bytes());
    revision.extend(after.LastWriteTime.to_le_bytes());
    revision.extend(after.ChangeTime.to_le_bytes());
    revision.extend(after.FileAttributes.to_le_bytes());
    Ok(Snapshot {
        identity,
        kind: info.kind,
        metadata: info.metadata,
        revision,
        links: standard_after.NumberOfLinks,
    })
}

#[derive(Clone, Copy, Debug)]
pub(super) enum Access {
    Observe,
    SourceCommit,
    TargetCommit,
}

/// The caller has already converted path to its literal native spelling. As in
/// the host, the immediate parent itself may be reached through a directory link.
pub(super) fn open_parent(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES | FILE_TRAVERSE)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
}

pub(super) fn open_child(parent: &File, name: &OsStr, access: Access) -> io::Result<File> {
    let (rights, share, options) = match access {
        Access::Observe => (
            FILE_READ_ATTRIBUTES | SYNCHRONIZE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            0,
        ),
        Access::SourceCommit => (
            FILE_READ_ATTRIBUTES | SYNCHRONIZE | DELETE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            0,
        ),
        Access::TargetCommit => (
            FILE_READ_ATTRIBUTES | SYNCHRONIZE | DELETE,
            FILE_SHARE_READ,
            FILE_NON_DIRECTORY_FILE,
        ),
    };
    let mut wide = counted_name(name)?;
    let mut unicode = unicode_name(&mut wide)?;
    let attributes = OBJECT_ATTRIBUTES {
        Length: size_of::<OBJECT_ATTRIBUTES>() as u32,
        RootDirectory: parent.as_raw_handle(),
        ObjectName: &mut unicode,
        // Preserve the host's native lookup flags; policy retains the complete
        // case-only/hard-link entry guard rather than inferring entry from FileId.
        ..Default::default()
    };
    let mut handle = ptr::null_mut();
    let mut status = IO_STATUS_BLOCK::default();
    // SAFETY: parent, counted UTF-16 name, attributes and outputs remain live
    // until synchronous FILE_OPEN returns. The final reparse object is not followed.
    let code = unsafe {
        NtCreateFile(
            &mut handle,
            rights,
            &attributes,
            &mut status,
            ptr::null(),
            0,
            share,
            FILE_OPEN,
            options | FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT,
            ptr::null(),
            0,
        )
    };
    if code < 0 {
        return Err(status_error(code));
    }
    // SAFETY: only successful NtCreateFile transfers this unique kernel handle.
    Ok(unsafe { File::from_raw_handle(handle) })
}

/// Used only for ambiguous hard-link names. The caller checks this opened
/// directory's identity against its retained parent before enumeration begins.
pub(super) fn open_enumeration(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .access_mode(FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES | SYNCHRONIZE)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
}

pub(super) fn has_exact_name(
    directory: &File,
    name: &OsStr,
    cancelled: &dyn Fn() -> bool,
) -> io::Result<bool> {
    let expected = counted_name(name)?;
    let mut buffer = aligned_zeroed(ENUMERATION_BYTES, ENUMERATION_BYTES)?;
    let mut observed_names = 0usize;
    let mut observed_bytes = 0usize;
    loop {
        check_cancelled(cancelled)?;
        buffer.fill(0);
        // SAFETY: a live synchronous directory handle and aligned, initialized
        // 64 KiB output allocation remain borrowed through the metadata query.
        if unsafe {
            GetFileInformationByHandleEx(
                directory.as_raw_handle(),
                FileIdBothDirectoryInfo,
                buffer.as_mut_ptr().cast(),
                ENUMERATION_BYTES as u32,
            )
        } == 0
        {
            let error = io::Error::last_os_error();
            return if error.raw_os_error() == Some(ERROR_NO_MORE_FILES as i32) {
                Ok(false)
            } else {
                Err(error)
            };
        }
        // SAFETY: every byte of this u64 allocation was initialized, and the
        // native query has returned. The immutable byte view cannot outlive it.
        let bytes =
            unsafe { std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>(), ENUMERATION_BYTES) };
        let mut offset = 0usize;
        let mut found = false;
        loop {
            check_cancelled(cancelled)?;
            let header = offset_of!(FILE_ID_BOTH_DIR_INFO, FileName);
            let header_end = offset.checked_add(header).ok_or_else(invalid_record)?;
            if !offset.is_multiple_of(align_of::<FILE_ID_BOTH_DIR_INFO>())
                || header_end > bytes.len()
            {
                return Err(invalid_record());
            }
            let next = read_u32(
                bytes,
                offset + offset_of!(FILE_ID_BOTH_DIR_INFO, NextEntryOffset),
            )? as usize;
            let length = read_u32(
                bytes,
                offset + offset_of!(FILE_ID_BOTH_DIR_INFO, FileNameLength),
            )? as usize;
            let short_length = bytes[offset + offset_of!(FILE_ID_BOTH_DIR_INFO, ShortNameLength)];
            if length == 0
                || !length.is_multiple_of(2)
                || short_length > 24
                || !short_length.is_multiple_of(2)
                || !header_end.is_multiple_of(align_of::<u16>())
            {
                return Err(invalid_record());
            }
            let end = header_end.checked_add(length).ok_or_else(invalid_record)?;
            let next_offset = if next == 0 {
                None
            } else {
                let next_offset = offset.checked_add(next).ok_or_else(invalid_record)?;
                if !next.is_multiple_of(8)
                    || next < header + length
                    || next_offset
                        .checked_add(header)
                        .is_none_or(|end| end > bytes.len())
                {
                    return Err(invalid_record());
                }
                Some(next_offset)
            };
            if end > next_offset.unwrap_or(bytes.len()) {
                return Err(invalid_record());
            }
            observed_names = observed_names.checked_add(1).ok_or_else(resource_limit)?;
            observed_bytes = observed_bytes
                .checked_add(length)
                .ok_or_else(resource_limit)?;
            if observed_names > MAX_ENUMERATED_NAMES || observed_bytes > MAX_ENUMERATED_NAME_BYTES {
                return Err(resource_limit());
            }
            let raw = &bytes[header_end..end];
            let units = || {
                raw.as_chunks::<2>()
                    .0
                    .iter()
                    .map(|pair| u16::from_le_bytes(*pair))
            };
            if units().any(|unit| unit == 0 || unit == 47 || unit == 92) {
                return Err(invalid_record());
            }
            found |= units().eq(expected.iter().copied());
            if let Some(next_offset) = next_offset {
                offset = next_offset;
            } else {
                break;
            }
        }
        if found {
            return Ok(true);
        }
    }
}

pub(super) fn same_name_ignore_case(left: &OsStr, right: &OsStr) -> io::Result<bool> {
    let left = counted_name(left)?;
    let right = counted_name(right)?;
    let left_length = i32::try_from(left.len()).map_err(|_| invalid_name())?;
    let right_length = i32::try_from(right.len()).map_err(|_| invalid_name())?;
    // SAFETY: both bounded counted UTF-16 arrays remain live through the call;
    // explicit lengths preserve unpaired units and require no terminator.
    let result = unsafe {
        CompareStringOrdinal(left.as_ptr(), left_length, right.as_ptr(), right_length, 1)
    };
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(result == CSTR_EQUAL)
    }
}

/// All storage and checked name conversion are prepared before namespace writes.
/// apply only changes the borrowed destination handle and makes the native call.
pub(super) struct RenameBuffer {
    buffer: Vec<u64>,
    length: u32,
}
impl RenameBuffer {
    pub(super) fn new(name: &OsStr) -> io::Result<Self> {
        let wide = counted_name(name)?;
        let name_bytes = wide.len().checked_mul(2).ok_or_else(invalid_name)?;
        let length = size_of::<FILE_RENAME_INFORMATION>()
            .checked_add(name_bytes)
            .and_then(|length| length.checked_add(size_of::<u16>()))
            .ok_or_else(resource_limit)?;
        let maximum = MAX_RENAME_NAME_BYTES + size_of::<FILE_RENAME_INFORMATION>() + 8;
        let mut buffer = aligned_zeroed(length, maximum)?;
        let length = u32::try_from(length).map_err(|_| resource_limit())?;
        let information = buffer.as_mut_ptr().cast::<FILE_RENAME_INFORMATION>();
        // SAFETY: zeroed u64 storage is aligned for FILE_RENAME_INFORMATION and
        // covers its full header, counted name and an extra zero UTF-16 terminator.
        unsafe {
            (*information).Anonymous.ReplaceIfExists = false;
            (*information).RootDirectory = ptr::null_mut();
            (*information).FileNameLength = name_bytes as u32;
            ptr::copy_nonoverlapping(
                wide.as_ptr(),
                (*information).FileName.as_mut_ptr(),
                wide.len(),
            );
        }
        Ok(Self { buffer, length })
    }

    pub(super) fn retained_capacity(&self) -> usize {
        self.buffer.capacity() * size_of::<u64>()
    }

    pub(super) fn apply(&mut self, source: &File, parent: Option<&File>) -> io::Result<()> {
        let information = self.buffer.as_mut_ptr().cast::<FILE_RENAME_INFORMATION>();
        let mut status = IO_STATUS_BLOCK::default();
        // SAFETY: new preallocated this aligned complete native payload. Both
        // borrowed handles outlive the synchronous call; no callback is retained.
        // A null root with one basename names the source handle's own parent.
        let code = unsafe {
            (*information).Anonymous.ReplaceIfExists = false;
            (*information).RootDirectory =
                parent.map_or(ptr::null_mut(), |file| file.as_raw_handle());
            let code = NtSetInformationFile(
                source.as_raw_handle(),
                &mut status,
                information.cast(),
                self.length,
                FileRenameInformation,
            );
            (*information).RootDirectory = ptr::null_mut();
            code
        };
        if code < 0 {
            Err(status_error(code))
        } else {
            Ok(())
        }
    }
}

/// Only the caller's exact owned stage handle is eligible. The kernel performs
/// the nonempty check; this helper never enumerates or removes child entries.
pub(super) fn remove_empty_directory(directory: &File) -> io::Result<()> {
    if !standard(directory)?.Directory {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Recovery stage is not a directory.",
        ));
    }
    let information = FILE_DISPOSITION_INFO_EX {
        Flags: FILE_DISPOSITION_FLAG_DELETE | FILE_DISPOSITION_FLAG_POSIX_SEMANTICS,
    };
    // SAFETY: the caller retains the stage's DELETE-capable directory handle;
    // fixed-size disposition input is live through this exact-handle operation.
    if unsafe {
        SetFileInformationByHandle(
            directory.as_raw_handle(),
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

/// Observe the selected handle's stored basename, including short-name expansion,
/// beneath the exact retained parent. No directory-list access is requested.
pub(super) fn observed_child_basename(
    file: &File,
    parent: &File,
    scratch_bytes: usize,
) -> io::Result<OsString> {
    let mut parent_path = Vec::new();
    normalized_path(parent, &mut parent_path, 0, scratch_bytes)?;
    let mut child_path = Vec::new();
    let parent_bytes = parent_path.capacity() * size_of::<u16>();
    normalized_path(file, &mut child_path, parent_bytes, scratch_bytes)?;
    let mut suffix = child_path
        .strip_prefix(parent_path.as_slice())
        .ok_or_else(invalid_record)?;
    if parent_path.last() != Some(&u16::from(b'\\')) {
        suffix = suffix
            .strip_prefix(&[u16::from(b'\\')])
            .ok_or_else(invalid_record)?;
    }
    if suffix.is_empty()
        || suffix.contains(&u16::from(b'\\'))
        || suffix.contains(&u16::from(b'/'))
        || std::mem::size_of_val(suffix) > MAX_RENAME_NAME_BYTES
    {
        return Err(invalid_record());
    }
    let buffers = parent_bytes
        .checked_add(child_path.capacity() * size_of::<u16>())
        .ok_or_else(resource_limit)?;
    // WTF-8 can use three bytes per UTF-16 unit; allow the small-allocation floor too.
    let name_bound = suffix
        .len()
        .checked_mul(3)
        .and_then(|size| size.checked_add(8))
        .ok_or_else(resource_limit)?;
    if buffers
        .checked_add(name_bound)
        .is_none_or(|size| size > scratch_bytes)
    {
        return Err(resource_limit());
    }
    let name = OsString::from_wide(suffix);
    if buffers
        .checked_add(name.capacity())
        .is_none_or(|size| size > scratch_bytes)
    {
        return Err(resource_limit());
    }
    Ok(name)
}

fn normalized_path(
    file: &File,
    buffer: &mut Vec<u16>,
    other_bytes: usize,
    scratch_bytes: usize,
) -> io::Result<()> {
    let mut required = 256;
    loop {
        let capacity = buffer.capacity().max(required);
        if capacity
            .checked_mul(size_of::<u16>())
            .and_then(|size| size.checked_add(other_bytes))
            .is_none_or(|size| size > scratch_bytes)
        {
            return Err(resource_limit());
        }
        grow_path_buffer(buffer, required)?;
        if buffer
            .capacity()
            .checked_mul(size_of::<u16>())
            .and_then(|size| size.checked_add(other_bytes))
            .is_none_or(|size| size > scratch_bytes)
        {
            return Err(resource_limit());
        }
        let capacity = u32::try_from(buffer.len()).map_err(|_| resource_limit())?;
        // SAFETY: the live handle and initialized, budgeted UTF-16 allocation
        // remain borrowed through this metadata-only normalized-name query.
        let count = unsafe {
            GetFinalPathNameByHandleW(
                file.as_raw_handle(),
                buffer.as_mut_ptr(),
                capacity,
                VOLUME_NAME_DOS,
            )
        };
        if count == 0 {
            return Err(io::Error::last_os_error());
        }
        if count >= capacity {
            required = if count > capacity {
                count as usize
            } else {
                buffer.len().checked_add(1).ok_or_else(resource_limit)?
            };
            continue;
        }
        let count = count as usize;
        if buffer[..count].contains(&0) || buffer[count] != 0 {
            return Err(invalid_record());
        }
        buffer.truncate(count);
        return Ok(());
    }
}

pub(super) fn final_path(file: &File) -> io::Result<PathBuf> {
    let mut buffer = Vec::new();
    grow_path_buffer(&mut buffer, 256)?;
    loop {
        let capacity = u32::try_from(buffer.len()).map_err(|_| resource_limit())?;
        // SAFETY: retained live handle and dynamically sized writable UTF-16
        // buffer are valid throughout the call. The result length is checked.
        let count = unsafe {
            GetFinalPathNameByHandleW(
                file.as_raw_handle(),
                buffer.as_mut_ptr(),
                capacity,
                VOLUME_NAME_DOS,
            )
        };
        if count == 0 {
            return Err(io::Error::last_os_error());
        }
        if count >= capacity {
            let required = if count > capacity {
                count as usize
            } else {
                buffer.len().checked_add(1).ok_or_else(resource_limit)?
            };
            grow_path_buffer(&mut buffer, required)?;
            continue;
        }
        let path = &buffer[..count as usize];
        if path.contains(&0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid native final path.",
            ));
        }
        return Ok(PathBuf::from(OsString::from_wide(path)));
    }
}

/// Confirm a locator after namespace effects using only caller-owned initialized
/// storage. The returned count excludes NUL; no allocation occurs, even on error.
pub(super) fn final_path_units(file: &File, output: &mut [u16]) -> io::Result<usize> {
    let capacity = output.len().min(MAX_FINAL_PATH_UNITS);
    let capacity =
        u32::try_from(capacity).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    if capacity == 0 {
        return Err(io::Error::from_raw_os_error(
            ERROR_INSUFFICIENT_BUFFER as i32,
        ));
    }
    // SAFETY: the borrowed handle and initialized writable slice remain valid
    // throughout this synchronous query. Capacity never exceeds the slice or
    // transport budget, and the native result is checked before indexing.
    let count = unsafe {
        GetFinalPathNameByHandleW(
            file.as_raw_handle(),
            output.as_mut_ptr(),
            capacity,
            VOLUME_NAME_DOS,
        )
    };
    if count == 0 {
        return Err(io::Error::last_os_error());
    }
    if count >= capacity {
        return Err(io::Error::from_raw_os_error(
            ERROR_INSUFFICIENT_BUFFER as i32,
        ));
    }
    let count = count as usize;
    if count * size_of::<u16>() > MAX_RENAME_RECOVERY_ADDRESS_BYTES
        || output[..count].contains(&0)
        || output[count] != 0
    {
        return Err(io::Error::from(io::ErrorKind::InvalidData));
    }
    Ok(count)
}

fn grow_path_buffer(buffer: &mut Vec<u16>, units: usize) -> io::Result<()> {
    if units > MAX_FINAL_PATH_UNITS {
        return Err(resource_limit());
    }
    if units > buffer.len() {
        buffer
            .try_reserve_exact(units - buffer.len())
            .map_err(|_| resource_limit())?;
        if buffer.capacity() > MAX_FINAL_PATH_UNITS {
            return Err(resource_limit());
        }
        buffer.resize(units, 0);
    }
    Ok(())
}

pub(super) fn local_fixed_ntfs(file: &File) -> bool {
    let Ok(path) = final_path(file) else {
        return false;
    };
    let Some(root) = path
        .ancestors()
        .find(|ancestor| ancestor.parent().is_none())
    else {
        return false;
    };
    let root: Vec<_> = root.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut filesystem = [0u16; 32];
    // SAFETY: root is a terminated handle-derived path and file retains the
    // queried handle. The filesystem output is fixed-size writable storage.
    unsafe {
        GetDriveTypeW(root.as_ptr()) == 3
            && GetVolumeInformationByHandleW(
                file.as_raw_handle(),
                ptr::null_mut(),
                0,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                filesystem.as_mut_ptr(),
                filesystem.len() as u32,
            ) != 0
            && filesystem.starts_with(&[78, 84, 70, 83, 0])
    }
}

pub(super) fn random_stage_name() -> io::Result<OsString> {
    let mut random = [0u8; 16];
    // SAFETY: null provider is required by SYSTEM_PREFERRED_RNG; the writable
    // stack array has exactly the declared 16-byte capacity.
    let status = unsafe {
        BCryptGenRandom(
            ptr::null_mut(),
            random.as_mut_ptr(),
            random.len() as u32,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    if status < 0 {
        return Err(status_error(status));
    }
    let mut name = String::new();
    name.try_reserve_exact(".midnight-recovery-".len() + random.len() * 2)
        .map_err(|_| resource_limit())?;
    name.push_str(".midnight-recovery-");
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in random {
        name.push(char::from(HEX[usize::from(byte >> 4)]));
        name.push(char::from(HEX[usize::from(byte & 15)]));
    }
    Ok(OsString::from(name))
}

pub(super) fn create_private_directory(parent: &File, name: &OsStr) -> io::Result<File> {
    let mut wide = counted_name(name)?;
    let mut unicode = unicode_name(&mut wide)?;
    let (user, user_length) = current_user()?;
    let sid = token_sid(&user, user_length)?;
    // SAFETY: token_sid validated the SID and its full extent in the live user allocation.
    let sid_length = unsafe { GetLengthSid(sid) } as usize;
    let acl_length = size_of::<ACL>()
        .checked_add(size_of::<ACCESS_ALLOWED_ACE>() - size_of::<u32>())
        .and_then(|length| length.checked_add(sid_length))
        .ok_or_else(resource_limit)?;
    let user_bytes = user
        .capacity()
        .checked_mul(size_of::<u64>())
        .ok_or_else(resource_limit)?;
    let acl_budget = MAX_SECURITY_BYTES
        .checked_sub(user_bytes + size_of::<SECURITY_DESCRIPTOR>())
        .ok_or_else(resource_limit)?;
    let mut acl = aligned_zeroed(acl_length, acl_budget)?;
    let acl_pointer = acl.as_mut_ptr().cast::<ACL>();
    let mut descriptor = SECURITY_DESCRIPTOR::default();
    let descriptor_pointer = (&mut descriptor as *mut SECURITY_DESCRIPTOR).cast();
    // SAFETY: SID, aligned ACL and absolute descriptor remain live until create
    // returns. A protected DACL grants FILE_ALL_ACCESS only to the process token
    // user, who is also set as owner; inherited broad permissions are excluded.
    let initialized = unsafe {
        InitializeAcl(acl_pointer, acl_length as u32, ACL_REVISION) != 0
            && AddAccessAllowedAce(acl_pointer, ACL_REVISION, FILE_ALL_ACCESS, sid) != 0
            && InitializeSecurityDescriptor(descriptor_pointer, 1) != 0
            && SetSecurityDescriptorDacl(descriptor_pointer, 1, acl_pointer, 0) != 0
            && SetSecurityDescriptorOwner(descriptor_pointer, sid, 0) != 0
            && SetSecurityDescriptorControl(
                descriptor_pointer,
                SE_DACL_PROTECTED,
                SE_DACL_PROTECTED,
            ) != 0
    };
    if !initialized {
        return Err(io::Error::last_os_error());
    }
    let attributes = OBJECT_ATTRIBUTES {
        Length: size_of::<OBJECT_ATTRIBUTES>() as u32,
        RootDirectory: parent.as_raw_handle(),
        ObjectName: &mut unicode,
        SecurityDescriptor: &descriptor,
        ..Default::default()
    };
    let mut handle = ptr::null_mut();
    let mut status = IO_STATUS_BLOCK::default();
    // SAFETY: all allocation, name checks and ACL setup finished before this
    // exclusive create. Parent, counted name, SID, ACL, descriptor and outputs
    // remain live through the synchronous call; no existing entry is opened.
    let code = unsafe {
        NtCreateFile(
            &mut handle,
            FILE_READ_ATTRIBUTES | FILE_TRAVERSE | SYNCHRONIZE | DELETE,
            &attributes,
            &mut status,
            ptr::null(),
            FILE_ATTRIBUTE_DIRECTORY,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            FILE_CREATE,
            FILE_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT,
            ptr::null(),
            0,
        )
    };
    if code < 0 {
        return Err(status_error(code));
    }
    // SAFETY: only successful exclusive creation transfers this owned handle.
    // Return immediately: subsequent snapshot failure belongs to caller receipts.
    Ok(unsafe { File::from_raw_handle(handle) })
}

fn current_user() -> io::Result<(Vec<u64>, usize)> {
    let mut handle = ptr::null_mut();
    // SAFETY: the current process pseudo-handle is valid and handle is writable.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut handle) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful OpenProcessToken transferred a unique owned token handle.
    let token = unsafe { OwnedHandle::from_raw_handle(handle) };
    let mut needed = 0u32;
    // SAFETY: a zero-sized probe only writes needed; the token stays live.
    let probed = unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            ptr::null_mut(),
            0,
            &mut needed,
        )
    };
    if probed != 0 {
        return Err(invalid_security());
    }
    let probe_error = io::Error::last_os_error();
    if probe_error.raw_os_error() != Some(ERROR_INSUFFICIENT_BUFFER as i32) {
        return Err(probe_error);
    }
    let requested = needed as usize;
    if requested < size_of::<TOKEN_USER>() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid native token user length.",
        ));
    }
    let mut user = aligned_zeroed(requested, MAX_SECURITY_BYTES)?;
    let mut returned = 0;
    // SAFETY: the aligned initialized allocation covers needed bytes and the
    // token/output-length storage remain live through this synchronous query.
    if unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            user.as_mut_ptr().cast(),
            needed,
            &mut returned,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    if (returned as usize) < size_of::<TOKEN_USER>() || returned > needed {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid native token user result.",
        ));
    }
    let returned = returned as usize;
    token_sid(&user, returned)?;
    Ok((user, returned))
}

fn token_sid(user: &[u64], bytes: usize) -> io::Result<PSID> {
    let available = user
        .len()
        .checked_mul(size_of::<u64>())
        .ok_or_else(resource_limit)?;
    if bytes < size_of::<TOKEN_USER>() || bytes > available {
        return Err(invalid_security());
    }
    // SAFETY: user starts at u64 alignment and covers TOKEN_USER, initialized by
    // the native query. Only its pointer field is read before validating bounds.
    let sid = unsafe { (*user.as_ptr().cast::<TOKEN_USER>()).User.Sid };
    let start = (sid as usize)
        .checked_sub(user.as_ptr() as usize)
        .ok_or_else(invalid_security)?;
    let header = offset_of!(SID, SubAuthority);
    if !(sid as usize).is_multiple_of(align_of::<SID>())
        || start.checked_add(header).is_none_or(|end| end > bytes)
    {
        return Err(invalid_security());
    }
    // SAFETY: the preceding check bounds the SID header inside initialized user
    // storage; reading SubAuthorityCount does not dereference an unchecked SID.
    let count = unsafe { *user.as_ptr().cast::<u8>().add(start + 1) };
    let length = header + usize::from(count) * size_of::<u32>();
    if length > SECURITY_MAX_SID_SIZE as usize
        || start.checked_add(length).is_none_or(|end| end > bytes)
    {
        return Err(invalid_security());
    }
    // SAFETY: the entire SID extent has been checked inside the live token-user
    // allocation. Native validation and length inspection cannot run off it.
    if unsafe { IsValidSid(sid) } == 0 || unsafe { GetLengthSid(sid) } as usize != length {
        return Err(invalid_security());
    }
    Ok(sid)
}

fn counted_name(name: &OsStr) -> io::Result<Vec<u16>> {
    let count = name.encode_wide().count();
    let bytes = count
        .checked_mul(size_of::<u16>())
        .ok_or_else(invalid_name)?;
    if count == 0 || bytes > MAX_RENAME_NAME_BYTES || u16::try_from(bytes).is_err() {
        return Err(invalid_name());
    }
    let mut wide = Vec::new();
    wide.try_reserve_exact(count)
        .map_err(|_| resource_limit())?;
    wide.extend(name.encode_wide());
    // A colon selects an alternate data stream rather than one directory entry.
    // Observed trailing dots/spaces and DOS-like spellings otherwise stay lossless.
    if wide.iter().any(|unit| [0, 47, 58, 92].contains(unit)) || wide == [46] || wide == [46, 46] {
        return Err(invalid_name());
    }
    Ok(wide)
}

fn unicode_name(wide: &mut [u16]) -> io::Result<UNICODE_STRING> {
    let bytes = wide
        .len()
        .checked_mul(size_of::<u16>())
        .ok_or_else(invalid_name)?;
    let bytes = u16::try_from(bytes).map_err(|_| invalid_name())?;
    Ok(UNICODE_STRING {
        Length: bytes,
        MaximumLength: bytes,
        Buffer: wide.as_mut_ptr(),
    })
}

fn aligned_zeroed(bytes: usize, maximum: usize) -> io::Result<Vec<u64>> {
    let words = bytes.div_ceil(size_of::<u64>());
    let allocation = words
        .checked_mul(size_of::<u64>())
        .ok_or_else(resource_limit)?;
    if allocation > maximum {
        return Err(resource_limit());
    }
    let mut buffer = Vec::new();
    buffer
        .try_reserve_exact(words)
        .map_err(|_| resource_limit())?;
    if buffer
        .capacity()
        .checked_mul(size_of::<u64>())
        .is_none_or(|bytes| bytes > maximum)
    {
        return Err(resource_limit());
    }
    buffer.resize(words, 0);
    Ok(buffer)
}

fn read_u32(bytes: &[u8], offset: usize) -> io::Result<u32> {
    let end = offset
        .checked_add(size_of::<u32>())
        .ok_or_else(invalid_record)?;
    let value = bytes.get(offset..end).ok_or_else(invalid_record)?;
    Ok(u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
}

fn check_cancelled(cancelled: &dyn Fn() -> bool) -> io::Result<()> {
    if cancelled() {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Rename alias check canceled.",
        ))
    } else {
        Ok(())
    }
}

fn status_error(status: i32) -> io::Error {
    // SAFETY: pure conversion of a native status returned by the preceding call.
    io::Error::from_raw_os_error(unsafe { RtlNtStatusToDosError(status) } as i32)
}

fn invalid_name() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "Rename requires one representable native basename.",
    )
}

fn invalid_record() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Invalid native directory record.",
    )
}

fn invalid_security() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "Invalid native token user SID.")
}

fn resource_limit() -> io::Error {
    io::Error::new(
        io::ErrorKind::OutOfMemory,
        "Rename native allocation or scan budget exceeded.",
    )
}
/// Test-only access to the real synchronous SDK boundary with a live cancel flag.
#[cfg(test)]
pub(super) fn test_call(
    api: &midnight_filesystem_plugin_api::abi::Api,
    instance: u64,
    request: midnight_filesystem_plugin_api::Request,
    cancelled: &std::sync::atomic::AtomicBool,
) -> Result<midnight_filesystem_plugin_api::Response, u32> {
    use midnight_filesystem_plugin_api::{
        DEFAULT_REPLY_BYTES, DEFAULT_REQUEST_BYTES, abi::*, decode_response, encode_request,
    };
    use std::sync::atomic::{AtomicBool, Ordering};

    unsafe extern "C" fn cancellation(context: u64) -> u32 {
        // SAFETY: test_call created this context from its live borrowed AtomicBool.
        // The synchronous ABI borrows the callback/context only until call returns.
        let flag = unsafe { &*(context as usize as *const AtomicBool) };
        u32::from(flag.load(Ordering::Acquire))
    }

    struct Release<'a> {
        release: unsafe extern "C" fn(u64, u64) -> u32,
        instance: u64,
        buffer: u64,
        status: &'a mut u32,
    }
    impl Drop for Release<'_> {
        fn drop(&mut self) {
            // SAFETY: the producing instance remains owned by the caller until
            // this helper returns. The guard releases this returned buffer once,
            // including descriptor/codec errors and unwinding during decoding.
            *self.status = unsafe { (self.release)(self.instance, self.buffer) };
        }
    }

    api.validate(size_of::<Api>() as u32)?;
    let call = api.call.ok_or(INVALID)?;
    let release = api.release_buffer.ok_or(INVALID)?;
    let payload = encode_request(&request, DEFAULT_REQUEST_BYTES as usize).map_err(|_| INVALID)?;
    let context = u64::try_from(cancelled as *const AtomicBool as usize).map_err(|_| INVALID)?;
    let input = Call {
        struct_size: size_of::<Call>() as u32,
        operation: request.operation() as u32,
        payload: Slice {
            data: payload.as_ptr(),
            len: payload.len() as u64,
        },
        cancel: Cancellation {
            context,
            cancelled: Some(cancellation),
        },
    };
    let mut output = Buffer::default();
    // SAFETY: payload, input, output and the borrowed AtomicBool remain mapped
    // until this synchronous call finishes. The SDK retains no cancellation data.
    let status = unsafe { call(instance, &input, &mut output) };
    if status != OK {
        if output.id != 0 {
            // SAFETY: release uses only the producing instance and returned ID;
            // do not inspect failed-call bytes, but retire any disclosed buffer.
            unsafe { release(instance, output.id) };
        }
        return Err(status);
    }
    let mut release_status = OK;
    let decoded = {
        let _release = Release {
            release,
            instance,
            buffer: output.id,
            status: &mut release_status,
        };
        (|| {
            output.validate(DEFAULT_REPLY_BYTES)?;
            let length = usize::try_from(output.bytes.len).map_err(|_| INVALID)?;
            if output.id == 0 || length == 0 || length > isize::MAX as usize {
                return Err(INVALID);
            }
            // SAFETY: successful SDK output owns length immutable readable bytes
            // until the release guard drops. Bounded validation rejects null data
            // and oversized lengths before constructing this nonempty byte view.
            let bytes = unsafe { std::slice::from_raw_parts(output.bytes.data, length) };
            decode_response(bytes, DEFAULT_REPLY_BYTES as usize).map_err(|_| INVALID)
        })()
    };
    decoded.and_then(|response| {
        if release_status == OK {
            Ok(response)
        } else {
            Err(release_status)
        }
    })
}
/// Test-only ACL owner. Explicit restore reports errors; unwind cleanup is a
/// best-effort guard for owned fixtures, separate from production pin disposal.
#[cfg(test)]
pub(super) struct TestMetadataAccess {
    file: File,
    saved: Vec<u64>,
    scratch: Vec<u64>,
    saved_length: usize,
    saved_control: u16,
    active: bool,
}

#[cfg(test)]
impl TestMetadataAccess {
    pub(super) fn restore(mut self) -> io::Result<()> {
        let result = self.restore_inner();
        if result.is_ok() {
            self.active = false;
        }
        result
    }

    fn restore_inner(&mut self) -> io::Result<()> {
        use windows_sys::{
            Wdk::Storage::FileSystem::NtSetSecurityObject,
            Win32::Security::{
                DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
                SE_DACL_AUTO_INHERIT_REQ, SE_DACL_AUTO_INHERITED,
                UNPROTECTED_DACL_SECURITY_INFORMATION,
            },
        };
        self.scratch.copy_from_slice(&self.saved);
        if self.saved_control & SE_DACL_AUTO_INHERITED != 0 {
            // SAFETY: scratch is a complete aligned copy of the saved native
            // self-relative descriptor. Only this copy receives the request bit;
            // saved remains the immutable exact restoration/readback baseline.
            if unsafe {
                SetSecurityDescriptorControl(
                    self.scratch.as_mut_ptr().cast(),
                    SE_DACL_AUTO_INHERIT_REQ,
                    SE_DACL_AUTO_INHERIT_REQ,
                )
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
        }
        let protection = if self.saved_control & SE_DACL_PROTECTED != 0 {
            PROTECTED_DACL_SECURITY_INFORMATION
        } else {
            UNPROTECTED_DACL_SECURITY_INFORMATION
        };
        // SAFETY: the same retained WRITE_DAC handle still names the fixture
        // after Rename. GetKernelObjectSecurity initialized the complete saved
        // self-relative descriptor; its aligned scratch copy remains live through
        // this call. Only DACL information changes; owner/group/SACL and pathnames
        // are untouched.
        let status = unsafe {
            NtSetSecurityObject(
                self.file.as_raw_handle(),
                DACL_SECURITY_INFORMATION | protection,
                self.scratch.as_mut_ptr().cast(),
            )
        };
        if status < 0 {
            return Err(status_error(status));
        }
        let length = test_read_dacl(&self.file, &mut self.scratch)?;
        let control = test_dacl_control(&mut self.scratch)?;
        if length != self.saved_length
            || control != self.saved_control
            || test_security_bytes(&self.saved, self.saved_length)?
                != test_security_bytes(&self.scratch, length)?
        {
            let saved = test_security_bytes(&self.saved, self.saved_length)?;
            let current = test_security_bytes(&self.scratch, length)?;
            let first_difference = (0..saved.len().max(current.len()))
                .find(|&offset| saved.get(offset) != current.get(offset));
            let saved_byte = first_difference
                .and_then(|offset| saved.get(offset))
                .copied();
            let current_byte = first_difference
                .and_then(|offset| current.get(offset))
                .copied();
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "Fixture DACL/control restoration could not be confirmed exactly: \
                     saved_length={}, current_length={length}, saved_control={:#06x}, \
                     current_control={control:#06x}, control_xor={:#06x}, \
                     first_difference={first_difference:?}, saved_byte={saved_byte:?}, \
                     current_byte={current_byte:?}.",
                    self.saved_length,
                    self.saved_control,
                    self.saved_control ^ control,
                ),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
impl Drop for TestMetadataAccess {
    fn drop(&mut self) {
        if self.active && std::thread::panicking() {
            let _ = self.restore_inner();
        }
    }
}

/// Temporarily deny file-content/listing access to one owned _build fixture.
/// The retained restore handle never requests either content or list rights.
#[cfg(test)]
pub(super) fn test_metadata_only(path: &Path) -> io::Result<TestMetadataAccess> {
    use windows_sys::{
        Wdk::Storage::FileSystem::NtSetSecurityObject,
        Win32::Security::{DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION},
    };
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
    let fixture_root = repository.join("_build").canonicalize()?;
    let literal =
        crate::path::literal(path).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    let parent = literal.parent().ok_or_else(invalid_name)?.canonicalize()?;
    if !parent.starts_with(&fixture_root) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "ACL qualification is restricted to owned _build fixtures.",
        ));
    }
    counted_name(literal.file_name().ok_or_else(invalid_name)?)?;
    let file = OpenOptions::new()
        .access_mode(READ_CONTROL | WRITE_DAC | FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(literal)?;
    let needed = test_dacl_size(&file)?;
    let (user, user_length) = current_user()?;
    let sid = token_sid(&user, user_length)?;
    // SAFETY: token_sid validated the full SID inside the live user buffer.
    let sid_length = unsafe { GetLengthSid(sid) } as usize;
    let acl_length = size_of::<ACL>()
        .checked_add(size_of::<ACCESS_ALLOWED_ACE>() - size_of::<u32>())
        .and_then(|length| length.checked_add(sid_length))
        .ok_or_else(resource_limit)?;
    let user_bytes = user
        .capacity()
        .checked_mul(size_of::<u64>())
        .ok_or_else(resource_limit)?;
    let mut budget = MAX_SECURITY_BYTES
        .checked_sub(user_bytes + size_of::<SECURITY_DESCRIPTOR>())
        .ok_or_else(resource_limit)?;
    let mut saved = aligned_zeroed(needed, budget)?;
    budget -= saved.capacity() * size_of::<u64>();
    let scratch = aligned_zeroed(needed, budget)?;
    budget -= scratch.capacity() * size_of::<u64>();
    let mut acl = aligned_zeroed(acl_length, budget)?;
    let saved_length = test_read_dacl(&file, &mut saved)?;
    let saved_control = test_dacl_control(&mut saved)?;
    let mut guard = TestMetadataAccess {
        file,
        saved,
        scratch,
        saved_length,
        saved_control,
        active: false,
    };
    let acl_pointer = acl.as_mut_ptr().cast::<ACL>();
    let mut descriptor = SECURITY_DESCRIPTOR::default();
    let descriptor_pointer = (&mut descriptor as *mut SECURITY_DESCRIPTOR).cast();
    let access = DELETE | FILE_READ_ATTRIBUTES | SYNCHRONIZE | READ_CONTROL | WRITE_DAC;
    // SAFETY: current-user SID, aligned ACL and descriptor allocations are live
    // and bounded together with both saved/readback buffers to 64 KiB. The one
    // explicit user ACE omits FILE_READ_DATA (also FILE_LIST_DIRECTORY); the
    // protected DACL excludes broad inherited grants. Ownership is not changed.
    let initialized = unsafe {
        InitializeAcl(acl_pointer, acl_length as u32, ACL_REVISION) != 0
            && AddAccessAllowedAce(acl_pointer, ACL_REVISION, access, sid) != 0
            && InitializeSecurityDescriptor(descriptor_pointer, 1) != 0
            && SetSecurityDescriptorDacl(descriptor_pointer, 1, acl_pointer, 0) != 0
            && SetSecurityDescriptorControl(
                descriptor_pointer,
                SE_DACL_PROTECTED,
                SE_DACL_PROTECTED,
            ) != 0
    };
    if !initialized {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: only this _build fixture's retained WRITE_DAC handle is changed.
    // Descriptor storage outlives the synchronous native assignment; saved
    // restoration state and all allocations exist before the ACL is restricted.
    let status = unsafe {
        NtSetSecurityObject(
            guard.file.as_raw_handle(),
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            descriptor_pointer,
        )
    };
    if status < 0 {
        return Err(status_error(status));
    }
    guard.active = true;
    Ok(guard)
}

#[cfg(test)]
fn test_dacl_size(file: &File) -> io::Result<usize> {
    use windows_sys::Win32::Security::{DACL_SECURITY_INFORMATION, GetKernelObjectSecurity};
    let mut needed = 0;
    // SAFETY: a live READ_CONTROL fixture handle and a zero-sized output probe.
    let result = unsafe {
        GetKernelObjectSecurity(
            file.as_raw_handle(),
            DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            0,
            &mut needed,
        )
    };
    if result != 0 {
        return Err(invalid_security());
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() != Some(ERROR_INSUFFICIENT_BUFFER as i32) {
        return Err(error);
    }
    if needed as usize > MAX_SECURITY_BYTES
        || (needed as usize)
            < size_of::<windows_sys::Win32::Security::SECURITY_DESCRIPTOR_RELATIVE>()
    {
        return Err(resource_limit());
    }
    Ok(needed as usize)
}

#[cfg(test)]
fn test_read_dacl(file: &File, storage: &mut [u64]) -> io::Result<usize> {
    use windows_sys::Win32::Security::{DACL_SECURITY_INFORMATION, GetKernelObjectSecurity};
    let capacity = storage
        .len()
        .checked_mul(size_of::<u64>())
        .ok_or_else(resource_limit)?;
    if capacity > MAX_SECURITY_BYTES {
        return Err(resource_limit());
    }
    storage.fill(0);
    let mut needed = 0;
    // SAFETY: the live handle retains READ_CONTROL; aligned initialized writable
    // storage covers exactly the stated capacity for this self-relative result.
    if unsafe {
        GetKernelObjectSecurity(
            file.as_raw_handle(),
            DACL_SECURITY_INFORMATION,
            storage.as_mut_ptr().cast(),
            capacity as u32,
            &mut needed,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    test_security_bytes(storage, needed as usize)?;
    Ok(needed as usize)
}

#[cfg(test)]
fn test_dacl_control(storage: &mut [u64]) -> io::Result<u16> {
    use windows_sys::Win32::Security::GetSecurityDescriptorControl;
    let mut control = 0;
    let mut revision = 0;
    // SAFETY: only successfully queried, complete native self-relative
    // descriptors reach this helper; writable scalar outputs remain live.
    if unsafe {
        GetSecurityDescriptorControl(storage.as_mut_ptr().cast(), &mut control, &mut revision)
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    if revision != 1 {
        return Err(invalid_security());
    }
    Ok(control)
}

#[cfg(test)]
fn test_security_bytes(storage: &[u64], length: usize) -> io::Result<&[u8]> {
    let capacity = storage
        .len()
        .checked_mul(size_of::<u64>())
        .ok_or_else(resource_limit)?;
    if length > capacity
        || length < size_of::<windows_sys::Win32::Security::SECURITY_DESCRIPTOR_RELATIVE>()
    {
        return Err(invalid_security());
    }
    // SAFETY: the entire storage allocation is initialized, and the checked
    // length is the native descriptor extent inside this retained allocation.
    Ok(unsafe { std::slice::from_raw_parts(storage.as_ptr().cast(), length) })
}
