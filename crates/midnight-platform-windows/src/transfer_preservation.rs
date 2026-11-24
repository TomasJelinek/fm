//! Existing-output metadata. A stage is born private before any captured ACL is applied.
//! Access-control preservation includes owner, group, DACL and mandatory integrity labels.
//! Privileged auditing policy is administered by Windows, not copied by this capability.
use super::*;
use std::{
    mem::{offset_of, size_of},
    ptr,
};
use windows_sys::{
    Wdk::Storage::FileSystem::{FILE_EA_INFORMATION, FileEaInformation, NtQueryInformationFile},
    Win32::{
        Foundation::RtlNtStatusToDosError,
        Security::*,
        System::{
            IO::{DeviceIoControl, IO_STATUS_BLOCK},
            Ioctl::FSCTL_SET_COMPRESSION,
        },
    },
};

const SECURITY: u32 = OWNER_SECURITY_INFORMATION
    | GROUP_SECURITY_INFORMATION
    | DACL_SECURITY_INFORMATION
    | LABEL_SECURITY_INFORMATION;
/// Attributes the replacement takes over exactly, through `FILE_BASIC_INFO`. The user
/// confirmed the replacement of this very file, so read-only, hidden and system are kept,
/// not reasons to refuse it.
const ATTRIBUTES: u32 = FILE_ATTRIBUTE_ARCHIVE
    | FILE_ATTRIBUTE_HIDDEN
    | FILE_ATTRIBUTE_SYSTEM
    | FILE_ATTRIBUTE_READONLY
    | FILE_ATTRIBUTE_NORMAL
    | FILE_ATTRIBUTE_NOT_CONTENT_INDEXED;
/// How the original is stored, which the replacement follows by its own means: it is
/// compressed exactly when the original was (whatever its folder does to new files),
/// encrypted when the original was (created so) or when its folder encrypts new files,
/// and written as ordinary data whatever the original's sparse ranges.
const STORAGE: u32 =
    FILE_ATTRIBUTE_COMPRESSED | FILE_ATTRIBUTE_ENCRYPTED | FILE_ATTRIBUTE_SPARSE_FILE;
/// The original's contents may not be on this computer (offline storage, cloud files).
const REMOTE: u32 =
    FILE_ATTRIBUTE_OFFLINE | FILE_ATTRIBUTE_RECALL_ON_OPEN | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS;
/// The attributes a readback compares with the original's.
const COMPARED: u32 = (ATTRIBUTES & !FILE_ATTRIBUTE_NORMAL) | FILE_ATTRIBUTE_COMPRESSED;

pub(super) struct Snapshot {
    security: Vec<u64>,
    attributes: u32,
    creation: i64,
    accessed: i64,
}

impl Snapshot {
    pub(super) fn capture(
        directory: &Directory,
        name: &OsStr,
        expected: &dyn TransferTarget,
        cancel: &Cancellation,
    ) -> Result<Self, FsError> {
        directory.revalidate(cancel)?;
        validate_name(name).map_err(|e| error(&directory.path, e))?;
        let path = directory.path.join(name);
        if expected.entry().item.path != path || expected.entry().item.kind != EntryKind::File {
            return Err(invalid(
                &path,
                "preservation requires the consented regular file",
            ));
        }
        expected.revalidate(cancel)?;
        let file = native::open_relative(
            &directory.file,
            name,
            FILE_READ_ATTRIBUTES | READ_CONTROL | SYNCHRONIZE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            FILE_OPEN,
            FILE_NON_DIRECTORY_FILE,
        )
        .map_err(|e| error(&path, e))?;
        if identity(&file, &path)? != expected.entry().identity {
            return Err(invalid(
                &path,
                "existing output changed before metadata capture",
            ));
        }
        let basic = basic(&file, &path)?;
        let attributes = basic.FileAttributes;
        // Replacing such a file downloads nothing, but its original would be kept for
        // recovery as a placeholder of contents that may exist nowhere else.
        if attributes & REMOTE != 0 {
            return Err(unsupported(
                &path,
                "Existing output is a cloud or offline file whose contents may not be on this \
                 computer; it was not replaced. Make it available on this device first.",
            ));
        }
        if attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(unsupported(
                &path,
                "Existing output is a link or another special file; it was not replaced.",
            ));
        }
        let other = attributes & !(ATTRIBUTES | STORAGE);
        if other != 0 {
            return Err(unsupported(
                &path,
                &format!(
                    "Existing output has attributes that its replacement could not keep \
                     ({other:#x}); it was not replaced."
                ),
            ));
        }
        require_single_stream(&file, &path)?;
        require_no_extended_attributes(&file, &path)?;
        let snapshot = Self {
            security: security(&file, &path)?,
            attributes: basic.FileAttributes,
            creation: basic.CreationTime,
            accessed: basic.LastAccessTime,
        };
        expected.revalidate(cancel)?;
        Ok(snapshot)
    }

    pub(super) fn initialize(&self, file: &File, path: &Path) -> Result<(), FsError> {
        let mut protected = self.security.clone();
        let descriptor = protected.as_mut_ptr().cast();
        // SAFETY: this owned descriptor copy is writable and initialized by Windows.
        // SetKernelObjectSecurity consumes descriptor control bits; the information
        // mask alone does not prevent an unprotected source from re-enabling inheritance.
        if unsafe { SetSecurityDescriptorControl(descriptor, SE_DACL_PROTECTED, SE_DACL_PROTECTED) }
            == 0
        {
            return Err(error(path, io::Error::last_os_error()));
        }
        let mut present = 0;
        let mut defaulted = 0;
        let mut acl = ptr::null_mut();
        // SAFETY: the native snapshot owns a valid descriptor and borrowed SACL output.
        if unsafe { GetSecurityDescriptorSacl(descriptor, &mut present, &mut acl, &mut defaulted) }
            == 0
        {
            return Err(error(path, io::Error::last_os_error()));
        }
        // Setting a missing label manufactures an explicit NULL SACL. Leave it absent
        // instead; readback still rejects an unexpected inherited mandatory label.
        let information = if present != 0 && !acl.is_null() {
            SECURITY
        } else {
            SECURITY & !LABEL_SECURITY_INFORMATION
        };
        // SAFETY: the owned empty stage has WRITE_DAC/WRITE_OWNER; the initialized
        // descriptor stays live. Protected DACL excludes permissive parent inheritance.
        if unsafe {
            SetKernelObjectSecurity(
                file.as_raw_handle(),
                information | PROTECTED_DACL_SECURITY_INFORMATION,
                descriptor,
            )
        } == 0
        {
            return Err(error(path, io::Error::last_os_error()));
        }
        self.match_compression(file, path)?;
        self.apply_attributes(file, path)?;
        self.verify(file, path)
    }

    /// Whether the replacement takes the original's read-only attribute.
    pub(super) fn read_only(&self) -> bool {
        self.attributes & FILE_ATTRIBUTE_READONLY != 0
    }

    /// Attributes the stage is created with: encryption, which only creation applies.
    pub(super) fn creation_attributes(&self) -> u32 {
        self.attributes & FILE_ATTRIBUTE_ENCRYPTED
    }

    /// Compresses the empty stage when the original was compressed, and leaves it plain
    /// when the original was not, whatever its folder gives new files.
    fn match_compression(&self, file: &File, path: &Path) -> Result<(), FsError> {
        let wanted = self.attributes & FILE_ATTRIBUTE_COMPRESSED != 0;
        if (basic(file, path)?.FileAttributes & FILE_ATTRIBUTE_COMPRESSED != 0) == wanted {
            return Ok(());
        }
        // COMPRESSION_FORMAT_DEFAULT or COMPRESSION_FORMAT_NONE.
        let format: u16 = if wanted { 1 } else { 0 };
        let mut returned = 0;
        // SAFETY: the owned stage has read and write data access; the fixed-size input
        // stays live for this synchronous control call.
        if unsafe {
            DeviceIoControl(
                file.as_raw_handle(),
                FSCTL_SET_COMPRESSION,
                (&format as *const u16).cast(),
                size_of::<u16>() as u32,
                ptr::null_mut(),
                0,
                &mut returned,
                ptr::null_mut(),
            )
        } == 0
        {
            return Err(error(path, io::Error::last_os_error()));
        }
        Ok(())
    }

    pub(super) fn apply_attributes(&self, file: &File, path: &Path) -> Result<(), FsError> {
        // Zero would leave the stage's attributes as they are; NORMAL clears them.
        let attributes = match self.attributes & ATTRIBUTES & !FILE_ATTRIBUTE_NORMAL {
            0 => FILE_ATTRIBUTE_NORMAL,
            kept => kept,
        };
        let information = FILE_BASIC_INFO {
            CreationTime: self.creation,
            LastAccessTime: self.accessed,
            FileAttributes: attributes,
            ..Default::default()
        };
        // SAFETY: the owned stage has WRITE_ATTRIBUTES; this fixed-size structure is live.
        // Zero last-write/change fields retain the new output's actual write timestamps.
        if unsafe {
            SetFileInformationByHandle(
                file.as_raw_handle(),
                FileBasicInfo,
                (&information as *const FILE_BASIC_INFO).cast(),
                size_of::<FILE_BASIC_INFO>() as u32,
            )
        } == 0
        {
            return Err(error(path, io::Error::last_os_error()));
        }
        Ok(())
    }

    pub(super) fn verify(&self, file: &File, path: &Path) -> Result<(), FsError> {
        let current = security(file, path)?;
        let mut control = 0;
        let mut revision = 0;
        // SAFETY: initialized descriptor and writable scalar outputs remain live.
        if unsafe {
            GetSecurityDescriptorControl(
                current.as_ptr().cast_mut().cast(),
                &mut control,
                &mut revision,
            )
        } == 0
        {
            return Err(error(path, io::Error::last_os_error()));
        }
        if control & SE_DACL_PROTECTED == 0 {
            return Err(unsupported(
                path,
                "The output stage DACL is not protected against inheritance.",
            ));
        }
        let names = [
            "owner",
            "group",
            "DACL presence",
            "DACL",
            "label presence",
            "label",
        ];
        for ((actual, expected), name) in security_parts(&current, path)?
            .iter()
            .zip(security_parts(&self.security, path)?)
            .zip(names)
        {
            if actual != &expected {
                return Err(unsupported(
                    path,
                    &format!(
                        "Existing output {name} could not be preserved exactly ({} versus {} bytes).",
                        actual.len(),
                        expected.len()
                    ),
                ));
            }
        }
        let actual = basic(file, path)?.FileAttributes;
        if self.attributes & FILE_ATTRIBUTE_ENCRYPTED != 0 && actual & FILE_ATTRIBUTE_ENCRYPTED == 0
        {
            return Err(unsupported(
                path,
                "Existing output is encrypted, and its replacement could not be encrypted; it \
                 was not replaced.",
            ));
        }
        if actual & COMPARED != self.attributes & COMPARED {
            return Err(unsupported(
                path,
                &format!(
                    "Existing output attributes could not be preserved exactly ({actual:#x} versus {:#x}).",
                    self.attributes
                ),
            ));
        }
        Ok(())
    }
}

fn basic(file: &File, path: &Path) -> Result<FILE_BASIC_INFO, FsError> {
    let mut value = FILE_BASIC_INFO::default();
    // SAFETY: live handle and exact, writable native structure.
    if unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileBasicInfo,
            (&mut value as *mut FILE_BASIC_INFO).cast(),
            size_of::<FILE_BASIC_INFO>() as u32,
        )
    } == 0
    {
        return Err(error(path, io::Error::last_os_error()));
    }
    Ok(value)
}

fn security(file: &File, path: &Path) -> Result<Vec<u64>, FsError> {
    let mut needed = 0;
    // SAFETY: live READ_CONTROL handle; zero-length query writes only needed.
    let probed = unsafe {
        GetKernelObjectSecurity(
            file.as_raw_handle(),
            SECURITY,
            ptr::null_mut(),
            0,
            &mut needed,
        )
    };
    if probed == 0 {
        // The size probe is expected to fail with ERROR_INSUFFICIENT_BUFFER; any other
        // failure (access denied, an unsupported filesystem) is the real cause.
        let failure = io::Error::last_os_error();
        if failure.raw_os_error() != Some(122) {
            return Err(error(path, failure));
        }
    }
    if needed == 0 || needed > 1024 * 1024 {
        return Err(unsupported(
            path,
            "Existing output security cannot be captured within its metadata budget.",
        ));
    }
    let mut storage = vec![0u64; (needed as usize).div_ceil(8)];
    // SAFETY: aligned writable storage covers the queried size. Races that increase the
    // required size fail this call; they never authorize partial security preservation.
    if unsafe {
        GetKernelObjectSecurity(
            file.as_raw_handle(),
            SECURITY,
            storage.as_mut_ptr().cast(),
            needed,
            &mut needed,
        )
    } == 0
    {
        return Err(error(path, io::Error::last_os_error()));
    }
    Ok(storage)
}

fn security_parts(storage: &[u64], path: &Path) -> Result<Vec<Vec<u8>>, FsError> {
    super::descriptor::checked(storage, size_of_val(storage)).map_err(|e| error(path, e))?;
    let descriptor = storage.as_ptr().cast_mut().cast();
    let mut owner = ptr::null_mut();
    let mut group = ptr::null_mut();
    let mut dacl = ptr::null_mut();
    let mut sacl = ptr::null_mut();
    let mut dacl_present = 0;
    let mut sacl_present = 0;
    let mut defaulted = 0;
    // SAFETY: storage came from successful native security queries, and all output
    // pointers remain borrowed only until their bytes are copied below.
    if unsafe {
        GetSecurityDescriptorOwner(descriptor, &mut owner, &mut defaulted) == 0
            || GetSecurityDescriptorGroup(descriptor, &mut group, &mut defaulted) == 0
            || GetSecurityDescriptorDacl(descriptor, &mut dacl_present, &mut dacl, &mut defaulted)
                == 0
            || GetSecurityDescriptorSacl(descriptor, &mut sacl_present, &mut sacl, &mut defaulted)
                == 0
    } {
        return Err(error(path, io::Error::last_os_error()));
    }
    let mut parts = Vec::with_capacity(6);
    for sid in [owner, group] {
        if sid.is_null() {
            parts.push(Vec::new());
        } else {
            // SAFETY: SID is native-validated and borrowed from live descriptor storage.
            let bytes =
                unsafe { std::slice::from_raw_parts(sid.cast::<u8>(), GetLengthSid(sid) as usize) };
            parts.push(bytes.to_vec());
        }
    }
    for (present, acl) in [(dacl_present, dacl), (sacl_present, sacl)] {
        parts.push(present.to_ne_bytes().to_vec());
        parts.push(if acl.is_null() {
            Vec::new()
        } else {
            // SAFETY: ACL is initialized by Windows and within live descriptor storage.
            unsafe {
                std::slice::from_raw_parts(acl.cast::<u8>(), (*acl).AclSize as usize).to_vec()
            }
        });
    }
    Ok(parts)
}

fn require_single_stream(file: &File, path: &Path) -> Result<(), FsError> {
    let mut storage = vec![0u64; 64 * 1024 / 8];
    // SAFETY: live handle and aligned writable bounded native stream buffer.
    if unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileStreamInfo,
            storage.as_mut_ptr().cast(),
            (storage.len() * 8) as u32,
        )
    } == 0
    {
        return Err(unsupported(
            path,
            "Cannot inspect every existing output stream; replacement refused.",
        ));
    }
    // SAFETY: Windows initialized at least one stream header on successful regular-file query.
    let header = unsafe { &*storage.as_ptr().cast::<FILE_STREAM_INFO>() };
    let expected: Vec<_> = "::$DATA"
        .encode_utf16()
        .flat_map(u16::to_ne_bytes)
        .collect();
    let start = offset_of!(FILE_STREAM_INFO, StreamName);
    // SAFETY: byte view covers exactly the initialized, aligned allocation.
    let bytes =
        unsafe { std::slice::from_raw_parts(storage.as_ptr().cast::<u8>(), storage.len() * 8) };
    if header.NextEntryOffset != 0
        || bytes.get(start..start.saturating_add(header.StreamNameLength as usize))
            != Some(expected.as_slice())
    {
        return Err(unsupported(
            path,
            "Existing output has alternate data streams; replacement would lose them and was refused.",
        ));
    }
    Ok(())
}

fn require_no_extended_attributes(file: &File, path: &Path) -> Result<(), FsError> {
    let mut attributes = FILE_EA_INFORMATION::default();
    let mut status = IO_STATUS_BLOCK::default();
    // SAFETY: synchronous query into the exact writable native structure of a live handle.
    let code = unsafe {
        NtQueryInformationFile(
            file.as_raw_handle(),
            &mut status,
            (&mut attributes as *mut FILE_EA_INFORMATION).cast(),
            size_of::<FILE_EA_INFORMATION>() as u32,
            FileEaInformation,
        )
    };
    if code < 0 {
        // SAFETY: pure conversion of the returned native error status.
        return Err(error(
            path,
            io::Error::from_raw_os_error(unsafe { RtlNtStatusToDosError(code) } as i32),
        ));
    }
    if attributes.EaSize != 0 {
        return Err(unsupported(
            path,
            "Existing output has extended attributes that cannot be preserved; replacement refused.",
        ));
    }
    Ok(())
}
