//! Narrow Linux Rename syscall boundary. No content reads or fallback namespace operations.
use midnight_filesystem_plugin_api::{ErrorCode, Failure};
use std::{
    ffi::CStr,
    fs::File,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::ffi::OsStrExt,
    },
    path::Path,
};
fn error() -> Failure {
    Failure::from_io(std::io::Error::last_os_error())
}
fn owned(fd: i32) -> Result<File, Failure> {
    if fd < 0 {
        return Err(error());
    }
    // SAFETY: successful open returns one new descriptor, transferred exactly once.
    Ok(unsafe { File::from_raw_fd(fd) })
}
#[repr(C)]
struct OpenHow {
    flags: u64,
    mode: u64,
    resolve: u64,
}
pub(crate) fn pin(parent: &File, name: &CStr) -> Result<File, Failure> {
    let how = OpenHow {
        flags: (libc::O_PATH | libc::O_NOFOLLOW | libc::O_CLOEXEC) as u64,
        mode: 0,
        // BENEATH | NO_SYMLINKS | NO_XDEV. O_PATH|O_NOFOLLOW may pin the final link itself.
        resolve: 0x08 | 0x04 | 0x01,
    };
    // SAFETY: live parent, terminated single component and complete immutable kernel input.
    // Unsupported openat2 fails explicitly; no weaker open/copy/delete fallback is used.
    let fd = unsafe {
        libc::syscall(
            libc::SYS_openat2,
            parent.as_raw_fd(),
            name.as_ptr(),
            &how,
            size_of::<OpenHow>(),
        )
    };
    owned(fd as i32)
}

/// Observe an owned descriptor's current locator and reopen it without allocating
/// pathname scratch. The caller separately confirms both descriptor identities.
pub(crate) fn observe_locator(file: &File, bytes: &mut Vec<u8>) -> Result<File, Failure> {
    const PREFIX: &[u8] = b"/proc/self/fd/";
    let mut descriptor = u32::try_from(file.as_raw_fd())
        .map_err(|_| Failure::new(ErrorCode::Invalid, "Invalid retained descriptor."))?;
    let mut digits = [0u8; 10];
    let mut start = digits.len();
    loop {
        start -= 1;
        digits[start] = b'0' + (descriptor % 10) as u8;
        descriptor /= 10;
        if descriptor == 0 {
            break;
        }
    }
    let mut link = [0u8; 32];
    link[..PREFIX.len()].copy_from_slice(PREFIX);
    link[PREFIX.len()..PREFIX.len() + digits.len() - start].copy_from_slice(&digits[start..]);
    bytes.resize(bytes.capacity(), 0);
    if bytes.is_empty() {
        return Err(Failure::new(
            ErrorCode::ResourceLimit,
            "No recovery locator capacity.",
        ));
    }
    // SAFETY: the generated link is terminated in the fixed stack buffer; the
    // live descriptor and pre-reserved writable output remain owned throughout.
    let length =
        unsafe { libc::readlink(link.as_ptr().cast(), bytes.as_mut_ptr().cast(), bytes.len()) };
    if length < 0 {
        return Err(error());
    }
    let length = length as usize;
    if length == bytes.len() {
        return Err(Failure::new(
            ErrorCode::ResourceLimit,
            "Current recovery locator exceeds its pre-reserved capacity; it was not truncated.",
        ));
    }
    if length == 0 || bytes[0] != b'/' || bytes[..length].contains(&0) {
        return Err(Failure::new(
            ErrorCode::Invalid,
            "Invalid current recovery locator.",
        ));
    }
    bytes[length] = 0;
    // SAFETY: static terminated root and read-only descriptor flags, without O_CREAT.
    let mut current = owned(unsafe {
        libc::open(
            c"/".as_ptr(),
            libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC,
        )
    })?;
    let mut component = 1usize;
    for end in 1..=length {
        if end != length && bytes[end] != b'/' {
            continue;
        }
        if component != end {
            let separator = bytes[end];
            bytes[end] = 0;
            let flags = libc::O_PATH
                | libc::O_CLOEXEC
                | if end == length {
                    libc::O_NOFOLLOW
                } else {
                    libc::O_DIRECTORY
                };
            // SAFETY: the retained parent is live; this in-bounds component is
            // temporarily terminated. Only O_PATH descriptors are opened, final
            // links are not followed, and no write/create flags are present.
            let next = unsafe {
                libc::openat(
                    current.as_raw_fd(),
                    bytes.as_ptr().add(component).cast(),
                    flags,
                )
            };
            bytes[end] = separator;
            current = owned(next)?;
        }
        component = end + 1;
    }
    bytes.truncate(length);
    Ok(current)
}

pub(crate) fn parent_path(path: &Path) -> Result<File, Failure> {
    if !path.is_absolute() {
        return Err(Failure::new(
            ErrorCode::Invalid,
            "Absolute parent required.",
        ));
    }
    // SAFETY: static terminated root and no O_CREAT/variadic creation mode.
    let mut directory = owned(unsafe {
        libc::open(
            c"/".as_ptr(),
            libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC,
        )
    })?;
    for part in path
        .as_os_str()
        .as_bytes()
        .split(|byte| *byte == b'/')
        .filter(|part| !part.is_empty())
    {
        let name = std::ffi::CString::new(part)
            .map_err(|_| Failure::new(ErrorCode::Invalid, "NUL in parent path."))?;
        // SAFETY: retained directory, terminated component and no O_CREAT.
        // Intermediate links retain ordinary native path semantics; the actual parent inode is pinned.
        directory = owned(unsafe {
            libc::openat(
                directory.as_raw_fd(),
                name.as_ptr(),
                libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC,
            )
        })?;
    }
    Ok(directory)
}
pub(crate) fn readable_directory(pin: &File) -> Result<File, Failure> {
    // SAFETY: '.' resolves relative to the retained directory; no user pathname is reopened for syncing.
    owned(unsafe {
        libc::openat(
            pin.as_raw_fd(),
            c".".as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    })
}
pub(crate) fn mkdir(parent: &File, name: &CStr) -> Result<(), Failure> {
    // SAFETY: valid retained parent, terminated component and explicit owner-only creation mode.
    if unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700) } != 0 {
        return Err(error());
    }
    Ok(())
}
pub(crate) fn rename(
    from: &File,
    old: &CStr,
    to: &File,
    new: &CStr,
    exchange: bool,
) -> Result<(), Failure> {
    let flags = if exchange {
        libc::RENAME_EXCHANGE
    } else {
        libc::RENAME_NOREPLACE
    };
    // SAFETY: all descriptors and terminated component names remain borrowed through the call.
    if unsafe {
        libc::renameat2(
            from.as_raw_fd(),
            old.as_ptr(),
            to.as_raw_fd(),
            new.as_ptr(),
            flags,
        )
    } != 0
    {
        return Err(error());
    }
    #[cfg(test)]
    if crate::rename_test_hooks::take_rename_failure() {
        return Err(Failure::new(
            ErrorCode::Io,
            "Controlled native error after actual namespace commit.",
        ));
    }
    Ok(())
}
pub(crate) fn rmdir(parent: &File, name: &CStr) -> Result<(), Failure> {
    // SAFETY: retained parent and terminated component; AT_REMOVEDIR refuses nonempty containers.
    if unsafe { libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), libc::AT_REMOVEDIR) } != 0 {
        return Err(error());
    }
    Ok(())
}
pub(crate) fn effective_uid() -> u32 {
    // SAFETY: geteuid has no arguments or ownership requirements.
    unsafe { libc::geteuid() }
}
pub(crate) fn sync(file: &File) -> Result<(), Failure> {
    #[cfg(test)]
    if crate::rename_test_hooks::take_sync_failure() {
        return Err(Failure::new(
            ErrorCode::Io,
            "Controlled directory sync failure.",
        ));
    }
    file.sync_all().map_err(Failure::from_io)
}
