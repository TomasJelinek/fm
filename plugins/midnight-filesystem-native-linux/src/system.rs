//! Audited Linux descriptor/iterator boundary. No whole-path PATH_MAX dependency.
use midnight_filesystem_plugin_api::*;
use midnight_filesystem_plugin_sdk::{
    CallContext,
    native::{NativeEntry, NativeInfo},
};
use std::{
    ffi::{CStr, CString, OsString},
    fs::File,
    os::{
        fd::{AsRawFd, FromRawFd, IntoRawFd},
        unix::ffi::{OsStrExt, OsStringExt},
    },
    path::Path,
};

pub fn cancel(context: &CallContext<'_>) -> Result<(), Failure> {
    if context.is_cancelled() {
        Err(Failure::new(
            ErrorCode::Cancelled,
            "Native operation canceled.",
        ))
    } else {
        Ok(())
    }
}
fn os_error() -> Failure {
    Failure::from_io(std::io::Error::last_os_error())
}
fn owned(fd: i32) -> Result<File, Failure> {
    if fd < 0 {
        return Err(os_error());
    }
    // SAFETY: a successful native open returns exactly one newly owned descriptor.
    Ok(unsafe { File::from_raw_fd(fd) })
}
pub fn open_path(path: &Path, flags: i32, context: &CallContext<'_>) -> Result<File, Failure> {
    cancel(context)?;
    if !path.is_absolute() {
        return Err(Failure::new(
            ErrorCode::Invalid,
            "Absolute native path required.",
        ));
    }
    #[cfg(test)]
    if flags & libc::O_DIRECTORY != 0
        && let Some(errno) = crate::rename_test_hooks::directory_open_failure(path)
    {
        return Err(Failure::from_io(std::io::Error::from_raw_os_error(errno)));
    }
    // SAFETY: static terminated root and no variadic creation mode because O_CREAT is absent.
    let mut directory = owned(unsafe {
        libc::open(
            c"/".as_ptr(),
            libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC,
        )
    })?;
    // Preserve '.'/'..' and trailing-slash semantics instead of std Path's
    // lexical normalization; the kernel interprets every native component.
    let raw = path.as_os_str().as_bytes();
    let flags = if raw.ends_with(b"/") {
        (flags | libc::O_DIRECTORY) & !libc::O_NOFOLLOW
    } else {
        flags
    };
    let mut components = raw
        .split(|byte| *byte == b'/')
        .filter(|part| !part.is_empty())
        .peekable();
    if components.peek().is_none() {
        // SAFETY: retained directory descriptor, static name and no O_CREAT.
        return owned(unsafe {
            libc::openat(
                directory.as_raw_fd(),
                c".".as_ptr(),
                flags | libc::O_CLOEXEC,
            )
        });
    }
    while let Some(component) = components.next() {
        cancel(context)?;
        let name = CString::new(component)
            .map_err(|_| Failure::new(ErrorCode::Invalid, "NUL in native path."))?;
        let current = if components.peek().is_none() {
            flags
        } else {
            libc::O_PATH | libc::O_DIRECTORY
        };
        // SAFETY: parent owns its descriptor, name is terminated and O_CREAT is absent.
        let next = owned(unsafe {
            libc::openat(
                directory.as_raw_fd(),
                name.as_ptr(),
                current | libc::O_CLOEXEC,
            )
        })?;
        directory = next;
    }
    cancel(context)?;
    Ok(directory)
}
pub struct Directory {
    handle: *mut libc::DIR,
}
// SAFETY: ownership may move between backend threads; all operations require &mut
// self and the SDK also serializes this cursor. No borrowed dirent escapes next().
unsafe impl Send for Directory {}
impl Directory {
    pub fn open(path: &Path, context: &CallContext<'_>) -> Result<Self, Failure> {
        let file = open_path(path, libc::O_RDONLY | libc::O_DIRECTORY, context)?;
        // SAFETY: on success fdopendir transfers the descriptor's ownership to DIR.
        let handle = unsafe { libc::fdopendir(file.as_raw_fd()) };
        if handle.is_null() {
            return Err(os_error());
        }
        let _ = file.into_raw_fd();
        Ok(Self { handle })
    }
    pub fn next(&mut self, context: &CallContext<'_>) -> Result<Option<NativeEntry>, Failure> {
        loop {
            cancel(context)?;
            // SAFETY: thread-local errno and uniquely owned valid DIR, serialized by &mut self.
            let entry = unsafe {
                *libc::__errno_location() = 0;
                libc::readdir(self.handle)
            };
            if entry.is_null() {
                // SAFETY: read this thread's errno immediately after readdir.
                return if unsafe { *libc::__errno_location() } == 0 {
                    Ok(None)
                } else {
                    Err(os_error())
                };
            }
            // SAFETY: readdir returns a NUL-terminated name valid until this DIR's next call.
            let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) };
            if name.to_bytes() == b"." || name.to_bytes() == b".." {
                continue;
            }
            // SAFETY: the DIR is live and its descriptor stays owned by it.
            let directory = unsafe { libc::dirfd(self.handle) };
            // One no-follow metadata query per row: the child is neither opened
            // nor mounted when it is an automount point.
            let mut info = match stat_child(directory, name) {
                Ok(value) => crate::adapter::stat_info(&value),
                Err(error) if error.code == ErrorCode::NotFound => return Err(error),
                Err(_) => {
                    // SAFETY: the current dirent is still live and its type is a small kernel tag.
                    let kind = match unsafe { (*entry).d_type } {
                        libc::DT_DIR => Kind::Directory,
                        libc::DT_REG => Kind::File,
                        libc::DT_LNK => Kind::Symlink,
                        _ => Kind::Other,
                    };
                    NativeInfo {
                        kind,
                        metadata: Metadata::default(),
                    }
                }
            };
            if info.kind == Kind::Symlink {
                cancel(context)?;
                if link_to_directory(directory, name) {
                    info.kind = Kind::DirectoryLink;
                }
                cancel(context)?;
            }
            return Ok(Some(NativeEntry {
                name: OsString::from_vec(name.to_bytes().to_vec()),
                info,
            }));
        }
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        // SAFETY: exactly one DIR owns the descriptor; no dirent borrow remains.
        unsafe { libc::closedir(self.handle) };
    }
}
fn stat_child(directory: i32, name: &CStr) -> Result<libc::stat, Failure> {
    // SAFETY: zero initializes a plain native output structure.
    let mut value: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: live directory descriptor, terminated child name and complete output.
    if unsafe {
        libc::fstatat(
            directory,
            name.as_ptr(),
            &mut value,
            libc::AT_SYMLINK_NOFOLLOW | libc::AT_NO_AUTOMOUNT,
        )
    } != 0
    {
        return Err(os_error());
    }
    Ok(value)
}
/// Display hint only: whether a listed link's target is a directory. Only the
/// target's type is read: nothing is opened and an existing automount point is
/// not mounted. The kernel still resolves the target path, so a server that
/// stops answering along it, or an autofs map key that is not present yet,
/// delays this lookup (and its listing page) until it answers. Any failure,
/// including a broken or inaccessible target, leaves an ordinary link.
fn link_to_directory(directory: i32, name: &CStr) -> bool {
    // SAFETY: zero initializes a plain native output structure.
    let mut value: libc::statx = unsafe { std::mem::zeroed() };
    // SAFETY: live directory descriptor, terminated child name and complete output.
    let status = unsafe {
        libc::statx(
            directory,
            name.as_ptr(),
            libc::AT_NO_AUTOMOUNT | libc::AT_STATX_DONT_SYNC,
            libc::STATX_TYPE,
            &mut value,
        )
    };
    status == 0
        && value.stx_mask & libc::STATX_TYPE != 0
        && u32::from(value.stx_mode) & libc::S_IFMT == libc::S_IFDIR
}
/// The listing's `link_to_directory` hint for a link named by an absolute path,
/// so explicit Stat (and the SDK's kind revalidation of a listed link) agrees
/// with the listing. A later directory open reports the target's own error.
pub fn path_link_to_directory(path: &Path, context: &CallContext<'_>) -> Result<bool, Failure> {
    let raw = path.as_os_str().as_bytes();
    let Some(separator) = raw.iter().rposition(|byte| *byte == b'/') else {
        return Ok(false);
    };
    let name = &raw[separator + 1..];
    if name.is_empty() || name == b"." || name == b".." {
        return Ok(false);
    }
    let name =
        CString::new(name).map_err(|_| Failure::new(ErrorCode::Invalid, "NUL in native path."))?;
    let parent = Path::new(std::ffi::OsStr::from_bytes(&raw[..separator.max(1)]));
    let directory = match open_path(parent, libc::O_PATH | libc::O_DIRECTORY, context) {
        Ok(directory) => link_to_directory(directory.as_raw_fd(), &name),
        // Like any other probe failure, a parent that cannot be reopened now
        // leaves an ordinary link. Cancellation is still reported.
        Err(_) => false,
    };
    cancel(context)?;
    Ok(directory)
}
pub fn filesystem(file: &File) -> Result<libc::statfs, Failure> {
    // SAFETY: zero initializes a plain native output structure.
    let mut value: libc::statfs = unsafe { std::mem::zeroed() };
    // SAFETY: retained descriptor and complete writable output structure.
    if unsafe { libc::fstatfs(file.as_raw_fd(), &mut value) } != 0 {
        return Err(os_error());
    }
    Ok(value)
}
pub fn reopen_regular(probe: &File) -> Result<File, Failure> {
    use std::os::unix::fs::MetadataExt;
    let before = probe.metadata().map_err(Failure::from_io)?;
    if !before.is_file() {
        return Err(Failure::new(
            ErrorCode::Unsupported,
            "Read requires a regular non-link file.",
        ));
    }
    // This link is our retained O_PATH descriptor, never a caller's pathname.
    // Reopening the original name could invoke a substituted device driver.
    let path = CString::new(format!("/proc/self/fd/{}", probe.as_raw_fd()))
        .map_err(|_| Failure::new(ErrorCode::Invalid, "Invalid descriptor address."))?;
    // SAFETY: owned live descriptor target, terminated internal path, no O_CREAT.
    let file = owned(unsafe {
        libc::open(
            path.as_ptr(),
            libc::O_RDONLY | libc::O_NONBLOCK | libc::O_CLOEXEC,
        )
    })?;
    let after = file.metadata().map_err(Failure::from_io)?;
    if !after.is_file() || (before.dev(), before.ino()) != (after.dev(), after.ino()) {
        return Err(Failure::new(
            ErrorCode::SourceChanged,
            "Descriptor identity changed while opening.",
        ));
    }
    Ok(file)
}
pub fn space_info(path: &Path, context: &CallContext<'_>) -> Result<(u64, u64), Failure> {
    let file = open_path(path, libc::O_PATH, context)?;
    let value = statvfs(&file)?;
    let unit = if value.f_frsize == 0 {
        value.f_bsize
    } else {
        value.f_frsize
    };
    let available = value
        .f_bavail
        .checked_mul(unit)
        .ok_or_else(|| Failure::new(ErrorCode::ResourceLimit, "Available space overflow."))?;
    let total = value
        .f_blocks
        .checked_mul(unit)
        .ok_or_else(|| Failure::new(ErrorCode::ResourceLimit, "Total space overflow."))?;
    Ok((available, total))
}

pub fn statvfs(file: &File) -> Result<libc::statvfs, Failure> {
    // SAFETY: zero initializes a plain native output structure.
    let mut value: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: retained descriptor and complete writable output structure.
    if unsafe { libc::fstatvfs(file.as_raw_fd(), &mut value) } != 0 {
        return Err(os_error());
    }
    Ok(value)
}
