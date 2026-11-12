//! Windows native boundary: raw UTF-16 paths, handle identity and restricted DLL search.
use crate::{Error, api::abi};
use std::{
    fmt,
    fs::{File, OpenOptions},
    io,
    os::windows::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawHandle,
    },
    path::Path,
};
use windows_sys::Win32::{
    Foundation::{FreeLibrary, HMODULE},
    Storage::FileSystem::*,
    System::LibraryLoader::*,
};
#[path = "native_windows_paths.rs"]
mod paths;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Snapshot {
    pub length: u64,
    volume: u64,
    id: [u8; 16],
    write: i64,
    change: i64,
}

impl Snapshot {
    pub(crate) fn version_bytes(&self) -> Vec<u8> {
        let mut bytes = b"windows-file-v1".to_vec();
        bytes.extend(self.volume.to_le_bytes());
        bytes.extend(self.id);
        bytes.extend(self.length.to_le_bytes());
        bytes.extend(self.write.to_le_bytes());
        bytes.extend(self.change.to_le_bytes());
        bytes
    }

    /// Format only the differing values from already captured handle snapshots.
    pub(crate) fn changed_fields(&self, after: &Self) -> String {
        let mut fields = Vec::new();
        if self.length != after.length {
            fields.push(format!(
                "length(before={}, after={})",
                self.length, after.length,
            ));
        }
        if self.volume != after.volume {
            fields.push(format!(
                "volume(before={}, after={})",
                self.volume, after.volume,
            ));
        }
        if self.id != after.id {
            fields.push(format!(
                "file_id(before={}, after={})",
                DiagnosticFileId(&self.id),
                DiagnosticFileId(&after.id),
            ));
        }
        if self.write != after.write {
            fields.push(format!(
                "LastWriteTime(before={}, after={})",
                self.write, after.write,
            ));
        }
        if self.change != after.change {
            fields.push(format!(
                "ChangeTime(before={}, after={})",
                self.change, after.change,
            ));
        }
        fields.join(", ")
    }
}

struct DiagnosticFileId<'a>(&'a [u8; 16]);

impl fmt::Display for DiagnosticFileId<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

pub(crate) fn open_read(path: &Path) -> io::Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(paths::literal(path)?)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Viewer needs a regular non-reparse file",
        ));
    }
    Ok(file)
}

pub(crate) fn snapshot(file: &File) -> io::Result<Snapshot> {
    let mut id = FILE_ID_INFO::default();
    let mut basic = FILE_BASIC_INFO::default();
    // SAFETY: file owns the live handle and each output is writable for its exact length.
    unsafe {
        if GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileIdInfo,
            (&mut id as *mut FILE_ID_INFO).cast(),
            size_of::<FILE_ID_INFO>() as u32,
        ) == 0
            || GetFileInformationByHandleEx(
                file.as_raw_handle(),
                FileBasicInfo,
                (&mut basic as *mut FILE_BASIC_INFO).cast(),
                size_of::<FILE_BASIC_INFO>() as u32,
            ) == 0
        {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(Snapshot {
        length: file.metadata()?.len(),
        volume: id.VolumeSerialNumber,
        id: id.FileId.Identifier,
        write: basic.LastWriteTime,
        change: basic.ChangeTime,
    })
}

pub(crate) struct Library(HMODULE);
// SAFETY: module handle may be used on any backend thread; Arc ownership prevents unload.
unsafe impl Send for Library {}
unsafe impl Sync for Library {}
impl Library {
    pub(crate) fn symbol(
        &self,
        name: &std::ffi::CStr,
    ) -> Result<std::ptr::NonNull<std::ffi::c_void>, Error> {
        // SAFETY: retained module and borrowed NUL-terminated symbol name.
        let function = unsafe { GetProcAddress(self.0, name.as_ptr().cast()) }
            .ok_or_else(|| Error(format!("Missing plugin export {}", name.to_string_lossy())))?;
        std::ptr::NonNull::new(function as *mut std::ffi::c_void)
            .ok_or_else(|| Error("Null plugin export".into()))
    }
    pub(crate) fn owns_address(&self, address: *const std::ffi::c_void) -> bool {
        let mut module = std::ptr::null_mut();
        // SAFETY: FROM_ADDRESS treats the argument as an address, not a string;
        // UNCHANGED_REFCOUNT does not transfer ownership of the returned module.
        unsafe {
            GetModuleHandleExW(
                GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS
                    | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
                address.cast(),
                &mut module,
            ) != 0
                && module == self.0
        }
    }
    pub(crate) fn open(path: &Path) -> Result<Self, Error> {
        let wide: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        if wide[..wide.len() - 1].contains(&0) {
            return Err(Error("NUL in library path".into()));
        }
        // SAFETY: absolute validated path, terminated UTF-16, no application/CWD/PATH search.
        let handle = unsafe {
            LoadLibraryExW(
                wide.as_ptr(),
                std::ptr::null_mut(),
                LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
            )
        };
        if handle.is_null() {
            return Err(io::Error::last_os_error().into());
        }
        Ok(Self(handle))
    }
    pub(crate) fn entry(&self) -> Result<abi::Entry, Error> {
        // SAFETY: live module and static terminated export name; ABI dictates the signature.
        let entry = unsafe { GetProcAddress(self.0, c"midnight_plugin_entry_v1".as_ptr().cast()) };
        entry
            .map(|function| unsafe {
                std::mem::transmute::<unsafe extern "system" fn() -> isize, abi::Entry>(function)
            })
            .ok_or_else(|| Error("Missing midnight_plugin_entry_v1 export".into()))
    }
}
impl Drop for Library {
    fn drop(&mut self) {
        // SAFETY: unique owned loader reference, all callbacks quiesce before last Arc drop.
        unsafe {
            FreeLibrary(self.0);
        }
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::Snapshot;

    pub(crate) fn snapshot(
        length: u64,
        volume: u64,
        id: [u8; 16],
        write: i64,
        change: i64,
    ) -> Snapshot {
        Snapshot {
            length,
            volume,
            id,
            write,
            change,
        }
    }
}
