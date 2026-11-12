//! Recoverable Windows replacement intentionally has a namespace gap: capture the
//! guarded original, then publish without replacement. A gap winner is never removed.
use super::*;
use midnight_core::TransferRecovery;
use std::{mem::size_of, os::windows::ffi::OsStrExt, ptr};
use windows_sys::{
    Wdk::{
        Foundation::OBJECT_ATTRIBUTES,
        Storage::FileSystem::{
            FILE_OPEN_REPARSE_POINT, FILE_RENAME_INFORMATION, FILE_SYNCHRONOUS_IO_NONALERT,
            FileRenameInformation, NtCreateFile, NtSetInformationFile,
        },
    },
    Win32::{
        Foundation::{RtlNtStatusToDosError, UNICODE_STRING},
        Security::*,
        System::{
            IO::IO_STATUS_BLOCK,
            Threading::{GetCurrentProcess, OpenProcessToken},
        },
    },
};

impl Directory {
    pub(in crate::transfer) fn source_stage(&self, cancel: &Cancellation) -> Result<Self, FsError> {
        create_named_directory(self, cancel, ".midnight-move-", DELETE)
    }
    pub(in crate::transfer) fn remove_empty_stage(self) -> Result<(), FsError> {
        self.revalidate(&Cancellation::default())?;
        // Exact directory handle; the kernel refuses nonempty directories. Never
        // enumerate/delete children, reopen a path or weaken the private ACL.
        native::remove(&self.file).map_err(|e| error(&self.path, e))
    }
    pub(in crate::transfer) fn capture_handle(
        &self,
        source: &File,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<PathBuf, FsError> {
        self.revalidate(cancel)?;
        validate_name(name).map_err(|e| error(&self.path, e))?;
        let path = self.path.join(name);
        rename_into(source, &self.file, name).map_err(|e| error(&path, e))?;
        Ok(path)
    }
    pub(in crate::transfer) fn check_recovery_parent(
        &self,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        self.revalidate(cancel)?;
        if !supported(self) {
            return Err(unsupported(
                &self.path,
                "Verified Move recovery requires local fixed NTFS",
            ));
        }
        // Native rename needs no DACL-query rights. Acquire them only for the
        // cross-volume recovery path. ReOpenFile cannot reopen directories;
        // compare the new handle to the retained identity before trusting its DACL.
        use std::os::windows::fs::OpenOptionsExt;
        let held = std::fs::OpenOptions::new()
            .access_mode(READ_CONTROL | FILE_READ_ATTRIBUTES)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(&self.path)
            .map_err(|e| error(&self.path, e))?;
        if identity(&held, &self.path)? != self.identity {
            return Err(invalid(
                &self.path,
                "recovery parent changed before DACL check",
            ));
        }
        trusted_recovery_parent(&held).map_err(|failure| unsupported(&self.path,
            &format!("Move needs a protected source parent: {failure}. Choose Copy to retain the source; no Move fallback is performed.")))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::transfer) struct Revision {
    size: u64,
    modified: u64,
    created: u64,
    attributes: u32,
    changed: i64,
}
impl Revision {
    pub(in crate::transfer) fn of(file: &File, path: &Path) -> Result<Self, FsError> {
        let metadata = file.metadata().map_err(|e| error(path, e))?;
        let mut basic = FILE_BASIC_INFO::default();
        // SAFETY: a live file handle and a writable fixed-size native metadata structure.
        if unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                FileBasicInfo,
                (&mut basic as *mut FILE_BASIC_INFO).cast(),
                size_of::<FILE_BASIC_INFO>() as u32,
            )
        } == 0
        {
            return Err(error(path, io::Error::last_os_error()));
        }
        Ok(Self {
            size: metadata.file_size(),
            modified: metadata.last_write_time(),
            created: metadata.creation_time(),
            attributes: metadata.file_attributes(),
            changed: basic.ChangeTime,
        })
    }
}

struct Target {
    directory: Directory,
    name: OsString,
    file: File,
    entry: TransferEntry,
    revision: Revision,
}
impl TransferTarget for Target {
    fn entry(&self) -> &TransferEntry {
        &self.entry
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.directory.revalidate(cancel)?;
        let visible = native::open_relative(
            &self.directory.file,
            &self.name,
            FILE_READ_ATTRIBUTES | SYNCHRONIZE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            FILE_OPEN,
            0,
        )
        .map_err(|e| error(&self.entry.item.path, e))?;
        if identity(&visible, &self.entry.item.path)? != self.entry.identity
            || Revision::of(&self.file, &self.entry.item.path)? != self.revision
        {
            return Err(invalid(
                &self.entry.item.path,
                "consented destination changed",
            ));
        }
        Ok(())
    }
}

pub(super) fn prepare_target(
    directory: &Directory,
    name: &OsStr,
    cancel: &Cancellation,
) -> Result<Option<Arc<dyn TransferTarget>>, FsError> {
    directory.revalidate(cancel)?;
    validate_name(name).map_err(|e| error(&directory.path, e))?;
    let path = directory.path.join(name);
    let file = match native::open_relative(
        &directory.file,
        name,
        FILE_READ_ATTRIBUTES | SYNCHRONIZE,
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
        FILE_OPEN,
        0,
    ) {
        Ok(file) => file,
        Err(failure) if failure.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(failure) => return Err(error(&path, failure)),
    };
    let (entry, revision) = target_snapshot(&file, &path, || {})?;
    Ok(Some(Arc::new(Target {
        entry,
        revision,
        file,
        name: name.to_owned(),
        directory: directory.clone(),
    })))
}

fn target_snapshot(
    file: &File,
    path: &Path,
    after_display_read: impl FnOnce(),
) -> Result<(TransferEntry, Revision), FsError> {
    let before = Revision::of(file, path)?;
    let display = entry(file, path)?;
    after_display_read();
    let revision = Revision::of(file, path)?;
    if before != revision {
        return Err(invalid(
            path,
            "destination changed while capturing conflict metadata",
        ));
    }
    Ok((display, revision))
}

pub(super) fn open_merge(
    directory: &Directory,
    name: &OsStr,
    expected: &TransferEntry,
    cancel: &Cancellation,
) -> Result<Arc<dyn TransferDirectory>, FsError> {
    directory.revalidate(cancel)?;
    validate_name(name).map_err(|e| error(&directory.path, e))?;
    let path = directory.path.join(name);
    if expected.item.kind != EntryKind::Directory || expected.item.path != path {
        return Err(invalid(
            &path,
            "merging requires the consented ordinary directory",
        ));
    }
    let file = native::open_relative(
        &directory.file,
        name,
        FILE_READ_ATTRIBUTES | FILE_TRAVERSE | SYNCHRONIZE,
        FILE_SHARE_READ | FILE_SHARE_WRITE,
        FILE_OPEN,
        FILE_DIRECTORY_FILE,
    )
    .map_err(|e| error(&path, e))?;
    if identity(&file, &path)? != expected.identity
        || entry(&file, &path)?.item.kind != EntryKind::Directory
    {
        return Err(invalid(&path, "merge destination identity or type changed"));
    }
    Ok(Arc::new(Directory::from_file(file, path)?))
}

pub(in crate::transfer) fn supported(directory: &Directory) -> bool {
    let Ok(path) = final_path(&directory.file) else {
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
    // SAFETY: the root is a terminated native handle-derived path; the output array
    // is writable. Unknown/remote/removable filesystems do not get recovery capability.
    unsafe {
        // DRIVE_FIXED is the documented GetDriveType value 3. Keeping this local
        // avoids enabling an unrelated WindowsProgramming feature for one constant.
        GetDriveTypeW(root.as_ptr()) == 3
            && GetVolumeInformationByHandleW(
                directory.file.as_raw_handle(),
                ptr::null_mut(),
                0,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                filesystem.as_mut_ptr(),
                filesystem.len() as u32,
            ) != 0
            && filesystem.starts_with(&[b'N' as u16, b'T' as u16, b'F' as u16, b'S' as u16, 0])
    }
}

pub(super) struct Replacement {
    name: OsString,
    expected: Arc<dyn TransferTarget>,
}

impl Stage {
    pub(super) fn bind_replacement(
        &mut self,
        name: &OsStr,
        expected: Arc<dyn TransferTarget>,
        policy: TransferReplacementPolicy,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        if policy != TransferReplacementPolicy::RecoverableCurrentEntry
            || !supported(&self.directory)
        {
            return Err(unsupported(
                &self.directory.path,
                "recoverable replacement requires local fixed NTFS; identity-conditional replacement is unsupported",
            ));
        }
        if self.published || self.cleaned {
            return Err(invalid(&self.path, "stage is no longer available"));
        }
        validate_name(name).map_err(|e| error(&self.path, e))?;
        let path = self.directory.path.join(name);
        if expected.entry().item.path != path || expected.entry().item.kind != EntryKind::File {
            return Err(unsupported(
                &path,
                "replacement requires a consented ordinary file",
            ));
        }
        self.directory.revalidate(cancel)?;
        expected.revalidate(cancel)?;
        self.replacement = Some(Replacement {
            name: name.to_owned(),
            expected,
        });
        Ok(())
    }

    pub(super) fn replace(
        &mut self,
        name: &OsStr,
        cancel: &Cancellation,
        after_capture: impl FnOnce(),
    ) -> Result<TransferPublication, FsError> {
        if self.published || self.cleaned {
            return Err(invalid(&self.path, "stage is no longer available"));
        }
        let replacement = self
            .replacement
            .as_ref()
            .ok_or_else(|| invalid(&self.path, "stage has no replacement consent"))?;
        if replacement.name != name {
            return Err(invalid(
                &self.path,
                "replacement basename differs from consent",
            ));
        }
        if !supported(&self.directory) {
            return Err(unsupported(
                &self.path,
                "recovery publication requires local fixed NTFS",
            ));
        }
        self.directory.revalidate(cancel)?;
        replacement.expected.revalidate(cancel)?;
        let path = self.directory.path.join(name);
        // The conflict pin permits updates while a decision is pending. This separate
        // commit guard now excludes writers/deleters; incompatible opens fail safely.
        let original = native::open_relative(
            &self.directory.file,
            name,
            FILE_READ_ATTRIBUTES | DELETE | SYNCHRONIZE,
            FILE_SHARE_READ,
            FILE_OPEN,
            FILE_NON_DIRECTORY_FILE,
        )
        .map_err(|e| error(&path, e))?;
        let original_identity = identity(&original, &path)?;
        if original_identity != replacement.expected.entry().identity
            || entry(&original, &path)?.item.kind != EntryKind::File
        {
            return Err(invalid(
                &path,
                "replacement destination identity or type changed",
            ));
        }
        replacement.expected.revalidate(cancel)?;
        let recovery = create_recovery(&self.directory, cancel)?;
        let staged_path = recovery.path.join("replacement");
        let staged_identity = identity(&self.file, &self.path)?;
        rename_into(&self.file, &recovery.file, OsStr::new("replacement"))
            .map_err(|e| error(&self.path, e))?;
        // From this point, even the verified replacement is persistent recovery data.
        // Drop must close its handle without deleting it if any later step fails.
        self.published = true;
        self.path = staged_path.clone();
        let staged_receipt = TransferRecovery {
            path: staged_path,
            identity: Some(staged_identity),
        };
        let original_path = recovery.path.join("original");
        if let Err(failure) = rename_into(&original, &recovery.file, OsStr::new("original")) {
            return Ok(TransferPublication::RecoveryRequired {
                published: None,
                recovery: vec![staged_receipt],
                error: error(&path, failure),
            });
        }
        let original_receipt = TransferRecovery {
            path: original_path,
            identity: Some(original_identity),
        };
        after_capture();
        // A new final entry created during the explicit gap wins; never remove it.
        let commit = self.directory.revalidate(cancel).and_then(|()| {
            rename_into(&self.file, &self.directory.file, name).map_err(|e| error(&path, e))
        });
        if let Err(error) = commit {
            return Ok(TransferPublication::RecoveryRequired {
                published: None,
                recovery: vec![original_receipt, staged_receipt],
                error,
            });
        }
        self.path = path.clone();
        self.published_name = Some(name.to_owned());
        Ok(TransferPublication::Replaced {
            path,
            recovery: original_receipt,
        })
    }
}

pub(in crate::transfer) fn rename_into(
    source: &File,
    parent: &File,
    name: &OsStr,
) -> io::Result<()> {
    let wide: Vec<_> = name.encode_wide().collect();
    let length = size_of::<FILE_RENAME_INFORMATION>() + wide.len() * 2;
    let mut buffer = vec![0u64; length.div_ceil(size_of::<u64>())];
    let information = buffer.as_mut_ptr().cast::<FILE_RENAME_INFORMATION>();
    let mut status = IO_STATUS_BLOCK::default();
    // SAFETY: aligned counted-name buffer and live source/parent handles. ReplaceIfExists
    // stays false; RootDirectory anchors both capture and publication without path traversal.
    let code = unsafe {
        (*information).RootDirectory = parent.as_raw_handle();
        (*information).FileNameLength = (wide.len() * 2) as u32;
        ptr::copy_nonoverlapping(
            wide.as_ptr(),
            (*information).FileName.as_mut_ptr(),
            wide.len(),
        );
        NtSetInformationFile(
            source.as_raw_handle(),
            &mut status,
            information.cast(),
            length as u32,
            FileRenameInformation,
        )
    };
    if code < 0 {
        // SAFETY: pure translation of a native error returned above.
        return Err(io::Error::from_raw_os_error(
            unsafe { RtlNtStatusToDosError(code) } as i32,
        ));
    }
    Ok(())
}

fn create_recovery(directory: &Directory, cancel: &Cancellation) -> Result<Directory, FsError> {
    create_named_directory(directory, cancel, ".midnight-recovery-", 0)
}
pub(in crate::transfer) fn native_move_recovery(
    directory: &Directory,
    cancel: &Cancellation,
) -> Result<Directory, FsError> {
    create_named_directory(directory, cancel, ".midnight-recovery-", DELETE)
}
fn create_named_directory(
    directory: &Directory,
    cancel: &Cancellation,
    prefix: &str,
    additional_access: u32,
) -> Result<Directory, FsError> {
    for _ in 0..16 {
        checked(cancel, &directory.path)?;
        let mut random = [0u8; 16];
        getrandom::fill(&mut random)
            .map_err(|e| error(&directory.path, io::Error::other(e.to_string())))?;
        let suffix: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        let name = OsString::from(format!("{prefix}{suffix}"));
        match create_private_directory(&directory.file, &name, additional_access) {
            Ok(file) => return Directory::from_file(file, directory.path.join(name)),
            Err(failure) if failure.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(failure) => return Err(error(&directory.path, failure)),
        }
    }
    Err(invalid(
        &directory.path,
        "exclusive recovery directory names collided",
    ))
}

fn current_user() -> io::Result<Vec<u64>> {
    use std::os::windows::io::{FromRawHandle, OwnedHandle};
    let mut token = ptr::null_mut();
    // SAFETY: current process pseudo-handle is valid and token is writable.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful OpenProcessToken transfers a unique handle owned by this guard.
    let token = unsafe { OwnedHandle::from_raw_handle(token) };
    let mut needed = 0;
    // SAFETY: zero-size probe only writes needed; token handle remains live.
    unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            ptr::null_mut(),
            0,
            &mut needed,
        );
    }
    let mut user = vec![0u64; (needed as usize).div_ceil(8)];
    // SAFETY: aligned writable allocation has the requested native length.
    if unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            user.as_mut_ptr().cast(),
            needed,
            &mut needed,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    if (needed as usize) < size_of::<TOKEN_USER>() {
        return Err(io::Error::other("invalid token user"));
    }
    Ok(user)
}

fn trusted_recovery_parent(file: &File) -> io::Result<()> {
    let user = current_user()?;
    // SAFETY: current_user initialized the aligned TOKEN_USER and its contained SID.
    let user_sid = unsafe { (*user.as_ptr().cast::<TOKEN_USER>()).User.Sid };
    let information = OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION;
    let mut needed = 0;
    // SAFETY: a live READ_CONTROL handle and a zero-size output probe.
    unsafe {
        GetKernelObjectSecurity(
            file.as_raw_handle(),
            information,
            ptr::null_mut(),
            0,
            &mut needed,
        );
    }
    if needed == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut storage = vec![0u64; (needed as usize).div_ceil(8)];
    let descriptor = storage.as_mut_ptr().cast();
    // SAFETY: the writable aligned allocation covers the probed native length.
    if unsafe {
        GetKernelObjectSecurity(
            file.as_raw_handle(),
            information,
            descriptor,
            needed,
            &mut needed,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let trusted = |sid: PSID| {
        // SAFETY: SID pointers borrow the live descriptor/token buffers. Validate
        // before comparing; administrators/System are outside this isolation boundary.
        !sid.is_null()
            && unsafe {
                IsValidSid(sid) != 0
                    && (EqualSid(sid, user_sid) != 0
                        || IsWellKnownSid(sid, WinLocalSystemSid) != 0
                        || IsWellKnownSid(sid, WinBuiltinAdministratorsSid) != 0)
            }
    };
    let mut owner = ptr::null_mut();
    let mut defaulted = 0;
    let mut present = 0;
    let mut acl = ptr::null_mut();
    // SAFETY: successful security read initialized this descriptor and its borrowed outputs.
    let valid = unsafe {
        GetSecurityDescriptorOwner(descriptor, &mut owner, &mut defaulted) != 0
            && GetSecurityDescriptorDacl(descriptor, &mut present, &mut acl, &mut defaulted) != 0
    };
    if !valid || !trusted(owner) || present == 0 || acl.is_null() {
        return Err(io::Error::other(
            "untrusted owner or unrestricted source-parent DACL",
        ));
    }
    let mutable = DELETE
        | FILE_DELETE_CHILD
        | WRITE_DAC
        | WRITE_OWNER
        | windows_sys::Win32::Foundation::GENERIC_ALL;
    // SAFETY: the native descriptor contains a valid ACL while storage remains live.
    for index in 0..unsafe { (*acl).AceCount } {
        let mut raw = ptr::null_mut();
        // SAFETY: index is bounded by the initialized ACL's native entry count.
        if unsafe { GetAce(acl, u32::from(index), &mut raw) } == 0 || raw.is_null() {
            return Err(io::Error::other("invalid source-parent ACL"));
        }
        // SAFETY: GetAce returned a native ACE within the live descriptor.
        let header = unsafe { &*raw.cast::<ACE_HEADER>() };
        if u32::from(header.AceFlags) & INHERIT_ONLY_ACE != 0 {
            continue;
        }
        match header.AceType {
            1 => {} // ACCESS_DENIED_ACE_TYPE cannot grant a mutation right.
            0 => {
                // ACCESS_ALLOWED_ACE_TYPE
                if usize::from(header.AceSize) < size_of::<ACCESS_ALLOWED_ACE>() {
                    return Err(io::Error::other("short source-parent access ACE"));
                }
                // SAFETY: the checked native access-allowed ACE contains its mask and SID.
                let ace = unsafe { &*raw.cast::<ACCESS_ALLOWED_ACE>() };
                let sid = ptr::addr_of!(ace.SidStart).cast_mut().cast();
                if ace.Mask & mutable != 0 && !trusted(sid) {
                    return Err(io::Error::other(
                        "another identity can replace recovery entries or their parent",
                    ));
                }
            }
            _ => {
                return Err(io::Error::other(
                    "conditional/object source-parent ACL is unsupported",
                ));
            }
        }
    }
    Ok(())
}

fn create_private_directory(
    parent: &File,
    name: &OsStr,
    additional_access: u32,
) -> io::Result<File> {
    create_private_object(parent, name, additional_access, true)
}

pub(super) fn create_private_file(parent: &File, name: &OsStr) -> io::Result<File> {
    create_private_object(
        parent,
        name,
        FILE_READ_DATA
            | FILE_WRITE_DATA
            | FILE_WRITE_ATTRIBUTES
            | DELETE
            | READ_CONTROL
            | WRITE_DAC
            | WRITE_OWNER,
        false,
    )
}

fn create_private_object(
    parent: &File,
    name: &OsStr,
    additional_access: u32,
    directory: bool,
) -> io::Result<File> {
    use std::os::windows::io::FromRawHandle;
    let user = current_user()?;
    // SAFETY: successful native query initialized TOKEN_USER in the aligned buffer.
    let sid = unsafe { (*user.as_ptr().cast::<TOKEN_USER>()).User.Sid };
    // SAFETY: sid is owned by the live user buffer returned by GetTokenInformation.
    let acl_length = size_of::<ACL>() + size_of::<ACCESS_ALLOWED_ACE>() - size_of::<u32>()
        + unsafe { GetLengthSid(sid) } as usize;
    let mut acl = vec![0u64; acl_length.div_ceil(8)];
    let acl_pointer = acl.as_mut_ptr().cast::<ACL>();
    let mut descriptor = SECURITY_DESCRIPTOR::default();
    let descriptor_pointer = (&mut descriptor as *mut SECURITY_DESCRIPTOR).cast();
    // SAFETY: descriptor/ACL allocations and token SID remain live through NtCreateFile.
    // A protected DACL grants only this token user access; inherited broad ACEs are excluded.
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
    let mut wide: Vec<_> = name.encode_wide().collect();
    let bytes =
        u16::try_from(wide.len() * 2).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    let mut unicode = UNICODE_STRING {
        Length: bytes,
        MaximumLength: bytes,
        Buffer: wide.as_mut_ptr(),
    };
    let attributes = OBJECT_ATTRIBUTES {
        Length: size_of::<OBJECT_ATTRIBUTES>() as u32,
        RootDirectory: parent.as_raw_handle(),
        ObjectName: &mut unicode,
        SecurityDescriptor: &descriptor,
        ..Default::default()
    };
    let mut handle = ptr::null_mut();
    let mut status = IO_STATUS_BLOCK::default();
    // SAFETY: relative counted name and private security descriptor outlive this synchronous
    // exclusive native creation; only success transfers ownership of handle to File.
    let code = unsafe {
        NtCreateFile(
            &mut handle,
            FILE_READ_ATTRIBUTES
                | SYNCHRONIZE
                | additional_access
                | if directory { FILE_TRAVERSE } else { 0 },
            &attributes,
            &mut status,
            ptr::null(),
            if directory {
                FILE_ATTRIBUTE_DIRECTORY
            } else {
                FILE_ATTRIBUTE_NORMAL
            },
            FILE_SHARE_READ | if directory { FILE_SHARE_WRITE } else { 0 },
            FILE_CREATE,
            (if directory {
                FILE_DIRECTORY_FILE
            } else {
                FILE_NON_DIRECTORY_FILE
            }) | FILE_OPEN_REPARSE_POINT
                | FILE_SYNCHRONOUS_IO_NONALERT,
            ptr::null(),
            0,
        )
    };
    if code < 0 {
        // SAFETY: pure native status conversion; no successful handle to release.
        return Err(io::Error::from_raw_os_error(
            unsafe { RtlNtStatusToDosError(code) } as i32,
        ));
    }
    // SAFETY: successful exclusive creation transferred a unique live kernel handle.
    Ok(unsafe { File::from_raw_handle(handle) })
}

#[cfg(test)]
#[path = "transfer_recovery_tests.rs"]
mod tests;
