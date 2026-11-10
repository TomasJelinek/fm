use crate::{adapter, system};
use midnight_filesystem_plugin_api::*;
use midnight_filesystem_plugin_sdk::{CallContext, native::NativeEntry};
use std::{
    ffi::{CStr, OsString},
    os::{
        fd::{AsRawFd, IntoRawFd},
        unix::ffi::OsStringExt,
    },
    path::Path,
};

pub struct Directory {
    handle: *mut libc::DIR,
}
// SAFETY: ownership moves between workers; each operation requires exclusive &mut
// access and the SDK serializes a cursor. No pointer borrowed from readdir escapes.
unsafe impl Send for Directory {}

impl Directory {
    pub(crate) fn open(path: &Path, context: &CallContext<'_>) -> Result<Self, Failure> {
        let file = system::open_directory(path, context)?;
        // SAFETY: live directory descriptor. Ownership transfers only on success.
        let handle = unsafe { libc::fdopendir(file.as_raw_fd()) };
        if handle.is_null() {
            return Err(system::os_error());
        }
        let _ = file.into_raw_fd();
        Ok(Self { handle })
    }
    pub(crate) fn next(
        &mut self,
        context: &CallContext<'_>,
    ) -> Result<Option<NativeEntry>, Failure> {
        loop {
            system::cancel(context)?;
            // SAFETY: this thread's errno and our exclusively owned live DIR.
            let pointer = unsafe {
                *libc::__error() = 0;
                libc::readdir(self.handle)
            };
            if pointer.is_null() {
                // SAFETY: thread-local errno is inspected immediately after readdir.
                return if unsafe { *libc::__error() } == 0 {
                    Ok(None)
                } else {
                    Err(system::os_error())
                };
            }
            // SAFETY: readdir's terminated name lives until the next call on this DIR.
            let name = unsafe { CStr::from_ptr((*pointer).d_name.as_ptr()) };
            if matches!(name.to_bytes(), b"." | b"..") {
                continue;
            }
            // SAFETY: a live DIR owns its descriptor throughout the metadata query.
            let value = system::stat_at(unsafe { libc::dirfd(self.handle) }, name)?;
            system::cancel(context)?;
            return Ok(Some(NativeEntry {
                name: OsString::from_vec(name.to_bytes().to_vec()),
                info: adapter::info(&value),
            }));
        }
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        // SAFETY: this owner acquired DIR after successful fdopendir and releases it once.
        unsafe { libc::closedir(self.handle) };
    }
}
