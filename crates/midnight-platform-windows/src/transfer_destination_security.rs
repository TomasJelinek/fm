//! The security of a stage in a job's private staging folder (WIN-8): what a file created
//! directly in the destination folder gets from that folder. The staging folder's owner-only
//! DACL is not inheritable, so NTFS would give a file created in it the token's default DACL,
//! which the file would keep through its publication (a rename). Each stage's descriptor is
//! computed anew from the destination folder's current one, so an ACL changed during the job
//! applies to the files staged after the change.
use super::*;
use std::{
    os::windows::{
        ffi::OsStrExt,
        io::{FromRawHandle, OwnedHandle},
    },
    ptr,
};
use windows_sys::{
    Wdk::{
        Foundation::OBJECT_ATTRIBUTES,
        Storage::FileSystem::{
            FILE_OPEN_REPARSE_POINT, FILE_SYNCHRONOUS_IO_NONALERT, NtCreateFile,
        },
    },
    Win32::{
        Foundation::{ERROR_INSUFFICIENT_BUFFER, RtlNtStatusToDosError, UNICODE_STRING},
        Security::{
            CreatePrivateObjectSecurityEx, DACL_SECURITY_INFORMATION, DestroyPrivateObjectSecurity,
            GENERIC_MAPPING, GROUP_SECURITY_INFORMATION, GetKernelObjectSecurity,
            GetSecurityDescriptorControl, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
            SE_DACL_AUTO_INHERIT_REQ, SE_DACL_AUTO_INHERITED, SEF_DACL_AUTO_INHERIT,
            SetKernelObjectSecurity, SetSecurityDescriptorControl, TOKEN_QUERY,
        },
        System::{
            IO::IO_STATUS_BLOCK,
            Threading::{GetCurrentProcess, OpenProcessToken},
        },
    },
};

/// The generic rights of files, as the I/O manager maps them for NTFS.
const FILE_MAPPING: GENERIC_MAPPING = GENERIC_MAPPING {
    GenericRead: FILE_GENERIC_READ,
    GenericWrite: FILE_GENERIC_WRITE,
    GenericExecute: FILE_GENERIC_EXECUTE,
    GenericAll: FILE_ALL_ACCESS,
};

#[cfg(test)]
thread_local! {
    /// Descriptors allocated on this thread and not yet freed, for the guard's tests.
    pub(super) static LIVE: std::cell::Cell<isize> = const { std::cell::Cell::new(0) };
}

/// The process token (TOKEN_QUERY), whose owner, primary group and default DACL NTFS gives
/// a new file and `FileDescriptor` substitutes for CREATOR OWNER and CREATOR GROUP.
pub(super) fn process_token() -> io::Result<OwnedHandle> {
    let mut token = ptr::null_mut();
    // SAFETY: the current-process pseudo handle is always valid and `token` is writable.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the successful call returned a new handle; this owner closes it once.
    Ok(unsafe { OwnedHandle::from_raw_handle(token) })
}

/// A file descriptor that `CreatePrivateObjectSecurityEx` allocated. It exists only after
/// that call succeeded, and Drop frees it with `DestroyPrivateObjectSecurity` on every path.
pub(super) struct FileDescriptor {
    descriptor: PSECURITY_DESCRIPTOR,
    /// The destination folder's DACL is auto-inherited (`SE_DACL_AUTO_INHERITED`).
    auto_inherited: bool,
}

impl FileDescriptor {
    /// What NTFS gives a file created now in the folder held by `folder` (READ_CONTROL):
    /// its current DACL's inheritable entries, CREATOR OWNER and CREATOR GROUP replaced by
    /// the token's owner and primary group, which own the file, or the token's default DACL
    /// when nothing is inheritable. As NTFS does, the entries are marked inherited
    /// (`INHERITED_ACE`, `SE_DACL_AUTO_INHERITED`) only when the folder's own DACL is
    /// auto-inherited. No SACL: the kernel gives the stage its SACL and mandatory label.
    pub(super) fn inherited_from(folder: &File, token: &OwnedHandle) -> io::Result<Self> {
        let parent = descriptor_of(
            folder,
            OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
        )?;
        let parent_descriptor: PSECURITY_DESCRIPTOR = parent.as_ptr().cast_mut().cast();
        let (mut control, mut revision) = (0u16, 0u32);
        // SAFETY: a self-relative descriptor the system returned, live in `parent`.
        if unsafe { GetSecurityDescriptorControl(parent_descriptor, &mut control, &mut revision) }
            == 0
        {
            return Err(io::Error::last_os_error());
        }
        let auto_inherited = control & SE_DACL_AUTO_INHERITED != 0;
        let mut created = ptr::null_mut();
        // SAFETY: the parent descriptor stays live in `parent`; there is no creator
        // descriptor or object type, and the live token handle has TOKEN_QUERY. Only a
        // successful call writes `created`, which the returned guard alone frees.
        let succeeded = unsafe {
            CreatePrivateObjectSecurityEx(
                parent_descriptor,
                ptr::null_mut(),
                &mut created,
                ptr::null(),
                0,
                if auto_inherited {
                    SEF_DACL_AUTO_INHERIT
                } else {
                    0
                },
                token.as_raw_handle(),
                &FILE_MAPPING,
            )
        };
        if succeeded == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self::owning(created, auto_inherited))
    }

    /// Take ownership of a descriptor that `CreatePrivateObjectSecurityEx` allocated.
    fn owning(descriptor: PSECURITY_DESCRIPTOR, auto_inherited: bool) -> Self {
        #[cfg(test)]
        LIVE.with(|live| live.set(live.get() + 1));
        Self {
            descriptor,
            auto_inherited,
        }
    }

    /// The access a stage created with this descriptor needs besides its own: `complete`
    /// sets the DACL of an auto-inherited one again.
    pub(super) fn access(&self) -> u32 {
        if self.auto_inherited { WRITE_DAC } else { 0 }
    }

    /// Create the file `name` in `parent` exclusively with this descriptor, as
    /// `native::open_relative` creates a stage.
    pub(super) fn create(
        &self,
        parent: &File,
        name: &OsStr,
        access: u32,
        share: u32,
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
            Length: std::mem::size_of::<OBJECT_ATTRIBUTES>() as u32,
            RootDirectory: parent.as_raw_handle(),
            ObjectName: &mut unicode,
            SecurityDescriptor: self.descriptor.cast_const().cast(),
            ..Default::default()
        };
        let mut handle = ptr::null_mut();
        let mut status = IO_STATUS_BLOCK::default();
        // SAFETY: the parent handle is live; the counted name, the descriptor this guard
        // owns, the attributes and the outputs outlive this synchronous exclusive creation.
        // Only success transfers ownership of `handle`.
        let code = unsafe {
            NtCreateFile(
                &mut handle,
                access | SYNCHRONIZE,
                &attributes,
                &mut status,
                ptr::null(),
                0,
                share,
                FILE_CREATE,
                FILE_NON_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT,
                ptr::null(),
                0,
            )
        };
        if code < 0 {
            // SAFETY: pure conversion of the status the failed call returned.
            return Err(io::Error::from_raw_os_error(
                unsafe { RtlNtStatusToDosError(code) } as i32,
            ));
        }
        // SAFETY: the successful exclusive creation returned a unique live handle.
        Ok(unsafe { File::from_raw_handle(handle) })
    }

    /// NTFS keeps a creation descriptor's entries and their `INHERITED_ACE` flags as given,
    /// but marks a new file's DACL auto-inherited only by inheriting it from the folder it
    /// is created in, and the staging folder gives none. Set the DACL of `file`, created
    /// with this descriptor and WRITE_DAC (`access`), again asking for automatic inheritance
    /// (`SE_DACL_AUTO_INHERIT_REQ`), as a file created in the destination has it.
    pub(super) fn complete(&mut self, file: &File) -> io::Result<()> {
        if !self.auto_inherited {
            return Ok(());
        }
        // SAFETY: the private self-relative descriptor this guard owns; the request bit is
        // one that callers may set.
        if unsafe {
            SetSecurityDescriptorControl(
                self.descriptor,
                SE_DACL_AUTO_INHERIT_REQ,
                SE_DACL_AUTO_INHERIT_REQ,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: a live handle opened with WRITE_DAC and the valid descriptor.
        if unsafe {
            SetKernelObjectSecurity(
                file.as_raw_handle(),
                DACL_SECURITY_INFORMATION,
                self.descriptor,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

impl Drop for FileDescriptor {
    fn drop(&mut self) {
        // SAFETY: `descriptor` came from a successful CreatePrivateObjectSecurityEx and is
        // freed exactly once, here.
        unsafe { DestroyPrivateObjectSecurity(&self.descriptor) };
        #[cfg(test)]
        LIVE.with(|live| live.set(live.get() - 1));
    }
}

/// The descriptor parts `information` of the object that `file` holds with READ_CONTROL,
/// self-relative in an aligned allocation. Most folders' descriptors fit the first buffer.
fn descriptor_of(file: &File, information: u32) -> io::Result<Vec<u64>> {
    let mut storage = vec![0u64; 64];
    loop {
        let length = u32::try_from(storage.len() * std::mem::size_of::<u64>())
            .map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
        let mut needed = 0;
        // SAFETY: a live handle; the aligned writable allocation covers `length` bytes.
        if unsafe {
            GetKernelObjectSecurity(
                file.as_raw_handle(),
                information,
                storage.as_mut_ptr().cast(),
                length,
                &mut needed,
            )
        } != 0
        {
            return Ok(storage);
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(ERROR_INSUFFICIENT_BUFFER as i32) || needed <= length {
            return Err(error);
        }
        storage = vec![0u64; (needed as usize).div_ceil(std::mem::size_of::<u64>())];
    }
}
