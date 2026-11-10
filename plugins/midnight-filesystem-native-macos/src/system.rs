//! Audited Darwin descriptor boundary. Full paths are never passed to a bounded pathname API.
use midnight_filesystem_plugin_api::{ErrorCode, Failure};
use midnight_filesystem_plugin_sdk::CallContext;
use std::{
    ffi::{CStr, CString},
    fs::File,
    os::{
        fd::{AsRawFd, FromRawFd, RawFd},
        unix::ffi::OsStrExt,
    },
    path::Path,
};

pub(crate) fn cancel(context: &CallContext<'_>) -> Result<(), Failure> {
    if context.is_cancelled() {
        Err(Failure::new(
            ErrorCode::Cancelled,
            "Native operation canceled.",
        ))
    } else {
        Ok(())
    }
}
pub(crate) fn os_error() -> Failure {
    Failure::from_io(std::io::Error::last_os_error())
}
pub(crate) fn owned(fd: RawFd) -> Result<File, Failure> {
    if fd < 0 {
        return Err(os_error());
    }
    // SAFETY: successful open/openat created one new descriptor whose ownership is transferred.
    Ok(unsafe { File::from_raw_fd(fd) })
}
pub(crate) fn stat_at(fd: RawFd, name: &CStr) -> Result<libc::stat, Failure> {
    // SAFETY: libc::stat is a plain output record and all bytes are initialized.
    let mut value: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: borrowed live directory descriptor, terminated name and complete output record.
    if unsafe { libc::fstatat(fd, name.as_ptr(), &mut value, libc::AT_SYMLINK_NOFOLLOW) } != 0 {
        return Err(os_error());
    }
    Ok(value)
}
pub(crate) fn stat_file(file: &File) -> Result<libc::stat, Failure> {
    // SAFETY: plain complete output record and borrowed live descriptor.
    let mut value: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(file.as_raw_fd(), &mut value) } != 0 {
        return Err(os_error());
    }
    Ok(value)
}
pub(crate) fn filesystem(file: &File) -> Result<libc::statfs, Failure> {
    // SAFETY: plain complete output record and borrowed live descriptor.
    let mut value: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstatfs(file.as_raw_fd(), &mut value) } != 0 {
        return Err(os_error());
    }
    Ok(value)
}

pub(crate) struct Parent {
    pub(crate) directory: File,
    name: CString,
    requires_directory: bool,
}
impl Parent {
    pub(crate) fn stat(&self) -> Result<libc::stat, Failure> {
        if self.requires_directory {
            return stat_file(&self.open(libc::O_RDONLY | libc::O_DIRECTORY)?);
        }
        stat_at(self.directory.as_raw_fd(), &self.name)
    }
    pub(crate) fn open(&self, flags: libc::c_int) -> Result<File, Failure> {
        let flags = flags | libc::O_CLOEXEC | libc::O_NOCTTY;
        let flags = if self.requires_directory {
            flags | libc::O_DIRECTORY
        } else {
            flags
        };
        // SAFETY: retained parent descriptor and terminated component; O_CREAT is never supplied.
        owned(unsafe { libc::openat(self.directory.as_raw_fd(), self.name.as_ptr(), flags) })
    }
}

pub(crate) fn parent(path: &Path, context: &CallContext<'_>) -> Result<Parent, Failure> {
    cancel(context)?;
    let raw = path.as_os_str().as_bytes();
    if !raw.starts_with(b"/") || raw.contains(&0) {
        return Err(Failure::new(
            ErrorCode::Invalid,
            "Absolute NUL-free native path required.",
        ));
    }
    // SAFETY: fixed terminated root, read-only directory access, no creation flags.
    let mut directory = owned(unsafe {
        libc::open(
            c"/".as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
        )
    })?;
    // Raw splitting preserves dot/dot-dot and trailing slash semantics; Path::components
    // would normalize away some entered components before the kernel sees them.
    let mut components = raw
        .split(|byte| *byte == b'/')
        .filter(|part| !part.is_empty())
        .peekable();
    while let Some(component) = components.next() {
        cancel(context)?;
        let name = CString::new(component)
            .map_err(|_| Failure::new(ErrorCode::Invalid, "NUL in native path."))?;
        if components.peek().is_none() {
            return Ok(Parent {
                directory,
                name,
                requires_directory: raw.ends_with(b"/"),
            });
        }
        // SAFETY: retained directory descriptor and one terminated component. The kernel
        // requires a directory, so traversal cannot open a device/FIFO for its contents.
        directory = owned(unsafe {
            libc::openat(
                directory.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
            )
        })?;
    }
    Ok(Parent {
        directory,
        name: c".".into(),
        requires_directory: true,
    })
}

pub(crate) fn stat_path(path: &Path, context: &CallContext<'_>) -> Result<libc::stat, Failure> {
    let value = parent(path, context)?.stat()?;
    cancel(context)?;
    Ok(value)
}
pub(crate) fn open_directory(path: &Path, context: &CallContext<'_>) -> Result<File, Failure> {
    let file = parent(path, context)?.open(libc::O_RDONLY | libc::O_DIRECTORY)?;
    cancel(context)?;
    Ok(file)
}

pub(crate) fn open_regular(path: &Path, context: &CallContext<'_>) -> Result<File, Failure> {
    let parent = parent(path, context)?;
    let before = parent.stat()?;
    if before.st_mode & libc::S_IFMT != libc::S_IFREG {
        return Err(Failure::new(
            ErrorCode::Unsupported,
            "Content requires a regular non-link file.",
        ));
    }
    let filesystem = filesystem(&parent.directory)?;
    let name: Vec<_> = filesystem
        .f_fstypename
        .iter()
        .map(|byte| *byte as u8)
        .collect();
    let name = crate::policy::native_field(&name)?;
    if matches!(
        name.as_slice(),
        b"devfs" | b"fdesc" | b"autofs" | b"procfs" | b"kernfs"
    ) {
        return Err(Failure::new(
            ErrorCode::Unsupported,
            "Pseudo-filesystem content has no supported known-length contract.",
        ));
    }
    cancel(context)?;
    let file = open_checked(&parent, &before)?;
    cancel(context)?;
    Ok(file)
}

fn open_checked(parent: &Parent, before: &libc::stat) -> Result<File, Failure> {
    // Darwin has no Linux O_PATH + /proc/self/fd reopen contract. A pathname
    // replacement may enter openat, but nonblocking/no-controlling-terminal flags
    // prevent FIFO waits/terminal acquisition. No bytes are read before fstat
    // establishes the same regular inode. This is not an atomic namespace snapshot.
    let file = parent.open(libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK)?;
    let after = stat_file(&file)?;
    if after.st_mode & libc::S_IFMT != libc::S_IFREG
        || (before.st_dev, before.st_ino, before.st_gen)
            != (after.st_dev, after.st_ino, after.st_gen)
    {
        return Err(Failure::new(
            ErrorCode::SourceChanged,
            "Native object changed during capture.",
        ));
    }
    Ok(file)
}

pub(crate) fn path_filesystem(
    path: &Path,
    context: &CallContext<'_>,
) -> Result<libc::statfs, Failure> {
    let parent = parent(path, context)?;
    let value = parent.stat()?;
    let filesystem = if value.st_mode & libc::S_IFMT == libc::S_IFDIR {
        filesystem(&parent.open(libc::O_RDONLY | libc::O_DIRECTORY)?)?
    } else {
        if value.st_dev != stat_file(&parent.directory)?.st_dev {
            return Err(Failure::new(
                ErrorCode::Unsupported,
                "Entry and parent filesystem differ.",
            ));
        }
        filesystem(&parent.directory)?
    };
    cancel(context)?;
    Ok(filesystem)
}

#[cfg(test)]
#[path = "system_tests.rs"]
mod tests;
