//! The helper runs as an administrator, so no other account may be able to change it.
//!
//! Without the shipped helper's digest only administrative identities may change the helper
//! and its folders. With the digest the current user may too: the helper must then be the
//! exact shipped content, read through the pin that keeps it until the elevated process has
//! opened its image. Code already running as the user could replace the application and the
//! helper together, but it could equally show its own consent prompt; another account that
//! could replace the helper (or plant a DLL beside it) would gain the user's administrator
//! consent, so every other non-administrative identity stays refused.
use crate::{Cause, HelperDigest, refusal};
use std::{
    fs::{File, OpenOptions},
    io,
    os::windows::{ffi::OsStrExt, fs::OpenOptionsExt, io::AsRawHandle},
    path::{Component, Path, PathBuf, Prefix},
    ptr,
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, GENERIC_ALL, GENERIC_WRITE, HANDLE, LocalFree},
    Security::{
        ACCESS_ALLOWED_ACE, ACE_HEADER, ACL,
        Authorization::{GetSecurityInfo, SE_FILE_OBJECT},
        DACL_SECURITY_INFORMATION, EqualSid, GetAce, GetTokenInformation, INHERIT_ONLY_ACE,
        IsValidSid, IsWellKnownSid, OWNER_SECURITY_INFORMATION, PSID, TOKEN_QUERY, TOKEN_USER,
        TokenUser, WinBuiltinAdministratorsSid, WinLocalSystemSid,
    },
    Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, DELETE, FILE_APPEND_DATA, FILE_ATTRIBUTE_DIRECTORY,
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_TAG_INFO, FILE_DELETE_CHILD,
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_TRAVERSE, FILE_WRITE_ATTRIBUTES,
        FILE_WRITE_DATA, FILE_WRITE_EA, FileAttributeTagInfo, GetDriveTypeW,
        GetFileInformationByHandle, GetFileInformationByHandleEx, READ_CONTROL, WRITE_DAC,
        WRITE_OWNER,
    },
    System::Threading::{GetCurrentProcess, OpenProcessToken},
};

// Declared in SystemServices/WindowsProgramming, which this crate does not enable.
const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;
const ACCESS_DENIED_ACE_TYPE: u8 = 1;
const DRIVE_FIXED: u32 = 3;
const IO_REPARSE_TAG_DEDUP: u32 = 0x8000_0013;
const IO_REPARSE_TAG_WOF: u32 = 0x8000_0017;

/// Reparse tags of kernel storage filters that neither redirect the name nor take content from
/// a user-mode provider: WOF compression (CompactOS, `compact /exe`) and Data Deduplication.
/// The data stays under the object's own permissions, which the policy checks. Links, Cloud
/// Files placeholders, app execution aliases and every other tag are refused.
const STORAGE_TAGS: [u32; 2] = [IO_REPARSE_TAG_WOF, IO_REPARSE_TAG_DEDUP];

/// Rights that delete, rename, re-permission or otherwise replace an existing object.
const REPLACE_RIGHTS: u32 = DELETE
    | FILE_DELETE_CHILD
    | WRITE_DAC
    | WRITE_OWNER
    | GENERIC_ALL
    | GENERIC_WRITE
    | FILE_WRITE_ATTRIBUTES
    | FILE_WRITE_EA;
/// Also deny content changes to the helper and new entries in its directory: the elevated
/// loader searches that directory first (DLLs, manifests and `.local` redirection).
const MODIFY_RIGHTS: u32 = REPLACE_RIGHTS | FILE_WRITE_DATA | FILE_APPEND_DATA;

/// `NT SERVICE\TrustedInstaller`, S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464:
/// revision 1, six sub-authorities, NT authority 5, then the native-endian sub-authorities.
static TRUSTED_INSTALLER: [u32; 8] = [
    u32::from_le_bytes([1, 6, 0, 0]),
    u32::from_le_bytes([0, 0, 0, 5]),
    80,
    956_008_885,
    3_418_522_649,
    1_831_038_044,
    1_853_292_631,
    2_271_478_464,
];

/// Where a build without the shipped helper's digest needs the helper: a refused helper
/// location and a missing helper both explain authorization with this sentence.
const ADMINISTRATOR_REQUIREMENT: &str = "Retry with authorization needs Midnight Explorer \
    installed where only administrators can change it, such as a folder under \
    C:\\Program Files.";
/// Where a build that records the shipped helper's digest needs it.
const SHIPPED_REQUIREMENT: &str = "Retry with authorization needs the authorization helper \
    shipped with this version of Midnight Explorer, in a folder that only you and \
    administrators can change.";

/// The sentence that explains a refused or missing helper; `shipped`: the build records the
/// shipped helper's digest.
pub(crate) fn requirement(shipped: bool) -> &'static str {
    if shipped {
        SHIPPED_REQUIREMENT
    } else {
        ADMINISTRATOR_REQUIREMENT
    }
}

/// Explain how to make authorization available; elevated listing is refused otherwise.
pub(super) fn unprotected(requirement: &str, path: &Path, reason: &str) -> io::Error {
    refusal(
        io::ErrorKind::PermissionDenied,
        Cause::Location,
        format!("{requirement} {} {reason}.", path.display()),
    )
}

/// Pin the helper and every ancestor against rename and deletion, then validate the pinned
/// objects: ordinary objects (no link or other reparse point except [`STORAGE_TAGS`]) on a
/// local fixed drive, owned by SYSTEM, Administrators or TrustedInstaller, with no write,
/// rename or permission rights for any other identity. With the `shipped` helper's digest the
/// current user is trusted too, and the pinned helper's content must have that digest. The
/// handles must stay open until the elevated process has started.
pub(super) fn pin_helper(helper: &Path, shipped: Option<&HelperDigest>) -> io::Result<Vec<File>> {
    let requirement = requirement(shipped.is_some());
    local_fixed_drive(helper, requirement)?;
    let user = shipped.map(|_| CurrentUser::query()).transpose()?;
    let user_sid = user.as_ref().map(CurrentUser::sid);
    let mut pins = Vec::new();
    for (index, path) in helper.ancestors().enumerate() {
        let file = if index == 0 {
            pin_file(path)?
        } else {
            pin_folder(path)?
        };
        let refused = |reason| unprotected(requirement, path, reason);
        let (attributes, tag) = attribute_tag(&file)?;
        object_policy(attributes, tag, index != 0).map_err(refused)?;
        let rights = if index <= 1 {
            MODIFY_RIGHTS
        } else {
            REPLACE_RIGHTS
        };
        check_security(&file, rights, user_sid).map_err(refused)?;
        if index == 0
            && let Some(shipped) = shipped
        {
            // Read through the pin, which shares only reading: the content cannot change
            // before the elevated process has opened its image.
            crate::verify_shipped(&mut &file, path, shipped, requirement)?;
        }
        pins.push(file);
    }
    Ok(pins)
}

/// The user of this process's token, whom a build with the shipped helper's digest trusts.
pub(super) struct CurrentUser(Vec<u64>);
impl CurrentUser {
    pub(super) fn query() -> io::Result<Self> {
        let mut token: HANDLE = ptr::null_mut();
        // SAFETY: the pseudo handle of this process and a writable output handle.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return Err(io::Error::last_os_error());
        }
        struct Token(HANDLE);
        impl Drop for Token {
            fn drop(&mut self) {
                // SAFETY: OpenProcessToken returned this handle, which is closed once.
                unsafe { CloseHandle(self.0) };
            }
        }
        let token = Token(token);
        let mut size = 0;
        // SAFETY: a size query without an output buffer; it fails with the needed size.
        unsafe { GetTokenInformation(token.0, TokenUser, ptr::null_mut(), 0, &mut size) };
        if (size as usize) < size_of::<TOKEN_USER>() {
            return Err(io::Error::last_os_error());
        }
        let mut buffer = vec![0u64; (size as usize).div_ceil(8)];
        // SAFETY: an aligned, writable buffer of at least `size` bytes.
        if unsafe {
            GetTokenInformation(
                token.0,
                TokenUser,
                buffer.as_mut_ptr().cast(),
                size,
                &mut size,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(buffer))
    }
    pub(super) fn sid(&self) -> PSID {
        // SAFETY: the buffer holds the TOKEN_USER that GetTokenInformation wrote; its SID
        // points into the same buffer, which lives as long as `self`.
        unsafe { (*self.0.as_ptr().cast::<TOKEN_USER>()).User.Sid }
    }
}

/// Open the helper itself (never a link target) with read sharing only: no process can write,
/// rename or delete it while it is pinned.
fn pin_file(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

/// Open an ancestor folder (never a link target) so that it can be neither renamed nor
/// removed, while other processes keep adding and changing its entries. Traverse access takes
/// part in share checks, unlike attribute and permission reads alone, so withholding delete
/// sharing refuses every open for delete; write sharing admits new entries.
fn pin_folder(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES | READ_CONTROL | FILE_TRAVERSE)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

/// The attributes and reparse tag (zero without a reparse point) of a pinned object.
fn attribute_tag(file: &File) -> io::Result<(u32, u32)> {
    let mut information = FILE_ATTRIBUTE_TAG_INFO::default();
    // SAFETY: a live handle and a writable output of exactly the queried class's size.
    if unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileAttributeTagInfo,
            (&mut information as *mut FILE_ATTRIBUTE_TAG_INFO).cast(),
            size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let tag = if information.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        information.ReparseTag
    } else {
        0
    };
    Ok((information.FileAttributes, tag))
}

/// The object type and reparse policy for the helper (`folder == false`) or an ancestor.
fn object_policy(attributes: u32, tag: u32, folder: bool) -> Result<(), &'static str> {
    if attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 && !STORAGE_TAGS.contains(&tag) {
        return Err("is a link or reparse point");
    }
    if (attributes & FILE_ATTRIBUTE_DIRECTORY != 0) != folder {
        return Err("is not an ordinary file or folder");
    }
    Ok(())
}

fn local_fixed_drive(helper: &Path, requirement: &str) -> io::Result<()> {
    let refused = |reason| Err(unprotected(requirement, helper, reason));
    let letter = match helper.components().next() {
        Some(Component::Prefix(prefix)) => match prefix.kind() {
            Prefix::VerbatimDisk(letter) | Prefix::Disk(letter) => letter,
            _ => return refused("is not on a local drive"),
        },
        _ => return refused("is not an absolute drive path"),
    };
    let root = [u16::from(letter), u16::from(b':'), u16::from(b'\\'), 0];
    // SAFETY: a terminated drive-root string; the query has no other side effects.
    if unsafe { GetDriveTypeW(root.as_ptr()) } != DRIVE_FIXED {
        // Ownership on network or removable media is not an administrator's statement.
        return refused("is not on a local fixed drive");
    }
    Ok(())
}

struct Descriptor(*mut std::ffi::c_void);
impl Drop for Descriptor {
    fn drop(&mut self) {
        // SAFETY: GetSecurityInfo allocated this descriptor with LocalAlloc.
        unsafe {
            LocalFree(self.0);
        }
    }
}

fn check_security(file: &File, rights: u32, user: Option<PSID>) -> Result<(), &'static str> {
    let mut owner = ptr::null_mut();
    let mut dacl = ptr::null_mut();
    let mut descriptor = ptr::null_mut();
    // SAFETY: a live handle with READ_CONTROL and writable outputs; the descriptor owns the
    // returned owner and DACL pointers until it is freed.
    let status = unsafe {
        GetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            ptr::null_mut(),
            &mut dacl,
            ptr::null_mut(),
            &mut descriptor,
        )
    };
    if status != 0 {
        return Err("has unreadable permissions");
    }
    let _descriptor = Descriptor(descriptor);
    policy(owner, dacl, rights, user)
}

/// The elevation policy over one object's owner and DACL. `user`: the current user, whom a
/// build with the shipped helper's digest trusts like an administrator.
fn policy(
    owner: PSID,
    dacl: *const ACL,
    rights: u32,
    user: Option<PSID>,
) -> Result<(), &'static str> {
    let trusted = |sid: PSID| {
        administrative(sid)
            // SAFETY: a valid SID from a live descriptor and the current user's from the token.
            || user.is_some_and(|user| !sid.is_null() && unsafe { EqualSid(sid, user) } != 0)
    };
    if !trusted(owner) {
        return Err(if user.is_some() {
            "is not owned by you, SYSTEM, Administrators or TrustedInstaller"
        } else {
            "is not owned by SYSTEM, Administrators or TrustedInstaller"
        });
    }
    if dacl.is_null() {
        return Err("has no access control list");
    }
    // SAFETY: a non-null DACL from a live descriptor; GetAce validates every index.
    for index in 0..unsafe { (*dacl).AceCount } {
        let mut raw = ptr::null_mut();
        if unsafe { GetAce(dacl, u32::from(index), &mut raw) } == 0 || raw.is_null() {
            return Err("has an invalid access control list");
        }
        // SAFETY: GetAce returned an entry inside the DACL, which starts with ACE_HEADER.
        let header = unsafe { &*raw.cast::<ACE_HEADER>() };
        if u32::from(header.AceFlags) & INHERIT_ONLY_ACE != 0 {
            continue;
        }
        match header.AceType {
            ACCESS_DENIED_ACE_TYPE => {}
            ACCESS_ALLOWED_ACE_TYPE => {
                if usize::from(header.AceSize) < size_of::<ACCESS_ALLOWED_ACE>() {
                    return Err("has an invalid access control entry");
                }
                // SAFETY: the size check above covers the fixed ACE and its SID start.
                let ace = unsafe { &*raw.cast::<ACCESS_ALLOWED_ACE>() };
                let sid = ptr::addr_of!(ace.SidStart).cast_mut().cast();
                if ace.Mask & rights != 0 && !trusted(sid) {
                    return Err(if user.is_some() {
                        "lets another non-administrative account change, rename or replace it"
                    } else {
                        "lets a non-administrative account change, rename or replace it"
                    });
                }
            }
            _ => return Err("has an unsupported conditional or object access entry"),
        }
    }
    Ok(())
}

fn administrative(sid: PSID) -> bool {
    // SAFETY: sid is null or points into a live security descriptor; the static SID is valid
    // and EqualSid only reads it.
    !sid.is_null()
        && unsafe {
            IsValidSid(sid) != 0
                && (IsWellKnownSid(sid, WinLocalSystemSid) != 0
                    || IsWellKnownSid(sid, WinBuiltinAdministratorsSid) != 0
                    || EqualSid(sid, TRUSTED_INSTALLER.as_ptr().cast_mut().cast()) != 0)
        }
}

/// The Shell's elevation path expects an ordinary drive path. Use the plain spelling of the
/// verbatim helper path only when it opens the same pinned file; otherwise keep it verbatim.
pub(super) fn shell_path(helper: &Path, pinned: &File) -> PathBuf {
    plain_drive_path(helper)
        .filter(|plain| same_file(plain, pinned))
        .unwrap_or_else(|| helper.to_owned())
}

fn plain_drive_path(path: &Path) -> Option<PathBuf> {
    let mut components = path.components();
    let Some(Component::Prefix(prefix)) = components.next() else {
        return None;
    };
    let Prefix::VerbatimDisk(letter) = prefix.kind() else {
        return None;
    };
    let mut plain = PathBuf::from(format!("{}:\\", char::from(letter)));
    for component in components {
        match component {
            Component::RootDir => {}
            Component::Normal(name) => {
                // Win32 normalization strips trailing dots and spaces; keep such names verbatim.
                if name
                    .encode_wide()
                    .last()
                    .is_none_or(|unit| unit == u16::from(b'.') || unit == u16::from(b' '))
                {
                    return None;
                }
                plain.push(name);
            }
            _ => return None,
        }
    }
    (plain.as_os_str().encode_wide().count() < 260).then_some(plain)
}

fn same_file(path: &Path, pinned: &File) -> bool {
    // Attribute-only access is outside sharing checks, so the pin cannot refuse this probe.
    let Ok(file) = OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
    else {
        return false;
    };
    matches!((identity(&file), identity(pinned)), (Some(left), Some(right)) if left == right)
}

fn identity(file: &File) -> Option<(u32, u32, u32)> {
    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: a live handle and a writable, correctly sized output structure.
    (unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut information) } != 0).then_some(
        (
            information.dwVolumeSerialNumber,
            information.nFileIndexHigh,
            information.nFileIndexLow,
        ),
    )
}

#[cfg(test)]
#[path = "windows_trust_tests.rs"]
mod tests;
