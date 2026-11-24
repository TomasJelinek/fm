//! Read and execute for the workers' AppContainer on one installed file, added to its DACL.
//!
//! A release ZIP carries no NTFS permissions: extracted files inherit only their folder's
//! DACL, and a folder in the user's profile grants nothing to AppContainers. The workers'
//! loader is then denied the libraries beside them, which the build and deployment scripts
//! grant with `Set-Acl`. This adds the same entry at run time.
use super::{check, worker_profile_sid};
use std::{
    io,
    os::windows::ffi::OsStrExt,
    path::Path,
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::LocalFree,
    Security::{
        ACL,
        Authorization::{
            EXPLICIT_ACCESS_W, GRANT_ACCESS, GetNamedSecurityInfoW, NO_MULTIPLE_TRUSTEE,
            SE_FILE_OBJECT, SetEntriesInAclW, SetNamedSecurityInfoW, TRUSTEE_IS_SID,
            TRUSTEE_IS_UNKNOWN, TRUSTEE_W,
        },
        DACL_SECURITY_INFORMATION, GetSecurityDescriptorControl, NO_INHERITANCE,
        PROTECTED_DACL_SECURITY_INFORMATION, SE_DACL_PROTECTED,
        UNPROTECTED_DACL_SECURITY_INFORMATION,
    },
    Storage::FileSystem::{FILE_GENERIC_EXECUTE, FILE_GENERIC_READ},
};

/// Read and execute, as `icacls ... (RX)` and .NET's `ReadAndExecute` grant it (0x1200A9).
const READ_AND_EXECUTE: u32 = FILE_GENERIC_READ | FILE_GENERIC_EXECUTE;

/// Memory that Windows allocated for this caller with `LocalAlloc`.
struct Local(*mut core::ffi::c_void);

impl Drop for Local {
    fn drop(&mut self) {
        // SAFETY: memory that a successful Windows call allocated for this caller.
        unsafe { LocalFree(self.0) };
    }
}

/// Add one explicit entry to `path`'s DACL that allows [`READ_AND_EXECUTE`] to the workers'
/// AppContainer, without inheritance flags. `SetEntriesInAclW` merges it into the entries the
/// file has, keeping them and their canonical order (an explicit allow entry the SID already
/// has is widened instead of repeated); inherited entries are derived from the folder again,
/// and the DACL's protection stays as it was. A file without a DACL (which allows everyone
/// everything) is refused rather than given a DACL with this entry alone. Needs `WRITE_DAC`
/// on the file: the owner, or full control.
pub(crate) fn grant_load(path: &Path) -> io::Result<()> {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    if wide[..wide.len() - 1].contains(&0) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "NUL in path"));
    }
    let sid = worker_profile_sid()?;
    let mut acl: *mut ACL = null_mut();
    let mut descriptor = null_mut();
    // SAFETY: a terminated path; Windows allocates the descriptor, which owns `acl`.
    let status = unsafe {
        GetNamedSecurityInfoW(
            wide.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            &mut acl,
            null_mut(),
            &mut descriptor,
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    let descriptor = Local(descriptor);
    if acl.is_null() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "The file has no permission list",
        ));
    }
    let (mut control, mut revision) = (0u16, 0u32);
    // SAFETY: a live descriptor and two writable outputs.
    check(unsafe { GetSecurityDescriptorControl(descriptor.0, &mut control, &mut revision) })?;
    let access = EXPLICIT_ACCESS_W {
        grfAccessPermissions: READ_AND_EXECUTE,
        grfAccessMode: GRANT_ACCESS,
        grfInheritance: NO_INHERITANCE,
        Trustee: TRUSTEE_W {
            pMultipleTrustee: null_mut(),
            MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_UNKNOWN,
            ptstrName: sid.0.cast(),
        },
    };
    let mut updated = null_mut();
    // SAFETY: one entry naming a live SID, and the file's ACL that `descriptor` keeps alive;
    // Windows allocates the merged ACL.
    let status = unsafe { SetEntriesInAclW(1, &access, acl, &mut updated) };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    let updated = Local(updated.cast());
    let protection = if control & SE_DACL_PROTECTED != 0 {
        PROTECTED_DACL_SECURITY_INFORMATION
    } else {
        UNPROTECTED_DACL_SECURITY_INFORMATION
    };
    // SAFETY: a terminated path and the merged ACL, live for the call; no owner or group.
    let status = unsafe {
        SetNamedSecurityInfoW(
            wide.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | protection,
            null_mut(),
            null_mut(),
            updated.0.cast(),
            null(),
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    Ok(())
}

#[cfg(test)]
#[path = "windows_load_grant_tests.rs"]
mod tests;
