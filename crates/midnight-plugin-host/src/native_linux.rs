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
impl Snapshot {
    pub(crate) fn version_bytes(&self) -> Vec<u8> {
        let mut bytes = b"linux-file-v1".to_vec();
        for value in [self.device, self.inode, self.length] {
            bytes.extend(value.to_le_bytes());
        }
        for value in [self.mtime, self.mtime_ns, self.ctime, self.ctime_ns] {
            bytes.extend(value.to_le_bytes());
        }
        bytes
    }
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
pub(crate) struct Library(*mut std::ffi::c_void, std::path::PathBuf);
// SAFETY: dlopen handles are backend-thread independent; Arc lifetime pins their code.
unsafe impl Send for Library {}
unsafe impl Sync for Library {}
impl Library {
    pub(crate) fn symbol(
        &self,
        name: &std::ffi::CStr,
    ) -> Result<std::ptr::NonNull<std::ffi::c_void>, Error> {
        // SAFETY: retained module and borrowed NUL-terminated symbol name.
        std::ptr::NonNull::new(unsafe { libc::dlsym(self.0, name.as_ptr()) })
            .ok_or_else(|| Error(format!("Missing plugin export {}", name.to_string_lossy())))
    }
    pub(crate) fn owns_address(&self, address: *const std::ffi::c_void) -> bool {
        // SAFETY: Dl_info contains nullable pointers; dladdr only inspects the
        // supplied address and writes the exact output object.
        let mut info: libc::Dl_info = unsafe { std::mem::zeroed() };
        if unsafe { libc::dladdr(address, &mut info) } == 0 || info.dli_fname.is_null() {
            return false;
        }
        // SAFETY: successful dladdr owns this immutable NUL-terminated module name
        // while our loader reference remains alive.
        let bytes = unsafe { std::ffi::CStr::from_ptr(info.dli_fname) }.to_bytes();
        let path = Path::new(std::ffi::OsStr::from_bytes(bytes));
        path.canonicalize().is_ok_and(|path| path == self.1)
    }
    pub(crate) fn open(path: &Path) -> Result<Self, Error> {
        let native_path = path.to_path_buf();
        let path = CString::new(path.as_os_str().as_bytes())
            .map_err(|_| Error("NUL in library path".into()))?;
        // SAFETY: validated absolute terminated native path; no global symbol export.
        let handle = unsafe { libc::dlopen(path.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
        if handle.is_null() {
            return Err(Error("dlopen failed for viewer library".into()));
        }
        Ok(Self(handle, native_path))
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
