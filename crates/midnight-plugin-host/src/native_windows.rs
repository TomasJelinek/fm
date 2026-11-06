//! Windows native boundary: raw UTF-16 paths, handle identity and restricted DLL search.
use crate::{Error, api::abi};
use std::{
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Snapshot {
    pub length: u64,
    volume: u64,
    id: [u8; 16],
    write: i64,
    change: i64,
}

pub(crate) fn open_read(path: &Path) -> io::Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
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
