//! Native test fixtures without spawning a shell: directory junctions and
//! non-surrogate (filter-style) reparse points, set with `FSCTL_SET_REPARSE_POINT`, and
//! DACLs that deny Everyone chosen rights, set through the object's own handle.
//! Standalone (std and windows-sys only) so integration tests can include it by path.
#![allow(dead_code, reason = "each test binary uses a subset of the fixtures")]
use std::{
    ffi::OsString,
    fs::{File, OpenOptions},
    io,
    mem::size_of,
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        fs::OpenOptionsExt,
        io::AsRawHandle,
    },
    path::{Path, PathBuf},
    ptr,
};
use windows_sys::Win32::{
    Security::{
        ACCESS_DENIED_ACE, ACE_HEADER, ACL, ACL_SIZE_INFORMATION, AclSizeInformation,
        AddAccessDeniedAce, AddAce, CreateWellKnownSid, DACL_SECURITY_INFORMATION, GetAce,
        GetAclInformation, GetKernelObjectSecurity, GetLengthSid, GetSecurityDescriptorControl,
        GetSecurityDescriptorDacl, InitializeAcl, InitializeSecurityDescriptor,
        SE_DACL_AUTO_INHERITED, SE_DACL_PROTECTED, SECURITY_DESCRIPTOR, SECURITY_MAX_SID_SIZE,
        SetKernelObjectSecurity, SetSecurityDescriptorControl, SetSecurityDescriptorDacl,
        WinWorldSid,
    },
    Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_DATA,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_WRITE_ATTRIBUTES,
        FILE_WRITE_DATA, READ_CONTROL, WRITE_DAC,
    },
    System::{
        IO::DeviceIoControl,
        Ioctl::{FSCTL_DELETE_REPARSE_POINT, FSCTL_SET_REPARSE_POINT},
    },
};

/// `IO_REPARSE_TAG_MOUNT_POINT`: a junction, which is a name surrogate.
pub const MOUNT_POINT: u32 = 0xA000_0003;
/// A third-party tag without the Microsoft or name-surrogate bits. Like a Cloud Files
/// placeholder, the directory variant sets the "may have children" bit (0x1000_0000).
pub const FILTER_FILE: u32 = 0x0000_4D45;
pub const FILTER_DIRECTORY: u32 = 0x1000_4D45;
const FIXTURE_GUID: [u8; 16] = *b"midnight-fixture";

fn open_for_reparse(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .access_mode(FILE_WRITE_DATA | FILE_WRITE_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

fn control(file: &File, code: u32, input: &[u8]) -> io::Result<()> {
    let mut returned = 0u32;
    // SAFETY: a live handle and an input buffer that outlives this synchronous request;
    // no output buffer is requested.
    if unsafe {
        DeviceIoControl(
            file.as_raw_handle(),
            code,
            input.as_ptr().cast(),
            input.len() as u32,
            ptr::null_mut(),
            0,
            &mut returned,
            ptr::null_mut(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Turns the existing empty directory `link` into a junction to the absolute `target`.
pub fn create_junction(link: &Path, target: &Path) -> io::Result<()> {
    // NT substitute names are not normalized: use backslashes and no verbatim prefix.
    let target: Vec<u16> = target
        .as_os_str()
        .encode_wide()
        .map(|unit| if unit == 47 { 92 } else { unit })
        .collect();
    let plain = target
        .strip_prefix(&[92, 92, 63, 92][..])
        .unwrap_or(&target)
        .to_vec();
    let mut substitute: Vec<u16> = r"\??\".encode_utf16().collect();
    substitute.extend(&plain);
    let substitute_bytes = substitute.len() * 2;
    let print_bytes = plain.len() * 2;
    // PathBuffer holds both names, each followed by a terminating NUL.
    let names = substitute_bytes + 2 + print_bytes + 2;
    let mut buffer = Vec::with_capacity(16 + names);
    buffer.extend(MOUNT_POINT.to_le_bytes());
    buffer.extend(
        u16::try_from(8 + names)
            .map_err(|_| invalid())?
            .to_le_bytes(),
    );
    buffer.extend(0u16.to_le_bytes());
    buffer.extend(0u16.to_le_bytes());
    buffer.extend((substitute_bytes as u16).to_le_bytes());
    buffer.extend(((substitute_bytes + 2) as u16).to_le_bytes());
    buffer.extend((print_bytes as u16).to_le_bytes());
    for unit in substitute.iter().chain(&[0]).chain(&plain).chain(&[0]) {
        buffer.extend(unit.to_le_bytes());
    }
    control(&open_for_reparse(link)?, FSCTL_SET_REPARSE_POINT, &buffer)
}

fn guid_buffer(tag: u32, data: &[u8]) -> Vec<u8> {
    let mut buffer = Vec::with_capacity(24 + data.len());
    buffer.extend(tag.to_le_bytes());
    buffer.extend((data.len() as u16).to_le_bytes());
    buffer.extend(0u16.to_le_bytes());
    buffer.extend(FIXTURE_GUID);
    buffer.extend(data);
    buffer
}

/// Adds a third-party non-surrogate reparse point to an existing file or directory.
/// No filter owns the tag: no-follow opens work, filter-mediated opens fail.
pub fn set_filter_tag(path: &Path, tag: u32) -> io::Result<()> {
    control(
        &open_for_reparse(path)?,
        FSCTL_SET_REPARSE_POINT,
        &guid_buffer(tag, b"midnight"),
    )
}

/// Removes the fixture reparse point again, leaving an ordinary entry.
pub fn clear_filter_tag(path: &Path, tag: u32) -> io::Result<()> {
    control(
        &open_for_reparse(path)?,
        FSCTL_DELETE_REPARSE_POINT,
        &guid_buffer(tag, &[]),
    )
}

fn repository_build() -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(Path::parent)
        .expect("the crate lives in <repository>/crates/<name>")
        .join("_build")
}

/// The repository's `_build/tmp`, spelled without `.`/`..` steps or forward slashes
/// (native mutation validation and junction substitute names reject them).
pub fn build_tmp_root() -> PathBuf {
    let root = repository_build().join("tmp");
    std::fs::create_dir_all(&root).expect("create _build/tmp");
    root
}

/// The original DACL of a fixture, saved by [`deny_to_everyone`] for [`restore_dacl`].
pub struct SavedDacl(Vec<u64>);

fn check(succeeded: i32) -> io::Result<()> {
    if succeeded == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Opens the fixture object itself for its DACL: a link is changed itself, never its
/// target. Only entries whose parent lies below the repository's `_build` are accepted.
fn open_for_security(path: &Path) -> io::Result<File> {
    let owned = path.parent().is_some_and(|parent| {
        let (Ok(parent), Ok(build)) = (
            std::fs::canonicalize(parent),
            std::fs::canonicalize(repository_build()),
        ) else {
            return false;
        };
        parent.starts_with(build)
    });
    if !owned {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "ACL fixtures change only their own entries below _build",
        ));
    }
    OpenOptions::new()
        .access_mode(READ_CONTROL | WRITE_DAC)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

fn dacl_descriptor(file: &File) -> io::Result<Vec<u64>> {
    let mut needed = 0u32;
    // SAFETY: a live handle and a writable size output; the size probe writes no data.
    unsafe {
        GetKernelObjectSecurity(
            file.as_raw_handle(),
            DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            0,
            &mut needed,
        )
    };
    if needed == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut descriptor = vec![0u64; (needed as usize).div_ceil(8)];
    // SAFETY: the aligned allocation covers `needed` writable bytes and stays live.
    check(unsafe {
        GetKernelObjectSecurity(
            file.as_raw_handle(),
            DACL_SECURITY_INFORMATION,
            descriptor.as_mut_ptr().cast(),
            needed,
            &mut needed,
        )
    })?;
    Ok(descriptor)
}

/// Denies `FILE_READ_DATA` (file content, directory listing) to Everyone; see
/// [`deny_to_everyone`].
pub fn deny_read_data(path: &Path) -> io::Result<SavedDacl> {
    deny_to_everyone(path, FILE_READ_DATA)
}

/// Denies the access `rights` to Everyone by prepending one explicit deny ACE to the
/// fixture's DACL, keeping its other ACEs and inheritance flags. Returns the original
/// DACL for [`restore_dacl`]. No shell is involved.
pub fn deny_to_everyone(path: &Path, rights: u32) -> io::Result<SavedDacl> {
    let file = open_for_security(path)?;
    let mut original = dacl_descriptor(&file)?;
    let descriptor = original.as_mut_ptr().cast();
    let (mut present, mut defaulted) = (0, 0);
    let mut dacl: *mut ACL = ptr::null_mut();
    // SAFETY: a self-relative descriptor returned by the OS and writable outputs; the
    // DACL pointer refers into `original`, which outlives every use below.
    check(unsafe {
        GetSecurityDescriptorDacl(descriptor, &mut present, &mut dacl, &mut defaulted)
    })?;
    if present == 0 || dacl.is_null() {
        return Err(io::Error::other("ACL fixtures need an existing DACL"));
    }
    let mut size = ACL_SIZE_INFORMATION::default();
    // SAFETY: a valid DACL and an exactly sized writable information structure.
    check(unsafe {
        GetAclInformation(
            dacl,
            (&mut size as *mut ACL_SIZE_INFORMATION).cast(),
            size_of::<ACL_SIZE_INFORMATION>() as u32,
            AclSizeInformation,
        )
    })?;
    let mut everyone = [0u64; (SECURITY_MAX_SID_SIZE as usize).div_ceil(8)];
    let mut sid_size = SECURITY_MAX_SID_SIZE;
    // SAFETY: an aligned writable buffer of SECURITY_MAX_SID_SIZE bytes.
    check(unsafe {
        CreateWellKnownSid(
            WinWorldSid,
            ptr::null_mut(),
            everyone.as_mut_ptr().cast(),
            &mut sid_size,
        )
    })?;
    let sid = everyone.as_mut_ptr().cast();
    // SAFETY: `sid` is the valid SID created above.
    let sid_length = unsafe { GetLengthSid(sid) } as usize;
    let deny = size_of::<ACCESS_DENIED_ACE>() - size_of::<u32>() + sid_length;
    let length = size.AclBytesInUse as usize + deny;
    let mut replacement = vec![0u64; length.div_ceil(8)];
    let acl = replacement.as_mut_ptr().cast::<ACL>();
    // SAFETY: `acl` covers `length` writable bytes; `dacl` and its ACEs stay valid in
    // `original`. Each copied ACE is read through its own header's size.
    unsafe {
        let revision = u32::from((*dacl).AclRevision);
        check(InitializeAcl(acl, length as u32, revision))?;
        check(AddAccessDeniedAce(acl, revision, rights, sid))?;
        for index in 0..size.AceCount {
            let mut ace = ptr::null_mut();
            check(GetAce(dacl, index, &mut ace))?;
            let ace_size = u32::from((*ace.cast::<ACE_HEADER>()).AceSize);
            check(AddAce(acl, revision, u32::MAX, ace, ace_size))?;
        }
    }
    let mut absolute = SECURITY_DESCRIPTOR::default();
    let replaced = (&mut absolute as *mut SECURITY_DESCRIPTOR).cast();
    let (mut control, mut revision) = (0u16, 0u32);
    // SAFETY: `absolute` is initialized in place (SECURITY_DESCRIPTOR_REVISION = 1) and
    // refers to `replacement`, which outlives the SetKernelObjectSecurity call. The
    // original control bits are read from the live self-relative descriptor.
    unsafe {
        check(InitializeSecurityDescriptor(replaced, 1))?;
        check(SetSecurityDescriptorDacl(replaced, 1, acl, 0))?;
        check(GetSecurityDescriptorControl(
            descriptor,
            &mut control,
            &mut revision,
        ))?;
        let inheritance = SE_DACL_PROTECTED | SE_DACL_AUTO_INHERITED;
        check(SetSecurityDescriptorControl(
            replaced,
            inheritance,
            control & inheritance,
        ))?;
        check(SetKernelObjectSecurity(
            file.as_raw_handle(),
            DACL_SECURITY_INFORMATION,
            replaced,
        ))?;
    }
    Ok(SavedDacl(original))
}

/// Writes the DACL saved by [`deny_to_everyone`] back to the fixture at `path`.
pub fn restore_dacl(path: &Path, saved: &SavedDacl) -> io::Result<()> {
    let file = open_for_security(path)?;
    // SAFETY: a live handle and the self-relative descriptor the OS returned earlier.
    check(unsafe {
        SetKernelObjectSecurity(
            file.as_raw_handle(),
            DACL_SECURITY_INFORMATION,
            saved.0.as_ptr().cast_mut().cast(),
        )
    })
}

/// A fresh directory below the repository's `_build/tmp`, removed on drop.
pub fn build_temp(prefix: &str) -> tempfile::TempDir {
    let root = build_tmp_root();
    tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in(root)
        .expect("create fixture directory")
}

/// A Unicode name that also contains an unpaired surrogate.
pub fn unusual_name(stem: &str) -> OsString {
    let mut units: Vec<u16> = stem.encode_utf16().collect();
    units.extend("-文档-".encode_utf16());
    units.push(0xd800);
    OsString::from_wide(&units)
}

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "junction target is too long")
}
