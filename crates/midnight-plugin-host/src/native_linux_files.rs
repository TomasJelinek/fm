//! Metadata-only qualification precedes every regular-file content open.
use std::{
    ffi::CString,
    fs::File,
    io,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{ffi::OsStrExt, fs::MetadataExt},
    },
    path::Path,
};

fn owned(fd: i32) -> io::Result<File> {
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: only a successful native open transfers one new owned descriptor.
    Ok(unsafe { File::from_raw_fd(fd) })
}

fn probe(path: &Path) -> io::Result<File> {
    if !path.is_absolute() {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    // SAFETY: terminated static root, no creation mode and one newly owned descriptor.
    let mut directory = owned(unsafe {
        libc::open(
            c"/".as_ptr(),
            libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC,
        )
    })?;
    let bytes = path.as_os_str().as_bytes();
    if bytes.contains(&0) {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    // Raw components retain kernel dot/dotdot semantics. A single whole-path
    // open would impose PATH_MAX on otherwise valid descriptor-relative paths.
    let mut components = bytes
        .split(|byte| *byte == b'/')
        .filter(|component| !component.is_empty())
        .peekable();
    while let Some(component) = components.next() {
        let name = CString::new(component).map_err(|_| io::ErrorKind::InvalidInput)?;
        let flags = if components.peek().is_none() {
            libc::O_PATH
                | libc::O_NOFOLLOW
                | if bytes.ends_with(b"/") {
                    libc::O_DIRECTORY
                } else {
                    0
                }
        } else {
            libc::O_PATH | libc::O_DIRECTORY
        };
        // SAFETY: held parent, terminated component, no O_CREAT; intermediate
        // navigation follows ordinary directory links, final links stay unopened.
        directory = owned(unsafe {
            libc::openat(
                directory.as_raw_fd(),
                name.as_ptr(),
                flags | libc::O_CLOEXEC,
            )
        })?;
    }
    Ok(directory)
}

fn reopen(probe: &File) -> io::Result<File> {
    let before = probe.metadata()?;
    if !before.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Viewer needs a regular file",
        ));
    }
    // This internal link names our live O_PATH inode, not the mutable caller's
    // pathname. A substituted device/FIFO must never receive a data open.
    let path = CString::new(format!("/proc/self/fd/{}", probe.as_raw_fd()))
        .map_err(|_| io::ErrorKind::InvalidInput)?;
    // SAFETY: the retained regular descriptor owns the target; no creation mode.
    let file = owned(unsafe {
        libc::open(
            path.as_ptr(),
            libc::O_RDONLY | libc::O_NONBLOCK | libc::O_CLOEXEC,
        )
    })?;
    let after = file.metadata()?;
    if !after.is_file() || (before.dev(), before.ino()) != (after.dev(), after.ino()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Native content identity changed while opening",
        ));
    }
    Ok(file)
}

pub(crate) fn open_read(path: &Path) -> io::Result<File> {
    reopen(&probe(path)?)
}

#[cfg(test)]
#[path = "native_linux_file_tests.rs"]
mod tests;
