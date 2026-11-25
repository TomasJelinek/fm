//! Make an installation the user owns private again, without administrator rights.
//!
//! Only permission entries change. Entries that let accounts other than the user, SYSTEM,
//! Administrators and TrustedInstaller change an object are removed; every other explicit
//! entry stays, such as the restricted workers' read and execute grants on their libraries.
//!
//! Objects below the folder are changed through handles opened without following links, and
//! only when the handle's final path is the expected one: another account that can still
//! write there could otherwise swap a folder on the way for a junction during the repair and
//! redirect the change to an object outside the installation.
use super::{
    Access, Identity, Local, MODIFY_RIGHTS, SddlAcl, inspect_descriptor, inspect_one, private_sddl,
};
use crate::Error;
use std::{
    ffi::OsString,
    fs::{File, OpenOptions},
    io,
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        fs::{MetadataExt, OpenOptionsExt},
        io::{AsRawHandle, FromRawHandle},
    },
    path::Path,
    ptr,
};
use windows_sys::Win32::{
    Foundation::INVALID_HANDLE_VALUE,
    Security::{Authorization::*, *},
    Storage::FileSystem::{
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_NAME_NORMALIZED, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
        GetFinalPathNameByHandleW, READ_CONTROL, ReOpenFile, VOLUME_NAME_DOS, WRITE_DAC,
    },
    System::SystemServices::ACCESS_ALLOWED_ACE_TYPE,
};

/// A DACL read from an object, kept alive by its security descriptor.
struct Dacl {
    _descriptor: Local,
    owner: PSID,
    acl: *mut ACL,
}

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

fn read(path: &Path) -> Result<Dacl, Error> {
    let name = wide(path);
    let mut owner = ptr::null_mut();
    let mut acl = ptr::null_mut();
    let mut descriptor = ptr::null_mut();
    // SAFETY: terminated path and writable outputs; the descriptor owns owner and ACL.
    let status = unsafe {
        GetNamedSecurityInfoW(
            name.as_ptr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            ptr::null_mut(),
            &mut acl,
            ptr::null_mut(),
            &mut descriptor,
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32).into());
    }
    Ok(Dacl {
        _descriptor: Local(descriptor),
        owner,
        acl,
    })
}

/// The owner and DACL of the object `file` refers to.
fn read_handle(file: &File) -> Result<Dacl, Error> {
    let mut owner = ptr::null_mut();
    let mut acl = ptr::null_mut();
    let mut descriptor = ptr::null_mut();
    // SAFETY: a live handle with READ_CONTROL and writable outputs; the descriptor owns owner
    // and ACL.
    let status = unsafe {
        GetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            ptr::null_mut(),
            &mut acl,
            ptr::null_mut(),
            &mut descriptor,
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32).into());
    }
    Ok(Dacl {
        _descriptor: Local(descriptor),
        owner,
        acl,
    })
}

/// The raw ACEs of a DACL (none for a missing DACL), borrowed from its descriptor.
fn aces(dacl: &Dacl) -> Result<Vec<&[u8]>, Error> {
    let acl = dacl.acl;
    let mut result = Vec::new();
    if acl.is_null() {
        return Ok(result);
    }
    // SAFETY: a valid ACL that `dacl`'s descriptor keeps alive while the slices are borrowed.
    for index in 0..unsafe { (*acl).AceCount } {
        let mut raw = ptr::null_mut();
        if unsafe { GetAce(acl, u32::from(index), &mut raw) } == 0 || raw.is_null() {
            return Err(Error("Invalid permission list".into()));
        }
        let header = unsafe { &*raw.cast::<ACE_HEADER>() };
        result.push(unsafe {
            std::slice::from_raw_parts(raw.cast::<u8>(), usize::from(header.AceSize))
        });
    }
    Ok(result)
}

/// Whether an ACE lets an untrusted account change the object (or, inherited, its children).
fn grants_others_change(ace: &[u8], identity: &Identity) -> bool {
    if ace.len() < size_of::<ACCESS_ALLOWED_ACE>() || u32::from(ace[0]) != ACCESS_ALLOWED_ACE_TYPE {
        return false;
    }
    // SAFETY: the slice is a complete ACCESS_ALLOWED_ACE from a valid ACL.
    let allowed = unsafe { &*ace.as_ptr().cast::<ACCESS_ALLOWED_ACE>() };
    let sid: PSID = ptr::addr_of!(allowed.SidStart).cast_mut().cast();
    // CREATOR OWNER becomes the owner of each new object, which is the user here.
    // SAFETY: the SID lies inside the ACE.
    let creator = unsafe { IsWellKnownSid(sid, WinCreatorOwnerSid) } != 0;
    allowed.Mask & MODIFY_RIGHTS != 0 && !creator && !identity.trusted(sid)
}

/// The explicit (not inherited) ACEs of `acl` that let no other account change the object.
fn kept_explicit(dacl: &Dacl, identity: &Identity) -> Result<Vec<u8>, Error> {
    let mut kept = Vec::new();
    for ace in aces(dacl)? {
        if u32::from(ace[1]) & INHERITED_ACE == 0 && !grants_others_change(ace, identity) {
            kept.extend_from_slice(ace);
        }
    }
    Ok(kept)
}

/// Build an ACL from raw ACEs; an empty list gives an empty ACL (which denies everything
/// not inherited), never a missing one (which would allow everything).
fn build_acl(aces: &[u8]) -> Result<Vec<u64>, Error> {
    let length = size_of::<ACL>() + aces.len();
    let length32 = u32::try_from(length).map_err(|_| Error("Permission list too long".into()))?;
    let mut buffer = vec![0_u64; length.div_ceil(8)];
    let acl = buffer.as_mut_ptr().cast::<ACL>();
    // SAFETY: an 8-byte aligned writable buffer of at least `length` bytes.
    if unsafe { InitializeAcl(acl, length32, ACL_REVISION_DS) } == 0 {
        return Err(io::Error::last_os_error().into());
    }
    if !aces.is_empty()
        // SAFETY: the ACEs are complete entries copied from valid ACLs and fit the buffer.
        && unsafe {
            AddAce(
                acl,
                ACL_REVISION_DS,
                u32::MAX,
                aces.as_ptr().cast(),
                aces.len() as u32,
            )
        } == 0
    {
        return Err(io::Error::last_os_error().into());
    }
    Ok(buffer)
}

fn set_unprotected(path: &Path, acl: *const ACL) -> Result<(), Error> {
    let name = wide(path);
    // SAFETY: terminated path and a valid ACL for the duration of the call.
    let status = unsafe {
        SetNamedSecurityInfoW(
            name.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | UNPROTECTED_DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            acl,
            ptr::null(),
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32).into());
    }
    Ok(())
}

/// Set `folder`'s DACL and reset every object below it to inherit from it, keeping their
/// explicit entries (which [`secure_entry`] then filters).
fn tree_reset(folder: &Path, acl: *const ACL, protection: u32) -> Result<(), Error> {
    let name = wide(folder);
    // SAFETY: terminated path and a valid ACL; no progress callback.
    let status = unsafe {
        TreeResetNamedSecurityInfoW(
            name.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | protection,
            ptr::null_mut(),
            ptr::null_mut(),
            acl,
            ptr::null(),
            1,
            None,
            ProgressInvokeNever,
            ptr::null(),
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32).into());
    }
    Ok(())
}

/// Whether `parent` passes on (by inheritance) an entry that lets other accounts change
/// objects below it.
fn parent_shares_inheritance(parent: &Path, identity: &Identity) -> Result<bool, Error> {
    let dacl = read(parent)?;
    if dacl.acl.is_null() {
        return Ok(true);
    }
    Ok(aces(&dacl)?.into_iter().any(|ace| {
        u32::from(ace[1]) & (OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE) != 0
            && grants_others_change(ace, identity)
    }))
}

/// Only the owner may secure a folder without administrator rights. The owner is the
/// user, or a group the user's token holds enabled: a folder an elevated administrator
/// created is owned by Administrators, which that same elevated user may change.
pub(crate) fn require_owner(folder: &Path) -> Result<(), Error> {
    let identity = Identity::current()?;
    let dacl = read(folder)?;
    // SAFETY: owner SID from the live descriptor and the retained token user SID.
    let user = !dacl.owner.is_null() && unsafe { EqualSid(dacl.owner, identity.user_sid()) } != 0;
    let mut member = 0;
    // SAFETY: the owner SID is live; a null token checks the calling thread's token, and
    // only an enabled group counts (a deny-only Administrators group of a standard token
    // does not).
    let group = !dacl.owner.is_null()
        && unsafe { CheckTokenMembership(ptr::null_mut(), dacl.owner, &mut member) } != 0
        && member != 0;
    if !user && !group {
        return Err(Error(format!(
            "{} belongs to another account, so only an administrator can change who may \
             modify it",
            folder.display()
        )));
    }
    Ok(())
}

/// See [`crate::secure_installation`]: inherit from a private parent, else protect.
pub(crate) fn secure_folder(folder: &Path) -> Result<(), Error> {
    let identity = Identity::current()?;
    let dacl = read(folder)?;
    let parent = folder
        .parent()
        .ok_or_else(|| Error("The installation folder has no parent".into()))?;
    if !parent_shares_inheritance(parent, &identity)? {
        let explicit = build_acl(&kept_explicit(&dacl, &identity)?)?;
        tree_reset(
            folder,
            explicit.as_ptr().cast(),
            UNPROTECTED_DACL_SECURITY_INFORMATION,
        )?;
        if inspect_one(folder, &identity, MODIFY_RIGHTS)? == Access::Private {
            return Ok(());
        }
    }
    let private = SddlAcl::parse(&private_sddl(&identity)?)?;
    tree_reset(folder, private.acl, PROTECTED_DACL_SECURITY_INFORMATION)
}

/// The final path of the object `file` refers to (`\\?\D:\...`), as `fs::canonicalize`
/// spells it.
fn final_path(file: &File) -> Result<OsString, Error> {
    let mut buffer = vec![0_u16; 512];
    loop {
        let capacity = u32::try_from(buffer.len()).map_err(|_| Error("Path too long".into()))?;
        // SAFETY: a live handle and a writable buffer of `capacity` units.
        let length = unsafe {
            GetFinalPathNameByHandleW(
                file.as_raw_handle(),
                buffer.as_mut_ptr(),
                capacity,
                FILE_NAME_NORMALIZED | VOLUME_NAME_DOS,
            )
        };
        if length == 0 {
            return Err(io::Error::last_os_error().into());
        }
        if length < capacity {
            buffer.truncate(length as usize);
            return Ok(OsString::from_wide(&buffer));
        }
        buffer.resize(length as usize + 1, 0);
    }
}

/// Open the object at `path` itself (never a link's target) for reading its permissions, and
/// only when it is no link or other reparse point and its final path is `expected`: no folder
/// on the way was swapped for a junction or link that leads out of the installation.
fn open_entry(path: &Path, expected: &Path) -> Result<File, Error> {
    let file = OpenOptions::new()
        .access_mode(READ_CONTROL)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    if file.metadata()?.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(Error(format!("{} is a link", path.display())));
    }
    if final_path(&file)? != expected.as_os_str() {
        return Err(Error(format!(
            "{} leads out of the installation; a folder on its way was replaced",
            path.display()
        )));
    }
    Ok(file)
}

/// Remove the entries that let other accounts change one object below the secured folder and
/// let it inherit again. Objects that are already private stay unchanged.
///
/// `expected` is the object's final path: the installation's canonical path joined with the
/// object's path inside it. The object is inspected and changed through one handle opened
/// without following links (see [`open_entry`]), never by name again.
pub(crate) fn secure_entry(path: &Path, expected: &Path) -> Result<(), Error> {
    let identity = Identity::current()?;
    let file = open_entry(path, expected)?;
    let dacl = read_handle(&file)?;
    if inspect_descriptor(path, dacl.owner, dacl.acl, &identity, MODIFY_RIGHTS)? == Access::Private
    {
        return Ok(());
    }
    let acl = build_acl(&kept_explicit(&dacl, &identity)?)?;
    // The same object again, now also allowed to change its permissions.
    // SAFETY: a live handle; ReOpenFile returns a new handle to the same object or fails.
    let raw = unsafe {
        ReOpenFile(
            file.as_raw_handle(),
            READ_CONTROL | WRITE_DAC,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
        )
    };
    if raw == INVALID_HANDLE_VALUE || raw.is_null() {
        return Err(io::Error::last_os_error().into());
    }
    // SAFETY: a new handle this function owns from here on.
    let writable = unsafe { File::from_raw_handle(raw) };
    // SAFETY: a live handle with WRITE_DAC and a valid ACL for the duration of the call.
    let status = unsafe {
        SetSecurityInfo(
            writable.as_raw_handle(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | UNPROTECTED_DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            acl.as_ptr().cast(),
            ptr::null(),
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32).into());
    }
    Ok(())
}

/// Give a copied file the explicit entries of its source that let no other account change
/// it (the copy inherits everything else from its new folder).
pub(crate) fn copy_permissions(source: &Path, target: &Path) -> Result<(), Error> {
    let identity = Identity::current()?;
    let dacl = read(source)?;
    let explicit = kept_explicit(&dacl, &identity)?;
    if explicit.is_empty() {
        return Ok(());
    }
    let acl = build_acl(&explicit)?;
    set_unprotected(target, acl.as_ptr().cast())
}

/// The object's DACL in SDDL form (`D:P...` when protected), for tests.
#[cfg(test)]
pub(crate) fn dacl_sddl(path: &Path) -> Result<String, Error> {
    let dacl = read(path)?;
    let mut raw = ptr::null_mut();
    let mut length = 0;
    // SAFETY: a live descriptor; Win32 allocates the terminated string and its length.
    if unsafe {
        ConvertSecurityDescriptorToStringSecurityDescriptorW(
            dacl._descriptor.0,
            1,
            DACL_SECURITY_INFORMATION,
            &mut raw,
            &mut length,
        )
    } == 0
    {
        return Err(io::Error::last_os_error().into());
    }
    let owned = Local(raw.cast());
    // SAFETY: `length` units including the terminator were written at `raw`.
    let units = unsafe { std::slice::from_raw_parts(raw, length.saturating_sub(1) as usize) };
    let text = String::from_utf16_lossy(units);
    drop(owned);
    Ok(text.trim_end_matches('\0').to_owned())
}
