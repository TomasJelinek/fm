//! Linux native boundary: byte paths, no-follow regular files, absolute RTLD_LOCAL modules.
use crate::{Error, api::abi};
use std::{
    ffi::CString,
    fs::{File, OpenOptions},
    io,
    os::unix::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt},
    },
    path::Path,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Snapshot {
    pub length: u64,
    device: u64,
    inode: u64,
    mtime: i64,
    mtime_ns: i64,
    ctime: i64,
    ctime_ns: i64,
}
pub(crate) fn open_read(path: &Path) -> io::Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Viewer needs a regular file",
        ));
    }
    Ok(file)
}
pub(crate) fn snapshot(file: &File) -> io::Result<Snapshot> {
    let m = file.metadata()?;
    Ok(Snapshot {
        length: m.len(),
        device: m.dev(),
        inode: m.ino(),
        mtime: m.mtime(),
        mtime_ns: m.mtime_nsec(),
        ctime: m.ctime(),
        ctime_ns: m.ctime_nsec(),
    })
}
pub(crate) struct Library(*mut std::ffi::c_void);
// SAFETY: dlopen handles are backend-thread independent; Arc lifetime pins their code.
unsafe impl Send for Library {}
unsafe impl Sync for Library {}
impl Library {
    pub(crate) fn open(path: &Path) -> Result<Self, Error> {
        let path = CString::new(path.as_os_str().as_bytes())
            .map_err(|_| Error("NUL in library path".into()))?;
        // SAFETY: validated absolute terminated native path; no global symbol export.
        let handle = unsafe { libc::dlopen(path.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
        if handle.is_null() {
            return Err(Error("dlopen failed for viewer library".into()));
        }
        Ok(Self(handle))
    }
    pub(crate) fn entry(&self) -> Result<abi::Entry, Error> {
        // SAFETY: live module; fixed export is required to implement the public C signature.
        let pointer = unsafe { libc::dlsym(self.0, c"midnight_plugin_entry_v1".as_ptr()) };
        if pointer.is_null() {
            return Err(Error("Missing midnight_plugin_entry_v1 export".into()));
        }
        Ok(unsafe { std::mem::transmute::<*mut std::ffi::c_void, abi::Entry>(pointer) })
    }
}
impl Drop for Library {
    fn drop(&mut self) {
        // SAFETY: unique owned loader reference; instance/jobs/buffers already quiescent.
        unsafe {
            libc::dlclose(self.0);
        }
    }
}
